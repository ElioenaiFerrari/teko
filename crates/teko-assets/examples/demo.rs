//! Demonstração dos bens: registro com quórum do órgão, gravame, negociação DIRETA com janela de
//! veto (oferta, aceite com o preço em escrow, finalização depois da janela) e veto do órgão.
//! DADOS DE TESTE. Uso: `make assets-demo`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_assets::*;
use teko_coin::{COIN_MODULE, CoinConfig};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Address, Ledger, LedgerError, Tx, TxError, address_of};

const CHAIN: &str = "teko-devnet";
const CLASSE: &[u8] = b"imovel";
/// Hash de brinquedo dos metadados do imóvel (32 bytes; o real vem de `teko_crypto::asset_meta`).
const META: [u8; 32] = [7; 32];
const ATIVO: &[u8] = b"matricula-123";
const PROP: Address = [0xAA; 32];
/// Janela de veto da classe nesta demonstração (segundos) e o horário inicial dos blocos.
const JANELA: u64 = 600;
const T0: u64 = 1_700_000_000;

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
    /// Horário do bloco (segundos): a janela de veto é medida por ele.
    agora: u64,
}

impl Mundo {
    fn nome(&self, a: &[u8]) -> &str {
        self.nomes
            .iter()
            .find(|(x, _)| x[..] == *a)
            .map(|(_, n)| *n)
            .unwrap_or("?")
    }

    /// Envia `op` assinada por `quem` (cada parte envia a PRÓPRIA transação) em `dt` segundos.
    fn enviar_como(&mut self, quem: &SigningKey, rotulo: &str, op: Op, dt: u64) {
        self.agora += dt;
        let t = Tx::new_signed(
            quem,
            CHAIN,
            self.l.nonce_of(&addr(quem)),
            ASSETS_MODULE,
            op.encode(),
        );
        let h = self.l.height() + 1;
        match self.l.apply_block_at(h, self.agora, &PROP, &[t]) {
            Ok(out) => {
                let ev: Vec<_> = out
                    .receipts
                    .iter()
                    .flat_map(|r| r.events.iter().map(|e| e.topic.clone()))
                    .collect();
                println!(
                    "  ✔ {rotulo}  (bloco {h}, t+{}s, eventos: {})",
                    self.agora - T0,
                    ev.join(", ")
                );
            }
            Err(LedgerError::Tx {
                error: TxError::Module(m),
                ..
            }) => println!("  ✘ {rotulo}  → RECUSADO: {m}"),
            Err(e) => println!("  ✘ {rotulo}  → RECUSADO: {e:?}"),
        }
    }

    /// Operações do órgão (registro, gravame, veto): o relayer só leva a transação.
    fn enviar(&mut self, rotulo: &str, op: Op) {
        let r = self.relayer.clone();
        self.enviar_como(&r, rotulo, op, 1);
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
        if let Some(o) = offer_of(&self.l, CLASSE, ATIVO) {
            println!(
                "    negociação: para {} por {} · {}",
                self.nome(&o.buyer),
                o.price,
                if o.accepted {
                    format!("PENDENTE até t+{}s", o.release_at - T0)
                } else {
                    "oferecida (aguardando o aceite)".into()
                }
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
            "    saldos: {} · escrow {} · oferta {}",
            s.join(" · "),
            teko_coin::balance(&self.l, &escrow_address(CLASSE, ATIVO)),
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

    fn oferta(&self, comprador: &SigningKey, preco: u64) -> Op {
        Op::Offer(OfferOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            buyer: addr(comprador).to_vec(),
            price: preco,
            version: self.versao(),
            metadata_hash: META.to_vec(),
        })
    }
}

/// O aceite leva os TERMOS que o comprador viu (preço, versão e hash dos metadados).
fn aceite(w: &Mundo) -> Op {
    let o = offer_of(&w.l, CLASSE, ATIVO).unwrap_or_default();
    Op::Accept(AcceptOp {
        class: CLASSE.to_vec(),
        asset: ATIVO.to_vec(),
        price: o.price,
        version: o.version,
        metadata_hash: o.metadata_hash,
    })
}

fn finalizar() -> Op {
    Op::Finalize(FinalizeOp {
        class: CLASSE.to_vec(),
        asset: ATIVO.to_vec(),
    })
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
        ..Default::default()
    };
    teko_coin::genesis(
        &mut l,
        cfg,
        &[(addr(&beto), 10_000), (addr(&carla), 50)],
        true,
    )
    .unwrap();
    let chaves: Vec<Vec<u8>> = orgaos.iter().map(public_key_bytes).collect();
    genesis_class_with_window(&mut l, CLASSE, &chaves, 2, JANELA).unwrap();
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
        agora: T0,
    };
    let (o0, o1) = (w.orgaos[0].clone(), w.orgaos[1].clone());
    println!(
        "Classe \"imovel\": órgão 2-de-3 · janela de veto de {JANELA}s · Beto tem 10000 e Carla 50 (moedas de TESTE)\n"
    );

