use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::error::NetError;

/// The whole handshake must finish in this time (anti slow-loris).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Plaintext bytes per Noise record (a Noise message is at most 65535 bytes, tag included).
const CHUNK: usize = 65_000;
/// Both ends re-key after this many records, in lockstep (forward secrecy over long sessions).
const REKEY_EVERY: u64 = 1 << 16;

/// Reads exactly `buf.len()` bytes before `deadline`, however slowly they come.
fn read_exact_by(
    stream: &mut TcpStream,
    buf: &mut [u8],
    deadline: Instant,
) -> Result<(), NetError> {
    let mut got = 0;
    while got < buf.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(NetError::Timeout);
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut buf[got..]) {
            Ok(0) => {
                return Err(NetError::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            Ok(n) => got += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(NetError::Timeout);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// One length-prefixed record (`u16` big endian + bytes), the unit of both the handshake and the
/// encrypted session.
pub(crate) fn read_record(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, NetError> {
    let mut len = [0u8; 2];
    read_exact_by(stream, &mut len, deadline)?;
    let mut buf = vec![0u8; u16::from_be_bytes(len) as usize];
    read_exact_by(stream, &mut buf, deadline)?;
    Ok(buf)
}

pub(crate) fn write_record(stream: &mut TcpStream, data: &[u8]) -> Result<(), NetError> {
    let len = u16::try_from(data.len()).map_err(|_| NetError::TooLarge)?;
    let mut out = Vec::with_capacity(2 + data.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    stream.write_all(&out)?;
    Ok(())
}

/// An established, authenticated and encrypted link to ONE validator. A logical message is a header
/// record (`kind`, body length) followed by the body in records of at most 65 000 bytes; the size is
/// checked against the limit of the `kind` BEFORE any body is buffered.
pub struct Session {
    stream: TcpStream,
    transport: snow::TransportState,
    peer: Vec<u8>,
    sent: u64,
    received: u64,
}

impl Session {
    pub(crate) fn new(stream: TcpStream, transport: snow::TransportState, peer: Vec<u8>) -> Self {
        Self {
            stream,
            transport,
            peer,
            sent: 0,
            received: 0,
        }
    }

    /// The peer's consensus key: who is on the other end.
    pub fn peer_key(&self) -> &[u8] {
        &self.peer
    }

    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    fn seal(&mut self, plain: &[u8]) -> Result<Vec<u8>, NetError> {
        let mut out = vec![0u8; plain.len() + 16];
        let n = self
            .transport
            .write_message(plain, &mut out)
            .map_err(|_| NetError::BadFrame)?;
        out.truncate(n);
        self.sent += 1;
        if self.sent.is_multiple_of(REKEY_EVERY) {
            self.transport.rekey_outgoing();
        }
        Ok(out)
    }

    fn open(&mut self, cipher: &[u8]) -> Result<Vec<u8>, NetError> {
        let mut out = vec![0u8; cipher.len()];
        let n = self
            .transport
            .read_message(cipher, &mut out)
            .map_err(|_| NetError::BadFrame)?;
        out.truncate(n);
        self.received += 1;
        if self.received.is_multiple_of(REKEY_EVERY) {
            self.transport.rekey_incoming();
        }
        Ok(out)
    }

    /// Seals one record without sending it (tests craft frames).
    #[cfg(test)]
    pub(crate) fn seal_for_test(&mut self, plain: &[u8]) -> Vec<u8> {
        self.seal(plain).unwrap_or_default()
    }

    /// Sends one message. `timeout` bounds the whole write.
    pub fn send(&mut self, kind: u32, body: &[u8], timeout: Duration) -> Result<(), NetError> {
        let len = u32::try_from(body.len()).map_err(|_| NetError::TooLarge)?;
        self.stream.set_write_timeout(Some(timeout))?;
        let mut head = Vec::with_capacity(8);
        head.extend_from_slice(&kind.to_be_bytes());
        head.extend_from_slice(&len.to_be_bytes());
        let rec = self.seal(&head)?;
        write_record(&mut self.stream, &rec)?;
        for chunk in body.chunks(CHUNK) {
            let rec = self.seal(chunk)?;
            write_record(&mut self.stream, &rec)?;
        }
        Ok(())
    }

    /// Receives one message within `timeout` (the WHOLE message, not each read). `max_for(kind)` is
    /// the largest body that kind may have; a larger announcement is refused before any body is read.
    pub fn recv(
        &mut self,
        max_for: &dyn Fn(u32) -> usize,
        timeout: Duration,
    ) -> Result<(u32, Vec<u8>), NetError> {
        let deadline = Instant::now() + timeout;
        let rec = read_record(&mut self.stream, deadline)?;
        let head = self.open(&rec)?;
        let (Some(k), Some(l)) = (head.get(..4), head.get(4..8)) else {
            return Err(NetError::BadFrame);
        };
        if head.len() != 8 {
            return Err(NetError::BadFrame);
        }
        let kind = u32::from_be_bytes([k[0], k[1], k[2], k[3]]);
        let len = u32::from_be_bytes([l[0], l[1], l[2], l[3]]) as usize;
        if len > max_for(kind) {
            return Err(NetError::TooLarge);
        }
        let mut body = Vec::with_capacity(len.min(1 << 20));
        while body.len() < len {
            let rec = read_record(&mut self.stream, deadline)?;
            let plain = self.open(&rec)?;
            if plain.is_empty() || body.len() + plain.len() > len {
                return Err(NetError::BadFrame);
            }
            body.extend_from_slice(&plain);
        }
        Ok((kind, body))
    }
}
