use std::collections::{BTreeMap, BTreeSet, HashMap};

use prost::Message;
use teko_bft::block_id;
use teko_coin::COIN_MODULE;
use teko_crypto::{hex, parse_wallet_address, wallet_address};
use teko_economy::{DecidedBlock, EconError, EconomyChain, EconomyGenesis};
use teko_ledger::{Address, Ledger, Tx};

use crate::model::*;

/// Why a block was refused (the index keeps the chain it had).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerError {
    Genesis(String),
    /// The block failed verification (certificate, proposer, execution or state root).
    Block {
        height: u64,
        why: String,
    },
}

impl core::fmt::Display for ExplorerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ExplorerError::Genesis(w) => write!(f, "genesis inválida: {w}"),
            ExplorerError::Block { height, why } => {
                write!(f, "bloco {height} recusado na verificação: {why}")?;
                if why.starts_with("BadTx(") {
                    f.write_str(
                        " — a cadeia não é compatível com esta versão do software (o formato mudou, por exemplo ADR-0032): recrie a economia de teste com `make devnet-stop`, `make devnet-founder FOUNDER=<seu tekow1…>` e `make devnet`",
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ExplorerError {}

/// Most items a list query returns.
pub const MAX_LIMIT: usize = 100;

pub struct Index {
    chain: EconomyChain,
    blocks: Vec<BlockView>,
    txs: Vec<TxView>,
    tx_by_id: HashMap<String, usize>,
    block_by_hash: HashMap<String, u64>,
    /// Every address that appeared (sender, recipient, reward owner, genesis).
    known: BTreeSet<Address>,
    by_account: BTreeMap<Address, Vec<usize>>,
    events: Vec<EventView>,
}

fn addr_text(a: &Address) -> String {
    wallet_address(a).unwrap_or_else(|_| hex::encode(a))
}

/// A readable label for class names and asset ids (text when printable, else hex).
fn label(bytes: &[u8]) -> String {
    match core::str::from_utf8(bytes) {
        Ok(s) if !s.is_empty() && s.chars().all(|c| !c.is_control()) => s.to_string(),
        _ => hex::encode(bytes),
    }
}

fn module_name(m: u32) -> String {
    if m == COIN_MODULE {
        "coin".into()
    } else if m == teko_assets::ASSETS_MODULE {
        "assets".into()
    } else {
        format!("module-{m}")
    }
}

fn to_address(bytes: &[u8]) -> Option<Address> {
    Address::try_from(bytes).ok()
}

fn kind_of(tx: &Tx) -> TxKind {
    if tx.module == COIN_MODULE {
        return match teko_coin::Op::decode(&tx.payload) {
            Ok(teko_coin::Op::Transfer { to, amount }) => TxKind::Transfer {
                to: addr_text(&to),
                amount: Amount::new(amount),
            },
            Ok(teko_coin::Op::Stake { amount }) => TxKind::Stake {
                amount: Amount::new(amount),
            },
            Ok(teko_coin::Op::Unstake { amount }) => TxKind::Unstake {
                amount: Amount::new(amount),
            },
            Ok(teko_coin::Op::Claim) => TxKind::Claim,
            Ok(teko_coin::Op::Slash { evidence }) => TxKind::Slash {
                validator: teko_bft::Evidence::decode(evidence.as_slice())
                    .ok()
                    .and_then(|e| e.validator().map(hex::encode)),
            },
            Ok(teko_coin::Op::Faucet) => TxKind::Faucet,
            Ok(teko_coin::Op::SetValidatorKey { key }) => TxKind::SetValidatorKey {
                key: hex::encode(&key),
            },
            Err(_) => TxKind::Unknown {
                module: tx.module,
                payload_len: tx.payload.len(),
            },
        };
    }
    if tx.module == teko_assets::ASSETS_MODULE {
        use teko_assets::Op;
        let parsed = match Op::decode(&tx.payload) {
            Some(Op::Register(m)) => Some(("register", m.class, m.asset)),
            Some(Op::Transfer(m)) => Some(("transfer", m.class, m.asset)),
            Some(Op::Swap(m)) => Some(("swap", m.class, m.asset)),
            Some(Op::AddLien(m)) => Some(("add_lien", m.class, m.asset)),
            Some(Op::ReleaseLien(m)) => Some(("release_lien", m.class, m.asset)),
            Some(Op::UpdateRegistrars(m)) => Some(("update_registrars", m.class, Vec::new())),
            None => None,
        };
        if let Some((op, class, asset)) = parsed {
            return TxKind::Assets {
                op: op.into(),
                class: label(&class),
                asset: label(&asset),
            };
        }
    }
    TxKind::Unknown {
        module: tx.module,
        payload_len: tx.payload.len(),
    }
}

impl Index {
    pub fn new(genesis: EconomyGenesis) -> Result<Self, ExplorerError> {
        let mut known = BTreeSet::new();
        for (_, owner) in &genesis.founders {
            known.insert(*owner);
        }
        for (a, _) in &genesis.allocations {
            known.insert(*a);
        }
        let chain =
            EconomyChain::new(genesis).map_err(|e| ExplorerError::Genesis(format!("{e:?}")))?;
        Ok(Self {
            chain,
            blocks: Vec::new(),
            txs: Vec::new(),
            tx_by_id: HashMap::new(),
            block_by_hash: HashMap::new(),
            known,
            by_account: BTreeMap::new(),
            events: Vec::new(),
        })
    }

    pub fn height(&self) -> u64 {
        self.chain.height()
    }

    pub fn chain(&self) -> &EconomyChain {
        &self.chain
    }

    fn ledger(&self) -> &Ledger {
        self.chain.ledger()
    }

    /// Verifies and indexes the blocks after the current height. `blocks` is the whole
    /// chain from height 1 (or at least up to the current height); only the new ones
    /// are processed. A bad block stops the sync and leaves the index as it was.
    /// Returns how many blocks were added.
    pub fn sync(&mut self, blocks: &[DecidedBlock]) -> Result<usize, ExplorerError> {
        let start = usize::try_from(self.height()).unwrap_or(usize::MAX);
        let mut added = 0;
        for d in blocks.get(start..).unwrap_or(&[]) {
            self.apply(d.clone())?;
            added += 1;
        }
        Ok(added)
    }

    /// Verifies and indexes `new` blocks, which must start right after the current
    /// height (as a node returns them for `from = height + 1`). Returns how many were
    /// added; a bad block stops the run and leaves the index as it was before it.
    pub fn extend(&mut self, new: &[DecidedBlock]) -> Result<usize, ExplorerError> {
        for d in new {
            self.apply(d.clone())?;
        }
        Ok(new.len())
    }

    fn apply(&mut self, d: DecidedBlock) -> Result<(), ExplorerError> {
        let height = self.height() + 1;
        let bad = |e: EconError| ExplorerError::Block {
            height,
            why: format!("{e:?}"),
        };
        let block = d.block().map_err(bad)?;
        let owner = self.chain.owner_of(&block.proposer);
        let minted_before = teko_coin::minted(self.ledger());
        let out = self.chain.apply_decided_outcome(d.clone()).map_err(bad)?;
        let reward = teko_coin::minted(self.ledger()).saturating_sub(minted_before);
        let hash = hex::encode(&block_id(&d.payload));
        let (round, precommits) = d
            .commit
            .as_ref()
            .map_or((0, 0), |c| (c.round, c.precommits.len()));
        self.known.insert(owner);

        let mut fees = 0u64;
        let mut views = Vec::with_capacity(block.txs.len());
        for (i, raw) in block.txs.iter().enumerate() {
            let Ok(tx) = Tx::from_bytes(raw) else {
                continue;
            };
            fees = fees.saturating_add(tx.fee);
            let id = hex::encode(&tx.id());
            let sender = tx.address();
            self.known.insert(sender);
            let kind = kind_of(&tx);
            if let TxKind::Transfer { .. } = &kind
                && let Ok(teko_coin::Op::Transfer { to, .. }) = teko_coin::Op::decode(&tx.payload)
            {
                self.known.insert(to);
                self.by_account.entry(to).or_default().push(self.txs.len());
            }
            let mut events = Vec::new();
            if let Some(r) = out.receipts.get(i) {
                for ev in &r.events {
                    let e = EventView {
                        height,
                        tx_id: id.clone(),
                        module: module_name(ev.module),
                        topic: ev.topic.clone(),
                        data_hex: hex::encode(&ev.data),
                    };
                    self.events.push(e.clone());
                    events.push(e);
                }
            }
            let view = TxView {
                id: id.clone(),
                height,
                index: i,
                timestamp: block.timestamp,
                sender: addr_text(&sender),
                module: module_name(tx.module),
                nonce: tx.nonce,
                fee: Amount::new(tx.fee),
                kind,
                events,
            };
            self.tx_by_id.insert(id, self.txs.len());
            self.by_account
                .entry(sender)
                .or_default()
                .push(self.txs.len());
            self.txs.push(view.clone());
            views.push(view);
        }
        self.block_by_hash.insert(hash.clone(), height);
        self.blocks.push(BlockView {
            height,
            hash,
            prev_hash: hex::encode(&block.prev_id),
            state_root: hex::encode(&block.state_root),
            timestamp: block.timestamp,
            proposer_key: hex::encode(&block.proposer),
            proposer_owner: addr_text(&owner),
            reward: Amount::new(reward),
            fees: Amount::new(fees),
            round,
            precommits,
            txs: views,
        });
        Ok(())
    }

    // ---------- queries ----------

    pub fn status(&self) -> StatusView {
        let l = self.ledger();
        let cfg = teko_coin::config(l);
        let minted = teko_coin::minted(l);
        let (cap, target, next) = cfg.as_ref().map_or((0, 0, 0), |c| {
            let r = teko_coin::reward_at(c, self.height() + 1);
            (c.cap, c.founder_target, r.min(c.cap.saturating_sub(minted)))
        });
        StatusView {
            chain_id: self.chain.chain_id().to_string(),
            height: self.height(),
            tip_hash: self
                .blocks
                .last()
                .map(|b| b.hash.clone())
                .unwrap_or_default(),
            state_root: hex::encode(&l.state_root()),
            minted: Amount::new(minted),
            burned: Amount::new(teko_coin::burned(l)),
            cap: Amount::new(cap),
            total_stake: Amount::new(teko_coin::total_stake(l)),
            next_reward: Amount::new(next),
            founder_phase_active: teko_coin::founder_phase_active(l),
            founder_target: Amount::new(target),
            validators: teko_coin::validator_set(l).len(),
            tx_total: self.txs.len(),
            verified: true,
        }
    }

    /// Newest first. `before` = list only blocks below this height.
    pub fn blocks(&self, before: Option<u64>, limit: usize) -> Vec<BlockSummary> {
        let top = before.map_or(self.blocks.len(), |b| {
            usize::try_from(b.saturating_sub(1))
                .unwrap_or(usize::MAX)
                .min(self.blocks.len())
        });
        self.blocks
            .get(..top)
            .unwrap_or(&[])
            .iter()
            .rev()
            .take(limit.min(MAX_LIMIT))
            .map(|b| BlockSummary {
                height: b.height,
                hash: b.hash.clone(),
                timestamp: b.timestamp,
                proposer_owner: b.proposer_owner.clone(),
                tx_count: b.txs.len(),
                reward: b.reward.clone(),
            })
            .collect()
    }

    pub fn block(&self, height: u64) -> Option<&BlockView> {
        let i = usize::try_from(height.checked_sub(1)?).ok()?;
        self.blocks.get(i)
    }

    pub fn block_by_hash(&self, hash: &str) -> Option<&BlockView> {
        self.block(*self.block_by_hash.get(&hash.to_ascii_lowercase())?)
    }

    pub fn tx(&self, id: &str) -> Option<&TxView> {
        self.txs.get(*self.tx_by_id.get(&id.to_ascii_lowercase())?)
    }

    /// Newest transactions first.
    pub fn latest_txs(&self, limit: usize) -> Vec<TxView> {
        self.txs
            .iter()
            .rev()
            .take(limit.min(MAX_LIMIT))
            .cloned()
            .collect()
    }

    pub fn account(&self, address: &str) -> Option<AccountView> {
        let a = parse_wallet_address(address).ok()?;
        let l = self.ledger();
        let idxs = self.by_account.get(&a).map_or(&[][..], Vec::as_slice);
        let assets = teko_assets::assets_of(l, &a)
            .into_iter()
            .map(|(class, id)| self.asset_view(&class, &id, &a))
            .collect();
        Some(AccountView {
            address: addr_text(&a),
            known: self.known.contains(&a),
            balance: Amount::new(teko_coin::balance(l, &a)),
            stake: Amount::new(teko_coin::stake_of(l, &a)),
            unbonding: teko_coin::unbonding_of(l, &a)
                .into_iter()
                .map(|(h, v)| Unbonding {
                    release_height: h,
                    amount: Amount::new(v),
                })
                .collect(),
            nonce: l.nonce_of(&a),
            assets,
            tx_count: idxs.len(),
            recent_txs: idxs
                .iter()
                .rev()
                .take(50)
                .filter_map(|i| self.txs.get(*i).cloned())
                .collect(),
        })
    }

    pub fn validators(&self) -> Vec<ValidatorView> {
        let l = self.ledger();
        let powers: HashMap<Vec<u8>, u64> = self
            .chain
            .validator_set()
            .map(|s| {
                s.validators()
                    .iter()
                    .map(|v| (v.key.clone(), v.power))
                    .collect()
            })
            .unwrap_or_default();
        teko_coin::validator_set(l)
            .into_iter()
            .map(|v| ValidatorView {
                power: powers.get(&v.key).copied().unwrap_or(0),
                key: hex::encode(&v.key),
                owner: addr_text(&v.owner),
                stake: Amount::new(v.stake),
            })
            .collect()
    }

    pub fn classes(&self) -> Vec<ClassView> {
        self.chain
            .genesis()
            .classes
            .iter()
            .filter_map(|c| {
                let rec = teko_assets::class_info(self.ledger(), c.name.as_bytes())?;
                Some(ClassView {
                    name: c.name.clone(),
                    threshold: rec.threshold,
                    registrars: rec.registrars.len(),
                    version: rec.version,
                })
            })
            .collect()
    }

    /// One asset by class and id (the chain keeps only the hash of its metadata).
    pub fn asset(&self, class: &str, id: &str) -> Option<AssetView> {
        let rec = teko_assets::asset(self.ledger(), class.as_bytes(), id.as_bytes())?;
        let owner = to_address(&rec.owner)?;
        Some(self.asset_view(class.as_bytes(), id.as_bytes(), &owner))
    }

    fn asset_view(&self, class: &[u8], id: &[u8], owner: &Address) -> AssetView {
        let rec = teko_assets::asset(self.ledger(), class, id);
        AssetView {
            class: label(class),
            id: label(id),
            owner: addr_text(owner),
            version: rec.as_ref().map_or(0, |r| r.version),
            metadata_hash: rec
                .as_ref()
                .map(|r| hex::encode(&r.metadata_hash))
                .unwrap_or_default(),
            liens: rec
                .map(|r| {
                    r.liens
                        .iter()
                        .map(|x| format!("{}#{}", x.kind, x.id))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Every asset owned by an address that appeared in the chain.
    pub fn assets(&self) -> Vec<AssetView> {
        let l = self.ledger();
        let mut out = Vec::new();
        for a in &self.known {
            for (class, id) in teko_assets::assets_of(l, a) {
                out.push(self.asset_view(&class, &id, a));
            }
        }
        out
    }

    /// Newest first; `topic` filters (for example `slashed`).
    pub fn events(&self, topic: Option<&str>, limit: usize) -> Vec<EventView> {
        self.events
            .iter()
            .rev()
            .filter(|e| topic.is_none_or(|t| e.topic == t))
            .take(limit.min(MAX_LIMIT))
            .cloned()
            .collect()
    }

    /// Searches a height, a block hash, a transaction id, a `tekow1…` address or a
    /// validator (consensus) key.
    pub fn search(&self, q: &str) -> SearchResult {
        let q = q.trim();
        if q.is_empty() {
            return SearchResult::NotFound;
        }
        if q.bytes().all(|b| b.is_ascii_digit())
            && let Some(b) = q.parse::<u64>().ok().and_then(|h| self.block(h))
        {
            return SearchResult::Block { height: b.height };
        }
        if let Some(t) = self.tx(q) {
            return SearchResult::Tx { id: t.id.clone() };
        }
        if let Some(b) = self.block_by_hash(q) {
            return SearchResult::Block { height: b.height };
        }
        if let Some(acc) = self.account(q) {
            return SearchResult::Account {
                address: acc.address,
            };
        }
        if let Ok(key) = hex::decode(q)
            && let Some(v) = teko_coin::validator_set(self.ledger())
                .into_iter()
                .find(|v| v.key == key)
        {
            return SearchResult::Validator {
                key: hex::encode(&v.key),
                owner: addr_text(&v.owner),
            };
        }
        SearchResult::NotFound
    }

    /// Supply conservation check (created − burned = balances + stake + unbonding).
    pub fn audit(&self) -> Result<(), String> {
        teko_coin::audit(self.ledger())
    }

    /// The owner address of a consensus key, as text.
    pub fn owner_text(&self, key: &[u8]) -> String {
        addr_text(&self.chain.owner_of(key))
    }

    pub fn raw_address(&self, text: &str) -> Option<Address> {
        parse_wallet_address(text)
            .ok()
            .or_else(|| to_address(&hex::decode(text).ok()?))
    }
}
