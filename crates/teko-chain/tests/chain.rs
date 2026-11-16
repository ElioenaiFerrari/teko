//! The vote chain with BFT finality (ADR-0034): what a replica accepts and what it refuses.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use prost::Message;
use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_bft::{App, Commit, Vote, VoteKind, block_id};
use teko_bulletin::{BulletinBoard, Entry, EntryKind, MemoryBoard};
use teko_chain::{
    Block, Chain, ChainError, DecidedBlock, NetworkKind, NetworkProfile, VoteApp, genesis_hash,
};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};

fn setup(kind: NetworkKind, n: usize) -> (Vec<SigningKey>, Chain) {
    let mut rng = StdRng::seed_from_u64(9);
    let keys: Vec<SigningKey> = (0..n).map(|_| generate_signing_key(&mut rng)).collect();
    let pubs = keys.iter().map(public_key_bytes).collect();
    let profile = NetworkProfile::new(kind, pubs, 2).unwrap();
    (keys, Chain::new(profile))
}

fn entries(n: u8) -> Vec<Entry> {
    (0..n)
        .map(|i| Entry::new(EntryKind::BALLOT, vec![i; 40]))
        .collect()
}

/// The institution the consensus names as proposer of the next height in round `round`.
fn proposer_index(c: &Chain, keys: &[SigningKey], round: u32) -> usize {
    let set = c.validator_set().unwrap();
    let who = set.proposer(c.height() + 1, round).key.clone();
    keys.iter()
        .position(|k| public_key_bytes(k) == who)
        .unwrap()
}

/// A final block: proposed by the named proposer, certified by the institutions in `who`.
fn decide(c: &Chain, keys: &[SigningKey], es: &[Entry], at: u64, who: &[usize]) -> DecidedBlock {
    decide_round(c, keys, es, at, who, 0)
}

fn decide_round(
    c: &Chain,
    keys: &[SigningKey],
    es: &[Entry],
    at: u64,
    who: &[usize],
    round: u32,
) -> DecidedBlock {
    let p = proposer_index(c, keys, round);
    let payload = c.propose(&public_key_bytes(&keys[p]), es, at);
    certify(c, keys, payload, who, round)
}

fn certify(
    c: &Chain,
    keys: &[SigningKey],
    payload: Vec<u8>,
    who: &[usize],
    round: u32,
) -> DecidedBlock {
    let height = c.height() + 1;
    let id = block_id(&payload);
    let precommits = who
        .iter()
        .map(|i| {
            Vote::new(
                &c.profile().chain_id,
                &keys[*i],
                VoteKind::Precommit,
                height,
                round,
                Some(&id),
            )
        })
        .collect();
    DecidedBlock {
        payload,
        commit: Some(Commit {
            height,
            round,
            block_id: id,
            precommits,
        }),
    }
}

#[test]
fn blocks_chain_and_a_second_replica_rebuilds_exactly_the_same_merkle_log() {
    let (k, mut c) = setup(NetworkKind::Devnet, 4);
    let mut reference = MemoryBoard::new();
    for round in 0..5u8 {
        let es = entries(round + 1);
        let b = decide(&c, &k, &es, 1_000 + u64::from(round), &[0, 1, 2]);
        c.apply(b).unwrap();
        for e in es {
            reference.append(e).unwrap();
        }
        assert_eq!(c.board().root(), reference.root(), "round {round}");
    }
    assert_eq!((c.height(), c.board().len()), (5, 15));
    let mut other = Chain::new(c.profile().clone());
    for b in c.blocks() {
        other.apply(b.clone()).unwrap();
    }
    assert_eq!(other.board().root(), c.board().root());
    assert_eq!(other.tip_hash(), c.tip_hash());
}

