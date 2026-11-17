use std::net::TcpStream;
use std::time::Instant;

use prost::Message;

use crate::error::NetError;
use crate::identity::{IdentityProof, NOISE_PARAMS, NetIdentity, PeerPolicy};
use crate::session::{HANDSHAKE_TIMEOUT, Session, read_record, write_record};

const ROLE_INITIATOR: u8 = 1;
const ROLE_RESPONDER: u8 = 2;

fn builder(id: &NetIdentity) -> Result<snow::Builder<'_>, NetError> {
    let params = NOISE_PARAMS
        .parse()
        .map_err(|_| NetError::Handshake("noise parameters".into()))?;
    snow::Builder::new(params)
        .local_private_key(&id.noise_private)
        .map_err(|e| NetError::Handshake(e.to_string()))
}

fn noise(e: snow::Error) -> NetError {
    NetError::Handshake(e.to_string())
}

/// Dials: Noise XX, the responder proves its institution in message 2 and we prove ours in message 3.
/// `expect` is the consensus key we meant to reach (a mismatch is refused); `policy` must accept it.
pub fn connect(
    stream: TcpStream,
    id: &NetIdentity,
    policy: &dyn PeerPolicy,
    expect: Option<&[u8]>,
) -> Result<Session, NetError> {
    connect_with(stream, id, policy, expect, id.proof(ROLE_INITIATOR))
}

/// `connect` with the proof to send (tests send forged ones).
pub(crate) fn connect_with(
    mut stream: TcpStream,
    id: &NetIdentity,
    policy: &dyn PeerPolicy,
    expect: Option<&[u8]>,
    ours: IdentityProof,
) -> Result<Session, NetError> {
    stream.set_nodelay(true)?;
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut hs = builder(id)?.build_initiator().map_err(noise)?;
    let mut buf = vec![0u8; 65_535];
    // -> e
    let n = hs.write_message(&[], &mut buf).map_err(noise)?;
    write_record(&mut stream, &buf[..n])?;
    // <- e, ee, s, es + the responder's proof
    let m2 = read_record(&mut stream, deadline)?;
    let mut payload = vec![0u8; 65_535];
    let n = hs.read_message(&m2, &mut payload).map_err(noise)?;
    let remote = hs
        .get_remote_static()
        .ok_or_else(|| NetError::Handshake("no remote static key".into()))?
        .to_vec();
    let peer = id.check_proof(&payload[..n], &remote, ROLE_RESPONDER, policy, expect)?;
    // -> s, se + our proof
    let proof = ours.encode_to_vec();
    let n = hs.write_message(&proof, &mut buf).map_err(noise)?;
    write_record(&mut stream, &buf[..n])?;
    let transport = hs.into_transport_mode().map_err(noise)?;
    Ok(Session::new(stream, transport, peer))
}

/// Accepts: the mirror of `connect`. Nothing but the handshake is read from a peer that has not proven
/// a key the policy accepts.
pub fn accept(
    mut stream: TcpStream,
    id: &NetIdentity,
    policy: &dyn PeerPolicy,
) -> Result<Session, NetError> {
    stream.set_nodelay(true)?;
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut hs = builder(id)?.build_responder().map_err(noise)?;
    let mut buf = vec![0u8; 65_535];
    let mut payload = vec![0u8; 65_535];
    // <- e
    let m1 = read_record(&mut stream, deadline)?;
    hs.read_message(&m1, &mut payload).map_err(noise)?;
    // -> e, ee, s, es + our proof
    let proof = id.proof(ROLE_RESPONDER).encode_to_vec();
    let n = hs.write_message(&proof, &mut buf).map_err(noise)?;
    write_record(&mut stream, &buf[..n])?;
    // <- s, se + the initiator's proof
    let m3 = read_record(&mut stream, deadline)?;
    let n = hs.read_message(&m3, &mut payload).map_err(noise)?;
    let remote = hs
        .get_remote_static()
        .ok_or_else(|| NetError::Handshake("no remote static key".into()))?
        .to_vec();
    let peer = id.check_proof(&payload[..n], &remote, ROLE_INITIATOR, policy, None)?;
    let transport = hs.into_transport_mode().map_err(noise)?;
    Ok(Session::new(stream, transport, peer))
}
