//! Pieces of the ledger transaction format that the DEVICE must reproduce byte for
//! byte, so it can sign a transaction it described on its own screen (principle 1:
//! what is signed is derived inside the device). `teko-ledger` uses the same code.

use alloc::vec::Vec;

/// Module id of the coin in `Tx::module`.
pub const MODULE_COIN: u32 = 1;

/// Canonical bytes covered by a ledger transaction signature (everything but the
/// signature): length-prefixed chain id, nonce, fee, module, payload and sender key.
pub fn tx_signing_bytes(
    chain_id: &str,
    nonce: u64,
    fee: u64,
    module: u32,
    payload: &[u8],
    sender: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    for part in [
        chain_id.as_bytes(),
        &nonce.to_be_bytes(),
        &fee.to_be_bytes(),
        &module.to_be_bytes(),
        payload,
        sender,
    ] {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}

/// Payload of a coin `Transfer` (op 0 ‖ 32-byte destination ‖ amount, big endian).
pub fn coin_transfer_payload(to: &[u8; 32], amount: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(41);
    p.push(0);
    p.extend_from_slice(to);
    p.extend_from_slice(&amount.to_be_bytes());
    p
}

/// Coin staking actions (values match the coin module op codes).
pub const STAKE_BOND: u8 = 1;
pub const STAKE_UNBOND: u8 = 2;
pub const STAKE_CLAIM: u8 = 3;

/// Payload of a coin `Stake` / `Unstake` / `Claim` operation (`Claim` carries no amount).
pub fn coin_stake_payload(action: u8, amount: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(9);
    p.push(action);
    if action != STAKE_CLAIM {
        p.extend_from_slice(&amount.to_be_bytes());
    }
    p
}

/// Module id of the assets module in `Tx::module`.
pub const MODULE_ASSETS: u32 = 2;

/// Module id of the credentials module in `Tx::module` (ADR-0037).
pub const MODULE_CREDENTIALS: u32 = 3;

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// A protobuf `bytes` field (wire type 2); an EMPTY value is omitted, as `prost` does.
fn put_bytes(out: &mut Vec<u8>, tag: u8, v: &[u8]) {
    if !v.is_empty() {
        out.push(tag << 3 | 2);
        put_varint(out, v.len() as u64);
        out.extend_from_slice(v);
    }
}

/// A protobuf `uint64` field (wire type 0); ZERO is omitted, as `prost` does.
fn put_u64(out: &mut Vec<u8>, tag: u8, v: u64) {
    if v != 0 {
        out.push(tag << 3);
        put_varint(out, v);
    }
}

/// Payload of an assets `Offer` (op 7 ‖ protobuf `OfferOp`): the owner offers `asset` to `buyer`
/// at `price`, bound to the asset `version` and its `metadata_hash` (ADR-0036). The DEVICE builds
/// this from the fields it shows, byte for byte what `teko-assets` encodes.
pub fn asset_offer_payload(
    class: &[u8],
    asset: &[u8],
    buyer: &[u8; 32],
    price: u64,
    version: u64,
    metadata_hash: &[u8; 32],
) -> Vec<u8> {
    let mut p = Vec::from([7u8]);
    put_bytes(&mut p, 1, class);
    put_bytes(&mut p, 2, asset);
    put_bytes(&mut p, 3, buyer);
    put_u64(&mut p, 4, price);
    put_u64(&mut p, 5, version);
    put_bytes(&mut p, 6, metadata_hash);
    p
}

/// Payload of an assets `Accept` (op 8 ‖ protobuf `AcceptOp`): the buyer accepts the offer on
/// exactly the terms it saw.
pub fn asset_accept_payload(
    class: &[u8],
    asset: &[u8],
    price: u64,
    version: u64,
    metadata_hash: &[u8; 32],
) -> Vec<u8> {
    let mut p = Vec::from([8u8]);
    put_bytes(&mut p, 1, class);
    put_bytes(&mut p, 2, asset);
    put_u64(&mut p, 3, price);
    put_u64(&mut p, 4, version);
    put_bytes(&mut p, 5, metadata_hash);
    p
}

/// Op bytes of the credentials module (`teko-credentials`), shared with the device.
pub const CRED_OP_PROPOSE: u8 = 1;
pub const CRED_OP_APPROVE: u8 = 2;
pub const CRED_OP_VETO: u8 = 3;
pub const CRED_OP_REVOKE: u8 = 4;

/// A protobuf `string` field has the same wire format as `bytes`.
fn put_str(out: &mut Vec<u8>, tag: u8, v: &str) {
    put_bytes(out, tag, v.as_bytes());
}

/// Payload of a credentials `Propose` (op 1 ‖ protobuf `ProposeOp`), byte for byte what
/// `teko-credentials` encodes (ADR-0037). The DEVICE builds it from the fields it shows.
pub fn cred_propose_payload(
    ctype: &[u8],
    subject: &[u8],
    commitment: &[u8; 32],
    claims: &[u8],
    valid_until: u64,
) -> Vec<u8> {
    cred_propose_payload_sealed(ctype, subject, commitment, claims, valid_until, &[])
}

/// Like `cred_propose_payload`, with the attributes SEALED to the holder (ADR-0039): the issuing
/// device seals `salt ‖ claims` to the holder's credential key and the ciphertext rides on the
/// proposal, so the holder can recover the attributes from the chain after restoring the wallet.
pub fn cred_propose_payload_sealed(
    ctype: &[u8],
    subject: &[u8],
    commitment: &[u8; 32],
    claims: &[u8],
    valid_until: u64,
    sealed: &[u8],
) -> Vec<u8> {
    let mut p = Vec::from([CRED_OP_PROPOSE]);
    put_bytes(&mut p, 1, ctype);
    put_bytes(&mut p, 2, subject);
    put_bytes(&mut p, 3, commitment);
    put_bytes(&mut p, 4, claims);
    put_u64(&mut p, 5, valid_until);
    put_bytes(&mut p, 6, sealed);
    p
}

/// Payload of a credentials `Approve`, `Veto` or `Revoke` (`op` is one of the `CRED_OP_*` bytes
/// 2..=4 ‖ protobuf `VoteOp`).
pub fn cred_vote_payload(op: u8, ctype: &[u8], id: &[u8; 32], reason: &str) -> Vec<u8> {
    let mut p = Vec::from([op]);
    put_bytes(&mut p, 1, ctype);
    put_bytes(&mut p, 2, id);
    put_str(&mut p, 3, reason);
    p
}

/// Identifier of a credential: a hash of everything the organ proposed.
pub fn cred_id(
    ctype: &[u8],
    subject: &[u8],
    commitment: &[u8],
    claims: &[u8],
    valid_until: u64,
) -> [u8; 32] {
    crate::keys::tagged_hash(
        crate::tags::CRED_ID,
        &[
            ctype,
            subject,
            commitment,
            claims,
            &valid_until.to_be_bytes(),
        ],
    )
}

/// Commitment of a PUBLIC credential: a hash of its claims in clear.
pub fn cred_public_commitment(claims: &[u8]) -> [u8; 32] {
    crate::keys::tagged_hash(crate::tags::CRED_CLAIMS, &[claims])
}

/// Commitment of a PRIVATE credential (ADR-0038): a hash of the SORTED per-claim digests, each
/// salted, so the holder can later disclose ONE claim (for example `age_over_18`) without the
/// others. The holder keeps the claims and the salt; the chain only the commitment.
pub fn cred_private_commitment(salt: &[u8; 16], claims: &[u8]) -> [u8; 32] {
    crate::civic::commitment_of(&crate::civic::claim_list(salt, claims))
}

/// Hash that a registrar QUORUM approval signs: network, action, class, asset, parties, price and
/// the asset version, so an approval cannot be replayed on another chain, action or version.
#[allow(clippy::too_many_arguments)]
pub fn asset_intent_hash(
    chain_id: &str,
    kind: u8,
    class: &[u8],
    asset: &[u8],
    from: &[u8; 32],
    to: &[u8; 32],
    amount: u64,
    version: u64,
    extra: &[u8],
) -> [u8; 32] {
    crate::tagged_hash(
        crate::tags::ASSET_INTENT,
        &[
            chain_id.as_bytes(),
            &[kind],
            class,
            asset,
            from,
            to,
            &amount.to_be_bytes(),
            &version.to_be_bytes(),
            extra,
        ],
    )
}
