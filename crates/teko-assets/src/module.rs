use prost::Message;
use teko_coin::{COIN_MODULE, Op as CoinOp};
use teko_crypto::{tagged_hash, tags, verify_tagged};
use teko_ledger::{Address, Ctx, Ledger, Module, ModuleError};

use crate::types::{
    AcceptOp, Approval, AssetRec, CancelOp, ClassRec, FinalizeOp, Intent, IntentKind, Lien,
    OfferOp, OfferRec, Op, RegisterOp, RegistrarsOp, VetoOp, intent_hash,
};

/// Module id of the assets module in `Tx::module`.
pub const ASSETS_MODULE: u32 = 2;

const MAX_CLASS: usize = 32;
const MAX_ASSET: usize = 64;
/// Length of the metadata hash stored with every asset.
pub const META_HASH_LEN: usize = 32;
const MAX_APPROVALS: usize = 32;
const MAX_REGISTRARS: usize = 16;
const MAX_LIENS: usize = 16;
const MAX_LIEN_KIND: usize = 32;
const ZERO: Address = [0; 32];
/// Veto reasons are public and short.
const MAX_REASON: usize = 200;
/// Default veto window of a class created without one (20 minutes).
pub const DEFAULT_WINDOW_SECS: u64 = 1_200;

pub(crate) fn k_class(class: &[u8]) -> Vec<u8> {
    [b"c:".as_slice(), class].concat()
}
pub(crate) fn k_asset(class: &[u8], id: &[u8]) -> Vec<u8> {
    let mut k = b"a:".to_vec();
    k.push(class.len() as u8);
    k.extend_from_slice(class);
    k.extend_from_slice(id);
    k
}
pub(crate) fn k_offer(class: &[u8], id: &[u8]) -> Vec<u8> {
    let mut k = b"f:".to_vec();
    k.push(class.len() as u8);
    k.extend_from_slice(class);
    k.extend_from_slice(id);
    k
}
pub(crate) fn k_owner(owner: &[u8], class: &[u8], id: &[u8]) -> Vec<u8> {
    let mut k = b"o:".to_vec();
    k.extend_from_slice(owner);
    k.push(class.len() as u8);
    k.extend_from_slice(class);
    k.extend_from_slice(id);
    k
}

fn err(m: &str) -> ModuleError {
    ModuleError::new(m)
}

/// Creates a class at genesis, governed by `registrars` with `threshold`-of-N and the default
/// veto window (20 minutes).
pub fn genesis_class(
    ledger: &mut Ledger,
    class: &[u8],
    registrars: &[Vec<u8>],
    threshold: u32,
) -> Result<(), String> {
    genesis_class_with_window(ledger, class, registrars, threshold, DEFAULT_WINDOW_SECS)
}

/// Like `genesis_class`, with the class's own veto window in seconds (must be positive: a zero
/// window would let anybody finalize at once and the organ could never veto).
pub fn genesis_class_with_window(
    ledger: &mut Ledger,
    class: &[u8],
    registrars: &[Vec<u8>],
    threshold: u32,
    window_secs: u64,
) -> Result<(), String> {
    check_registrars(class, registrars, threshold).map_err(|e| e.0)?;
    if window_secs == 0 {
        return Err("the veto window must be positive".into());
    }
    let rec = ClassRec {
        registrars: registrars.to_vec(),
        threshold,
        version: 0,
        window_secs,
    };
    ledger
        .genesis_put(ASSETS_MODULE, &k_class(class), rec.encode_to_vec())
        .map_err(|e| format!("{e:?}"))
}

/// The escrow that holds the price of a pending trade of `asset`: an address derived from the
/// asset, with no key, so only the module can move what is in it.
pub fn escrow_address(class: &[u8], asset: &[u8]) -> Address {
    tagged_hash(tags::ASSET_ESCROW, &[class, asset])
}

