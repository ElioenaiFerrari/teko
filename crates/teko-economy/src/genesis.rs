use teko_coin::CoinConfig;
use teko_crypto::amount::YPY_PER_TEKO;
use teko_crypto::{hex, parse_wallet_address, wallet_address};
use teko_ledger::{Address, Ledger};

/// An asset class governed by an M-of-N registrar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSpec {
    pub name: String,
    pub threshold: u32,
    pub registrars: Vec<Vec<u8>>,
    /// Seconds a pending trade of this class waits before anyone can finalize it; the registrar
    /// quorum may veto inside it (ADR-0036).
    pub window_secs: u64,
}

/// A credential type governed by an M-of-N issuing organ (ADR-0037).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredTypeSpec {
    pub name: String,
    pub threshold: u32,
    /// Members: the `tekow1…` addresses of their device wallets.
    pub members: Vec<Address>,
    /// Contest window in seconds (a veto is possible inside it).
    pub contest_secs: u64,
    /// 0 = never expires; otherwise the longest validity in seconds.
    pub validity_secs: u64,
    /// Public types keep their claims in clear on the chain.
    pub public: bool,
}

/// Everything that defines the first state. Public data, no secrets.
///
/// Text format (`economy.txt`), one `key=value` per line, `#` comments:
/// ```text
/// chain_id=teko-devnet-1
/// units=ypy6                                  (amounts below are in ypy: 1 TEKO = 1.000.000 ypy)
/// cap=21000000000000
/// initial_reward=50000000
/// halving_interval=210000
/// fee_per_byte=1
/// unbonding_period=20
/// slash_bps=1000
/// founder_target=1000000000
/// min_total_stake=100000000
/// min_validator_stake=10000000
/// max_validators=16
/// vote_chain_id=teko-devnet-1                       (the vote chain whose institutions bond here, ADR-0035)
/// founder=<hex consensus key> tekow1…        (the OWNER address gets the rewards)
/// class=imovel 2 window=1200 <hex key> <hex key> <hex key>   (`window=` optional: default 1200 s)
/// credtype=cnh-b 2 contest=604800 validity=157680000 public=0 tekow1… tekow1… tekow1…
///                                                   (credential type, ADR-0037; all options optional)
/// alloc=tekow1… 5000000000                          (TEST networks only; normally absent)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomyGenesis {
    pub chain_id: String,
    pub coin: CoinConfig,
    pub founders: Vec<(Vec<u8>, Address)>,
    pub classes: Vec<ClassSpec>,
    pub credtypes: Vec<CredTypeSpec>,
    pub allocations: Vec<(Address, u64)>,
}

impl EconomyGenesis {
    /// Devnet/testnet parameters (ADR-0029, amounts in ypy): Bitcoin-style cap (21M TEKO)
    /// and reward (50 TEKO). The LOCAL devnet uses N = 1.000 TEKO so the founder phase ends
    /// in ~20 blocks; the documented default for a testnet is 1.000.000 TEKO.
    pub fn test_defaults(chain_id: &str, founder_key: Vec<u8>, owner: Address) -> Self {
        Self {
            chain_id: chain_id.to_string(),
            coin: CoinConfig {
                cap: 21_000_000 * YPY_PER_TEKO,
                initial_reward: 50 * YPY_PER_TEKO,
                halving_interval: 210_000,
                // Fees are OFF in the local test network (the simulated registrar and
                // counterparty have no coins to pay them); the fee mechanism itself is
                // covered by the coin module's tests.
                fee_per_byte: 0,
                unbonding_period: 20,
                slash_bps: 1000,
                faucet_amount: 0,
                faucet_cooldown: 0,
                founder_target: 1_000 * YPY_PER_TEKO,
                min_total_stake: 100 * YPY_PER_TEKO,
                min_validator_stake: 10 * YPY_PER_TEKO,
                max_validators: 16,
                vote_chain_id: String::new(),
            },
            founders: vec![(founder_key, owner)],
            classes: Vec::new(),
            credtypes: Vec::new(),
            allocations: Vec::new(),
        }
    }

