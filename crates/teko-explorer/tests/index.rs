#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The explorer index over a real chain: one founder validator, real commit
//! certificates, no sockets.

use rand::SeedableRng;
use rand::rngs::StdRng;
use teko_bft::{Commit, Vote, VoteKind, block_id};
use teko_coin::{COIN_MODULE, Op};
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes, wallet_address};
use teko_economy::{DecidedBlock, EconomyChain, EconomyGenesis};
use teko_explorer::{ExplorerError, Index, SearchResult, TxKind};
use teko_ledger::{Address, Tx, address_of};

const CHAIN: &str = "teko-explorer-test";
const REWARD: u64 = 1_000;

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut StdRng::seed_from_u64(seed))
}

fn addr(k: &SigningKey) -> Address {
    address_of(&public_key_bytes(k))
}

fn text(k: &SigningKey) -> String {
    wallet_address(&addr(k)).unwrap()
}

fn genesis(node: &SigningKey, owner: &Address) -> EconomyGenesis {
    let mut g = EconomyGenesis::test_defaults(CHAIN, public_key_bytes(node), *owner);
    g.coin.initial_reward = REWARD;
    g.coin.halving_interval = 10_000;
    g.coin.founder_target = 1_000_000;
    g.coin.min_total_stake = 1;
    g.coin.min_validator_stake = 100;
    g.coin.fee_per_byte = 0;
    g
}

/// Builds final blocks (with real certificates) from a founder-only validator set.
struct Maker {
    chain: EconomyChain,
    node: SigningKey,
    blocks: Vec<DecidedBlock>,
}

impl Maker {
    fn new(g: &EconomyGenesis, node: &SigningKey) -> Self {
        Self {
            chain: EconomyChain::new(g.clone()).unwrap(),
            node: node.clone(),
            blocks: Vec::new(),
        }
    }

    fn make(&mut self, txs: &[Tx]) {
        let h = self.chain.height() + 1;
        let payload = self
            .chain
            .propose(&public_key_bytes(&self.node), txs, 1_700_000_000 + h);
        let id = block_id(&payload);
        let vote = Vote::new(CHAIN, &self.node, VoteKind::Precommit, h, 0, Some(&id));
        let d = DecidedBlock {
            payload,
            commit: Some(Commit {
                height: h,
                round: 0,
                block_id: id,
                precommits: vec![vote],
            }),
        };
        self.chain.apply_decided(d.clone()).unwrap();
        self.blocks.push(d);
    }

    fn nonce(&self, k: &SigningKey) -> u64 {
        self.chain.ledger().nonce_of(&addr(k))
    }
}

fn transfer(k: &SigningKey, nonce: u64, to: &Address, amount: u64) -> Tx {
    Tx::new_signed(
        k,
        CHAIN,
        nonce,
        COIN_MODULE,
        Op::Transfer { to: *to, amount }.encode(),
    )
}

fn setup() -> (Maker, EconomyGenesis, SigningKey, SigningKey) {
    let (node, wallet) = (key(1), key(2));
    let g = genesis(&node, &addr(&wallet));
    (Maker::new(&g, &node), g, node, wallet)
}

#[test]
fn rewards_go_to_the_owner_and_the_supply_matches_the_ledger() {
    let (mut m, g, _node, wallet) = setup();
    for _ in 0..3 {
        m.make(&[]);
    }
    let mut ix = Index::new(g).unwrap();
    assert_eq!(ix.sync(&m.blocks).unwrap(), 3);
    let st = ix.status();
    assert_eq!(st.height, 3);
    assert_eq!(st.minted.ypy, 3 * REWARD);
    assert!(st.founder_phase_active && st.verified);
    assert_eq!(
        st.state_root,
        teko_crypto::hex::encode(&m.chain.ledger().state_root())
    );

    let b = ix.block(2).unwrap();
    assert_eq!(b.reward.ypy, REWARD);
    assert_eq!(b.proposer_owner, text(&wallet));
    assert_eq!(b.precommits, 1);

    let acc = ix.account(&text(&wallet)).unwrap();
    assert!(acc.known);
    assert_eq!(acc.balance.ypy, 3 * REWARD);
    ix.audit().unwrap();
}

