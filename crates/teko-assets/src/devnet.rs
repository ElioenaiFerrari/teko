//! Local DEVNET assets world (development only), built on the coin devnet economy.
//!
//! It plays the institutions that do not exist yet on a real network, with TEST keys in
//! the dev directory:
//!
//!   * a SIMULATED REGISTRAR (órgão): 3 keys, 2-of-3, that registers assets and places
//!     or releases liens;
//!   * a SIMULATED COUNTERPARTY: another citizen to sell to or buy from;
//!   * a relayer that pays the fees of the órgão's own transactions.
//!
//! The citizen's consent always comes from THEIR device (closure `consent`).
//! Nothing here exists in a production build.

use std::fs;
use std::path::Path;

use teko_coin::COIN_MODULE;
use teko_coin::devnet::{CHAIN_ID, Economy, TransferRecord};
use teko_crypto::{SigningKey, generate_signing_key, hex, public_key_bytes};
use teko_ledger::{Address, Ledger, Tx, address_of};

use crate::{
    ASSETS_MODULE, Approval, Intent, LienOp, Op, RegisterOp, ReleaseOp, SwapOp, asset, assets_of,
    genesis_class, module,
};

/// What the devnet screens need from an economy: the local test file or a real node.
/// The ledger it exposes is always VERIFIED by this process.
pub trait World {
    /// The network id signed into every transaction.
    fn chain_id(&self) -> &str;
    fn ledger(&self) -> &Ledger;
    /// Sends a signed transaction and returns once it is accepted (and, for a node,
    /// included in a block).
    fn submit(&mut self, tx: Tx) -> Result<(), String>;
    fn history(&self, who: &Address) -> Vec<TransferRecord>;

    fn nonce(&self, who: &Address) -> u64 {
        self.ledger().nonce_of(who)
    }
    fn balance(&self, who: &Address) -> u64 {
        teko_coin::balance(self.ledger(), who)
    }
    /// How a person gets test coins in THIS world (shown when a buyer cannot pay).
    fn funding_hint(&self) -> &'static str {
        "use `make fund`"
    }
    /// A fee that covers the per-byte price of a coin transfer.
    fn estimate_fee(&self, sender_key: &[u8], nonce: u64, to: &Address, amount: u64) -> u64 {
        let price = teko_coin::config(self.ledger()).map_or(0, |c| c.fee_per_byte);
        let probe = Tx {
            chain_id: self.chain_id().into(),
            nonce,
            module: COIN_MODULE,
            payload: teko_coin::Op::Transfer { to: *to, amount }.encode(),
            sender: sender_key.to_vec(),
            signature: vec![0u8; 64],
            fee: u64::MAX,
        };
        price.saturating_mul(probe.to_bytes().len() as u64)
    }
}

impl World for Economy {
    fn chain_id(&self) -> &str {
        CHAIN_ID
    }
    fn ledger(&self) -> &Ledger {
        Economy::ledger(self)
    }
    fn submit(&mut self, tx: Tx) -> Result<(), String> {
        Economy::submit(self, tx)
    }
    fn history(&self, who: &Address) -> Vec<TransferRecord> {
        Economy::history(self, who)
    }
}

/// The only asset class of the devnet world.
pub const CLASS: &str = "imovel";
const REGISTRAR_FILES: [&str; 3] = ["registrar-0.key", "registrar-1.key", "registrar-2.key"];
const COUNTERPARTY_FILE: &str = "counterparty.key";
const RELAYER_FILE: &str = "relayer.key";
/// Coins the relayer holds so the órgão's own transactions can pay fees.
const RELAYER_FUNDS: u64 = 100_000_000;

fn load_or_create(path: &Path) -> Result<SigningKey, String> {
    if let Ok(text) = fs::read_to_string(path) {
        let b =
            hex::decode(text.trim()).map_err(|_| format!("{}: hex inválido", path.display()))?;
        return SigningKey::from_slice(&b)
            .map_err(|_| format!("{}: chave inválida", path.display()));
    }
    let mut rng = rand::rand_core::UnwrapErr(rand::rngs::SysRng);
    let k = generate_signing_key(&mut rng);
    fs::write(path, hex::encode(&k.to_bytes())).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(k)
}

/// The simulated people and institutions of the devnet world (TEST keys in `dir`).
pub struct Cast {
    pub registrars: Vec<SigningKey>,
    pub counterparty: SigningKey,
    pub relayer: SigningKey,
}

