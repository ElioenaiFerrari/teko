//! Local DEVNET assets world (development only), built on the coin devnet economy.
//!
//! It plays the institutions that do not exist yet on a real network, with TEST keys in
//! the dev directory:
//!
//!   * a SIMULATED REGISTRAR (órgão) PER CLASS (`imovel`, `veiculo`): 3 keys, 2-of-3,
//!     that registers assets and places or releases liens;
//!   * a SIMULATED COUNTERPARTY: another citizen to sell to or buy from;
//!   * a relayer that pays the fees of the órgão's own transactions (never of a citizen's).
//!
//! The citizen's own transactions (offer, accept) are built AND SIGNED by THEIR device
//! (closure `TxSigner`): they leave from the citizen's wallet, not from the relayer (ADR-0036).
//! Nothing here exists in a production build.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use teko_coin::COIN_MODULE;
use teko_coin::devnet::{CHAIN_ID, Economy, TransferRecord};
use teko_crypto::{SigningKey, asset_meta, generate_signing_key, hex, public_key_bytes};
use teko_ledger::{Address, Ledger, Tx, address_of};

use crate::metadata::MetadataStore;
use crate::{
    ASSETS_MODULE, AcceptOp, Approval, FinalizeOp, Intent, LienOp, OfferOp, OfferRec, Op,
    RegisterOp, ReleaseOp, VetoOp, asset, assets_of, class_info, genesis_class, module, offer_of,
    offers,
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

/// The asset classes of the devnet world, each with its own órgão (ADR-0032).
pub const CLASSES: [&str; 2] = ["imovel", "veiculo"];
const COUNTERPARTY_FILE: &str = "counterparty.key";
const RELAYER_FILE: &str = "relayer.key";
/// Name of the off-chain metadata folder inside a network or dev directory.
pub const METADATA_DIR: &str = "assets-metadata";
/// Coins the relayer holds so the órgão's own transactions can pay fees.
const RELAYER_FUNDS: u64 = 100_000_000;

/// Key file of member `i` of the órgão of `class`. `imovel` keeps the original names
/// (`registrar-0.key`), the others are `registrar-<class>-<i>.key`.
pub fn registrar_file(class: &str, i: usize) -> String {
    if class == "imovel" {
        format!("registrar-{i}.key")
    } else {
        format!("registrar-{class}-{i}.key")
    }
}

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
    /// The órgão (3 keys) of each class.
    pub registrars: BTreeMap<String, Vec<SigningKey>>,
    pub counterparty: SigningKey,
    pub relayer: SigningKey,
    /// Where the off-chain metadata records live (`<dir>/assets-metadata`).
    pub store: MetadataStore,
}

impl Cast {
    pub fn load(dir: &Path) -> Result<Cast, String> {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut registrars = BTreeMap::new();
        for class in CLASSES {
            let keys = (0..3)
                .map(|i| load_or_create(&dir.join(registrar_file(class, i))))
                .collect::<Result<Vec<_>, _>>()?;
            registrars.insert(class.to_string(), keys);
        }
        Ok(Cast {
            registrars,
            counterparty: load_or_create(&dir.join(COUNTERPARTY_FILE))?,
            relayer: load_or_create(&dir.join(RELAYER_FILE))?,
            store: MetadataStore::new(dir.join(METADATA_DIR)),
        })
    }

    /// The same people, with the registrar keys of a real node network for `class` (so
    /// the órgão simulado signs for the class that network's genesis defined).
    pub fn with_registrars(mut self, class: &str, keys: Vec<SigningKey>) -> Cast {
        if !keys.is_empty() {
            self.registrars.insert(class.to_string(), keys);
        }
        self
    }

    /// Keeps the metadata records in `dir` (for example the network directory, where the
    /// explorer looks for them).
    pub fn with_store(mut self, dir: impl Into<std::path::PathBuf>) -> Cast {
        self.store = MetadataStore::new(dir);
        self
    }

