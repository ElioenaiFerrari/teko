//! Local DEVNET economy (development only): a text file of genesis allocations of TEST
//! coins, so the Suite can show a wallet balance before the economy ledger runs on the
//! network (M6). Format: one `tekow1… amount` pair per line; `#` starts a comment.
//! Nothing here exists on a real network: allocations are refused outside the devnet.

use std::fs;
use std::io::Write;
use std::path::Path;

use teko_crypto::parse_wallet_address;
use teko_ledger::{Address, Ledger};

use crate::{CoinConfig, balance, genesis, module};

/// File name inside the dev directory.
pub const ECONOMY_FILE: &str = "economy.txt";

/// Parameters of the devnet economy (no rewards: only the listed allocations exist).
pub fn devnet_config() -> CoinConfig {
    CoinConfig {
        cap: 1_000_000_000,
        initial_reward: 0,
        halving_interval: 1_000_000,
        fee_per_byte: 0,
        unbonding_period: 100,
        slash_bps: 1000,
        faucet_amount: 0,
        faucet_cooldown: 0,
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

fn balance_of_allocations(all: &[(Address, u64)], who: &Address) -> Result<u64, String> {
    let mut l = Ledger::new("teko-devnet", vec![module()]).map_err(|e| format!("{e:?}"))?;
    genesis(&mut l, devnet_config(), all, true).map_err(|e| format!("{e:?}"))?;
    Ok(balance(&l, who))
}

/// Balance of `address` according to the devnet allocations (None = no economy file).
pub fn balance_of(path: &Path, address: &str) -> Result<Option<u64>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let who = parse_wallet_address(address).map_err(|_| "endereço inválido".to_string())?;
    let all = read_allocations(path)?;
    balance_of_allocations(&all, &who).map(Some)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use teko_crypto::wallet_address;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("teko-econ-{name}-{}", std::process::id()));
        let _ = fs::remove_file(&d);
        d
    }

    #[test]
    fn allocations_roundtrip_and_balance() {
        let p = tmp("a");
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
        let p = tmp("b");
        let a = wallet_address(&[1; 32]).unwrap();
        assert!(append_allocation(&p, "tekow1xxxx", 5).is_err());
        assert!(append_allocation(&p, &a, 0).is_err());
        assert!(append_allocation(&p, &a, u64::MAX).is_err());
        assert!(!p.exists());
        fs::write(&p, "isto nao e valido\n").unwrap();
        assert!(read_allocations(&p).is_err());
        let _ = fs::remove_file(&p);
    }
}
