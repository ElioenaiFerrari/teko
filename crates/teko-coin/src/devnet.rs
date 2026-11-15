//! Local DEVNET economy (development only): a text file of genesis allocations of TEST
//! coins, so the Suite can show a wallet balance before the economy ledger runs on the
//! network (M6). Format: one `tekow1… amount` pair per line; `#` starts a comment.
//! Nothing here exists on a real network: allocations are refused outside the devnet.

use std::fs;
use std::io::Write;
use std::path::Path;

use teko_crypto::{hex, parse_wallet_address};
use teko_ledger::{Address, Ledger, Tx};

use crate::{COIN_MODULE, CoinConfig, Op, balance, genesis, module};

/// File name inside the dev directory.
pub const ECONOMY_FILE: &str = "economy.txt";
/// Accepted transactions, one hex-encoded `Tx` per line, in order.
pub const TXS_FILE: &str = "economy.txs";
/// Network id of the local devnet economy (signed into every transaction).
pub const CHAIN_ID: &str = "teko-devnet";
/// Receives the fees and rewards of the local devnet.
pub const PROPOSER: Address = [0xEE; 32];

/// Parameters of the devnet economy (no rewards: only the listed allocations exist).
pub fn devnet_config() -> CoinConfig {
    CoinConfig {
        cap: 21_000_000 * teko_crypto::amount::YPY_PER_TEKO,
        initial_reward: 0,
        halving_interval: 1_000_000,
        fee_per_byte: 1,
        unbonding_period: 3,
        slash_bps: 1000,
        faucet_amount: 0,
        faucet_cooldown: 0,
        ..Default::default()
    }
}

/// Reads the allocations. A missing file means "no test coins" (empty list).
pub fn read_allocations(path: &Path) -> Result<Vec<(Address, u64)>, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("não consegui ler {}: {e}", path.display())),
    };
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(addr), Some(amt), None) = (it.next(), it.next(), it.next()) else {
            return Err(format!("linha {}: esperado `endereço quantidade`", n + 1));
        };
        let a = parse_wallet_address(addr)
            .map_err(|_| format!("linha {}: endereço inválido", n + 1))?;
        let v: u64 = amt
            .parse()
            .map_err(|_| format!("linha {}: quantidade inválida", n + 1))?;
        out.push((a, v));
    }
    Ok(out)
}

/// Appends an allocation of TEST coins (validates the address checksum).
pub fn append_allocation(path: &Path, address: &str, amount: u64) -> Result<(), String> {
    parse_wallet_address(address)
        .map_err(|_| "endereço inválido (confira o checksum)".to_string())?;
    if amount == 0 {
        return Err("a quantidade deve ser positiva".into());
    }
    let mut all = read_allocations(path)?;
    all.push((
        parse_wallet_address(address).map_err(|_| "endereço inválido".to_string())?,
        amount,
    ));
    // Refuse what the genesis would refuse (cap exceeded), before writing.
    balance_of_allocations(&all, &[0; 32])?;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("não consegui gravar {}: {e}", path.display()))?;
    writeln!(f, "{address} {amount}  # moeda de TESTE (devnet)")
        .map_err(|e| format!("falha ao gravar: {e}"))
}

/// The coin-only devnet ledger (genesis + allocations).
pub fn build_ledger(all: &[(Address, u64)]) -> Result<Ledger, String> {
    let mut l = Ledger::new(CHAIN_ID, vec![module()])
        .map_err(|e| format!("{e:?}"))?
        .with_fee_module(COIN_MODULE)
        .map_err(|e| format!("{e:?}"))?;
    genesis(&mut l, devnet_config(), all, true).map_err(|e| format!("{e:?}"))?;
    Ok(l)
}

fn balance_of_allocations(all: &[(Address, u64)], who: &Address) -> Result<u64, String> {
    Ok(balance(&build_ledger(all)?, who))
}

/// A confirmed transfer, for the wallet history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRecord {
    pub height: u64,
    pub from: Address,
    pub to: Address,
    pub amount: u64,
    pub fee: u64,
}

/// The local devnet economy: genesis allocations plus the accepted transactions,
/// replayed through the real `Ledger` and coin module (one transaction per block).
/// DEVELOPMENT ONLY: the economy ledger is not on the network yet (M6).
pub struct Economy {
    ledger: Ledger,
    txs: Vec<Tx>,
    path: std::path::PathBuf,
}

impl Economy {
    /// Opens (replaying everything) the economy stored in `dir` (coin only).
    pub fn open(dir: &Path) -> Result<Economy, String> {
        Self::open_with(dir, build_ledger)
    }