    pub fn registrars_for(&self, class: &str) -> Result<&[SigningKey], String> {
        self.registrars
            .get(class)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("classe sem órgão registrador: {class}"))
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
    for class in CLASSES {
        let keys: Vec<Vec<u8>> = cast
            .registrars_for(class)?
            .iter()
            .map(public_key_bytes)
            .collect();
        genesis_class(&mut l, class.as_bytes(), &keys, 2)?;
    }
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

/// What the citizen's DEVICE is asked to build, show and sign (ADR-0036): ONE transaction of its
/// own wallet. The device derives the transaction from these fields and signs after the button.
pub struct TradeRequest<'a> {
    pub action: TradeAction,
    pub chain_id: &'a str,
    pub class: &'a str,
    pub asset: &'a str,
    pub seller: &'a Address,
    pub buyer: &'a Address,
    pub price: u64,
    /// The asset version the trade is bound to.
    pub version: u64,
    /// The asset's canonical metadata RECORD (the device shows a summary and signs its hash).
    pub metadata: &'a [u8],
    /// Seconds the trade waits after the buyer accepts (the class's veto window).
    pub window_secs: u64,
    pub nonce: u64,
    pub fee: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeAction {
    /// The owner offers the asset to a named buyer.
    Offer,
    /// The named buyer accepts (the price goes to the escrow).
    Accept,
}

/// A transaction supplier for the citizen: the DEVICE (shows and signs), or a software key.
pub type TxSigner<'a> = dyn FnMut(&TradeRequest<'_>) -> Result<Tx, String> + 'a;

/// Fee of an assets transaction carrying `op` signed by a key of `key_len` bytes.
fn assets_fee(econ: &dyn World, op: &Op, key_len: usize) -> u64 {
    let price = teko_coin::config(econ.ledger()).map_or(0, |c| c.fee_per_byte);
    price.saturating_mul(op.encode().len() as u64 + key_len as u64 + 160)
}

/// The fee a device-signed assets transaction should carry (the Suite passes it to the device).
pub fn device_fee(econ: &dyn World, action: TradeAction) -> u64 {
    let probe = match action {
        TradeAction::Offer => Op::Offer(OfferOp {
            class: vec![0; 32],
            asset: vec![0; 64],
            buyer: vec![0; 32],
            price: u64::MAX,
            version: u64::MAX,
            metadata_hash: vec![0; 32],
        }),
        TradeAction::Accept => Op::Accept(AcceptOp {
            class: vec![0; 32],
            asset: vec![0; 64],
            price: u64::MAX,
            version: u64::MAX,
            metadata_hash: vec![0; 32],
        }),
    };
    assets_fee(econ, &probe, 33)
}

fn sign_as(econ: &dyn World, key: &SigningKey, op: Op) -> Tx {
    let me = address_of(&public_key_bytes(key));
    let pk = public_key_bytes(key);
    let fee = assets_fee(econ, &op, pk.len());
    Tx::new_signed_with_fee(
        key,
        econ.chain_id(),
        econ.nonce(&me),
        fee,
        ASSETS_MODULE,
        op.encode(),
    )
}

/// The checks both directions share: the asset exists, belongs to `seller`, has no lien, has
/// no trade in progress and its metadata record is here AND matches the hash on the chain.
fn trade_context(
    econ: &dyn World,
    cast: &Cast,
    class: &str,
    asset_id: &str,
    seller: &Address,
) -> Result<(crate::AssetRec, Vec<u8>, u64), String> {
    let a =
        asset(econ.ledger(), class.as_bytes(), asset_id.as_bytes()).ok_or("bem desconhecido")?;
    let owner: Address = a.owner.as_slice().try_into().map_err(|_| "dono inválido")?;
    if owner != *seller {
        return Err("quem vende não é o dono deste bem".into());
    }
    if !a.liens.is_empty() {
        return Err("o bem tem gravame (penhora): peça a liberação ao órgão".into());
    }
    if offer_of(econ.ledger(), class.as_bytes(), asset_id.as_bytes()).is_some() {
        return Err("este bem já tem uma negociação em andamento".into());
    }
    let record = cast
        .store
        .get_verified(class, asset_id, &a.metadata_hash)
        .ok_or("os metadados deste bem não estão neste computador (ou não conferem com a chain): venda bloqueada")?;
    let window = class_info(econ.ledger(), class.as_bytes()).map_or(0, |c| c.window_secs);
    Ok((a, record, window))
}

