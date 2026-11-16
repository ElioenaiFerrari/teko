#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The economy chain driven by real BFT engines, in-process (no sockets): the founder
//! alone, then the set changes when stake appears, plus a client that verifies the
//! chain without trusting anyone.

use std::collections::VecDeque;

use prost::Message;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_bft::{Action, Commit, Engine, Evidence, Message as BftMsg, Vote, VoteKind, block_id};
use teko_coin::Op;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes, wallet_address};
use teko_economy::{DecidedBlock, EconApp, EconError, EconomyChain, EconomyGenesis, quantize};
use teko_ledger::{Address, Tx, address_of};

const CHAIN: &str = "teko-econ-test";

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}
fn addr(k: &SigningKey) -> Address {
    address_of(&public_key_bytes(k))
}

fn genesis(node: &SigningKey, owner: &Address, target: u64) -> EconomyGenesis {
    let mut g = EconomyGenesis::test_defaults(CHAIN, public_key_bytes(node), *owner);
    g.coin.initial_reward = 1_000;
    g.coin.halving_interval = 10_000;
    g.coin.founder_target = target;
    g.coin.min_total_stake = 1;
    g.coin.min_validator_stake = 100;
    g.coin.fee_per_byte = 0;
    g.coin.unbonding_period = 5;
    g
}

/// A cluster of nodes: each has an engine over its own copy of the chain; messages are
/// delivered in order; the leader's decision is relayed to everyone as a certificate.
struct Cluster {
    nodes: Vec<Engine<EconApp>>,
    queue: VecDeque<(usize, BftMsg)>,
    evidence: Vec<Evidence>,
    /// Nodes that decided and still have to start the next height.
    pending: Vec<usize>,
}

impl Cluster {
    fn new(g: &EconomyGenesis, keys: Vec<SigningKey>) -> Self {
        let nodes = keys
            .iter()
            .map(|k| {
                let app = EconApp::new(
                    EconomyChain::new(g.clone()).unwrap(),
                    k,
                    Box::new(|| 1_700_000_000),
                );
                Engine::new(CHAIN, k.clone(), app).unwrap()
            })
            .collect();
        let mut c = Self {
            nodes,
            queue: VecDeque::new(),
            evidence: Vec::new(),
            pending: Vec::new(),
        };
        c.pending = (0..c.nodes.len()).rev().collect();
        c
    }

    fn start(&mut self, i: usize) {
        let h = self.nodes[i].app().chain.height() + 1;
        let set = self.nodes[i].app().chain.validator_set().unwrap();
        let acts = self.nodes[i].start_height(h, set);
        self.run(i, acts);
    }

    fn run(&mut self, from: usize, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Broadcast(m) => {
                    for j in 0..self.nodes.len() {
                        if j != from {
                            self.queue.push_back((j, m.clone()));
                        }
                    }
                }
                Action::Schedule { .. } => {}
                Action::Evidence(e) => self.evidence.push(e),
                Action::Decide { commit, payload } => {
                    let d = DecidedBlock {
                        payload: payload.clone(),
                        commit: Some(commit.clone()),
                    };
                    self.nodes[from].app_mut().on_decided(d.clone()).unwrap();
                    // catch-up for nodes that have not decided yet (same certificate)
                    for j in 0..self.nodes.len() {
                        if j != from
                            && self.nodes[j].app().chain.height()
                                < self.nodes[from].app().chain.height()
                        {
                            let acts = self.nodes[j].on_commit(&commit, &payload);
                            self.run(j, acts);
                        }
                    }
                    self.pending.push(from);
                }
            }
        }
    }

    /// Delivers messages until the chain reaches `height` on every node (or gives up).
    fn until(&mut self, height: u64) {
        let mut guard = 0;
        while self.nodes.iter().any(|n| n.app().chain.height() < height) {
            guard += 1;
            assert!(guard < 200_000, "the cluster stalled");
            let Some((to, m)) = self.queue.pop_front() else {
                // messages first; then start the next height of a node that is still behind
                if let Some(pos) = self
                    .pending
                    .iter()
                    .position(|i| self.nodes[*i].app().chain.height() < height)
                {
                    let i = self.pending.remove(pos);
                    self.start(i);
                    continue;
                }
                // nothing in flight: fire the timeouts of the first node that is stuck
                let (i, r) = self
                    .nodes
                    .iter()
                    .enumerate()
                    .find(|(_, n)| n.app().chain.height() < height)
                    .map(|(i, n)| (i, n.round()))
                    .unwrap();
                for step in [
                    teko_bft::Step::Propose,
                    teko_bft::Step::Prevote,
                    teko_bft::Step::Precommit,
                ] {
                    for j in 0..self.nodes.len() {
                        let round = self.nodes[j].round();
                        let acts = self.nodes[j].on_timeout(round, step);
                        self.run(j, acts);
                    }
                }
                let _ = (i, r);
                continue;
            };
            let acts = self.nodes[to].on_message(m);
            self.run(to, acts);
        }
    }

    fn submit_all(&mut self, tx: &Tx) {
        for n in &mut self.nodes {
            let _ = n.app_mut().submit(tx.clone());
        }
    }

    fn chain(&self, i: usize) -> &EconomyChain {
        &self.nodes[i].app().chain
    }
}

