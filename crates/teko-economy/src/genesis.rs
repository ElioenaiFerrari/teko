use teko_coin::CoinConfig;
use teko_crypto::{hex, parse_wallet_address, wallet_address};
use teko_ledger::{Address, Ledger};

/// An asset class governed by an M-of-N registrar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSpec {
    pub name: String,
    pub threshold: u32,
    pub registrars: Vec<Vec<u8>>,
}

/// Everything that defines the first state. Public data, no secrets.
///
/// Text format (`economy.txt`), one `key=value` per line, `#` comments:
/// ```text
/// chain_id=teko-devnet-1
/// cap=20000000
/// initial_reward=50000
/// halving_interval=100
/// fee_per_byte=1
/// unbonding_period=20
/// slash_bps=1000
/// founder_target=1000000
/// min_total_stake=100000
/// min_validator_stake=10000
/// max_validators=16
/// founder=<hex consensus key> tekow1…        (the OWNER address gets the rewards)
/// class=imovel 2 <hex key> <hex key> <hex key>
/// alloc=tekow1… 5000                          (TEST networks only; normally absent)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomyGenesis {
    pub chain_id: String,
    pub coin: CoinConfig,
    pub founders: Vec<(Vec<u8>, Address)>,
    pub classes: Vec<ClassSpec>,
    pub allocations: Vec<(Address, u64)>,
}

impl EconomyGenesis {
    /// Devnet/testnet parameters: the founder reaches N = 1.000.000 in a few dozen blocks.
    pub fn test_defaults(chain_id: &str, founder_key: Vec<u8>, owner: Address) -> Self {
        Self {
            chain_id: chain_id.to_string(),
            coin: CoinConfig {
                cap: 20_000_000,
                initial_reward: 50_000,
                halving_interval: 100,
                // Fees are OFF in the local test network (the simulated registrar and
                // counterparty have no coins to pay them); the fee mechanism itself is
                // covered by the coin module's tests.
                fee_per_byte: 0,
                unbonding_period: 20,
                slash_bps: 1000,
                faucet_amount: 0,
                faucet_cooldown: 0,
                founder_target: 1_000_000,
                min_total_stake: 100_000,
                min_validator_stake: 10_000,
                max_validators: 16,
            },
            founders: vec![(founder_key, owner)],
            classes: Vec::new(),
            allocations: Vec::new(),
        }
    }

    /// Builds the genesis ledger (coin + assets, coin as the fee module). Test-network
    /// features (founders, allocations) are allowed: a locked network never gets here.
    pub fn build_ledger(&self) -> Result<Ledger, String> {
        let mut l = Ledger::new(
            &self.chain_id,
            vec![teko_coin::module(), teko_assets::module()],
        )
        .map_err(|e| format!("{e:?}"))?
        .with_fee_module(teko_coin::COIN_MODULE)
        .map_err(|e| format!("{e:?}"))?;
        teko_coin::genesis(&mut l, self.coin.clone(), &self.allocations, true)
            .map_err(|e| format!("{e:?}"))?;
        if !self.founders.is_empty() {
            teko_coin::genesis_founders(&mut l, &self.founders, true)
                .map_err(|e| format!("{e:?}"))?;
        }
        for c in &self.classes {
            teko_assets::genesis_class(&mut l, c.name.as_bytes(), &c.registrars, c.threshold)?;
        }
        Ok(l)
    }