#[test]
fn every_entry_has_a_certified_block_and_a_proof_that_a_device_can_check_alone() {
    let (k, mut c) = setup(NetworkKind::Devnet, 4);
    assert!(c.inclusion_evidence(0).is_none(), "nothing yet");
    for round in 0..4u8 {
        let b = decide(
            &c,
            &k,
            &entries(round + 1),
            1_000 + u64::from(round),
            &[0, 1, 2],
        );
        c.apply(b).unwrap();
    }
    let n = c.board().len();
    for i in 0..n {
        let ev = c.inclusion_evidence(i).unwrap();
        let block = ev.decided.block().unwrap();
        // the FIRST block that holds it
        assert!(block.log_size > i);
        let before = c
            .blocks()
            .iter()
            .filter_map(|d| d.block().ok())
            .filter(|b| b.log_size > i)
            .map(|b| b.height)
            .min()
            .unwrap();
        assert_eq!(block.height, before);
        let leaf = c.board().entry(i).unwrap().leaf_hash();
        let root: [u8; 32] = block.log_root.as_slice().try_into().unwrap();
        assert!(teko_bulletin::merkle::verify_inclusion(
            &leaf,
            i,
            block.log_size,
            &ev.proof,
            &root
        ));
        // another entry's leaf does not verify with this proof
        let other = c.board().entry((i + 1) % n).unwrap().leaf_hash();
        if other != leaf {
            assert!(!teko_bulletin::merkle::verify_inclusion(
                &other,
                i,
                block.log_size,
                &ev.proof,
                &root
            ));
        }
    }
    assert!(c.inclusion_evidence(n).is_none());
}

#[test]
fn finality_needs_more_than_two_thirds_of_the_institutions() {
    // 4 institutions: 3 are a quorum, 2 are not (so the chain tolerates one failure)
    let (k, mut c) = setup(NetworkKind::Devnet, 4);
    let es = entries(2);
    assert_eq!(
        c.apply(decide(&c, &k, &es, 1, &[0, 1])),
        Err(ChainError::Commit)
    );
    assert_eq!(c.height(), 0, "nothing was committed");
    c.apply(decide(&c, &k, &es, 1, &[0, 1, 3])).unwrap();
    // 3 institutions tolerate none: 2 of 3 is not a quorum
    let (k3, mut c3) = setup(NetworkKind::Devnet, 3);
    assert_eq!(
        c3.apply(decide(&c3, &k3, &es, 1, &[0, 1])),
        Err(ChainError::Commit)
    );
    c3.apply(decide(&c3, &k3, &es, 1, &[0, 1, 2])).unwrap();
}

#[test]
fn a_certificate_is_not_transferable_forged_or_made_by_outsiders() {
    let (k, mut c) = setup(NetworkKind::Devnet, 4);
    let es = entries(2);
    let good = decide(&c, &k, &es, 1, &[0, 1, 2]);
    // no certificate
    let mut none = good.clone();
    none.commit = None;
    assert_eq!(c.apply(none), Err(ChainError::Commit));
    // a certificate for ANOTHER block
    let other = decide(&c, &k, &entries(3), 1, &[0, 1, 2]);
    let mut swapped = good.clone();
    swapped.commit = other.commit;
    assert_eq!(c.apply(swapped), Err(ChainError::Commit));
    // the same validator counted twice
    let mut dup = good.clone();
    let c0 = dup.commit.as_mut().unwrap();
    let first = c0.precommits[0].clone();
    c0.precommits.push(first);
    assert_eq!(c.apply(dup), Err(ChainError::Commit));
    // an outsider's precommit makes the whole certificate invalid
    let mut rng = StdRng::seed_from_u64(77);
    let outsider = generate_signing_key(&mut rng);
    let mut out = good.clone();
    let id = block_id(&good.payload);
    out.commit.as_mut().unwrap().precommits.push(Vote::new(
        &c.profile().chain_id,
        &outsider,
        VoteKind::Precommit,
        1,
        0,
        Some(&id),
    ));
    assert_eq!(c.apply(out), Err(ChainError::Commit));
    // a certificate signed for another network does not verify here
    let (_, mut test) = setup(NetworkKind::Testnet, 4);
    assert!(test.apply(good.clone()).is_err(), "devnet block on testnet");
    c.apply(good).unwrap();
}

