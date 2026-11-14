//! `Transport` abstraction between host and device. TCP loopback today
//! (ADR-0010); USB HID later with the same framing.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use teko_protocol::{CRC_LEN, Frame, HEADER_LEN, ProtocolError, parse_header};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("connection closed")]
    Closed,
}

pub trait Transport {
    fn write_frame(&mut self, frame: &Frame) -> Result<(), TransportError>;
    fn read_frame(&mut self) -> Result<Frame, TransportError>;
}

/// Frames over any byte stream.
pub struct StreamTransport<S: Read + Write> {
    stream: S,
}

pub type TcpTransport = StreamTransport<TcpStream>;

impl<S: Read + Write> StreamTransport<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub fn get_ref(&self) -> &S {
        &self.stream
    }
}

fn read_exact_or_closed<S: Read>(s: &mut S, buf: &mut [u8]) -> Result<(), TransportError> {
    s.read_exact(buf).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe => TransportError::Closed,
        _ => TransportError::Io(e),
    })
}

impl<S: Read + Write> Transport for StreamTransport<S> {
    fn write_frame(&mut self, frame: &Frame) -> Result<(), TransportError> {
        self.stream.write_all(&frame.encode())?;
        self.stream.flush()?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Frame, TransportError> {
        let mut header = [0u8; HEADER_LEN];
        read_exact_or_closed(&mut self.stream, &mut header)?;
        let (_, len) = parse_header(&header)?;
        let mut rest = vec![0u8; len + CRC_LEN];
        read_exact_or_closed(&mut self.stream, &mut rest)?;
        let mut all = Vec::with_capacity(HEADER_LEN + rest.len());
        all.extend_from_slice(&header);
        all.extend_from_slice(&rest);
        Ok(Frame::decode(&all)?)
    }
}

/// Connects to a device (emulator) listening on `addr`.
pub fn connect_tcp(
    addr: impl ToSocketAddrs,
    timeout: Duration,
) -> Result<TcpTransport, TransportError> {
    let mut last: Option<io::Error> = None;
    for a in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => {
                s.set_nodelay(true)?;
                return Ok(StreamTransport::new(s));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(TransportError::Io(last.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no address")
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::io::Cursor;

    struct Duplex {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }
    impl Read for Duplex {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.input.read(b)
        }
    }
    impl Write for Duplex {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn frame_roundtrip_over_stream() {
        let f = Frame::new(0, 2, 9, vec![1, 2, 3]);
        let mut t = StreamTransport::new(Duplex {
            input: Cursor::new(f.encode()),
            output: vec![],
        });
        assert_eq!(t.read_frame().unwrap(), f);
        t.write_frame(&f).unwrap();
        assert_eq!(t.get_ref().output, f.encode());
        assert!(matches!(t.read_frame(), Err(TransportError::Closed)));
    }

    #[test]
    fn garbage_is_a_protocol_error_not_a_panic() {
        let mut t = StreamTransport::new(Duplex {
            input: Cursor::new(vec![0xFF; 64]),
            output: vec![],
        });
        assert!(matches!(t.read_frame(), Err(TransportError::Protocol(_))));
    }
}
