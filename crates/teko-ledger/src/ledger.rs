use std::collections::BTreeMap;

use crate::state::State;
use crate::tx::{Address, Hash, Tx, TxError};

/// Upper bound of transactions in one block (bounds execution time).
pub const MAX_TXS_PER_BLOCK: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    WrongHeight {
        expected: u64,
        got: u64,
    },
    TooManyTxs,
    /// Transaction `index` of the block is invalid; the whole block is rejected.
    Tx {
        index: usize,
        error: TxError,
    },
    GenesisClosed,
    ReservedModule,
    DuplicateModule(u32),
    UnknownModule(u32),
    /// A module's `begin_block` failed.
    BeginBlock(String),
}

/// Failure reported by a module while executing a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleError(pub String);

impl ModuleError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Something that happened during execution, for indexers and clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub module: u32,
    pub topic: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub tx_id: Hash,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockOutcome {
    pub height: u64,
    pub state_root: Hash,
    pub receipts: Vec<Receipt>,
}

/// Writes of a block in progress, on top of the committed state.
struct Overlay<'a> {
    base: &'a State,
    writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

impl Overlay<'_> {
    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        match self.writes.get(key) {
            Some(w) => w.clone(),
            None => self.base.get(key).map(<[u8]>::to_vec),
        }
    }
}

fn ns_key(module: u32, key: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(5 + key.len());
    k.push(1);
    k.extend_from_slice(&module.to_be_bytes());
    k.extend_from_slice(key);
    k
}

fn nonce_key(addr: &Address) -> Vec<u8> {
    let mut k = Vec::with_capacity(33);
    k.push(0);
    k.extend_from_slice(addr);
    k
}

fn read_nonce(raw: Option<Vec<u8>>) -> u64 {
    raw.and_then(|v| <[u8; 8]>::try_from(v.as_slice()).ok())
        .map(u64::from_be_bytes)
        .unwrap_or(0)
}

/// What a module sees while executing one transaction. It can WRITE only its own
/// namespace; it can READ other modules' (for example the coin module's balances).
pub struct Ctx<'a, 'b> {
    overlay: &'a mut Overlay<'b>,
    module: u32,
    pub height: u64,
    pub chain_id: &'a str,
    /// Validator that proposed the block being executed (fees and rewards go here).
    pub proposer: Address,
    /// Address of the transaction signer (already authenticated).
    pub sender: Address,
    /// SEC1 public key of the signer (empty in `begin_block`).
    pub sender_key: &'a [u8],
    events: Vec<Event>,
}

impl Ctx<'_, '_> {
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.overlay.get(&ns_key(self.module, key))
    }

    pub fn set(&mut self, key: &[u8], value: Vec<u8>) {
        self.overlay
            .writes
            .insert(ns_key(self.module, key), Some(value));
    }

    pub fn delete(&mut self, key: &[u8]) {
        self.overlay.writes.insert(ns_key(self.module, key), None);
    }

    /// Read-only access to another module's namespace.
    pub fn read_module(&self, module: u32, key: &[u8]) -> Option<Vec<u8>> {
        self.overlay.get(&ns_key(module, key))
    }

    pub fn emit(&mut self, topic: &str, data: Vec<u8>) {
        self.events.push(Event {
            module: self.module,
            topic: topic.into(),
            data,
        });
    }
}

/// A pluggable rule set (coin, assets, ...). Must be DETERMINISTIC: no clocks, no
/// randomness, no I/O, no hash-map iteration order.
pub trait Module {
    /// Non-zero module id carried in `Tx::module`.
    fn id(&self) -> u32;
    fn execute(&self, ctx: &mut Ctx<'_, '_>, payload: &[u8]) -> Result<(), ModuleError>;

    /// Runs once per block, before its transactions (block rewards). `ctx.sender`
    /// is the proposer.
    fn begin_block(&self, _ctx: &mut Ctx<'_, '_>) -> Result<(), ModuleError> {
        Ok(())
    }

    /// Minimum fee for a transaction of `tx_len` encoded bytes (fee module only).
    fn min_fee(&self, _ctx: &Ctx<'_, '_>, _tx_len: usize) -> u64 {
        0
    }

    /// Charges `fee` from `ctx.sender` (fee module only). Failing here rejects the
    /// transaction, for example when the balance does not cover it.
    fn charge_fee(&self, _ctx: &mut Ctx<'_, '_>, _fee: u64) -> Result<(), ModuleError> {
        Ok(())
    }
}

