//! The two doors of a node (ADR-0048). The PEER door speaks Noise: a validator proves its institution
//! before anything else is read, and every request is rate limited per session. The PUBLIC door is
//! plain request/response for clients and answers only what a client may ask, with caps per IP.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use teko_p2p::{ConnGuard, NetError, TokenBucket, accept};

use crate::net::{Caller, allowed, max_request};
use crate::node::{Shared, ack, handle};
use crate::wire::{self, Msg};

/// A session with no request for this long is closed (the peer dials again when it has something).
const IDLE: Duration = Duration::from_secs(300);
/// One request is read (and one reply written) within this.
const IO: Duration = Duration::from_secs(10);

pub(crate) fn peer_accept_loop(s: &Arc<Shared>, listener: &TcpListener) {
    while !s.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, from)) => {
                let net = &s.net;
                if net.is_banned_ip(from.ip()) {
                    continue;
                }
                let Some(guard) = net.peer_conns.try_acquire(from.ip()) else {
                    continue;
                };
                let s2 = s.clone();
                thread::spawn(move || peer_session(&s2, stream, from, guard));
            }
            Err(_) => thread::sleep(Duration::from_millis(15)),
        }
    }
}

fn peer_session(s: &Arc<Shared>, stream: TcpStream, from: SocketAddr, mut guard: ConnGuard) {
    let net = &s.net;
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let mut sess = match accept(stream, &net.identity, &**net) {
        Ok(x) => x,
        Err(_) => {
            // a handshake that fails is cheap for us and a probe or a mistake for them
            net.strike_ip(from.ip(), 5);
            return;
        }
    };
    let key = sess.peer_key().to_vec();
    if net.is_banned(&key) || !guard.bind_key(&key) {
        return;
    }
    let mut bucket = TokenBucket::new(500, 200, Instant::now());
    let mut last = Instant::now();
    loop {
        if s.stop.load(Ordering::SeqCst) || net.stop.load(Ordering::SeqCst) {
            return;
        }
        // wait for a request in one-second slices, so a stopped node lets go of its sessions
        let _ = sess.stream().set_read_timeout(Some(Duration::from_secs(1)));
        let mut one = [0u8; 1];
        match sess.stream().peek(&mut one) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if last.elapsed() > IDLE {
                    return;
                }
                continue;
            }
            Err(_) => return,
        }
        last = Instant::now();
        let (kind, body) = match sess.recv(&max_request, IO) {
            Ok(x) => x,
            Err(NetError::TooLarge | NetError::BadFrame) => {
                net.strike(&key, 20);
                return;
            }
            Err(_) => return,
        };
        let reply = if !bucket.try_take(1, Instant::now()) {
            net.strike(&key, 2);
            ack(false, "rate limited", 0)
        } else if !allowed(kind, &Caller::Peer(&key)) {
            net.strike(&key, 5);
            ack(false, "not a peer message", 0)
        } else {
            handle(s, &Msg { kind, body }, &Caller::Peer(&key))
        };
        if sess.send(reply.kind, &reply.body, IO).is_err() || net.is_banned(&key) {
            return;
        }
    }
}

pub(crate) fn public_accept_loop(s: &Arc<Shared>, listener: &TcpListener) {
    while !s.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, from)) => {
                let net = &s.net;
                if net.is_banned_ip(from.ip()) {
                    continue;
                }
                let Some(guard) = net.public_conns.try_acquire(from.ip()) else {
                    continue;
                };
                let s2 = s.clone();
                thread::spawn(move || {
                    let _guard = guard;
                    let _ = public_request(&s2, stream, from);
                });
            }
            Err(_) => thread::sleep(Duration::from_millis(15)),
        }
    }
}

fn public_request(
    s: &Arc<Shared>,
    mut stream: TcpStream,
    from: SocketAddr,
) -> Result<(), crate::error::NodeError> {
    stream.set_nonblocking(false)?;
    stream.set_write_timeout(Some(IO))?;
    let net = &s.net;
    if !net.public_allows(from.ip()) {
        return wire::write_msg(&mut stream, &ack(false, "rate limited", 0));
    }
    let req = match wire::read_msg_by(&mut stream, Instant::now() + IO) {
        Ok(r) => r,
        Err(e) => {
            net.strike_ip(from.ip(), 1);
            return Err(e);
        }
    };
    let reply = if allowed(req.kind, &Caller::Public) {
        handle(s, &req, &Caller::Public)
    } else {
        net.strike_ip(from.ip(), 2);
        ack(false, "not served on the public port", 0)
    };
    wire::write_msg(&mut stream, &reply)
}