    /// Builds the genesis ledger (coin + assets, coin as the fee module). Test-network
    /// features (founders, allocations) are allowed: a locked network never gets here.
    pub fn build_ledger(&self) -> Result<Ledger, String> {
        let mut l = Ledger::new(
            &self.chain_id,
            vec![
                teko_coin::module(),
                teko_assets::module(),
                teko_credentials::module(),
            ],
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
            teko_assets::genesis_class_with_window(
                &mut l,
                c.name.as_bytes(),
                &c.registrars,
                c.threshold,
                c.window_secs,
            )?;
        }
        for c in &self.credtypes {
            let members: Vec<Vec<u8>> = c.members.iter().map(|a| a.to_vec()).collect();
            teko_credentials::genesis_type(
                &mut l,
                c.name.as_bytes(),
                &members,
                c.threshold,
                c.contest_secs,
                c.validity_secs,
                c.public,
            )?;
        }
        Ok(l)
    }

    pub fn to_text(&self) -> String {
        let c = &self.coin;
        let mut s = format!(
            "# Economia da rede de teste (parâmetros públicos, sem segredos)\nunits=ypy6\nchain_id={}\ncap={}\ninitial_reward={}\nhalving_interval={}\nfee_per_byte={}\nunbonding_period={}\nslash_bps={}\nfounder_target={}\nmin_total_stake={}\nmin_validator_stake={}\nmax_validators={}\n",
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
        if !c.vote_chain_id.is_empty() {
            s.push_str(&format!("vote_chain_id={}\n", c.vote_chain_id));
        }
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
                "class={} {} window={} {}\n",
                cl.name,
                cl.threshold,
                cl.window_secs,
                keys.join(" ")
            ));
        }
        for c in &self.credtypes {
            let keys: Vec<String> = c
                .members
                .iter()
                .map(|a| wallet_address(a).unwrap_or_default())
                .collect();
            s.push_str(&format!(
                "credtype={} {} contest={} validity={} public={} {}\n",
                c.name,
                c.threshold,
                c.contest_secs,
                c.validity_secs,
                u8::from(c.public),
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
            credtypes: Vec::new(),
            allocations: Vec::new(),
        };
        let mut saw_units = false;
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
                "units" if v == "ypy6" => saw_units = true,
                "units" => return Err(format!("units={v}: escala desconhecida (esperado ypy6)")),
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
                "vote_chain_id" => g.coin.vote_chain_id = v.to_string(),
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
                    let mut window_secs = teko_assets::DEFAULT_WINDOW_SECS;
                    let mut registrars = Vec::new();
                    for tok in it {
                        if let Some(w) = tok.strip_prefix("window=") {
                            window_secs = w
                                .parse()
                                .map_err(|_| format!("linha {}: window inválida", n + 1))?;
                        } else {
                            registrars.push(
                                hex::decode(tok)
                                    .map_err(|_| format!("linha {}: chave inválida", n + 1))?,
                            );
                        }
                    }
                    g.classes.push(ClassSpec {
                        name: name.to_string(),
                        threshold: th
                            .parse()
                            .map_err(|_| format!("linha {}: M inválido", n + 1))?,
                        registrars,
                        window_secs,
                    });
                }
                "credtype" => {
                    let mut it = v.split_whitespace();
                    let (Some(name), Some(th)) = (it.next(), it.next()) else {
                        return Err(format!("linha {}: credtype=<nome> <M> <chaves…>", n + 1));
                    };
                    let bad = |what: &str| format!("linha {}: {what} inválido", n + 1);
                    let mut spec = CredTypeSpec {
                        name: name.to_string(),
                        threshold: th.parse().map_err(|_| bad("M"))?,
                        members: Vec::new(),
                        contest_secs: teko_credentials::DEFAULT_CONTEST_SECS,
                        validity_secs: 0,
                        public: false,
                    };
                    for tok in it {
                        if let Some(x) = tok.strip_prefix("contest=") {
                            spec.contest_secs = x.parse().map_err(|_| bad("contest"))?;
                        } else if let Some(x) = tok.strip_prefix("validity=") {
                            spec.validity_secs = x.parse().map_err(|_| bad("validity"))?;
                        } else if let Some(x) = tok.strip_prefix("public=") {
                            spec.public = match x {
                                "0" => false,
                                "1" => true,
                                _ => return Err(bad("public")),
                            };
                        } else {
                            spec.members
                                .push(parse_wallet_address(tok).map_err(|_| bad("endereço"))?);
                        }
                    }
                    g.credtypes.push(spec);
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
        if !saw_units {
            return Err("economy.txt na escala antiga (sem `units=ypy6`, ADR-0029): recrie a economia de teste com `make devnet-stop`, `make devnet-founder FOUNDER=<seu tekow1…>` e `make devnet`".into());
        }
        if g.chain_id.is_empty() {
            return Err("falta chain_id".into());
        }
        g.build_ledger()?; // validates everything
        Ok(g)
    }
}