fn check_registrars(class: &[u8], regs: &[Vec<u8>], threshold: u32) -> Result<(), ModuleError> {
    if class.is_empty() || class.len() > MAX_CLASS {
        return Err(err("invalid class name"));
    }
    if regs.is_empty() || regs.len() > MAX_REGISTRARS {
        return Err(err("invalid number of registrars"));
    }
    if threshold == 0 || threshold as usize > regs.len() {
        return Err(err("invalid threshold"));
    }
    for (i, r) in regs.iter().enumerate() {
        if r.len() != 33 && r.len() != 65 {
            return Err(err("invalid registrar key"));
        }
        if regs.iter().take(i).any(|o| o == r) {
            return Err(err("duplicate registrar"));
        }
    }
    Ok(())
}

pub fn module() -> Box<dyn Module> {
    Box::new(AssetsModule)
}

pub struct AssetsModule;

fn load_class(ctx: &Ctx<'_, '_>, class: &[u8]) -> Result<ClassRec, ModuleError> {
    let raw = ctx
        .get(&k_class(class))
        .ok_or_else(|| err("unknown class"))?;
    ClassRec::decode(raw.as_slice()).map_err(|_| err("corrupt class"))
}

fn load_asset(ctx: &Ctx<'_, '_>, class: &[u8], id: &[u8]) -> Result<AssetRec, ModuleError> {
    let raw = ctx
        .get(&k_asset(class, id))
        .ok_or_else(|| err("unknown asset"))?;
    AssetRec::decode(raw.as_slice()).map_err(|_| err("corrupt asset"))
}

fn addr32(b: &[u8]) -> Result<Address, ModuleError> {
    Address::try_from(b).map_err(|_| err("invalid address"))
}

fn valid(intent_h: &[u8; 32], a: &Approval) -> bool {
    verify_tagged(&a.key, tags::ASSET_APPROVAL, intent_h, &a.signature).is_ok()
}

/// At least `threshold` DISTINCT registrar members signed this intent.
fn quorum(h: &[u8; 32], approvals: &[Approval], class: &ClassRec) -> bool {
    let mut seen: Vec<&[u8]> = Vec::new();
    for a in approvals {
        if class.registrars.contains(&a.key) && !seen.contains(&a.key.as_slice()) && valid(h, a) {
            seen.push(&a.key);
        }
    }
    seen.len() >= class.threshold as usize
}

fn check_len(approvals: &[Approval]) -> Result<(), ModuleError> {
    if approvals.len() > MAX_APPROVALS {
        return Err(err("too many approvals"));
    }
    Ok(())
}

fn save_asset(ctx: &mut Ctx<'_, '_>, class: &[u8], id: &[u8], rec: &AssetRec) {
    ctx.set(&k_asset(class, id), rec.encode_to_vec());
}

fn move_owner(ctx: &mut Ctx<'_, '_>, class: &[u8], id: &[u8], from: &[u8], to: &[u8]) {
    ctx.delete(&k_owner(from, class, id));
    ctx.set(&k_owner(to, class, id), vec![1]);
}

fn encode_registrars(regs: &[Vec<u8>], threshold: u32) -> Vec<u8> {
    let mut out = threshold.to_be_bytes().to_vec();
    for r in regs {
        out.extend_from_slice(&(r.len() as u32).to_be_bytes());
        out.extend_from_slice(r);
    }
    out
}

impl Module for AssetsModule {
    fn id(&self) -> u32 {
        ASSETS_MODULE
    }