fn require_funds(econ: &dyn World, buyer: &Address, price: u64) -> Result<(), String> {
    if econ.balance(buyer) < price {
        return Err(format!(
            "o comprador não tem saldo: precisa de {}, tem {} ({})",
            teko_crypto::amount::format_amount(price),
            teko_crypto::amount::format_amount(econ.balance(buyer)),
            econ.funding_hint()
        ));
    }
    Ok(())
}

/// I (the owner) offer `asset_id` to the simulated counterparty for `price` coins: MY device
/// builds and signs the offer from my wallet; the counterparty (software) accepts at once, so
/// the trade becomes PENDING in the class's veto window. The record must be in the off-chain
/// store AND match the hash on the chain. Finalization is a later, separate step
/// (`finalize_pending`) that anyone can send once the window is over.
pub fn sell_to_counterparty(
    econ: &mut dyn World,
    cast: &Cast,
    me: &Address,
    class: &str,
    asset_id: &str,
    price: u64,
    signer: &mut TxSigner<'_>,
) -> Result<(), String> {
    if price == 0 {
        return Err("informe um preço maior que zero".into());
    }
    let cp = cast.counterparty_address();
    let (a, record, window) = trade_context(econ, cast, class, asset_id, me)?;
    require_funds(econ, &cp, price)?;
    let nonce = econ.nonce(me);
    let fee = device_fee(econ, TradeAction::Offer);
    let chain = econ.chain_id().to_string();
    let tx = signer(&TradeRequest {
        action: TradeAction::Offer,
        chain_id: &chain,
        class,
        asset: asset_id,
        seller: me,
        buyer: &cp,
        price,
        version: a.version,
        metadata: &record,
        window_secs: window,
        nonce,
        fee,
    })?;
    econ.submit(tx)?;
    // the simulated counterparty accepts (its own transaction, from its own wallet)
    let accept = Op::Accept(AcceptOp {
        class: class.into(),
        asset: asset_id.as_bytes().to_vec(),
        price,
        version: a.version,
        metadata_hash: a.metadata_hash.clone(),
    });
    let tx = sign_as(econ, &cast.counterparty, accept);
    econ.submit(tx)
}

/// The simulated counterparty offers me `asset_id` for `price` coins (from its own wallet) and
/// MY device builds and signs the acceptance: the price goes to the escrow and the trade is
/// PENDING in the veto window.
pub fn buy_from_counterparty(
    econ: &mut dyn World,
    cast: &Cast,
    me: &Address,
    class: &str,
    asset_id: &str,
    price: u64,
    signer: &mut TxSigner<'_>,
) -> Result<(), String> {
    if price == 0 {
        return Err("informe um preço maior que zero".into());
    }
    let cp = cast.counterparty_address();
    let (a, record, window) = trade_context(econ, cast, class, asset_id, &cp)?;
    require_funds(econ, me, price)?;
    let offer = Op::Offer(OfferOp {
        class: class.into(),
        asset: asset_id.as_bytes().to_vec(),
        buyer: me.to_vec(),
        price,
        version: a.version,
        metadata_hash: a.metadata_hash.clone(),
    });
    let tx = sign_as(econ, &cast.counterparty, offer);
    econ.submit(tx)?;
    let nonce = econ.nonce(me);
    let fee = device_fee(econ, TradeAction::Accept);
    let chain = econ.chain_id().to_string();
    let signed = signer(&TradeRequest {
        action: TradeAction::Accept,
        chain_id: &chain,
        class,
        asset: asset_id,
        seller: &cp,
        buyer: me,
        price,
        version: a.version,
        metadata: &record,
        window_secs: window,
        nonce,
        fee,
    });
    // if I did not accept (rejected on my device, or the network refused), the simulated
    // counterparty withdraws its offer so the asset is not left blocked by a dead offer
    let result = signed.and_then(|tx| econ.submit(tx));
    if result.is_err() {
        let cancel = Op::Cancel(crate::CancelOp {
            class: class.into(),
            asset: asset_id.as_bytes().to_vec(),
        });
        let tx = sign_as(econ, &cast.counterparty, cancel);
        let _ = econ.submit(tx);
    }
    result
}

