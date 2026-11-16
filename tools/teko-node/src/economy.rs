//! The economy service of a node: the economy ledger (moeda, bens, stake) on a BFT lane (ADR-0028).
//!
//! The consensus host is the generic `Lane`; this module is what is specific to the economy:
//! the application glue, equivocation turned into a `Slash`, the transaction wire messages and
//! the status reply.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prost::Message;
use teko_bft::{Commit, Evidence, ValidatorSet};
use teko_crypto::SigningKey;
use teko_economy::{DecidedBlock, EconApp, EconomyChain, EconomyGenesis};
use teko_ledger::Tx;

use crate::error::NodeError;
use crate::lane::{self, Kinds, Lane, LaneApp, Setup, ack, lock_lane, send_all};
use crate::node::Shared;
use crate::wire::{self, EconStatusReply, EconValidator, Msg, SubmitTxs};

/// Public configuration of the economy chain of this network.
#[derive(Clone)]
pub struct EconomyConfig {
    pub genesis: EconomyGenesis,
}

pub(crate) type Econ = Lane<EconApp>;

pub(crate) const KINDS: Kinds = Kinds {
    bft: wire::ECON_BFT,
    commit: wire::ECON_COMMIT,
    blocks: wire::ECON_BLOCKS,
};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl LaneApp for EconApp {
    type Decided = DecidedBlock;
    const NAME: &'static str = "economy";

    fn height(&self) -> u64 {
        self.chain.height()
    }

    fn validator_set(&self) -> Option<ValidatorSet> {
        self.chain.validator_set().ok()
    }

    fn pending(&self) -> usize {
        self.mempool_len()
    }

    fn apply(&mut self, d: DecidedBlock) -> Result<(), NodeError> {
        self.on_decided(d).map_err(|e| NodeError::replay(&e))
    }

    fn parts(d: &DecidedBlock) -> Option<(u64, Vec<u8>, Commit)> {
        let b = d.block().ok()?;
        Some((b.height, d.payload.clone(), d.commit.clone()?))
    }

    fn make(payload: Vec<u8>, commit: Commit) -> DecidedBlock {
        DecidedBlock {
            payload,
            commit: Some(commit),
        }
    }

    fn blocks_from(&self, from: u64, max: usize) -> Vec<Vec<u8>> {
        self.chain
            .blocks_from(from)
            .iter()
            .take(max)
            .map(Message::encode_to_vec)
            .collect()
    }

    /// Turns an equivocation proof into a `Slash` transaction signed by this node, queues it
    /// and gossips it (the ledger also refuses a repeat). The node account pays no fee, so this
    /// works while `fee_per_byte` is 0 (ADR-0030).
    fn on_evidence(&mut self, ev: &Evidence, key: &SigningKey) -> Vec<Msg> {
        slash_msgs(
            self,
            key,
            teko_coin::Op::Slash {
                evidence: ev.encode_to_vec(),
            },
        )
    }
}

/// Queues a penalty transaction signed by this node and returns the message that gossips it.
fn slash_msgs(app: &mut EconApp, key: &SigningKey, op: teko_coin::Op) -> Vec<Msg> {
    let me = teko_ledger::address_of(&teko_crypto::public_key_bytes(key));
    let queued = app.queued_from(&me);
    let nonce = app.chain.ledger().nonce_of(&me) + queued;
    let tx = Tx::new_signed(
        key,
        app.chain.ledger().chain_id(),
        nonce,
        teko_coin::COIN_MODULE,
        op.encode(),
    );
    let raw = tx.to_bytes();
    if app.submit(tx).is_ok() {
        vec![wire::msg(wire::ECON_TX_FWD, &SubmitTxs { txs: vec![raw] })]
    } else {
        Vec::new()
    }
}

/// Equivocation detected on the VOTE chain becomes a `SlashVotes` in the economy, where the
/// institution's bond is (ADR-0035). Returns the messages that gossip the transactions.
pub(crate) fn punish_vote_equivocations(
    econ: &mut Econ,
    key: &SigningKey,
    evidence: &[Evidence],
) -> Vec<Msg> {
    let mut out = Vec::new();
    for ev in evidence {
        out.extend(slash_msgs(
            econ.app_mut(),
            key,
            teko_coin::Op::SlashVotes {
                evidence: ev.encode_to_vec(),
            },
        ));
    }
    out
}

/// Opens the economy lane of a node: the chain is rebuilt from `economy.blocks` (every block
/// re-verified) and the engine starts idle.
pub(crate) fn open(
    dir: &Path,
    cfg: &EconomyConfig,
    key: &SigningKey,
    index: usize,
    peers: Vec<SocketAddr>,
    batch: Duration,
) -> Result<Econ, NodeError> {
    let chain = EconomyChain::new(cfg.genesis.clone())
        .map_err(|e| NodeError::Economy(format!("genesis: {e:?}")))?;
    let app = EconApp::new(chain, key, Box::new(now_secs));
    Lane::open(
        dir,
        app,
        Setup {
            chain_id: &cfg.genesis.chain_id,
            key,
            index,
            peers,
            kinds: KINDS,
            batch,
        },
    )
}

