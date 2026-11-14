//! Demonstração dos bens: registro com quórum do órgão, gravame, venda contra
//! pagamento atômica e recusas. DADOS DE TESTE. Uso: `make assets-demo`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_assets::*;
use teko_coin::{COIN_MODULE, CoinConfig};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Address, Ledger, LedgerError, Tx, TxError, address_of};

const CHAIN: &str = "teko-devnet";
const CLASSE: &[u8] = b"imovel";
const ATIVO: &[u8] = b"matricula-123";
const PROP: Address = [0xAA; 32];

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}
fn addr(k: &SigningKey) -> Address {
    address_of(&public_key_bytes(k))
}

struct Mundo {
    l: Ledger,
    orgaos: Vec<SigningKey>,
    nomes: Vec<(Address, &'static str)>,
    relayer: SigningKey,
}

impl Mundo {
    fn nome(&self, a: &[u8]) -> &str {
        self.nomes
            .iter()
            .find(|(x, _)| x[..] == *a)
            .map(|(_, n)| *n)
            .unwrap_or("?")
    }

    fn enviar(&mut self, rotulo: &str, op: Op) {
        let t = Tx::new_signed(
            &self.relayer,
            CHAIN,
            self.l.nonce_of(&addr(&self.relayer)),
            ASSETS_MODULE,
            op.encode(),
        );
        let h = self.l.height() + 1;
        match self.l.apply_block(h, &PROP, &[t]) {
            Ok(out) => {
                let ev: Vec<_> = out
                    .receipts
                    .iter()
                    .flat_map(|r| r.events.iter().map(|e| e.topic.clone()))
                    .collect();
                println!("  ✔ {rotulo}  (bloco {h}, eventos: {})", ev.join(", "));
            }
            Err(LedgerError::Tx {
                error: TxError::Module(m),
                ..
            }) => println!("  ✘ {rotulo}  → RECUSADO: {m}"),
            Err(e) => println!("  ✘ {rotulo}  → RECUSADO: {e:?}"),
        }
    }

    fn estado(&self, saldos: &[&SigningKey]) {
        if let Some(a) = asset(&self.l, CLASSE, ATIVO) {
            println!(
                "    {}: dono {} · versão {} · gravames {}",
                String::from_utf8_lossy(ATIVO),
                self.nome(&a.owner),
                a.version,
                a.liens.len()
            );
        }
        let s: Vec<String> = saldos
            .iter()
            .map(|k| {
                format!(
                    "{} {}",
                    self.nome(&addr(k)),
                    teko_coin::balance(&self.l, &addr(k))
                )
            })
            .collect();
        println!(
            "    saldos: {} · oferta {}",
            s.join(" · "),
            if teko_coin::audit(&self.l).is_ok() {
                "conservada"
            } else {
                "FALHOU"
            }
        );
    }

    fn versao(&self) -> u64 {
        asset(&self.l, CLASSE, ATIVO)
            .map(|a| a.version)
            .unwrap_or(0)
    }

    fn assinar(&self, quem: &[&SigningKey], i: &Intent) -> Vec<Approval> {
        quem.iter().map(|k| Approval::sign(k, CHAIN, i)).collect()
    }
}

fn main() {
    println!("== Teko · demonstração de bens (DADOS DE TESTE, devnet) ==\n");
    let (ana, beto, carla) = (key(1), key(2), key(3));
    let orgaos: Vec<SigningKey> = (0..3).map(|i| key(100 + i)).collect();
    let mut l = Ledger::new(CHAIN, vec![teko_coin::module(), module()])
        .unwrap()
        .with_fee_module(COIN_MODULE)
        .unwrap();
    let cfg = CoinConfig {
        cap: 1_000_000,
        initial_reward: 0,
        halving_interval: 10,
        fee_per_byte: 0,
        unbonding_period: 5,
        slash_bps: 1000,
        faucet_amount: 0,
        faucet_cooldown: 0,
    };
    teko_coin::genesis(
        &mut l,
        cfg,
        &[(addr(&beto), 10_000), (addr(&carla), 50)],
        true,
    )
    .unwrap();
    let chaves: Vec<Vec<u8>> = orgaos.iter().map(public_key_bytes).collect();
    genesis_class(&mut l, CLASSE, &chaves, 2).unwrap();
    let nomes = vec![
        (addr(&ana), "Ana"),
        (addr(&beto), "Beto"),
        (addr(&carla), "Carla"),
    ];
    let mut w = Mundo {
        l,
        orgaos,
        nomes,
        relayer: key(777),
    };
    let (o0, o1) = (w.orgaos[0].clone(), w.orgaos[1].clone());
    println!(
        "Classe \"imovel\": órgão registrador 2-de-3 · Beto tem 10000 e Carla 50 (moedas de TESTE)\n"
    );

    println!("1) Registrar a matrícula para a Ana");
    let reg = Intent::register(CLASSE, ATIVO, addr(&ana), b"escritura-v1");
    let op = |ap| {
        Op::Register(RegisterOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            owner: addr(&ana).to_vec(),
            metadata_hash: b"escritura-v1".to_vec(),
            approvals: ap,
        })
    };
    w.enviar("com 1 assinatura do órgão", op(w.assinar(&[&o0], &reg)));
    w.enviar(
        "com 2 assinaturas do órgão",
        op(w.assinar(&[&o0, &o1], &reg)),
    );
    w.estado(&[&ana, &beto, &carla]);