#[test]
fn only_the_proposer_the_certificate_names_can_have_proposed_the_block() {
    let (k, mut c) = setup(NetworkKind::Devnet, 4);
    let es = entries(2);
    let right = proposer_index(&c, &k, 0);
    let wrong = (right + 1) % 4;
    // a block "proposed" by another institution, even if certified by a quorum
    let payload = c.propose(&public_key_bytes(&k[wrong]), &es, 1);
    let b = certify(&c, &k, payload, &[0, 1, 2], 0);
    assert!(matches!(c.apply(b), Err(ChainError::Block(_))));
    // a later round names another proposer: that block is fine in that round
    let r1 = proposer_index(&c, &k, 1);
    c.apply(decide_round(&c, &k, &es, 1, &[0, 1, 2], 1))
        .unwrap();
    let _ = r1;
}

#[test]
fn every_tampered_field_is_refused() {
    let (k, mut c) = setup(NetworkKind::Devnet, 4);
    c.apply(decide(&c, &k, &entries(2), 1_000, &[0, 1, 2]))
        .unwrap();
    let p = proposer_index(&c, &k, 0);
    let good_payload = c.propose(&public_key_bytes(&k[p]), &entries(3), 2_000);
    let check = |name: &str, f: &dyn Fn(&mut Block)| {
        let mut b = Block::decode(good_payload.as_slice()).unwrap();
        f(&mut b);
        // re-certified by a quorum (a certificate can be valid for a bad block): the CONTENT is refused
        let bad = certify(&c, &k, b.encode_to_vec(), &[0, 1, 2], 0);
        let mut copy = Chain::new(c.profile().clone());
        for x in c.blocks() {
            copy.apply(x.clone()).unwrap();
        }
        assert!(copy.apply(bad).is_err(), "{name} must be refused");
    };
    check("chain_id", &|b| b.chain_id = "teko-other-1".into());
    check("height", &|b| b.height += 1);
    check("prev_hash", &|b| b.prev_hash[0] ^= 1);
    check("proposer", &|b| b.proposer = vec![1; 33]);
    check("entry", &|b| b.entries[0].payload[0] ^= 1);
    check("extra entry", &|b| b.entries.push(b.entries[0].clone()));
    check("no entries", &|b| b.entries.clear());
    check("log_root", &|b| b.log_root[0] ^= 1);
    check("log_size", &|b| b.log_size += 1);
    check("entry kind", &|b| b.entries[0].kind = 300);
    // trailing bytes are not accepted either
    let mut trailing = good_payload.clone();
    trailing.push(0);
    assert!(c.check(&public_key_bytes(&k[p]), &trailing).is_err());
    c.apply(certify(&c, &k, good_payload, &[0, 1, 2], 0))
        .unwrap();
}

#[test]
fn block_time_only_moves_forward_and_a_validator_only_votes_near_its_clock() {
    let (k, mut c) = setup(NetworkKind::Devnet, 4);
    c.apply(decide(&c, &k, &entries(1), 1_000, &[0, 1, 2]))
        .unwrap();
    let p = public_key_bytes(&k[proposer_index(&c, &k, 0)]);
    // proposing with a clock at or before the previous block still moves forward (build never goes back)
    let b = Block::decode(c.propose(&p, &entries(1), 500).as_slice()).unwrap();
    assert_eq!(b.timestamp, 1_001);
    // a block that does not move time forward is refused by replay
    let mut stale = b.clone();
    stale.timestamp = 1_000;
    assert_eq!(
        c.check(&p, &stale.encode_to_vec()).err(),
        Some(ChainError::BadTime)
    );
    stale.timestamp = 900;
    assert_eq!(
        c.check(&p, &stale.encode_to_vec()).err(),
        Some(ChainError::BadTime)
    );

    // a validator votes only for a time within 60 s of ITS clock
    let clock = 1_000_000u64;
    let fresh = Chain::new(c.profile().clone());
    let app = VoteApp::new(fresh, &k[0], Box::new(move || clock));
    let me = public_key_bytes(&k[0]);
    let set = app.chain.validator_set().unwrap();
    let proposer = set.proposer(1, 0).key.clone();
    let mk = |t: u64| app.chain.propose(&proposer, &entries(1), t);
    assert!(app.validate_from(1, &proposer, &mk(clock + 59)));
    assert!(!app.validate_from(1, &proposer, &mk(clock + 3_600)));
    assert!(!app.validate_from(1, &proposer, &mk(clock - 3_600)));
    let _ = me;
}