    fn execute(&self, ctx: &mut Ctx<'_, '_>, payload: &[u8]) -> Result<(), ModuleError> {
        let chain = ctx.chain_id.to_string();
        match Op::decode(payload).ok_or_else(|| err("malformed assets payload"))? {
            Op::Register(op) => register(ctx, &chain, op),
            Op::Offer(op) => offer(ctx, op),
            Op::Accept(op) => accept(ctx, op),
            Op::Cancel(op) => cancel(ctx, op),
            Op::Finalize(op) => finalize(ctx, op),
            Op::Veto(op) => veto(ctx, &chain, op),
            Op::AddLien(op) => {
                check_len(&op.approvals)?;
                if op.kind.is_empty() || op.kind.len() > MAX_LIEN_KIND {
                    return Err(err("invalid lien kind"));
                }
                let class = load_class(ctx, &op.class)?;
                let mut a = load_asset(ctx, &op.class, &op.asset)?;
                if a.liens.len() >= MAX_LIENS {
                    return Err(err("too many liens"));
                }
                let h = intent_hash(
                    &chain,
                    &Intent {
                        kind: IntentKind::AddLien,
                        class: op.class.clone(),
                        asset: op.asset.clone(),
                        from: addr32(&a.owner)?,
                        to: ZERO,
                        amount: 0,
                        version: a.version,
                        extra: op.kind.as_bytes().to_vec(),
                    },
                );
                if !quorum(&h, &op.approvals, &class) {
                    return Err(err("registrar quorum missing"));
                }
                // any change to the asset kills a pending trade (and refunds the buyer)
                drop_offer(ctx, &op.class, &op.asset)?;
                let id = a.next_lien;
                a.next_lien += 1;
                a.liens.push(Lien { id, kind: op.kind });
                a.version += 1;
                save_asset(ctx, &op.class, &op.asset, &a);
                ctx.emit("lien-added", op.asset);
                Ok(())
            }
            Op::ReleaseLien(op) => {
                check_len(&op.approvals)?;
                let class = load_class(ctx, &op.class)?;
                let mut a = load_asset(ctx, &op.class, &op.asset)?;
                let h = intent_hash(
                    &chain,
                    &Intent {
                        kind: IntentKind::ReleaseLien,
                        class: op.class.clone(),
                        asset: op.asset.clone(),
                        from: addr32(&a.owner)?,
                        to: ZERO,
                        amount: 0,
                        version: a.version,
                        extra: op.lien_id.to_be_bytes().to_vec(),
                    },
                );
                if !quorum(&h, &op.approvals, &class) {
                    return Err(err("registrar quorum missing"));
                }
                let before = a.liens.len();
                a.liens.retain(|l| l.id != op.lien_id);
                if a.liens.len() == before {
                    return Err(err("no such lien"));
                }
                drop_offer(ctx, &op.class, &op.asset)?;
                a.version += 1;
                save_asset(ctx, &op.class, &op.asset, &a);
                ctx.emit("lien-released", op.asset);
                Ok(())
            }
            Op::UpdateRegistrars(op) => update_registrars(ctx, &chain, op),
        }
    }
}

fn register(ctx: &mut Ctx<'_, '_>, chain: &str, op: RegisterOp) -> Result<(), ModuleError> {
    check_len(&op.approvals)?;
    if op.asset.is_empty() || op.asset.len() > MAX_ASSET {
        return Err(err("invalid asset id"));
    }
    // the chain stores only a HASH of the off-chain metadata record (ADR-0032)
    if op.metadata_hash.len() != META_HASH_LEN {
        return Err(err("metadata hash must be 32 bytes"));
    }
    let class = load_class(ctx, &op.class)?;
    if ctx.get(&k_asset(&op.class, &op.asset)).is_some() {
        return Err(err("asset already registered"));
    }
    let owner = addr32(&op.owner)?;
    let h = intent_hash(
        chain,
        &Intent {
            kind: IntentKind::Register,
            class: op.class.clone(),
            asset: op.asset.clone(),
            from: owner,
            to: ZERO,
            amount: 0,
            version: 0,
            extra: op.metadata_hash.clone(),
        },
    );
    if !quorum(&h, &op.approvals, &class) {
        return Err(err("registrar quorum missing"));
    }
    let rec = AssetRec {
        owner: owner.to_vec(),
        metadata_hash: op.metadata_hash,
        version: 0,
        liens: Vec::new(),
        next_lien: 0,
    };
    save_asset(ctx, &op.class, &op.asset, &rec);
    ctx.set(&k_owner(&owner, &op.class, &op.asset), vec![1]);
    ctx.emit("registered", op.asset);
    Ok(())
}

fn load_offer(ctx: &Ctx<'_, '_>, class: &[u8], id: &[u8]) -> Result<OfferRec, ModuleError> {
    let raw = ctx
        .get(&k_offer(class, id))
        .ok_or_else(|| err("no offer for this asset"))?;
    OfferRec::decode(raw.as_slice()).map_err(|_| err("corrupt offer"))
}

fn save_offer(ctx: &mut Ctx<'_, '_>, class: &[u8], id: &[u8], o: &OfferRec) {
    ctx.set(&k_offer(class, id), o.encode_to_vec());
}

