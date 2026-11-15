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

/// Intent kinds that a wallet owner or buyer signs (values match `teko-assets`).
pub const INTENT_TRANSFER: u8 = 2;
pub const INTENT_SWAP: u8 = 3;

/// Hash that an asset CONSENT signs: network, action, class, asset, parties, price and
/// the asset version. Shared by the device (which signs it) and `teko-assets` (which
/// verifies it), so a consent cannot be replayed on another chain, action or version.
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