#[test]
fn the_mempool_dedupes_and_what_a_block_carries_leaves_it() {
    let (k, c) = setup(NetworkKind::Devnet, 4);
    let mut app = VoteApp::new(c, &k[0], Box::new(|| 1_000));
    let es = entries(3);
    for e in &es {
        assert_eq!(app.submit(e.clone()), Ok(true));
    }
    assert_eq!(
        app.submit(es[0].clone()),
        Ok(false),
        "a repeat is not queued twice"
    );
    assert_eq!(app.mempool_len(), 3);
    assert!(
        app.submit(Entry::new(EntryKind::BALLOT, vec![0; 2 * 1024 * 1024]))
            .is_err()
    );
    // a block with two of them: the mempool keeps only the third, and the included ones stay "seen"
    let b = decide(&app.chain, &k, &es[..2], 1_000, &[0, 1, 2]);
    app.on_decided(b).unwrap();
    assert_eq!(app.mempool_len(), 1);
    assert_eq!(app.submit(es[0].clone()), Ok(false), "already in the log");
}

#[test]
fn a_block_is_never_empty_and_the_first_one_ties_to_the_chain_id() {
    let (k, c) = setup(NetworkKind::Devnet, 4);
    let p = public_key_bytes(&k[proposer_index(&c, &k, 0)]);
    assert!(
        c.check(&p, &c.propose(&p, &[], 1)).is_err(),
        "no empty blocks"
    );
    let b = Block::decode(c.propose(&p, &entries(1), 1).as_slice()).unwrap();
    assert_eq!(b.prev_hash, genesis_hash(&c.profile().chain_id).to_vec());
    assert_eq!(b.height, 1);
}

// ───────────────────────── rotation of the institutions (ADR-0045) ─────────────────────────

use teko_bft::{ValidatorSetChange, set_hash};

fn extra_key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}

/// An approved change replacing the LAST of the 4 institutions with `newcomer`.
fn replace_last(
    keys: &[SigningKey],
    newcomer: &SigningKey,
    epoch: u64,
    effective: u64,
    approvers: &[usize],
) -> ValidatorSetChange {
    let current: Vec<Vec<u8>> = keys[..4].iter().map(public_key_bytes).collect();
    let mut new = current.clone();
    new[3] = public_key_bytes(newcomer);
    let mut c = ValidatorSetChange {
        chain_id: "teko-devnet-1".into(),
        epoch,
        effective_height: effective,
        prev_set_hash: set_hash(&current).to_vec(),
        validators: new,
        ..Default::default()
    };
    for i in approvers {
        c.sign(&keys[*i]);
    }
    c
}

fn change_entry(c: &ValidatorSetChange) -> Entry {
    Entry::new(EntryKind::VALIDATOR_SET, c.encode_to_vec())
}

#[test]
fn an_approved_change_takes_effect_at_its_height_old_blocks_stay_valid_and_a_replica_rebuilds_the_epochs()
 {
    let (mut keys, mut c) = setup(NetworkKind::Devnet, 4);
    let newcomer = extra_key(501);
    keys.push(newcomer.clone());
    let change = replace_last(&keys, &newcomer, 1, 5, &[0, 1, 2]);
    // block 1: ordinary; block 2 carries the change (it takes effect at height 5)
    let b1 = decide(&c, &keys, &entries(2), 1_000, &[0, 1, 2]);
    c.apply(b1).unwrap();
    let b2 = decide(&c, &keys, &[change_entry(&change)], 1_001, &[0, 1, 2]);
    c.apply(b2).unwrap();
    assert_eq!(c.epochs().len(), 2, "the epoch exists, pending");
    assert_eq!(c.set_at(4).len(), 4);
    // blocks 3 and 4 still belong to the OLD set (the outgoing institution may certify them)
    for h in [3u64, 4] {
        assert_eq!(c.set_at(h)[3], public_key_bytes(&keys[3]), "height {h}");
        let b = decide(&c, &keys, &entries(1), 1_000 + h, &[0, 1, 3]);
        c.apply(b).unwrap();
    }
    // from height 5 the NEW set rules: the outgoing key is no longer an institution
    assert_eq!(c.set_at(5)[3], public_key_bytes(&newcomer));
    let old_votes = decide(&c, &keys, &entries(1), 1_100, &[0, 1, 3]);
    assert!(
        c.apply(old_votes).is_err(),
        "the removed institution's vote no longer counts: 2 of 4"
    );
    let with_new = decide(&c, &keys, &entries(1), 1_101, &[0, 1, 4]);
    c.apply(with_new).unwrap();
    assert_eq!(c.height(), 5);
    // a second replica rebuilds exactly the same epochs from the blocks alone
    let mut other = Chain::new(c.profile().clone());
    for b in c.blocks() {
        other.apply(b.clone()).unwrap();
    }
    assert_eq!(other.epochs(), c.epochs());
    assert_eq!(other.tip_hash(), c.tip_hash());
    // the approved changes a device needs to follow
    assert_eq!(c.changes_after(0), vec![change.clone()]);
    assert!(c.changes_after(1).is_empty());
}

