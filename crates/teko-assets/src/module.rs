use prost::Message;
use teko_coin::{COIN_MODULE, Op as CoinOp};
use teko_crypto::{tags, verify_tagged};
use teko_ledger::{Address, Ctx, Ledger, Module, ModuleError, address_of};

use crate::types::{
    Approval, AssetRec, ClassRec, Intent, IntentKind, Lien, Op, RegisterOp, RegistrarsOp,
    intent_hash,
};

/// Module id of the assets module in `Tx::module`.
pub const ASSETS_MODULE: u32 = 2;

const MAX_CLASS: usize = 32;
const MAX_ASSET: usize = 64;
const MAX_META: usize = 64;
const MAX_APPROVALS: usize = 32;
const MAX_REGISTRARS: usize = 16;
const MAX_LIENS: usize = 16;
const MAX_LIEN_KIND: usize = 32;
const ZERO: Address = [0; 32];

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

/// Creates a class at genesis, governed by `registrars` with `threshold`-of-N.
pub fn genesis_class(
    ledger: &mut Ledger,
    class: &[u8],
    registrars: &[Vec<u8>],
    threshold: u32,
) -> Result<(), String> {
    check_registrars(class, registrars, threshold).map_err(|e| e.0)?;
    let rec = ClassRec {
        registrars: registrars.to_vec(),
        threshold,
        version: 0,
    };
    ledger
        .genesis_put(ASSETS_MODULE, &k_class(class), rec.encode_to_vec())
        .map_err(|e| format!("{e:?}"))
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

/// `who` (an address) signed this intent.
fn signed_by(h: &[u8; 32], approvals: &[Approval], who: &Address) -> bool {
    approvals
        .iter()
        .any(|a| address_of(&a.key) == *who && valid(h, a))
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
            Op::Transfer(op) => {
                check_len(&op.approvals)?;
                let to = addr32(&op.to)?;
                transfer_checked(
                    ctx,
                    &chain,
                    &op.class,
                    &op.asset,
                    to,
                    0,
                    IntentKind::Transfer,
                    &op.approvals,
                )?;
                Ok(())
            }
            Op::Swap(op) => {
                check_len(&op.approvals)?;
                if op.price == 0 {
                    return Err(err("price must be positive"));
                }
                let buyer = addr32(&op.buyer)?;
                let seller = transfer_checked(
                    ctx,
                    &chain,
                    &op.class,
                    &op.asset,
                    buyer,
                    op.price,
                    IntentKind::Swap,
                    &op.approvals,
                )?;
                // Payment in the same transaction: if it fails, the whole swap fails.
                let pay = CoinOp::Transfer {
                    to: seller,
                    amount: op.price,
                }
                .encode();
                ctx.call_as(COIN_MODULE, buyer, &pay)
                    .map_err(|e| ModuleError::new(format!("payment failed: {}", e.0)))?;
                ctx.emit("swapped", op.asset.clone());
                Ok(())
            }
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
    if op.asset.is_empty() || op.asset.len() > MAX_ASSET || op.metadata_hash.len() > MAX_META {
        return Err(err("invalid asset id or metadata"));
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

/// Shared by Transfer and Swap: needs owner + buyer + registrar quorum, no liens.
/// Moves ownership and returns the previous owner.
#[allow(clippy::too_many_arguments)]
fn transfer_checked(
    ctx: &mut Ctx<'_, '_>,
    chain: &str,
    class_name: &[u8],
    asset_id: &[u8],
    to: Address,
    price: u64,
    kind: IntentKind,
    approvals: &[Approval],
) -> Result<Address, ModuleError> {
    let class = load_class(ctx, class_name)?;
    let mut a = load_asset(ctx, class_name, asset_id)?;
    let owner = addr32(&a.owner)?;
    if owner == to {
        return Err(err("seller and buyer are the same"));
    }
    let h = intent_hash(
        chain,
        &Intent {
            kind,
            class: class_name.to_vec(),
            asset: asset_id.to_vec(),
            from: owner,
            to,
            amount: price,
            version: a.version,
            extra: Vec::new(),
        },
    );
    if !signed_by(&h, approvals, &owner) {
        return Err(err("owner consent missing"));
    }
    if !signed_by(&h, approvals, &to) {
        return Err(err("buyer consent missing"));
    }
    if !quorum(&h, approvals, &class) {
        return Err(err("registrar quorum missing"));
    }
    if !a.liens.is_empty() {
        return Err(err("asset has active liens"));
    }
    a.owner = to.to_vec();
    a.version += 1;
    save_asset(ctx, class_name, asset_id, &a);
    move_owner(ctx, class_name, asset_id, &owner, &to);
    ctx.emit("transferred", asset_id.to_vec());
    Ok(owner)
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