impl Cast {
    pub fn load(dir: &Path) -> Result<Cast, String> {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let registrars = REGISTRAR_FILES
            .iter()
            .map(|f| load_or_create(&dir.join(f)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Cast {
            registrars,
            counterparty: load_or_create(&dir.join(COUNTERPARTY_FILE))?,
            relayer: load_or_create(&dir.join(RELAYER_FILE))?,
        })
    }

    /// The same people, with the registrar keys of a real node network (so the órgão
    /// simulado signs for the class that network's genesis defined).
    pub fn with_registrars(mut self, registrars: Vec<SigningKey>) -> Cast {
        if !registrars.is_empty() {
            self.registrars = registrars;
        }
        self
    }

    pub fn counterparty_address(&self) -> Address {
        address_of(&public_key_bytes(&self.counterparty))
    }

    fn relayer_address(&self) -> Address {
        address_of(&public_key_bytes(&self.relayer))
    }
}

fn build(cast: &Cast, allocs: &[(Address, u64)]) -> Result<Ledger, String> {
    let mut all = allocs.to_vec();
    all.push((cast.relayer_address(), RELAYER_FUNDS));
    let mut l = Ledger::new(CHAIN_ID, vec![teko_coin::module(), module()])
        .map_err(|e| format!("{e:?}"))?
        .with_fee_module(COIN_MODULE)
        .map_err(|e| format!("{e:?}"))?;
    teko_coin::genesis(&mut l, teko_coin::devnet::devnet_config(), &all, true)
        .map_err(|e| format!("{e:?}"))?;
    let keys: Vec<Vec<u8>> = cast.registrars.iter().map(public_key_bytes).collect();
    genesis_class(&mut l, CLASS.as_bytes(), &keys, 2)?;
    Ok(l)
}

/// Opens the devnet world stored in `dir` (coin + assets, replaying every transaction).
pub fn open(dir: &Path) -> Result<(Economy, Cast), String> {
    let cast = Cast::load(dir)?;
    let econ = Economy::open_with(dir, |a| build(&cast, a))?;
    Ok((econ, cast))
}

/// Coin balance of `who` in the devnet world stored in `dir` (None = no economy file).
pub fn balance_of(dir: &Path, who: &Address) -> Result<Option<u64>, String> {
    if !dir.join(teko_coin::devnet::ECONOMY_FILE).exists() {
        return Ok(None);
    }
    Ok(Some(open(dir)?.0.balance(who)))
}

/// A consent supplier: the DEVICE of the person (shows and signs), or a software key.
/// Arguments: kind (2 transfer, 3 swap), asset id, seller, buyer, price, version.
pub type Consent<'a> =
    dyn FnMut(u8, &str, &Address, &Address, u64, u64) -> Result<Approval, String> + 'a;

fn submit_assets_op(econ: &mut dyn World, cast: &Cast, op: Op) -> Result<(), String> {
    let me = cast.relayer_address();
    let nonce = econ.nonce(&me);
    let pk = public_key_bytes(&cast.relayer);
    // fee: price per byte over a generously sized envelope
    let price = teko_coin::config(econ.ledger())
        .map(|c| c.fee_per_byte)
        .unwrap_or(0);
    let fee = price.saturating_mul(op.encode().len() as u64 + pk.len() as u64 + 160);
    let chain = econ.chain_id().to_string();
    let tx = Tx::new_signed_with_fee(
        &cast.relayer,
        &chain,
        nonce,
        fee,
        ASSETS_MODULE,
        op.encode(),
    );
    econ.submit(tx)
}

fn quorum(cast: &Cast, chain_id: &str, i: &Intent) -> Vec<Approval> {
    cast.registrars
        .iter()
        .take(2)
        .map(|k| Approval::sign(k, chain_id, i))
        .collect()
}

/// The simulated registrar registers `asset_id` to `owner`.
pub fn register(
    econ: &mut dyn World,
    cast: &Cast,
    owner: &Address,
    asset_id: &str,
) -> Result<(), String> {
    let meta = format!("escritura de {asset_id}");
    let i = Intent::register(
        CLASS.as_bytes(),
        asset_id.as_bytes(),
        *owner,
        meta.as_bytes(),
    );
    let approvals = quorum(cast, econ.chain_id(), &i);
    submit_assets_op(
        econ,
        cast,
        Op::Register(RegisterOp {
            class: CLASS.into(),
            asset: asset_id.as_bytes().to_vec(),
            owner: owner.to_vec(),
            metadata_hash: meta.into_bytes(),
            approvals,
        }),
    )
}

/// The simulated registrar adds (`on`) or releases the first lien of `asset_id`.
pub fn set_lien(econ: &mut dyn World, cast: &Cast, asset_id: &str, on: bool) -> Result<(), String> {
    let a =
        asset(econ.ledger(), CLASS.as_bytes(), asset_id.as_bytes()).ok_or("bem desconhecido")?;
    let owner: Address = a.owner.as_slice().try_into().map_err(|_| "dono inválido")?;
    if on {
        let i = Intent::add_lien(
            CLASS.as_bytes(),
            asset_id.as_bytes(),
            owner,
            a.version,
            "penhora",
        );
        let ap = quorum(cast, econ.chain_id(), &i);
        submit_assets_op(
            econ,
            cast,
            Op::AddLien(LienOp {
                class: CLASS.into(),
                asset: asset_id.as_bytes().to_vec(),
                kind: "penhora".into(),
                approvals: ap,
            }),
        )
    } else {
        let lien = a.liens.first().ok_or("o bem não tem gravame")?.id;
        let i = Intent::release_lien(
            CLASS.as_bytes(),
            asset_id.as_bytes(),
            owner,
            a.version,
            lien,
        );
        let ap = quorum(cast, econ.chain_id(), &i);
        submit_assets_op(
            econ,
            cast,
            Op::ReleaseLien(ReleaseOp {
                class: CLASS.into(),
                asset: asset_id.as_bytes().to_vec(),
                lien_id: lien,
                approvals: ap,
            }),
        )
    }
}

/// Sells (`me` is the owner) or buys (`me` is the buyer) `asset_id` against `price`
/// coins with the simulated counterparty. MY consent comes from `consent` (my device);
/// the counterparty and the registrar quorum sign in software; the ledger executes the
/// atomic swap. Returns my coin balance afterwards.
pub fn trade_with_counterparty(
    econ: &mut dyn World,
    cast: &Cast,
    me: &Address,
    asset_id: &str,
    price: u64,
    selling: bool,
    consent: &mut Consent<'_>,
) -> Result<u64, String> {
    if price == 0 {
        return Err("informe um preço maior que zero".into());
    }
    let a =
        asset(econ.ledger(), CLASS.as_bytes(), asset_id.as_bytes()).ok_or("bem desconhecido")?;
    let owner: Address = a.owner.as_slice().try_into().map_err(|_| "dono inválido")?;
    let cp = cast.counterparty_address();
    let (seller, buyer) = if selling { (*me, cp) } else { (cp, *me) };
    if owner != seller {
        return Err(if selling {
            "este bem não é seu".into()
        } else {
            "a contraparte simulada não é dona deste bem".into()
        });
    }
    if !a.liens.is_empty() {
        return Err("o bem tem gravame (penhora): peça a liberação ao órgão".into());
    }
    if econ.balance(&buyer) < price {
        return Err(format!(
            "o comprador não tem saldo: precisa de {}, tem {} ({})",
            teko_crypto::amount::format_amount(price),
            teko_crypto::amount::format_amount(econ.balance(&buyer)),
            econ.funding_hint()
        ));
    }
    let i = Intent::swap(
        CLASS.as_bytes(),
        asset_id.as_bytes(),
        seller,
        buyer,
        price,
        a.version,
    );
    // 1) MY consent, on my device
    let mine = consent(3, asset_id, &seller, &buyer, price, a.version)?;
    // 2) the counterparty (software) and the registrar quorum
    let mut approvals = vec![mine];
    let chain = econ.chain_id().to_string();
    approvals.push(Approval::sign(&cast.counterparty, &chain, &i));
    approvals.extend(quorum(cast, econ.chain_id(), &i));
    // the relayer submits (and pays the fee)
    let op = Op::Swap(SwapOp {
        class: CLASS.into(),
        asset: asset_id.as_bytes().to_vec(),
        buyer: buyer.to_vec(),
        price,
        approvals,
    });
    submit_assets_op(econ, cast, op)?;
    Ok(econ.balance(me))
}

/// A wallet's assets: (asset id, version, number of liens).
pub fn owned(econ: &dyn World, who: &Address) -> Vec<(String, u64, usize)> {
    assets_of(econ.ledger(), who)
        .into_iter()
        .filter_map(|(c, id)| {
            let a = asset(econ.ledger(), &c, &id)?;
            Some((
                String::from_utf8_lossy(&id).into_owned(),
                a.version,
                a.liens.len(),
            ))
        })
        .collect()
}