#[test]
fn a_set_change_that_breaks_a_rule_makes_the_block_invalid() {
    let (mut keys, mut c) = setup(NetworkKind::Devnet, 4);
    let newcomer = extra_key(502);
    keys.push(newcomer.clone());
    c.apply(decide(&c, &keys, &entries(1), 1_000, &[0, 1, 2]))
        .unwrap();
    let me = public_key_bytes(&keys[0]);
    let refused = |c: &Chain, es: &[Entry]| {
        let payload = c.propose(&me, es, 1_500);
        c.check(&me, &payload).is_err()
    };
    let good = replace_last(&keys, &newcomer, 1, 6, &[0, 1, 2]);
    assert!(
        !refused(&c, &[change_entry(&good)]),
        "the sound change is accepted"
    );
    // not more than 2/3, an outsider approving, a wrong epoch, a wrong replaced set
    assert!(refused(
        &c,
        &[change_entry(&replace_last(&keys, &newcomer, 1, 6, &[0, 1]))]
    ));
    let mut outsider = replace_last(&keys, &newcomer, 1, 6, &[0, 1]);
    outsider.sign(&newcomer);
    assert!(
        refused(&c, &[change_entry(&outsider)]),
        "the newcomer cannot approve itself"
    );
    assert!(refused(
        &c,
        &[change_entry(&replace_last(
            &keys,
            &newcomer,
            2,
            6,
            &[0, 1, 2]
        ))]
    ));
    let mut stale = replace_last(&keys, &newcomer, 1, 6, &[]);
    stale.prev_set_hash = vec![7; 32];
    for k in &keys[..3] {
        stale.sign(k);
    }
    assert!(refused(&c, &[change_entry(&stale)]));
    // it must take effect AFTER the block that carries it (that block is height 2)
    assert!(refused(
        &c,
        &[change_entry(&replace_last(
            &keys,
            &newcomer,
            1,
            2,
            &[0, 1, 2]
        ))]
    ));
    assert!(refused(
        &c,
        &[change_entry(&replace_last(
            &keys,
            &newcomer,
            1,
            1,
            &[0, 1, 2]
        ))]
    ));
    // two changes in one block, and a malformed one
    assert!(refused(&c, &[change_entry(&good), change_entry(&good)]));
    assert!(refused(
        &c,
        &[Entry::new(EntryKind::VALIDATOR_SET, vec![1, 2, 3])]
    ));
    // a set of one institution, and a repeated key
    let mut one = good.clone();
    one.validators.truncate(1);
    one.signers.clear();
    one.signatures.clear();
    for k in &keys[..3] {
        one.sign(k);
    }
    assert!(refused(&c, &[change_entry(&one)]));
    let mut dup = good.clone();
    dup.validators[3] = dup.validators[0].clone();
    dup.signers.clear();
    dup.signatures.clear();
    for k in &keys[..3] {
        dup.sign(k);
    }
    assert!(refused(&c, &[change_entry(&dup)]));
    // while one is PENDING no other can be carried
    c.apply(decide(&c, &keys, &[change_entry(&good)], 1_001, &[0, 1, 2]))
        .unwrap();
    let again = replace_last(&keys, &newcomer, 2, 9, &[0, 1, 2]);
    assert!(
        refused(&c, &[change_entry(&again)]),
        "a change is already pending"
    );
}