/// The simulated registrar quorum VETOES the pending trade of `asset_id` for a public `reason`.
pub fn veto_pending(
    econ: &mut dyn World,
    cast: &Cast,
    class: &str,
    asset_id: &str,
    reason: &str,
) -> Result<(), String> {
    let a =
        asset(econ.ledger(), class.as_bytes(), asset_id.as_bytes()).ok_or("bem desconhecido")?;
    let o = offer_of(econ.ledger(), class.as_bytes(), asset_id.as_bytes())
        .filter(|o| o.accepted)
        .ok_or("não há negociação pendente neste bem")?;
    let i = Intent::veto(
        class.as_bytes(),
        asset_id.as_bytes(),
        a.owner.as_slice().try_into().map_err(|_| "dono inválido")?,
        o.buyer
            .as_slice()
            .try_into()
            .map_err(|_| "comprador inválido")?,
        o.price,
        a.version,
        reason,
    );
    let approvals = quorum(cast, class, econ.chain_id(), &i)?;
    submit_assets_op(
        econ,
        cast,
        Op::Veto(VetoOp {
            class: class.into(),
            asset: asset_id.as_bytes().to_vec(),
            reason: reason.into(),
            approvals,
        }),
    )
}

/// Completes an accepted trade whose veto window is over (anyone may; here the relayer does).
pub fn finalize_pending(
    econ: &mut dyn World,
    cast: &Cast,
    class: &str,
    asset_id: &str,
) -> Result<(), String> {
    submit_assets_op(
        econ,
        cast,
        Op::Finalize(FinalizeOp {
            class: class.into(),
            asset: asset_id.as_bytes().to_vec(),
        }),
    )
}

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

fn quorum(cast: &Cast, class: &str, chain_id: &str, i: &Intent) -> Result<Vec<Approval>, String> {
    Ok(cast
        .registrars_for(class)?
        .iter()
        .take(2)
        .map(|k| Approval::sign(k, chain_id, i))
        .collect())
}

/// The simulated registrar of `class` registers the asset described by `record` (a
/// canonical metadata record, see `teko_crypto::asset_meta`) to `owner`. The chain
/// gets only the record's hash; the record is saved in the off-chain store. Returns the
/// asset id (the chassis of a vehicle, the registration number of a property).
pub fn register(
    econ: &mut dyn World,
    cast: &Cast,
    owner: &Address,
    class: &str,
    record: &[u8],
) -> Result<String, String> {
    let id = asset_meta::asset_id(class, record).map_err(|e| e.to_string())?;
    if asset(econ.ledger(), class.as_bytes(), id.as_bytes()).is_some() {
        return Err(format!("este bem já está registrado ({class}: {id})"));
    }
    let hash = asset_meta::hash(class, record);
    let i = Intent::register(class.as_bytes(), id.as_bytes(), *owner, &hash);
    let approvals = quorum(cast, class, econ.chain_id(), &i)?;
    submit_assets_op(
        econ,
        cast,
        Op::Register(RegisterOp {
            class: class.into(),
            asset: id.as_bytes().to_vec(),
            owner: owner.to_vec(),
            metadata_hash: hash.to_vec(),
            approvals,
        }),
    )?;
    cast.store.put(class, record)?;
    Ok(id)
}