pub struct Ledger {
    chain_id: String,
    height: u64,
    state: State,
    modules: BTreeMap<u32, Box<dyn Module>>,
    fee_module: Option<u32>,
}

impl Ledger {
    pub fn new(chain_id: &str, modules: Vec<Box<dyn Module>>) -> Result<Self, LedgerError> {
        let mut map = BTreeMap::new();
        for m in modules {
            let id = m.id();
            if id == 0 {
                return Err(LedgerError::ReservedModule);
            }
            if map.insert(id, m).is_some() {
                return Err(LedgerError::DuplicateModule(id));
            }
        }
        Ok(Self {
            chain_id: chain_id.into(),
            height: 0,
            state: State::new(),
            modules: map,
            fee_module: None,
        })
    }

    /// Designates the module that prices and collects fees (usually the coin).
    pub fn with_fee_module(mut self, id: u32) -> Result<Self, LedgerError> {
        if !self.modules.contains_key(&id) {
            return Err(LedgerError::UnknownModule(id));
        }
        self.fee_module = Some(id);
        Ok(self)
    }

    /// All entries of a module whose key starts with `prefix` (keys without the
    /// namespace), in key order. For queries such as "list the validators".
    pub fn scan(&self, module: u32, prefix: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let full = ns_key(module, prefix);
        let skip = 5;
        self.state
            .iter()
            .filter(|(k, _)| k.starts_with(&full))
            .filter_map(|(k, v)| k.get(skip..).map(|rest| (rest.to_vec(), v.to_vec())))
            .collect()
    }

    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// Height of the last applied block (0 = only genesis).
    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn state_root(&self) -> Hash {
        self.state.root()
    }

    /// Number of transactions already executed by `addr` (the next valid nonce).
    pub fn nonce_of(&self, addr: &Address) -> u64 {
        read_nonce(self.state.get(&nonce_key(addr)).map(<[u8]>::to_vec))
    }

    /// Reads a module's key from the committed state.
    pub fn read(&self, module: u32, key: &[u8]) -> Option<&[u8]> {
        self.state.get(&ns_key(module, key))
    }

    /// Initial state, allowed only before the first block.
    pub fn genesis_put(
        &mut self,
        module: u32,
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), LedgerError> {
        if self.height != 0 {
            return Err(LedgerError::GenesisClosed);
        }
        self.state.put(ns_key(module, key), value);
        Ok(())
    }

    /// Mempool admission against the committed state (signature, chain, nonce, module).
    pub fn check_tx(&self, tx: &Tx) -> Result<(), TxError> {
        tx.verify()?;
        if tx.chain_id != self.chain_id {
            return Err(TxError::WrongChain);
        }
        let expected = self.nonce_of(&tx.address());
        if tx.nonce != expected {
            return Err(TxError::BadNonce {
                expected,
                got: tx.nonce,
            });
        }
        if !self.modules.contains_key(&tx.module) {
            return Err(TxError::UnknownModule(tx.module));
        }
        if let Some(fm) = self.fee_module.and_then(|id| self.modules.get(&id)) {
            let mut overlay = Overlay {
                base: &self.state,
                writes: BTreeMap::new(),
            };
            let ctx = Ctx {
                overlay: &mut overlay,
                module: self.fee_module.unwrap_or(0),
                height: self.height,
                chain_id: &self.chain_id,
                proposer: [0; 32],
                sender: tx.address(),
                sender_key: &tx.sender,
                events: Vec::new(),
            };
            let min = fm.min_fee(&ctx, tx.to_bytes().len());
            if tx.fee < min {
                return Err(TxError::FeeTooLow { min, got: tx.fee });
            }
        }
        Ok(())
    }

