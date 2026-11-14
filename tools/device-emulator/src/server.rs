use std::io;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use rand_core::{CryptoRng, RngCore};
use teko_device_core::{Device, Hal};
use teko_protocol::{ErrorCode, ProtocolError, Request, Response};
use teko_transport::{StreamTransport, Transport, TransportError};

use crate::link::{LinkSlot, Linked};

fn to_io(e: TransportError) -> io::Error {
    match e {
        TransportError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

/// A host that goes quiet (or stalls mid-frame) is dropped, so a stray local
/// process cannot hold the device's single "USB port" forever. The Suite polls
/// every 2 s, so a healthy connection never reaches this.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Serves ONE host connection until it closes. A new connection starts LOCKED
/// (like replugging a USB device): the PIN is needed again.
pub fn handle_connection<H: Hal, R: RngCore + CryptoRng>(
    dev: &mut Device<Linked<H>, R>,
    slot: &LinkSlot,
    stream: TcpStream,
) -> io::Result<()> {
    handle_connection_with(dev, slot, stream, IDLE_TIMEOUT)
}

pub fn handle_connection_with<H: Hal, R: RngCore + CryptoRng>(
    dev: &mut Device<Linked<H>, R>,
    slot: &LinkSlot,
    stream: TcpStream,
    idle: Duration,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(idle))?;
    dev.lock();
    let mut transport = StreamTransport::new(stream.try_clone()?);
    loop {
        let frame = match transport.read_frame() {
            Ok(f) => f,
            Err(TransportError::Closed) => break,
            Err(TransportError::Protocol(_)) => {
                // Stream is desynchronised: report and drop the connection.
                let r = Response::failure(ErrorCode::MalformedFrame, "frame inválido");
                let _ = transport.write_frame(&r.to_frame(0));
                break;
            }
            Err(TransportError::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                break; // idle or stalled: drop the connection
            }
            Err(e) => {
                dev.lock();
                return Err(to_io(e));
            }
        };
        let id = frame.request_id;
        let response = match Request::from_frame(&frame) {
            Ok(req) => {
                slot.set(stream.try_clone()?, id);
                let r = dev.handle(req);
                slot.clear();
                r
            }
            Err(ProtocolError::UnknownMessage(_)) => {
                Response::failure(ErrorCode::UnknownMessage, "mensagem desconhecida")
            }
            Err(_) => Response::failure(ErrorCode::MalformedFrame, "payload inválido"),
        };
        transport
            .write_frame(&response.to_frame(id))
            .map_err(to_io)?;
    }
    dev.lock();
    Ok(())
}

/// Accepts connections one at a time, forever (the device has one USB port).
pub fn serve<H: Hal, R: RngCore + CryptoRng>(
    listener: &TcpListener,
    dev: &mut Device<Linked<H>, R>,
    slot: &LinkSlot,
    mut on_connection: impl FnMut(&str),
) -> io::Result<()> {
    loop {
        let (stream, peer) = listener.accept()?;
        on_connection(&format!("conectado: {peer}"));
        let result = handle_connection(dev, slot, stream);
        on_connection(&match result {
            Ok(()) => "desconectado".to_string(),
            Err(e) => format!("conexão encerrada: {e}"),
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use rand::rngs::OsRng;
    use std::time::Instant;
    use teko_attestation::TrustPolicy;
    use teko_device_core::DeviceConfig;
    use teko_device_core::testing::ScriptedHal;

    #[test]
    fn idle_connection_is_dropped_and_the_port_is_freed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let slot = LinkSlot::default();
            let trust = TrustPolicy {
                authority_root: vec![2; 33],
                fw_signers: vec![vec![2; 33]],
                fw_threshold: 1,
                revoked_batches: vec![],
            };
            let mut dev = Device::new(
                Linked::new(ScriptedHal::new(), slot.clone()),
                OsRng,
                DeviceConfig::default(),
                trust,
                None,
            )
            .unwrap();
            for _ in 0..2 {
                let (s, _) = listener.accept().unwrap();
                let started = Instant::now();
                handle_connection_with(&mut dev, &slot, s, Duration::from_millis(200)).unwrap();
                tx.send(started.elapsed()).unwrap();
            }
        });
        // A silent client is dropped after the idle timeout...
        let _hog = TcpStream::connect(addr).unwrap();
        let waited = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            waited >= Duration::from_millis(150) && waited < Duration::from_secs(3),
            "{waited:?}"
        );
        // ...so the next client is served instead of being locked out.
        let _next = TcpStream::connect(addr).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    }
}