/// Moves `amount` coins as `from` (the escrow, or the buyer who authorised it).
fn pay(ctx: &mut Ctx<'_, '_>, from: Address, to: Address, amount: u64) -> Result<(), ModuleError> {
    if amount == 0 {
        return Ok(());
    }
    let payload = CoinOp::Transfer { to, amount }.encode();
    ctx.call_as(COIN_MODULE, from, &payload)
}

/// Removes a pending offer, refunding the escrow to the buyer if it was accepted.
fn drop_offer(ctx: &mut Ctx<'_, '_>, class: &[u8], id: &[u8]) -> Result<(), ModuleError> {
    let Some(raw) = ctx.get(&k_offer(class, id)) else {
        return Ok(());
    };
    let o = OfferRec::decode(raw.as_slice()).map_err(|_| err("corrupt offer"))?;
    if o.accepted {
        pay(ctx, escrow_address(class, id), addr32(&o.buyer)?, o.price)
            .map_err(|e| ModuleError::new(format!("refund failed: {}", e.0)))?;
    }
    ctx.delete(&k_offer(class, id));
    ctx.emit("offer-cancelled", id.to_vec());
    Ok(())
}

/// The OWNER offers the asset to one named buyer. The transaction signature is the consent; the
/// offer is bound to the version and metadata hash the owner saw.
fn offer(ctx: &mut Ctx<'_, '_>, op: OfferOp) -> Result<(), ModuleError> {
    load_class(ctx, &op.class)?;
    let a = load_asset(ctx, &op.class, &op.asset)?;
    let owner = addr32(&a.owner)?;
    if ctx.sender != owner {
        return Err(err("only the owner can offer"));
    }
    let buyer = addr32(&op.buyer)?;
    if buyer == owner {
        return Err(err("seller and buyer are the same"));
    }
    if op.version != a.version {
        return Err(err("offer is for another version of the asset"));
    }
    if op.metadata_hash != a.metadata_hash {
        return Err(err("offer is for another asset record"));
    }
    if !a.liens.is_empty() {
        return Err(err("asset has active liens"));
    }
    if ctx.get(&k_offer(&op.class, &op.asset)).is_some() {
        return Err(err("the asset already has an offer"));
    }
    let rec = OfferRec {
        buyer: buyer.to_vec(),
        price: op.price,
        version: a.version,
        metadata_hash: a.metadata_hash,
        accepted: false,
        release_at: 0,
    };
    save_offer(ctx, &op.class, &op.asset, &rec);
    ctx.emit("offered", op.asset);
    Ok(())
}

/// The NAMED BUYER accepts: the price goes to the escrow (all or nothing) and the class's veto
/// window starts, measured by the block time.
fn accept(ctx: &mut Ctx<'_, '_>, op: AcceptOp) -> Result<(), ModuleError> {
    let class = load_class(ctx, &op.class)?;
    let a = load_asset(ctx, &op.class, &op.asset)?;
    let mut o = load_offer(ctx, &op.class, &op.asset)?;
    let buyer = addr32(&o.buyer)?;
    if ctx.sender != buyer {
        return Err(err("only the named buyer can accept"));
    }
    if o.accepted {
        return Err(err("the offer was already accepted"));
    }
    if o.version != a.version || !a.liens.is_empty() {
        return Err(err("offer is for another version of the asset"));
    }
    // the buyer accepts the terms it SAW: any change since then voids the acceptance
    if op.price != o.price || op.version != o.version || op.metadata_hash != o.metadata_hash {
        return Err(err("the offer terms changed: review and accept again"));
    }
    pay(ctx, buyer, escrow_address(&op.class, &op.asset), o.price)
        .map_err(|e| ModuleError::new(format!("escrow payment failed: {}", e.0)))?;
    o.accepted = true;
    o.release_at = ctx.time.saturating_add(class.window_secs);
    save_offer(ctx, &op.class, &op.asset, &o);
    ctx.emit("accepted", op.asset);
    Ok(())
}

/// The OWNER withdraws an offer nobody accepted yet.
fn cancel(ctx: &mut Ctx<'_, '_>, op: CancelOp) -> Result<(), ModuleError> {
    let a = load_asset(ctx, &op.class, &op.asset)?;
    let o = load_offer(ctx, &op.class, &op.asset)?;
    if ctx.sender != addr32(&a.owner)? {
        return Err(err("only the owner can cancel"));
    }
    if o.accepted {
        return Err(err(
            "the offer was already accepted: only the organ can veto",
        ));
    }
    ctx.delete(&k_offer(&op.class, &op.asset));
    ctx.emit("offer-cancelled", op.asset);
    Ok(())
}

