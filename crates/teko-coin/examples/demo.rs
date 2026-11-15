//! Demonstração da moeda: roda alguns blocos em memória e imprime saldos, stake,
//! recompensa, taxa, penalidade e a conservação da oferta. DADOS DE TESTE.
//! Uso: `make coin-demo`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use prost::Message;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_bft::{Evidence, Vote, VoteKind};
use teko_coin::*;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};
use teko_ledger::{Address, Ledger, Tx, address_of};

const CHAIN: &str = "teko-devnet";

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}
fn addr(k: &SigningKey) -> Address {
    address_of(&public_key_bytes(k))
}

struct Pessoa<'a> {
    nome: &'a str,
    chave: SigningKey,
}

fn estado(l: &Ledger, pessoas: &[&Pessoa], proposer: &Address) {
    println!(
        "    {:<10} {:>10} {:>10} {:>12}",
        "", "saldo", "stake", "desvinculando"
    );
    for p in pessoas {
        let a = addr(&p.chave);
        let unb: u64 = unbonding_of(l, &a).iter().map(|(_, x)| x).sum();
        println!(
            "    {:<10} {:>10} {:>10} {:>12}",
            p.nome,
            balance(l, &a),
            stake_of(l, &a),
            unb
        );
    }
    println!(
        "    {:<10} {:>10}   (proponente dos blocos)",
        "Proponente",
        balance(l, proposer)
    );
    println!(
        "    criado {} | queimado {} | teto {} | conservação: {}",
        minted(l),
        burned(l),
        config(l).map(|c| c.cap).unwrap_or(0),
        if audit(l).is_ok() { "OK" } else { "FALHOU" }
    );
}

