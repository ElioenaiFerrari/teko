use alloc::vec::Vec;

use crate::crc::crc32;
use crate::error::ProtocolError;

pub const MAGIC: [u8; 2] = *b"TK";
pub const PROTO_VERSION: u8 = 1;
/// magic(2) ver(1) flags(1) type(2) request_id(4) payload_len(4)
pub const HEADER_LEN: usize = 14;
pub const CRC_LEN: usize = 4;
pub const MAX_PAYLOAD: usize = 64 * 1024;
/// Only firmware chunks may be larger.
pub const MAX_CHUNK_PAYLOAD: usize = 1024 * 1024;

/// Message type of `FirmwareChunk` (kept here to size-limit before decoding).
pub(crate) const FIRMWARE_CHUNK_TYPE: u16 = 0x0013;

const FLAG_RESPONSE: u8 = 0x01;
const FLAG_ERROR: u8 = 0x02;
const FLAGS_KNOWN: u8 = FLAG_RESPONSE | FLAG_ERROR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub flags: u8,
    pub msg_type: u16,
    pub request_id: u32,
    pub payload: Vec<u8>,
}

fn max_payload_for(msg_type: u16) -> usize {
    if msg_type == FIRMWARE_CHUNK_TYPE {
        MAX_CHUNK_PAYLOAD
    } else {
        MAX_PAYLOAD
    }
}

/// Validates a header and returns `(msg_type, payload_len)`.
/// Callers read exactly `payload_len + CRC_LEN` more bytes afterwards.
pub fn parse_header(header: &[u8]) -> Result<(u16, usize), ProtocolError> {
    let h: &[u8; HEADER_LEN] = header
        .get(..HEADER_LEN)
        .and_then(|s| s.try_into().ok())
        .ok_or(ProtocolError::Truncated)?;
    if h[0..2] != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    if h[2] != PROTO_VERSION {
        return Err(ProtocolError::UnsupportedVersion(h[2]));
    }
    if h[3] & !FLAGS_KNOWN != 0 {
        return Err(ProtocolError::ReservedFlags);
    }
    let msg_type = u16::from_be_bytes([h[4], h[5]]);
    let len = u32::from_be_bytes([h[10], h[11], h[12], h[13]]);
    if len as usize > max_payload_for(msg_type) {
        return Err(ProtocolError::PayloadTooLarge(len));
    }
    Ok((msg_type, len as usize))
}

impl Frame {
    pub fn new(flags: u8, msg_type: u16, request_id: u32, payload: Vec<u8>) -> Self {
        Self {
            flags,
            msg_type,
            request_id,
            payload,
        }
    }

    pub fn is_response(&self) -> bool {
        self.flags & FLAG_RESPONSE != 0
    }

    pub fn is_error(&self) -> bool {
        self.flags & FLAG_ERROR != 0
    }

    pub fn request_flags() -> u8 {
        0
    }

    pub fn response_flags(error: bool) -> u8 {
        FLAG_RESPONSE | if error { FLAG_ERROR } else { 0 }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len() + CRC_LEN);
        out.extend_from_slice(&MAGIC);
        out.push(PROTO_VERSION);
        out.push(self.flags);
        out.extend_from_slice(&self.msg_type.to_be_bytes());
        out.extend_from_slice(&self.request_id.to_be_bytes());
        out.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.payload);
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }

    /// Decodes a complete frame. Total: any byte string yields `Ok` or `Err`.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let (msg_type, len) = parse_header(bytes)?;
        let body_end = HEADER_LEN + len;
        if bytes.len() != body_end + CRC_LEN {
            return Err(ProtocolError::LengthMismatch);
        }
        let (body, crc_bytes) = bytes.split_at(body_end);
        let crc = u32::from_be_bytes(crc_bytes.try_into().map_err(|_| ProtocolError::Truncated)?);
        if crc != crc32(body) {
            return Err(ProtocolError::BadCrc);
        }
        let flags = body.get(3).copied().ok_or(ProtocolError::Truncated)?;
        let rid: [u8; 4] = body
            .get(6..10)
            .and_then(|s| s.try_into().ok())
            .ok_or(ProtocolError::Truncated)?;
        let payload = body.get(HEADER_LEN..).ok_or(ProtocolError::Truncated)?;
        Ok(Self {
            flags,
            msg_type,
            request_id: u32::from_be_bytes(rid),
            payload: payload.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn roundtrip(t in any::<u16>(), rid in any::<u32>(), p in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let f = Frame::new(0, t, rid, p);
            prop_assert_eq!(Frame::decode(&f.encode()).unwrap(), f);
        }

        #[test]
        fn decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = Frame::decode(&bytes);
        }

        #[test]
        fn single_bit_flip_is_detected(t in any::<u16>(), p in proptest::collection::vec(any::<u8>(), 0..64), bit in 0usize..4096) {
            let enc = Frame::new(0, t, 7, p).encode();
            let mut bad = enc.clone();
            let idx = bit % (bad.len() * 8);
            bad[idx / 8] ^= 1 << (idx % 8);
            prop_assert_ne!(Frame::decode(&bad).ok(), Frame::decode(&enc).ok());
        }
    }

    #[test]
    fn rejects_oversize_without_alloc() {
        let mut h = Vec::new();
        h.extend_from_slice(&MAGIC);
        h.push(PROTO_VERSION);
        h.push(0);
        h.extend_from_slice(&1u16.to_be_bytes());
        h.extend_from_slice(&0u32.to_be_bytes());
        h.extend_from_slice(&(u32::MAX).to_be_bytes());
        assert!(matches!(
            parse_header(&h),
            Err(ProtocolError::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn rejects_reserved_flags_and_bad_version() {
        let mut f = Frame::new(0, 1, 1, Vec::new()).encode();
        f[3] = 0x80;
        assert_eq!(Frame::decode(&f), Err(ProtocolError::ReservedFlags));
        let mut g = Frame::new(0, 1, 1, Vec::new()).encode();
        g[2] = 9;
        assert_eq!(Frame::decode(&g), Err(ProtocolError::UnsupportedVersion(9)));
    }
}