    println!("1) Registrar a matrícula para a Ana (o órgão)");
    let reg = Intent::register(CLASSE, ATIVO, addr(&ana), &META);
    let op = |ap| {
        Op::Register(RegisterOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            owner: addr(&ana).to_vec(),
            metadata_hash: META.to_vec(),
            approvals: ap,
        })
    };
    w.enviar("com 1 assinatura do órgão", op(w.assinar(&[&o0], &reg)));
    w.enviar(
        "com 2 assinaturas do órgão",
        op(w.assinar(&[&o0, &o1], &reg)),
    );
    w.estado(&[&ana, &beto, &carla]);

    println!("\n2) Penhora (gravame) lançada pelo órgão: o bem não pode ser oferecido");
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
    let o = w.oferta(&beto, 4000);
    w.enviar_como(&ana, "Ana oferece ao Beto", o, 1);

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

    println!(
        "\n4) Cada um só faz o que é seu: quem não é dono não oferece; só o comprador nomeado aceita"
    );
    let o = w.oferta(&carla, 10);
    w.enviar_como(&beto, "Beto (não é o dono) oferece", o, 1);
    let o = w.oferta(&beto, 4000);
    w.enviar_como(&ana, "Ana oferece ao Beto por 4000", o, 1);
    let a = aceite(&w);
    w.enviar_como(&carla, "Carla (não é o comprador) aceita", a, 1);
    w.estado(&[&ana, &beto, &carla]);

    println!("\n5) Quem aceita sem dinheiro não leva nada: o aceite é atômico");
    w.enviar_como(
        &ana,
        "Ana cancela a oferta",
        Op::Cancel(CancelOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
        }),
        1,
    );
    let o = w.oferta(&carla, 4000);
    w.enviar_como(&ana, "Ana oferece à Carla (50 moedas) por 4000", o, 1);
    let a = aceite(&w);
    w.enviar_como(&carla, "Carla aceita", a, 1);
    w.enviar_como(
        &ana,
        "Ana cancela de novo",
        Op::Cancel(CancelOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
        }),
        1,
    );
    w.estado(&[&ana, &beto, &carla]);

    println!("\n6) Negociação direta: oferta → aceite (preço em escrow) → janela → finalização");
    let o = w.oferta(&beto, 4000);
    w.enviar_como(&ana, "Ana oferece ao Beto por 4000", o, 1);
    let a = aceite(&w);
    w.enviar_como(&beto, "Beto aceita (4000 vão para o escrow)", a, 1);
    w.estado(&[&ana, &beto, &carla]);
    w.enviar_como(&carla, "alguém tenta finalizar cedo", finalizar(), 10);
    w.agora += JANELA;
    w.enviar_como(
        &carla,
        "passou a janela: a Carla (qualquer um) finaliza",
        finalizar(),
        1,
    );
    w.estado(&[&ana, &beto, &carla]);
    w.enviar_como(&carla, "finalizar de novo", finalizar(), 1);

    println!("\n7) Beto revende para a Ana por 3000, mas o ÓRGÃO VETA dentro da janela");
    let o = w.oferta(&ana, 3000);
    w.enviar_como(&beto, "Beto oferece à Ana por 3000", o, 1);
    let a = aceite(&w);
    w.enviar_como(&ana, "Ana aceita (3000 vão para o escrow)", a, 1);
    let dono = asset(&w.l, CLASSE, ATIVO).unwrap();
    let motivo = "penhora judicial ainda nao lancada";
    let veto = Intent::veto(
        CLASSE,
        ATIVO,
        dono.owner.as_slice().try_into().unwrap(),
        addr(&ana),
        3000,
        dono.version,
        motivo,
    );
    w.enviar(
        "o quórum do órgão veta, com motivo público",
        Op::Veto(VetoOp {
            class: CLASSE.to_vec(),
            asset: ATIVO.to_vec(),
            reason: motivo.into(),
            approvals: w.assinar(&[&o0, &o1], &veto),
        }),
    );
    w.estado(&[&ana, &beto, &carla]);
    println!("\nFim.");
}