/// Valid TEST record for `class` whose asset id is `id` (a vehicle needs a valid chassis;
/// for a property any registration number works). For tests and demos.
pub fn sample_record(class: &str, id: &str) -> Result<Vec<u8>, String> {
    let mut fields = asset_meta::sample(class, 1).ok_or("classe sem esquema")?;
    let key = asset_meta::id_field(class).ok_or("classe sem esquema")?;
    for (k, v) in &mut fields {
        if k == key {
            *v = id.to_string();
        }
    }
    let pairs: Vec<(&str, &str)> = fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    asset_meta::canonical(class, &pairs).map_err(|e| e.to_string())
}

/// The simulated registrar of `class` adds (`on`) or releases the first lien of
/// `asset_id`.
pub fn set_lien(
    econ: &mut dyn World,
    cast: &Cast,
    class: &str,
    asset_id: &str,
    on: bool,
) -> Result<(), String> {
    let a =
        asset(econ.ledger(), class.as_bytes(), asset_id.as_bytes()).ok_or("bem desconhecido")?;
    let owner: Address = a.owner.as_slice().try_into().map_err(|_| "dono inválido")?;
    if on {
        let i = Intent::add_lien(
            class.as_bytes(),
            asset_id.as_bytes(),
            owner,
            a.version,
            "penhora",
        );
        let ap = quorum(cast, class, econ.chain_id(), &i)?;
        submit_assets_op(
            econ,
            cast,
            Op::AddLien(LienOp {
                class: class.into(),
                asset: asset_id.as_bytes().to_vec(),
                kind: "penhora".into(),
                approvals: ap,
            }),
        )
    } else {
        let lien = a.liens.first().ok_or("o bem não tem gravame")?.id;
        let i = Intent::release_lien(
            class.as_bytes(),
            asset_id.as_bytes(),
            owner,
            a.version,
            lien,
        );
        let ap = quorum(cast, class, econ.chain_id(), &i)?;
        submit_assets_op(
            econ,
            cast,
            Op::ReleaseLien(ReleaseOp {
                class: class.into(),
                asset: asset_id.as_bytes().to_vec(),
                lien_id: lien,
                approvals: ap,
            }),
        )
    }
}

/// A trade in progress, as the screens show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTrade {
    pub class: String,
    pub asset: String,
    pub seller: Address,
    pub buyer: Address,
    pub price: u64,
    pub accepted: bool,
    /// Block time from which anyone may finalize (0 until accepted).
    pub release_at: u64,
}

/// The trades in progress that involve `who` (as seller or buyer).
pub fn pending_trades(econ: &dyn World, who: &Address) -> Vec<PendingTrade> {
    offers(econ.ledger())
        .into_iter()
        .filter_map(|(class, id, o)| trade_view(econ, &class, &id, &o))
        .filter(|t| t.seller == *who || t.buyer == *who)
        .collect()
}

fn trade_view(econ: &dyn World, class: &[u8], id: &[u8], o: &OfferRec) -> Option<PendingTrade> {
    let a = asset(econ.ledger(), class, id)?;
    Some(PendingTrade {
        class: String::from_utf8_lossy(class).into_owned(),
        asset: String::from_utf8_lossy(id).into_owned(),
        seller: a.owner.as_slice().try_into().ok()?,
        buyer: o.buyer.as_slice().try_into().ok()?,
        price: o.price,
        accepted: o.accepted,
        release_at: o.release_at,
    })
}

/// An asset owned by a wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedAsset {
    pub class: String,
    pub id: String,
    pub version: u64,
    pub liens: usize,
}

/// A wallet's assets, in key order.
pub fn owned(econ: &dyn World, who: &Address) -> Vec<OwnedAsset> {
    assets_of(econ.ledger(), who)
        .into_iter()
        .filter_map(|(c, id)| {
            let a = asset(econ.ledger(), &c, &id)?;
            Some(OwnedAsset {
                class: String::from_utf8_lossy(&c).into_owned(),
                id: String::from_utf8_lossy(&id).into_owned(),
                version: a.version,
                liens: a.liens.len(),
            })
        })
        .collect()
}