    /// Like `open`, with a caller-provided ledger builder (more modules, extra genesis).
    /// The builder receives the allocations read from `economy.txt`.
    pub fn open_with(
        dir: &Path,
        build: impl FnOnce(&[(Address, u64)]) -> Result<Ledger, String>,
    ) -> Result<Economy, String> {
        let allocs = read_allocations(&dir.join(ECONOMY_FILE))?;
        let mut ledger = build(&allocs)?;
        let path = dir.join(TXS_FILE);
        let mut txs = Vec::new();
        if let Ok(text) = fs::read_to_string(&path) {
            for (n, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let bytes =
                    hex::decode(line).map_err(|_| format!("{TXS_FILE}:{}: hex inválido", n + 1))?;
                let tx = Tx::from_bytes(&bytes)
                    .map_err(|_| format!("{TXS_FILE}:{}: transação inválida", n + 1))?;
                let h = ledger.height() + 1;
                ledger
                    .apply_block(h, &PROPOSER, std::slice::from_ref(&tx))
                    .map_err(|e| format!("{TXS_FILE}:{}: {e:?}", n + 1))?;
                txs.push(tx);
            }
        }
        Ok(Economy { ledger, txs, path })
    }

    pub fn balance(&self, who: &Address) -> u64 {
        balance(&self.ledger, who)
    }

    /// The underlying ledger (read-only), for module queries.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// The accepted transactions, in order.
    pub fn txs(&self) -> &[Tx] {
        &self.txs
    }

    /// The next valid nonce of `who`.
    pub fn nonce(&self, who: &Address) -> u64 {
        self.ledger.nonce_of(who)
    }

    /// A fee that is guaranteed to cover the price per byte of a transfer from
    /// `sender_key` (SEC1 public key) to `to`.
    pub fn estimate_fee(&self, sender_key: &[u8], nonce: u64, to: &Address, amount: u64) -> u64 {
        let price = crate::config(&self.ledger)
            .map(|c| c.fee_per_byte)
            .unwrap_or(0);
        let probe = Tx {
            chain_id: CHAIN_ID.into(),
            nonce,
            module: COIN_MODULE,
            payload: Op::Transfer { to: *to, amount }.encode(),
            sender: sender_key.to_vec(),
            signature: vec![0u8; 64],
            fee: u64::MAX, // longest varint: the real transaction is never longer
        };
        price.saturating_mul(probe.to_bytes().len() as u64)
    }

    /// Validates `tx` against the current state and, if it applies, stores it.
    pub fn submit(&mut self, tx: Tx) -> Result<(), String> {
        let h = self.ledger.height() + 1;
        self.ledger
            .apply_block(h, &PROPOSER, std::slice::from_ref(&tx))
            .map_err(|e| describe(&format!("{e:?}")))?;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("não consegui gravar {}: {e}", self.path.display()))?;
        writeln!(f, "{}", hex::encode(&tx.to_bytes()))
            .map_err(|e| format!("falha ao gravar: {e}"))?;
        self.txs.push(tx);
        Ok(())
    }

    /// Transfers that involve `who`, oldest first.
    pub fn history(&self, who: &Address) -> Vec<TransferRecord> {
        history_from_txs(
            self.txs.iter().enumerate().map(|(i, t)| (i as u64 + 1, t)),
            who,
        )
    }
}

/// The coin transfers among `txs` (each with the height of its block) that involve `who`.
pub fn history_from_txs<'a>(
    txs: impl Iterator<Item = (u64, &'a Tx)>,
    who: &Address,
) -> Vec<TransferRecord> {
    txs.filter_map(|(height, tx)| {
        if tx.module != COIN_MODULE {
            return None;
        }
        let (op, rest) = tx.payload.split_first()?;
        if *op != 0 || rest.len() != 40 {
            return None;
        }
        let to: Address = rest.get(..32)?.try_into().ok()?;
        let amount = u64::from_be_bytes(rest.get(32..)?.try_into().ok()?);
        let from = tx.address();
        (from == *who || to == *who).then_some(TransferRecord {
            height,
            from,
            to,
            amount,
            fee: tx.fee,
        })
    })
    .collect()
}

fn describe(debug: &str) -> String {
    if debug.contains("insufficient funds") {
        "saldo insuficiente".into()
    } else if debug.contains("BadNonce") {
        "transação fora de ordem (nonce): atualize e tente de novo".into()
    } else if debug.contains("FeeTooLow") {
        "taxa abaixo do mínimo".into()
    } else if debug.contains("BadSignature") {
        "assinatura inválida".into()
    } else {
        format!("recusada pelo livro: {debug}")
    }
}