fn tx(sk: &SigningKey, ch: &EconomyChain, op: Op) -> Tx {
    Tx::new_signed(
        sk,
        CHAIN,
        ch.ledger().nonce_of(&addr(sk)),
        teko_coin::COIN_MODULE,
        op.encode(),
    )
}

#[test]
fn the_founder_alone_produces_blocks_and_its_owner_collects_the_rewards() {
    let (node, wallet) = (key(1), key(2));
    let g = genesis(&node, &addr(&wallet), 1_000_000);
    // an OBSERVER (node 1) follows the founder (node 0) without voting
    let mut c = Cluster::new(&g, vec![node.clone(), key(3)]);
    c.until(5);
    for i in 0..2 {
        assert_eq!(c.chain(i).height(), 5);
        assert_eq!(
            teko_coin::balance(c.chain(i).ledger(), &addr(&wallet)),
            5_000
        );
        // the node key itself never receives anything: the OWNER does
        assert_eq!(teko_coin::balance(c.chain(i).ledger(), &addr(&node)), 0);
    }
    assert_eq!(
        c.chain(0).ledger().state_root(),
        c.chain(1).ledger().state_root()
    );
    assert_eq!(teko_coin::minted(c.chain(0).ledger()), 5_000); // no pre-mine: only rewards
    assert!(teko_coin::founder_phase_active(c.chain(0).ledger()));
}

#[test]
fn the_phase_ends_when_the_stake_arrives_and_a_second_validator_joins_by_stake() {
    let (node, wallet, joiner) = (key(10), key(11), key(12));
    let g = genesis(&node, &addr(&wallet), 3_000);
    let mut c = Cluster::new(&g, vec![node.clone(), joiner.clone()]);
    c.until(4);
    // the owner stakes the rewards and sends coins to the joiner's account
    let stake = tx(&wallet, c.chain(0), Op::Stake { amount: 3_000 });
    c.submit_all(&stake);
    c.until(5);
    let pay = tx(
        &wallet,
        c.chain(0),
        Op::Transfer {
            to: addr(&joiner),
            amount: 1_000,
        },
    );
    c.submit_all(&pay);
    c.until(7);
    // the joiner bonds (its account key becomes a consensus key by default)
    let bond = tx(&joiner, c.chain(0), Op::Stake { amount: 1_000 });
    c.submit_all(&bond);
    c.until(10);
    let l = c.chain(0).ledger();
    assert!(
        teko_coin::founder_phase_over(l),
        "stake 3000 reached the target"
    );
    // after the phase the set comes from stake: owner's node key (3000) and the joiner (1000)
    let set = c.chain(0).validator_set().unwrap();
    assert_eq!(set.validators().len(), 2);
    let powers: Vec<u64> = set.validators().iter().map(|v| v.power).collect();
    assert_eq!(powers.iter().sum::<u64>(), 10_000);
    // both validators keep deciding blocks together
    let before = c.chain(0).height();
    c.until(before + 40);
    for i in 0..2 {
        assert_eq!(
            c.chain(i).ledger().state_root(),
            c.chain(0).ledger().state_root()
        );
    }
    // rewards of blocks proposed by the joiner go to the joiner's OWN account
    assert!(teko_coin::balance(c.chain(0).ledger(), &addr(&joiner)) > 0);
    teko_coin::audit(c.chain(0).ledger()).unwrap();
}