    println!("\n2) Penhora (gravame) registrada pelo órgão: o bem não pode ser vendido");
    let lien = Intent::add_lien(CLASSE, ATIVO, addr(&ana), w.versao(), "penhora");
    w.enviar(
        "gravame \"penhora\"",
        Op::AddLien(LienOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            kind: "penhora".into(),
            approvals: w.assinar(&[&o0, &o1], &lien),
        }),
    );
    let sw = |w: &Mundo, buyer: &SigningKey, price: u64, quem: &[&SigningKey]| {
        let i = Intent::swap(CLASSE, ATIVO, addr(&ana), addr(buyer), price, w.versao());
        Op::Swap(SwapOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            buyer: addr(buyer).to_vec(),
            price,
            approvals: w.assinar(quem, &i),
        })
    };
    let op1 = sw(&w, &beto, 4000, &[&ana, &beto, &o0, &o1]);
    w.enviar("venda para o Beto", op1);

    println!("\n3) O órgão libera o gravame");
    let rel = Intent::release_lien(CLASSE, ATIVO, addr(&ana), w.versao(), 0);
    w.enviar(
        "liberar gravame",
        Op::ReleaseLien(ReleaseOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            lien_id: 0,
            approvals: w.assinar(&[&o0, &o1], &rel),
        }),
    );
    w.estado(&[&ana, &beto, &carla]);

    println!("\n4) Venda sem o consentimento do comprador é recusada");
    let op2 = sw(&w, &beto, 4000, &[&ana, &o0, &o1]);
    w.enviar("Ana + órgão, sem o Beto", op2);

    println!("\n5) Comprador sem dinheiro: a troca é atômica, nada se move");
    let op3 = sw(&w, &carla, 4000, &[&ana, &carla, &o0, &o1]);
    w.enviar("venda para a Carla (50 moedas) por 4000", op3);
    w.estado(&[&ana, &beto, &carla]);

    println!("\n6) Venda contra pagamento: Ana + Beto + órgão (2-de-3)");
    let v_antes = w.versao();
    let op4 = sw(&w, &beto, 4000, &[&ana, &beto, &o0, &o1]);
    let guardado = match &op4 {
        Op::Swap(s) => s.clone(),
        _ => unreachable!(),
    };
    w.enviar("venda para o Beto por 4000", op4);
    w.estado(&[&ana, &beto, &carla]);

    println!("\n7) Beto revende o imóvel para a Ana por 3000 (o bem muda de versão de novo)");
    let volta = Intent::swap(CLASSE, ATIVO, addr(&beto), addr(&ana), 3000, w.versao());
    w.enviar(
        "revenda Beto → Ana",
        Op::Swap(SwapOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            buyer: addr(&ana).to_vec(),
            price: 3000,
            approvals: w.assinar(&[&beto, &ana, &o0, &o1], &volta),
        }),
    );
    w.estado(&[&ana, &beto, &carla]);

    println!("\n8) Alguém reenvia as assinaturas ANTIGAS da venda Ana → Beto (versão {v_antes})");
    w.enviar("repetição das aprovações antigas", Op::Swap(guardado));
    w.estado(&[&ana, &beto, &carla]);
    println!("\nFim.");
}