/// Balance of `address` according to the devnet economy in `path`'s directory
/// (None = no economy file).
pub fn balance_of(path: &Path, address: &str) -> Result<Option<u64>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let who = parse_wallet_address(address).map_err(|_| "endereço inválido".to_string())?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(Some(Economy::open(dir)?.balance(&who)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use teko_crypto::wallet_address;

    #[test]
    fn allocations_roundtrip_and_balance() {
        let p = tmpdir("c").join(ECONOMY_FILE);
        let a = wallet_address(&[1; 32]).unwrap();
        let b = wallet_address(&[2; 32]).unwrap();
        assert_eq!(balance_of(&p, &a).unwrap(), None);
        append_allocation(&p, &a, 700).unwrap();
        append_allocation(&p, &a, 300).unwrap();
        append_allocation(&p, &b, 5).unwrap();
        assert_eq!(balance_of(&p, &a).unwrap(), Some(1000));
        assert_eq!(balance_of(&p, &b).unwrap(), Some(5));
        assert_eq!(
            balance_of(&p, &wallet_address(&[3; 32]).unwrap()).unwrap(),
            Some(0)
        );
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn bad_input_is_refused() {
        let p = tmpdir("d").join(ECONOMY_FILE);
        let a = wallet_address(&[1; 32]).unwrap();
        assert!(append_allocation(&p, "tekow1xxxx", 5).is_err());
        assert!(append_allocation(&p, &a, 0).is_err());
        assert!(append_allocation(&p, &a, u64::MAX).is_err());
        assert!(!p.exists());
        fs::write(&p, "isto nao e valido\n").unwrap();
        assert!(read_allocations(&p).is_err());
        let _ = fs::remove_file(&p);
    }

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("teko-econ-dir-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn key(seed: u64) -> teko_crypto::SigningKey {
        use rand::SeedableRng;
        teko_crypto::generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(seed))
    }

    fn transfer(
        e: &Economy,
        from: &teko_crypto::SigningKey,
        to: &Address,
        amount: u64,
        fee: Option<u64>,
    ) -> Tx {
        let pk = teko_crypto::public_key_bytes(from);
        let me = teko_crypto::ledger_address(&pk);
        let nonce = e.nonce(&me);
        let fee = fee.unwrap_or_else(|| e.estimate_fee(&pk, nonce, to, amount));
        Tx::new_signed_with_fee(
            from,
            CHAIN_ID,
            nonce,
            fee,
            COIN_MODULE,
            Op::Transfer { to: *to, amount }.encode(),
        )
    }

    #[test]
    fn the_local_economy_moves_coins_with_fees_and_history() {
        let dir = tmpdir("a");
        let (a, b) = (key(1), key(2));
        let aa = teko_crypto::ledger_address(&teko_crypto::public_key_bytes(&a));
        let ba = teko_crypto::ledger_address(&teko_crypto::public_key_bytes(&b));
        append_allocation(
            &dir.join(ECONOMY_FILE),
            &wallet_address(&aa).unwrap(),
            10_000,
        )
        .unwrap();
        let mut e = Economy::open(&dir).unwrap();
        assert_eq!(
            (e.balance(&aa), e.balance(&ba), e.nonce(&aa)),
            (10_000, 0, 0)
        );
        let tx = transfer(&e, &a, &ba, 3000, None);
        let fee = tx.fee;
        assert!(fee > 0);
        e.submit(tx.clone()).unwrap();
        assert_eq!(e.balance(&ba), 3000);
        assert_eq!(e.balance(&aa), 10_000 - 3000 - fee);
        assert_eq!(e.balance(&PROPOSER), fee);
        assert_eq!(e.nonce(&aa), 1);
        // the same signed transaction cannot be replayed
        assert!(e.submit(tx).unwrap_err().contains("nonce"));
        // overspending and a too-low fee are refused and leave no trace
        let big = transfer(&e, &b, &aa, 5000, None);
        assert_eq!(e.submit(big).unwrap_err(), "saldo insuficiente");
        let cheap = transfer(&e, &a, &ba, 1, Some(0));
        assert_eq!(e.submit(cheap).unwrap_err(), "taxa abaixo do mínimo");
        // everything survives a restart (replay) and the history is right
        let e2 = Economy::open(&dir).unwrap();
        assert_eq!(e2.balance(&ba), 3000);
        let h = e2.history(&ba);
        assert_eq!(h.len(), 1);
        assert_eq!(
            (h[0].from, h[0].to, h[0].amount, h[0].fee),
            (aa, ba, 3000, fee)
        );
        assert_eq!(e2.history(&aa), h);
        assert!(e2.history(&PROPOSER).is_empty());
        // balance_of (what the Suite calls) sees the transfers too
        let got = balance_of(&dir.join(ECONOMY_FILE), &wallet_address(&ba).unwrap()).unwrap();
        assert_eq!(got, Some(3000));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tampered_transactions_file_is_rejected_on_open() {
        let dir = tmpdir("b");
        let a = key(3);
        let aa = teko_crypto::ledger_address(&teko_crypto::public_key_bytes(&a));
        append_allocation(&dir.join(ECONOMY_FILE), &wallet_address(&aa).unwrap(), 5000).unwrap();
        fs::write(dir.join(TXS_FILE), "zz\n").unwrap();
        assert!(Economy::open(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