#[test]
fn a_client_replays_and_verifies_the_whole_chain_without_trusting_the_node() {
    let (node, wallet) = (key(20), key(21));
    let g = genesis(&node, &addr(&wallet), 1_000_000);
    let mut c = Cluster::new(&g, vec![node.clone()]);
    let pay = tx(
        &wallet,
        c.chain(0),
        Op::Transfer {
            to: [7; 32],
            amount: 1,
        },
    );
    c.until(3);
    let pay2 = tx(
        &wallet,
        c.chain(0),
        Op::Transfer {
            to: [7; 32],
            amount: 250,
        },
    );
    let _ = pay;
    c.submit_all(&pay2);
    c.until(6);
    let blocks = c.chain(0).blocks().to_vec();
    let client = EconomyChain::replay(g.clone(), &blocks).unwrap();
    assert_eq!(client.height(), 6);
    assert_eq!(
        client.ledger().state_root(),
        c.chain(0).ledger().state_root()
    );
    assert_eq!(teko_coin::balance(client.ledger(), &[7; 32]), 250);

    // a missing certificate, a certificate for another block, a changed payload, a
    // reordered chain and a certificate by a key that is not a validator are all refused
    let mut no_cert = blocks.clone();
    no_cert[2].commit = None;
    assert_eq!(
        EconomyChain::replay(g.clone(), &no_cert).err(),
        Some(EconError::Commit)
    );
    let mut swapped = blocks.clone();
    swapped.swap(1, 2);
    assert!(EconomyChain::replay(g.clone(), &swapped).is_err());
    let mut forged = blocks.clone();
    let mut p = forged[3].block().unwrap();
    p.txs.push(vec![1, 2, 3]);
    forged[3].payload = p.encode_to_vec();
    assert!(EconomyChain::replay(g.clone(), &forged).is_err());
    let imposter = key(99);
    let mut fake = blocks.clone();
    let mut b = fake[0].block().unwrap();
    b.proposer = public_key_bytes(&imposter);
    fake[0].payload = b.encode_to_vec();
    let id = block_id(&fake[0].payload);
    let v = Vote::new(CHAIN, &imposter, VoteKind::Precommit, 1, 0, Some(&id));
    fake[0].commit = Some(Commit {
        height: 1,
        round: 0,
        block_id: id,
        precommits: vec![v],
    });
    assert!(EconomyChain::replay(g.clone(), &fake).is_err());
    // a wrong genesis (another network) cannot replay the blocks either
    let mut other = g.clone();
    other.chain_id = "another-network".into();
    assert!(EconomyChain::replay(other, &blocks).is_err());
}

#[test]
fn invalid_proposals_are_not_accepted_by_the_checker() {
    let (node, wallet) = (key(30), key(31));
    let g = genesis(&node, &addr(&wallet), 1_000_000);
    let chain = EconomyChain::new(g).unwrap();
    let me = public_key_bytes(&node);
    let good = chain.propose(&me, &[], 1);
    assert!(chain.check(&me, &good).is_ok());
    // wrong proposer claimed, wrong root, wrong height, trailing bytes
    assert_eq!(
        chain.check(&public_key_bytes(&key(32)), &good),
        Err(EconError::BadProposer)
    );
    let mut b = teko_economy::EconBlock::decode(good.as_slice()).unwrap();
    b.state_root = vec![0; 32];
    assert_eq!(
        chain.check(&me, &b.encode_to_vec()),
        Err(EconError::StateRoot)
    );
    let mut b = teko_economy::EconBlock::decode(good.as_slice()).unwrap();
    b.height = 5;
    assert!(matches!(
        chain.check(&me, &b.encode_to_vec()),
        Err(EconError::WrongHeight { .. })
    ));
    let mut extra = good.clone();
    extra.push(0);
    assert_eq!(chain.check(&me, &extra), Err(EconError::Decode));
    // a stranger who is not a validator cannot propose
    let stranger = public_key_bytes(&key(33));
    let theirs = chain.propose(&stranger, &[], 1);
    assert_eq!(
        chain.check(&stranger, &theirs),
        Err(EconError::NotAValidator)
    );
    // an invalid transaction never gets into a proposal (select_valid drops it)
    let poor = key(34);
    let bad = Tx::new_signed(
        &poor,
        CHAIN,
        0,
        teko_coin::COIN_MODULE,
        Op::Transfer {
            to: [1; 32],
            amount: 5,
        }
        .encode(),
    );
    let p = chain.propose(&me, &[bad], 1);
    assert!(
        teko_economy::EconBlock::decode(p.as_slice())
            .unwrap()
            .txs
            .is_empty()
    );
}