    pub fn to_text(&self) -> String {
        let c = &self.coin;
        let mut s = format!(
            "# Economia da rede de teste (parâmetros públicos, sem segredos)\nchain_id={}\ncap={}\ninitial_reward={}\nhalving_interval={}\nfee_per_byte={}\nunbonding_period={}\nslash_bps={}\nfounder_target={}\nmin_total_stake={}\nmin_validator_stake={}\nmax_validators={}\n",
            self.chain_id,
            c.cap,
            c.initial_reward,
            c.halving_interval,
            c.fee_per_byte,
            c.unbonding_period,
            c.slash_bps,
            c.founder_target,
            c.min_total_stake,
            c.min_validator_stake,
            c.max_validators
        );
        for (k, o) in &self.founders {
            s.push_str(&format!(
                "founder={} {}\n",
                hex::encode(k),
                wallet_address(o).unwrap_or_default()
            ));
        }
        for cl in &self.classes {
            let keys: Vec<String> = cl.registrars.iter().map(|k| hex::encode(k)).collect();
            s.push_str(&format!(
                "class={} {} {}\n",
                cl.name,
                cl.threshold,
                keys.join(" ")
            ));
        }
        for (a, n) in &self.allocations {
            s.push_str(&format!(
                "alloc={} {n}\n",
                wallet_address(a).unwrap_or_default()
            ));
        }
        s
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut g = EconomyGenesis {
            chain_id: String::new(),
            coin: CoinConfig::default(),
            founders: Vec::new(),
            classes: Vec::new(),
            allocations: Vec::new(),
        };
        let num = |v: &str, k: &str| -> Result<u64, String> {
            v.parse().map_err(|_| format!("{k}: número inválido"))
        };
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| format!("linha {}: esperado chave=valor", n + 1))?;
            let (k, v) = (k.trim(), v.trim());
            match k {
                "chain_id" => g.chain_id = v.to_string(),
                "cap" => g.coin.cap = num(v, k)?,
                "initial_reward" => g.coin.initial_reward = num(v, k)?,
                "halving_interval" => g.coin.halving_interval = num(v, k)?,
                "fee_per_byte" => g.coin.fee_per_byte = num(v, k)?,
                "unbonding_period" => g.coin.unbonding_period = num(v, k)?,
                "slash_bps" => g.coin.slash_bps = num(v, k)? as u32,
                "founder_target" => g.coin.founder_target = num(v, k)?,
                "min_total_stake" => g.coin.min_total_stake = num(v, k)?,
                "min_validator_stake" => g.coin.min_validator_stake = num(v, k)?,
                "max_validators" => g.coin.max_validators = num(v, k)? as u32,
                "founder" => {
                    let mut it = v.split_whitespace();
                    let (Some(key), Some(owner), None) = (it.next(), it.next(), it.next()) else {
                        return Err(format!("linha {}: founder=<chave> <endereço>", n + 1));
                    };
                    g.founders.push((
                        hex::decode(key).map_err(|_| format!("linha {}: chave inválida", n + 1))?,
                        parse_wallet_address(owner)
                            .map_err(|_| format!("linha {}: endereço inválido", n + 1))?,
                    ));
                }
                "class" => {
                    let mut it = v.split_whitespace();
                    let (Some(name), Some(th)) = (it.next(), it.next()) else {
                        return Err(format!("linha {}: class=<nome> <M> <chaves…>", n + 1));
                    };
                    let registrars = it
                        .map(|h| {
                            hex::decode(h).map_err(|_| format!("linha {}: chave inválida", n + 1))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    g.classes.push(ClassSpec {
                        name: name.to_string(),
                        threshold: th
                            .parse()
                            .map_err(|_| format!("linha {}: M inválido", n + 1))?,
                        registrars,
                    });
                }
                "alloc" => {
                    let mut it = v.split_whitespace();
                    let (Some(a), Some(amt), None) = (it.next(), it.next(), it.next()) else {
                        return Err(format!("linha {}: alloc=<endereço> <valor>", n + 1));
                    };
                    g.allocations.push((
                        parse_wallet_address(a)
                            .map_err(|_| format!("linha {}: endereço inválido", n + 1))?,
                        num(amt, k)?,
                    ));
                }
                other => return Err(format!("linha {}: chave desconhecida `{other}`", n + 1)),
            }
        }
        if g.chain_id.is_empty() {
            return Err("falta chain_id".into());
        }
        g.build_ledger()?; // validates everything
        Ok(g)
    }
}