#[test]
fn a_transfer_shows_up_with_its_decoded_kind_the_recipient_and_the_search() {
    let (mut m, g, _node, wallet) = setup();
    m.make(&[]);
    let bob = key(3);
    let tx = transfer(&wallet, m.nonce(&wallet), &addr(&bob), 400);
    let id = teko_crypto::hex::encode(&tx.id());
    m.make(&[tx]);
    let mut ix = Index::new(g).unwrap();
    ix.sync(&m.blocks).unwrap();

    let t = ix.tx(&id).unwrap();
    assert_eq!(t.height, 2);
    assert_eq!(t.sender, text(&wallet));
    match &t.kind {
        TxKind::Transfer { to, amount } => {
            assert_eq!(to, &text(&bob));
            assert_eq!(amount.ypy, 400);
            assert_eq!(amount.text, "0,0004 TEKO");
        }
        other => panic!("unexpected kind {other:?}"),
    }
    let bob_view = ix.account(&text(&bob)).unwrap();
    assert!(bob_view.known);
    assert_eq!(bob_view.balance.ypy, 400);
    assert_eq!(bob_view.tx_count, 1);
    assert_eq!(
        bob_view.recent_txs.first().map(|x| x.id.clone()),
        Some(id.clone())
    );

    assert_eq!(ix.search(&id), SearchResult::Tx { id: id.clone() });
    assert_eq!(ix.search("2"), SearchResult::Block { height: 2 });
    let hash = ix.block(1).unwrap().hash.clone();
    assert_eq!(ix.search(&hash), SearchResult::Block { height: 1 });
    assert_eq!(
        ix.search(&text(&bob)),
        SearchResult::Account {
            address: text(&bob)
        }
    );
    assert_eq!(ix.search("nada disso"), SearchResult::NotFound);
    assert_eq!(ix.search("99"), SearchResult::NotFound);
}

#[test]
fn a_block_with_a_forged_certificate_is_refused_and_the_index_keeps_its_chain() {
    let (mut m, g, _node, _wallet) = setup();
    m.make(&[]);
    m.make(&[]);
    let mut bad = m.blocks.clone();
    // certificate signed by someone who is not a validator
    let impostor = key(99);
    let id = block_id(&bad[1].payload);
    let vote = Vote::new(CHAIN, &impostor, VoteKind::Precommit, 2, 0, Some(&id));
    bad[1].commit = Some(Commit {
        height: 2,
        round: 0,
        block_id: id,
        precommits: vec![vote],
    });
    let mut ix = Index::new(g).unwrap();
    let err = ix.sync(&bad).unwrap_err();
    assert!(
        matches!(err, ExplorerError::Block { height: 2, .. }),
        "{err}"
    );
    assert_eq!(
        ix.height(),
        1,
        "the good block 1 stays; the forged one is not indexed"
    );
    // the honest chain then completes
    assert_eq!(ix.sync(&m.blocks).unwrap(), 1);
    assert_eq!(ix.height(), 2);
}

#[test]
fn syncing_in_pieces_equals_syncing_at_once() {
    let (mut m, g, _node, wallet) = setup();
    for _ in 0..2 {
        m.make(&[]);
    }
    let tx = transfer(&wallet, m.nonce(&wallet), &addr(&key(5)), 10);
    m.make(&[tx]);
    m.make(&[]);

    let mut whole = Index::new(g.clone()).unwrap();
    whole.sync(&m.blocks).unwrap();
    let mut parts = Index::new(g).unwrap();
    parts.sync(&m.blocks[..2]).unwrap();
    assert_eq!(parts.sync(&m.blocks).unwrap(), 2);
    assert_eq!(
        parts.sync(&m.blocks).unwrap(),
        0,
        "nothing new: nothing done"
    );
    assert_eq!(whole.status(), parts.status());
    assert_eq!(whole.blocks(None, 100), parts.blocks(None, 100));
    assert_eq!(whole.latest_txs(10), parts.latest_txs(10));
}