#[test]
fn equivocation_evidence_becomes_a_slash_of_the_owner() {
    let (node, wallet) = (key(40), key(41));
    let g = genesis(&node, &addr(&wallet), 1_000_000);
    let mut c = Cluster::new(&g, vec![node.clone()]);
    c.until(3);
    let st = tx(&wallet, c.chain(0), Op::Stake { amount: 2_000 });
    c.submit_all(&st);
    c.until(5);
    // the NODE key signs two conflicting prevotes at an old height
    let h = c.chain(0).height();
    let a = Vote::new(CHAIN, &node, VoteKind::Prevote, h, 0, Some(&[1; 32]));
    let b = Vote::new(CHAIN, &node, VoteKind::Prevote, h, 0, Some(&[2; 32]));
    let ev = Evidence {
        a: Some(a),
        b: Some(b),
    };
    let reporter = key(42);
    let alloc_fee = tx(
        &wallet,
        c.chain(0),
        Op::Transfer {
            to: addr(&reporter),
            amount: 10,
        },
    );
    c.submit_all(&alloc_fee);
    c.until(6);
    let slash = tx(
        &reporter,
        c.chain(0),
        Op::Slash {
            evidence: ev.encode_to_vec(),
        },
    );
    c.submit_all(&slash);
    c.until(8);
    let l = c.chain(0).ledger();
    assert_eq!(teko_coin::stake_of(l, &addr(&wallet)), 1_800); // 10% of the OWNER's stake
    assert_eq!(teko_coin::burned(l), 200);
    teko_coin::audit(l).unwrap();
    let _ = wallet_address(&addr(&wallet));
}

#[test]
fn stake_becomes_bounded_voting_power() {
    assert_eq!(quantize(&[1]), vec![10_000]);
    assert_eq!(quantize(&[1, 1]), vec![5_000, 5_000]);
    assert_eq!(quantize(&[900_000, 100_000]), vec![9_000, 1_000]);
    // tiny validators keep a vote; nobody gets zero power; the sum never exceeds the units
    let p = quantize(&[1_000_000_000, 1]);
    assert_eq!(p[1], 1);
    assert!(p.iter().sum::<u64>() <= 10_001);
    assert_eq!(quantize(&[]), Vec::<u64>::new());
}

#[test]
fn block_time_is_the_proposers_it_only_moves_forward_and_must_be_near_the_voters_clock() {
    use teko_bft::App;
    let (node, wallet) = (key(40), key(41));
    let g = genesis(&node, &addr(&wallet), 1_000_000);
    let mut c = Cluster::new(&g, vec![node.clone()]);
    c.until(4);
    let chain = c.chain(0);
    let me = public_key_bytes(&node);

    // every block's time is strictly after the previous one
    let times: Vec<u64> = chain
        .blocks()
        .iter()
        .map(|d| d.block().unwrap().timestamp)
        .collect();
    assert!(times.windows(2).all(|w| w[0] < w[1]), "{times:?}");

    // a block at or before the previous block's time is refused; one second later is fine
    let last = chain.last_timestamp();
    assert_eq!(last, *times.last().unwrap());
    assert_eq!(
        chain.check(&me, &chain.propose(&me, &[], last)),
        Err(EconError::BadTime)
    );
    assert_eq!(
        chain.check(&me, &chain.propose(&me, &[], last - 5)),
        Err(EconError::BadTime)
    );
    assert!(chain.check(&me, &chain.propose(&me, &[], last + 1)).is_ok());

    // a validator votes only for a time near ITS clock (60 s)
    let clock_now = 1_700_000_100u64;
    let fresh = EconomyChain::new(g.clone()).unwrap();
    let app = EconApp::new(fresh, &node, Box::new(move || clock_now));
    let near = app.chain.propose(&me, &[], clock_now + 59);
    let far = app.chain.propose(&me, &[], clock_now + 3_600);
    let past = app.chain.propose(&me, &[], clock_now - 3_600);
    assert!(app.validate_from(1, &me, &near));
    assert!(!app.validate_from(1, &me, &far));
    assert!(!app.validate_from(1, &me, &past));
}