fn bloco(l: &mut Ledger, proposer: &Address, txs: Vec<Tx>) {
    let h = l.height() + 1;
    let kept = l.select_valid(proposer, txs.clone());
    if kept.len() != txs.len() {
        println!(
            "    (aviso: {} transação(ões) recusada(s) pelo livro)",
            txs.len() - kept.len()
        );
    }
    let out = l.apply_block(h, proposer, &kept).unwrap();
    println!(
        "  bloco {h}: {} transação(ões), raiz do estado {}…",
        kept.len(),
        hex(&out.state_root[..4])
    );
    for r in out.receipts.iter().flat_map(|r| &r.events) {
        println!("    evento: {}", r.topic);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    println!("== Teko · demonstração da moeda (DADOS DE TESTE, devnet) ==\n");
    let alice = Pessoa {
        nome: "Alice",
        chave: key(1),
    };
    let bruno = Pessoa {
        nome: "Bruno",
        chave: key(2),
    };
    let validador = Pessoa {
        nome: "Validador",
        chave: key(3),
    };
    let proposer: Address = [0xAA; 32];

    let cfg = CoinConfig {
        cap: 100_000,
        initial_reward: 100,
        halving_interval: 5,
        fee_per_byte: 1,
        unbonding_period: 4,
        slash_bps: 1000,
        faucet_amount: 500,
        faucet_cooldown: 3,
        ..Default::default()
    };
    println!(
        "Parâmetros: teto 100000 · recompensa 100 (cai pela metade a cada 5 blocos) · taxa 1/byte"
    );
    println!("            desvinculação 4 blocos · penalidade 10% · faucet 500 a cada 3 blocos\n");

    let mut l = Ledger::new(CHAIN, vec![module()])
        .unwrap()
        .with_fee_module(COIN_MODULE)
        .unwrap();
    genesis(&mut l, cfg, &[(addr(&alice.chave), 5_000)], true).unwrap();
    let todos = [&alice, &bruno, &validador];

    println!("1) Gênese da devnet: Alice recebe 5000 (alocação de TESTE, só existe na devnet)");
    estado(&l, &todos, &proposer);

    println!("\n2) Alice transfere 3000 ao Bruno pagando taxa (preço por byte, vai ao proponente)");
    let t = Tx::new_signed_with_fee(
        &alice.chave,
        CHAIN,
        0,
        200,
        COIN_MODULE,
        Op::Transfer {
            to: addr(&bruno.chave),
            amount: 3000,
        }
        .encode(),
    );
    println!(
        "    tamanho da transação: {} bytes · taxa mínima: {}",
        t.to_bytes().len(),
        t.to_bytes().len()
    );
    bloco(&mut l, &proposer, vec![t]);
    estado(&l, &todos, &proposer);

    println!("\n3) Taxa baixa demais é recusada (o livro descarta a transação)");
    let barata = Tx::new_signed_with_fee(
        &alice.chave,
        CHAIN,
        1,
        1,
        COIN_MODULE,
        Op::Transfer {
            to: addr(&bruno.chave),
            amount: 1,
        }
        .encode(),
    );
    bloco(&mut l, &proposer, vec![barata]);

    println!("\n4) A faucet de TESTE dá 500 ao Validador (rotulada; só existe na devnet)");
    let f = Tx::new_signed_with_fee(
        &validador.chave,
        CHAIN,
        0,
        200,
        COIN_MODULE,
        Op::Faucet.encode(),
    );
    // sem saldo para a taxa: o validador primeiro recebe da Alice
    let ajuda = Tx::new_signed_with_fee(
        &alice.chave,
        CHAIN,
        1,
        200,
        COIN_MODULE,
        Op::Transfer {
            to: addr(&validador.chave),
            amount: 1500,
        }
        .encode(),
    );
    bloco(&mut l, &proposer, vec![ajuda]);
    bloco(&mut l, &proposer, vec![f]);
    estado(&l, &todos, &proposer);

    println!("\n5) O Validador trava 1000 em stake e passa a valer no consenso");
    let s = Tx::new_signed_with_fee(
        &validador.chave,
        CHAIN,
        1,
        200,
        COIN_MODULE,
        Op::Stake { amount: 1000 }.encode(),
    );
    bloco(&mut l, &proposer, vec![s]);
    println!(
        "    validadores no consenso: {:?}",
        validators(&l)
            .iter()
            .map(|(k, s)| (hex(&k[..4]), *s))
            .collect::<Vec<_>>()
    );
    estado(&l, &todos, &proposer);

    println!("\n6) O Validador vota em dois blocos diferentes na mesma rodada (equivocação)");
    let a = Vote::new(
        CHAIN,
        &validador.chave,
        VoteKind::Prevote,
        l.height(),
        0,
        Some(&[1; 32]),
    );
    let b = Vote::new(
        CHAIN,
        &validador.chave,
        VoteKind::Prevote,
        l.height(),
        0,
        Some(&[2; 32]),
    );
    let ev = Evidence {
        a: Some(a),
        b: Some(b),
    }
    .encode_to_vec();
    let slash = Tx::new_signed_with_fee(
        &bruno.chave,
        CHAIN,
        0,
        800,
        COIN_MODULE,
        Op::Slash {
            evidence: ev.clone(),
        }
        .encode(),
    );
    bloco(&mut l, &proposer, vec![slash]);
    println!("    10% do stake foi QUEIMADO (a oferta total diminui):");
    estado(&l, &todos, &proposer);
    let repetida = Tx::new_signed_with_fee(
        &bruno.chave,
        CHAIN,
        1,
        800,
        COIN_MODULE,
        Op::Slash { evidence: ev }.encode(),
    );
    println!("    mesma prova de novo: não pune duas vezes");
    bloco(&mut l, &proposer, vec![repetida]);

    println!("\n7) O Validador pede para sair e só resgata depois do prazo de desvinculação");
    let u = Tx::new_signed_with_fee(
        &validador.chave,
        CHAIN,
        2,
        200,
        COIN_MODULE,
        Op::Unstake { amount: 900 }.encode(),
    );
    bloco(&mut l, &proposer, vec![u]);
    let cedo = Tx::new_signed_with_fee(
        &validador.chave,
        CHAIN,
        3,
        200,
        COIN_MODULE,
        Op::Claim.encode(),
    );
    println!("    tentativa de resgate imediata:");
    bloco(&mut l, &proposer, vec![cedo]);
    for _ in 0..4 {
        bloco(&mut l, &proposer, vec![]);
    }
    let claim = Tx::new_signed_with_fee(
        &validador.chave,
        CHAIN,
        3,
        200,
        COIN_MODULE,
        Op::Claim.encode(),
    );
    bloco(&mut l, &proposer, vec![claim]);
    estado(&l, &todos, &proposer);

    println!("\n8) Halving: a recompensa por bloco cai pela metade a cada 5 blocos");
    let c = config(&l).unwrap();
    for h in [1, 5, 6, 11, 16, 5 * 64 + 1] {
        println!("    bloco {h:>3}: recompensa {}", reward_at(&c, h));
    }
    println!(
        "\nFim. Conservação da oferta: {}",
        if audit(&l).is_ok() { "OK" } else { "FALHOU" }
    );
}