/// ANYONE completes an accepted trade once the veto window is over: the asset goes to the buyer
/// and the escrow pays the seller, all or nothing.
fn finalize(ctx: &mut Ctx<'_, '_>, op: FinalizeOp) -> Result<(), ModuleError> {
    let mut a = load_asset(ctx, &op.class, &op.asset)?;
    let o = load_offer(ctx, &op.class, &op.asset)?;
    if !o.accepted {
        return Err(err("the offer was not accepted yet"));
    }
    if ctx.time < o.release_at {
        return Err(err("veto window still open"));
    }
    if o.version != a.version || !a.liens.is_empty() {
        return Err(err("offer is for another version of the asset"));
    }
    let seller = addr32(&a.owner)?;
    let buyer = addr32(&o.buyer)?;
    pay(ctx, escrow_address(&op.class, &op.asset), seller, o.price)
        .map_err(|e| ModuleError::new(format!("payment failed: {}", e.0)))?;
    a.owner = buyer.to_vec();
    a.version += 1;
    save_asset(ctx, &op.class, &op.asset, &a);
    move_owner(ctx, &op.class, &op.asset, &seller, &buyer);
    ctx.delete(&k_offer(&op.class, &op.asset));
    ctx.emit("transferred", op.asset);
    Ok(())
}

/// The registrar QUORUM vetoes a pending trade inside the window, with a public reason; the
/// buyer is refunded and the asset never left the owner.
fn veto(ctx: &mut Ctx<'_, '_>, chain: &str, op: VetoOp) -> Result<(), ModuleError> {
    check_len(&op.approvals)?;
    let class = load_class(ctx, &op.class)?;
    let a = load_asset(ctx, &op.class, &op.asset)?;
    let o = match load_offer(ctx, &op.class, &op.asset) {
        Ok(o) if o.accepted => o,
        _ => return Err(err("nothing pending to veto")),
    };
    if ctx.time >= o.release_at {
        return Err(err("the veto window is over"));
    }
    if op.reason.is_empty() || op.reason.len() > MAX_REASON {
        return Err(err("invalid veto reason"));
    }
    let h = intent_hash(
        chain,
        &Intent::veto(
            &op.class,
            &op.asset,
            addr32(&a.owner)?,
            addr32(&o.buyer)?,
            o.price,
            a.version,
            &op.reason,
        ),
    );
    if !quorum(&h, &op.approvals, &class) {
        return Err(err("registrar quorum missing"));
    }
    pay(
        ctx,
        escrow_address(&op.class, &op.asset),
        addr32(&o.buyer)?,
        o.price,
    )
    .map_err(|e| ModuleError::new(format!("refund failed: {}", e.0)))?;
    ctx.delete(&k_offer(&op.class, &op.asset));
    ctx.emit("vetoed", op.reason.into_bytes());
    Ok(())
}

fn update_registrars(
    ctx: &mut Ctx<'_, '_>,
    chain: &str,
    op: RegistrarsOp,
) -> Result<(), ModuleError> {
    check_len(&op.approvals)?;
    let mut class = load_class(ctx, &op.class)?;
    check_registrars(&op.class, &op.registrars, op.threshold)?;
    let h = intent_hash(
        chain,
        &Intent {
            kind: IntentKind::UpdateRegistrars,
            class: op.class.clone(),
            asset: Vec::new(),
            from: ZERO,
            to: ZERO,
            amount: 0,
            version: class.version,
            extra: encode_registrars(&op.registrars, op.threshold),
        },
    );
    // The CURRENT registrar quorum authorises the change.
    if !quorum(&h, &op.approvals, &class) {
        return Err(err("registrar quorum missing"));
    }
    class.registrars = op.registrars;
    class.threshold = op.threshold;
    class.version += 1;
    ctx.set(&k_class(&op.class), class.encode_to_vec());
    ctx.emit("registrars-updated", op.class);
    Ok(())
}