#[test]
fn block_list_is_newest_first_and_paginates() {
    let (mut m, g, _node, _wallet) = setup();
    for _ in 0..5 {
        m.make(&[]);
    }
    let mut ix = Index::new(g).unwrap();
    ix.sync(&m.blocks).unwrap();
    let all: Vec<u64> = ix.blocks(None, 100).iter().map(|b| b.height).collect();
    assert_eq!(all, vec![5, 4, 3, 2, 1]);
    let page: Vec<u64> = ix.blocks(Some(4), 2).iter().map(|b| b.height).collect();
    assert_eq!(page, vec![3, 2]);
    assert!(ix.blocks(Some(1), 10).is_empty());
    assert!(ix.block(0).is_none() && ix.block(6).is_none());
}

#[test]
fn validators_and_the_founder_phase_are_visible() {
    let (mut m, g, node, wallet) = setup();
    m.make(&[]);
    let mut ix = Index::new(g).unwrap();
    ix.sync(&m.blocks).unwrap();
    let v = ix.validators();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].key, teko_crypto::hex::encode(&public_key_bytes(&node)));
    assert_eq!(v[0].owner, text(&wallet));
    assert!(v[0].power >= 1);
    assert_eq!(
        ix.search(&v[0].key),
        SearchResult::Validator {
            key: v[0].key.clone(),
            owner: text(&wallet)
        }
    );
    assert_eq!(ix.status().validators, 1);
}

#[test]
fn an_incompatible_saved_chain_is_explained_with_the_steps_to_recreate_it() {
    let e = ExplorerError::Block {
        height: 7,
        why: "BadTx(\"Tx { index: 0, error: Module(\\\"metadata hash must be 32 bytes\\\") }\")"
            .into(),
    };
    let text = e.to_string();
    assert!(
        text.contains("bloco 7") && text.contains("make devnet-founder"),
        "{text}"
    );
    // other failures keep the plain message
    let plain = ExplorerError::Block {
        height: 1,
        why: "Commit".into(),
    }
    .to_string();
    assert!(!plain.contains("make devnet-founder"));
}

#[test]
fn an_asset_registered_to_someone_shows_in_their_history_not_only_the_signers() {
    use teko_assets::{ASSETS_MODULE, Approval, Intent, Op as AssetOp, RegisterOp};
    let (node, wallet) = (key(1), key(2));
    let registrar = key(40);
    let mut g = genesis(&node, &addr(&wallet));
    g.classes.push(teko_economy::ClassSpec {
        name: "imovel".into(),
        threshold: 1,
        registrars: vec![public_key_bytes(&registrar)],
        window_secs: 1_200,
    });
    let mut m = Maker::new(&g, &node);
    m.make(&[]);
    let (owner, relayer) = (key(41), key(42));
    let meta = [7u8; 32];
    let intent = Intent::register(b"imovel", b"casa-1", addr(&owner), &meta);
    let approval = Approval::sign(&registrar, CHAIN, &intent);
    let tx = Tx::new_signed(
        &relayer,
        CHAIN,
        0,
        ASSETS_MODULE,
        AssetOp::Register(RegisterOp {
            class: b"imovel".to_vec(),
            asset: b"casa-1".to_vec(),
            owner: addr(&owner).to_vec(),
            metadata_hash: meta.to_vec(),
            approvals: vec![approval],
        })
        .encode(),
    );
    m.make(&[tx]);
    let mut ix = Index::new(g).unwrap();
    ix.sync(&m.blocks).unwrap();
    // the OWNER never signed anything, yet the registration is in their history
    let acc = ix.account(&text(&owner)).unwrap();
    assert!(acc.known);
    assert_eq!(acc.recent_txs.len(), 1, "{:?}", acc.recent_txs);
    assert_eq!(acc.assets.len(), 1);
}