    fn run(
        &self,
        overlay: &mut Overlay<'_>,
        height: u64,
        proposer: &Address,
        tx: &Tx,
    ) -> Result<Receipt, TxError> {
        tx.verify()?;
        if tx.chain_id != self.chain_id {
            return Err(TxError::WrongChain);
        }
        let sender = tx.address();
        let nk = nonce_key(&sender);
        let expected = read_nonce(overlay.get(&nk));
        if tx.nonce != expected {
            return Err(TxError::BadNonce {
                expected,
                got: tx.nonce,
            });
        }
        let next = expected.checked_add(1).ok_or(TxError::BadNonce {
            expected,
            got: tx.nonce,
        })?;
        let module = self
            .modules
            .get(&tx.module)
            .ok_or(TxError::UnknownModule(tx.module))?;
        overlay.writes.insert(nk, Some(next.to_be_bytes().to_vec()));
        let mut events = Vec::new();
        if let Some((fid, fm)) = self
            .fee_module
            .and_then(|id| self.modules.get(&id).map(|m| (id, m)))
        {
            let mut ctx = Ctx {
                overlay,
                module: fid,
                height,
                chain_id: &self.chain_id,
                proposer: *proposer,
                sender,
                sender_key: &tx.sender,
                events: Vec::new(),
            };
            let min = fm.min_fee(&ctx, tx.to_bytes().len());
            if tx.fee < min {
                return Err(TxError::FeeTooLow { min, got: tx.fee });
            }
            fm.charge_fee(&mut ctx, tx.fee)
                .map_err(|e| TxError::Module(e.0))?;
            events.append(&mut ctx.events);
        }
        let mut ctx = Ctx {
            overlay,
            module: tx.module,
            height,
            chain_id: &self.chain_id,
            proposer: *proposer,
            sender,
            sender_key: &tx.sender,
            events: Vec::new(),
        };
        module
            .execute(&mut ctx, &tx.payload)
            .map_err(|e| TxError::Module(e.0))?;
        events.append(&mut ctx.events);
        Ok(Receipt {
            tx_id: tx.id(),
            events,
        })
    }

    fn begin_block(
        &self,
        overlay: &mut Overlay<'_>,
        height: u64,
        proposer: &Address,
    ) -> Result<Vec<Event>, LedgerError> {
        let mut events = Vec::new();
        for (id, m) in &self.modules {
            let mut ctx = Ctx {
                overlay,
                module: *id,
                height,
                chain_id: &self.chain_id,
                proposer: *proposer,
                sender: *proposer,
                sender_key: &[],
                events: Vec::new(),
            };
            m.begin_block(&mut ctx)
                .map_err(|e| LedgerError::BeginBlock(e.0))?;
            events.append(&mut ctx.events);
        }
        Ok(events)
    }

    /// Proposer helper: executes `txs` in order on the next height and keeps only the
    /// ones that succeed (a block with an invalid transaction is rejected whole).
    pub fn select_valid(&self, proposer: &Address, txs: Vec<Tx>) -> Vec<Tx> {
        let height = self.height.saturating_add(1);
        let mut overlay = Overlay {
            base: &self.state,
            writes: BTreeMap::new(),
        };
        let mut kept = Vec::new();
        for tx in txs.into_iter().take(MAX_TXS_PER_BLOCK) {
            let snapshot = overlay.writes.clone();
            match self.run(&mut overlay, height, proposer, &tx) {
                Ok(_) => kept.push(tx),
                Err(_) => overlay.writes = snapshot,
            }
        }
        kept
    }

    /// Executes a block ATOMICALLY: either every transaction applies and the height
    /// advances, or nothing changes.
    pub fn apply_block(
        &mut self,
        height: u64,
        proposer: &Address,
        txs: &[Tx],
    ) -> Result<BlockOutcome, LedgerError> {
        let expected = self.height.saturating_add(1);
        if height != expected {
            return Err(LedgerError::WrongHeight {
                expected,
                got: height,
            });
        }
        if txs.len() > MAX_TXS_PER_BLOCK {
            return Err(LedgerError::TooManyTxs);
        }
        let (writes, receipts) = {
            let mut overlay = Overlay {
                base: &self.state,
                writes: BTreeMap::new(),
            };
            let mut receipts = Vec::with_capacity(txs.len());
            self.begin_block(&mut overlay, height, proposer)?;
            for (index, tx) in txs.iter().enumerate() {
                let r = self
                    .run(&mut overlay, height, proposer, tx)
                    .map_err(|error| LedgerError::Tx { index, error })?;
                receipts.push(r);
            }
            (overlay.writes, receipts)
        };
        for (k, v) in writes {
            match v {
                Some(v) => self.state.put(k, v),
                None => self.state.remove(&k),
            }
        }
        self.height = height;
        Ok(BlockOutcome {
            height,
            state_root: self.state.root(),
            receipts,
        })
    }

    /// Same as `apply_block`, decoding canonical transaction bytes first.
    pub fn apply_block_bytes(
        &mut self,
        height: u64,
        proposer: &Address,
        raw: &[Vec<u8>],
    ) -> Result<BlockOutcome, LedgerError> {
        let mut txs = Vec::with_capacity(raw.len());
        for (index, bytes) in raw.iter().enumerate() {
            txs.push(Tx::from_bytes(bytes).map_err(|error| LedgerError::Tx { index, error })?);
        }
        self.apply_block(height, proposer, &txs)
    }
}