pub(crate) fn status(e: &Econ) -> EconStatusReply {
    let chain = &e.app().chain;
    let l = chain.ledger();
    let set = chain.validator_set().ok();
    let powers: BTreeMap<Vec<u8>, u64> = set
        .as_ref()
        .map(|s| {
            s.validators()
                .iter()
                .map(|v| (v.key.clone(), v.power))
                .collect()
        })
        .unwrap_or_default();
    EconStatusReply {
        chain_id: chain.chain_id().to_string(),
        height: chain.height(),
        state_root: l.state_root().to_vec(),
        mempool: e.app().mempool_len() as u32,
        founder_phase_active: teko_coin::founder_phase_active(l),
        total_stake: teko_coin::total_stake(l),
        validators: powers
            .iter()
            .map(|(k, p)| EconValidator {
                key: k.clone(),
                owner: chain.owner_of(k).to_vec(),
                power: *p,
            })
            .collect(),
        is_validator: powers.contains_key(&e.me()),
        round: e.engine.round(),
        evidence: e.evidence,
        founder_target: teko_coin::config(l).map_or(0, |c| c.founder_target),
    }
}

/// Handles the economy wire messages. Returns None for kinds that are not ours.
pub(crate) fn handle(s: &Arc<Shared>, req: &Msg) -> Option<Msg> {
    let econ = s.econ.as_ref()?;
    if let Some(r) = lane::handle(econ, req) {
        return Some(r);
    }
    let reply = match req.kind {
        wire::ECON_TX | wire::ECON_TX_FWD => {
            let Ok(b) = wire::decode::<SubmitTxs>(req) else {
                return Some(ack(false, "malformed", 0));
            };
            let mut accepted = 0u32;
            let mut fwd = Vec::new();
            {
                let mut e = lock_lane(econ);
                for raw in &b.txs {
                    let Ok(tx) = Tx::from_bytes(raw) else {
                        return Some(ack(false, "invalid transaction", accepted));
                    };
                    match e.app_mut().submit(tx) {
                        Ok(true) => {
                            accepted += 1;
                            fwd.push(raw.clone());
                        }
                        Ok(false) => {}
                        Err(why) => return Some(ack(false, &why, accepted)),
                    }
                }
                if req.kind == wire::ECON_TX && !fwd.is_empty() {
                    let m = wire::msg(wire::ECON_TX_FWD, &SubmitTxs { txs: fwd });
                    let (peers, index) = (e.peers.clone(), e.index);
                    drop(e);
                    send_all(&peers, index, vec![m]);
                }
            }
            ack(true, "queued", accepted)
        }
        wire::ECON_STATUS => {
            let e = lock_lane(econ);
            wire::msg(wire::ECON_STATUS_REPLY, &status(&e))
        }
        _ => return None,
    };
    Some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use teko_bft::{Evidence, VoteKind};
    use teko_crypto::{generate_signing_key, public_key_bytes};

    #[test]
    fn an_equivocation_becomes_one_slash_transaction_per_offence() {
        let d = std::env::temp_dir().join(format!("teko-econ-slash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut rng = rand::rngs::StdRng::seed_from_u64(5);
        let me = generate_signing_key(&mut rng);
        let bad = generate_signing_key(&mut rng);
        let owner = teko_ledger::address_of(&public_key_bytes(&me));
        let genesis =
            EconomyGenesis::test_defaults("teko-test-economy", public_key_bytes(&me), owner);
        let cfg = EconomyConfig { genesis };
        let mut e = open(&d, &cfg, &me, 0, Vec::new(), Duration::from_millis(20)).unwrap();
        let chain_id = cfg.genesis.chain_id.clone();
        let a = teko_bft::Vote::new(&chain_id, &bad, VoteKind::Prevote, 1, 0, Some(&[1; 32]));
        let b = teko_bft::Vote::new(&chain_id, &bad, VoteKind::Prevote, 1, 0, Some(&[2; 32]));
        let ev = Evidence {
            a: Some(a),
            b: Some(b),
        };
        let mut out = Vec::new();
        e.report(&ev, &mut out);
        assert_eq!(out.len(), 1, "the slash is gossiped");
        assert_eq!(e.app().mempool_len(), 1);
        e.report(&ev, &mut out);
        assert_eq!(out.len(), 1, "the same offence is not reported twice");
        assert_eq!(e.app().mempool_len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
