use std::collections::{BTreeMap, BTreeSet};

use teko_crypto::{SigningKey, public_key_bytes};

use crate::types::{Commit, Evidence, Proposal, ValidatorSet, Vote, VoteKind, block_id};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Propose,
    Prevote,
    Precommit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Proposal(Proposal),
    Vote(Vote),
}

/// What the engine asks its host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Send to every other validator (the engine has already counted it as its own).
    Broadcast(Message),
    /// Call `on_timeout(round, step)` after this many milliseconds.
    Schedule {
        round: u32,
        step: Step,
        after_ms: u64,
    },
    /// Final: this block is decided at this height; `commit` proves it.
    Decide { commit: Commit, payload: Vec<u8> },
    /// A validator signed two conflicting votes: slashable proof.
    Evidence(Evidence),
}

/// The application: builds blocks when this node proposes and judges proposals.
pub trait App {
    fn build(&mut self, height: u64) -> Vec<u8>;
    fn validate(&self, height: u64, payload: &[u8]) -> bool;

    /// Like `validate`, also told WHO proposed the block (the validator the engine
    /// already checked is the right one for the round). Applications whose blocks name
    /// their proposer (to pay rewards) override this to cross-check it.
    fn validate_from(&self, height: u64, _proposer: &[u8], payload: &[u8]) -> bool {
        self.validate(height, payload)
    }
}

/// Timeouts grow with the round so a slow network eventually fits inside them.
pub fn timeout_ms(step: Step, round: u32) -> u64 {
    let r = u64::from(round.min(30));
    match step {
        Step::Propose => 1_000 + 500 * r,
        Step::Prevote | Step::Precommit => 500 + 250 * r,
    }
}

/// validator -> block id (empty = nil) -> vote.
type VotePool = BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, Vote>>;

pub struct Engine<A: App> {
    chain_id: String,
    key: SigningKey,
    me: Vec<u8>,
    set: ValidatorSet,
    app: A,
    height: u64,
    round: u32,
    step: Step,
    /// `(block id, round)` this validator is locked on (it precommitted it).
    locked: Option<(Vec<u8>, u32)>,
    /// `(block id, payload, round)` that gathered 2/3 prevotes (re-proposed if we lead).
    valid: Option<(Vec<u8>, Vec<u8>, u32)>,
    proposals: BTreeMap<u32, Proposal>,
    /// `(round, kind) -> validator -> block id (empty = nil) -> vote`. A validator that
    /// signs two different values is an EQUIVOCATOR: both votes are kept and counted per
    /// value (the algorithm's proofs assume a faulty validator may say anything), and the
    /// pair is reported as evidence.
    votes: BTreeMap<(u32, VoteKind), VotePool>,
    fired: BTreeSet<(u32, u8)>,
    decided: bool,
    active: bool,
}

impl<A: App> Engine<A> {
    pub fn new(chain_id: &str, key: SigningKey, app: A) -> Result<Self, crate::BftError> {
        let me = public_key_bytes(&key);
        Ok(Self {
            chain_id: chain_id.to_string(),
            key,
            set: ValidatorSet::new(vec![crate::Validator {
                key: me.clone(),
                power: 1,
            }])?,
            me,
            app,
            height: 0,
            round: 0,
            step: Step::Propose,
            locked: None,
            valid: None,
            proposals: BTreeMap::new(),
            votes: BTreeMap::new(),
            fired: BTreeSet::new(),
            decided: false,
            active: false,
        })
    }

    pub fn app(&self) -> &A {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut A {
        &mut self.app
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn round(&self) -> u32 {
        self.round
    }

    pub fn key(&self) -> &[u8] {
        &self.me
    }

    /// Begins deciding `height` with validator set `set`.
    pub fn start_height(&mut self, height: u64, set: ValidatorSet) -> Vec<Action> {
        self.start_height_at(height, set, 0)
    }

    /// Like `start_height`, beginning at round `first_round`. A validator that restarts
    /// uses it to skip the rounds it may already have voted in (a vote in a NEW round
    /// is never a conflict), so a restart does not make it sign twice in one round.
    pub fn start_height_at(
        &mut self,
        height: u64,
        set: ValidatorSet,
        first_round: u32,
    ) -> Vec<Action> {
        self.set = set;
        self.height = height;
        self.locked = None;
        self.valid = None;
        self.proposals.clear();
        self.votes.clear();
        self.fired.clear();
        self.decided = false;
        self.active = true;
        let mut out = Vec::new();
        self.start_round(first_round, &mut out);
        self.evaluate(&mut out);
        out
    }

    pub fn on_message(&mut self, m: Message) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.active || self.decided {
            return out;
        }
        match m {
            Message::Proposal(p) => self.on_proposal(p),
            Message::Vote(v) => self.on_vote(v, &mut out),
        }
        self.evaluate(&mut out);
        out
    }

    /// Everything THIS validator has said at this height (its proposals and votes).
    /// Hosts call it periodically: messages lost before the network recovers must be
    /// sent again for the protocol to make progress (Tendermint assumes eventually
    /// reliable gossip).
    pub fn resend(&self) -> Vec<Action> {
        if !self.active || self.decided {
            return Vec::new();
        }
        let mut out: Vec<Action> = self
            .proposals
            .values()
            .filter(|p| p.proposer == self.me)
            .map(|p| Action::Broadcast(Message::Proposal(p.clone())))
            .collect();
        for pool in self.votes.values() {
            if let Some(by_value) = pool.get(&self.me) {
                for v in by_value.values() {
                    out.push(Action::Broadcast(Message::Vote(v.clone())));
                }
            }
        }
        out
    }

    /// Catch-up: another node already decided this height. Accepts the block only with
    /// a valid commit certificate (more than 2/3 of the power), the right payload and
    /// the application's approval.
    pub fn on_commit(&mut self, commit: &Commit, payload: &[u8]) -> Vec<Action> {
        if !self.active
            || self.decided
            || commit.height != self.height
            || commit.block_id != block_id(payload)
            || !commit.verify(&self.chain_id, &self.set)
            || !self.app.validate_from(
                self.height,
                &self.set.proposer(commit.height, commit.round).key,
                payload,
            )
        {
            return Vec::new();
        }
        self.decided = true;
        vec![Action::Decide {
            commit: commit.clone(),
            payload: payload.to_vec(),
        }]
    }

    pub fn on_timeout(&mut self, round: u32, step: Step) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.active || self.decided || round != self.round {
            return out;
        }
        match step {
            Step::Propose if self.step == Step::Propose => {
                self.cast(VoteKind::Prevote, None, &mut out);
                self.step = Step::Prevote;
            }
            Step::Prevote if self.step == Step::Prevote => {
                self.cast(VoteKind::Precommit, None, &mut out);
                self.step = Step::Precommit;
            }
            Step::Precommit => self.start_round(self.round + 1, &mut out),
            _ => {}
        }
        self.evaluate(&mut out);
        out
    }

    // ───────────────────────── internals ─────────────────────────

    fn start_round(&mut self, round: u32, out: &mut Vec<Action>) {
        self.round = round;
        self.step = Step::Propose;
        if self.set.proposer(self.height, round).key == self.me {
            let (payload, vr) = match &self.valid {
                Some((_, payload, r)) => (payload.clone(), *r as i32),
                None => (self.app.build(self.height), -1),
            };
            let p = Proposal::new(&self.chain_id, &self.key, self.height, round, vr, payload);
            self.proposals.insert(round, p.clone());
            out.push(Action::Broadcast(Message::Proposal(p)));
        } else {
            out.push(Action::Schedule {
                round,
                step: Step::Propose,
                after_ms: timeout_ms(Step::Propose, round),
            });
        }
    }

    fn on_proposal(&mut self, p: Proposal) {
        if p.height != self.height
            || p.proposer != self.set.proposer(p.height, p.round).key
            || !p.verify(&self.chain_id)
            || self.proposals.contains_key(&p.round)
        {
            return;
        }
        self.proposals.insert(p.round, p);
    }

    fn on_vote(&mut self, v: Vote, out: &mut Vec<Action>) {
        let Some(kind) = v.vote_kind() else { return };
        if v.height != self.height
            || self.set.power_of(&v.validator).is_none()
            || !v.verify(&self.chain_id)
        {
            return;
        }
        let by_value = self
            .votes
            .entry((v.round, kind))
            .or_default()
            .entry(v.validator.clone())
            .or_default();
        if by_value.contains_key(&v.block_id) {
            return;
        }
        if let Some(prev) = by_value.values().next() {
            out.push(Action::Evidence(Evidence {
                a: Some(prev.clone()),
                b: Some(v.clone()),
            }));
        }
        by_value.insert(v.block_id.clone(), v);
    }

    /// Signs, counts as our own and broadcasts a vote.
    fn cast(&mut self, kind: VoteKind, id: Option<Vec<u8>>, out: &mut Vec<Action>) {
        if self.set.power_of(&self.me).is_none() {
            return; // an observer (not in the validator set) never votes
        }
        let v = Vote::new(
            &self.chain_id,
            &self.key,
            kind,
            self.height,
            self.round,
            id.as_deref(),
        );
        self.votes
            .entry((self.round, kind))
            .or_default()
            .entry(self.me.clone())
            .or_default()
            .insert(v.block_id.clone(), v.clone());
        out.push(Action::Broadcast(Message::Vote(v)));
    }

    fn power_for(&self, round: u32, kind: VoteKind, id: Option<&[u8]>) -> u64 {
        let want = id.unwrap_or(&[]);
        self.votes.get(&(round, kind)).map_or(0, |pool| {
            pool.iter()
                .filter(|(_, by_value)| by_value.contains_key(want))
                .filter_map(|(who, _)| self.set.power_of(who))
                .fold(0u64, u64::saturating_add)
        })
    }

    fn power_any(&self, round: u32, kind: VoteKind) -> u64 {
        self.votes.get(&(round, kind)).map_or(0, |pool| {
            pool.keys()
                .filter_map(|who| self.set.power_of(who))
                .fold(0u64, u64::saturating_add)
        })
    }

    fn evaluate(&mut self, out: &mut Vec<Action>) {
        for _ in 0..256 {
            if self.decided || self.try_decide(out) {
                return;
            }
            if self.try_skip_round(out) {
                continue;
            }
            let mut progressed = false;
            progressed |= self.rule_propose(out);
            progressed |= self.rule_prevote_timeout(out);
            progressed |= self.rule_lock(out);
            progressed |= self.rule_prevote_nil(out);
            progressed |= self.rule_precommit_timeout(out);
            if !progressed {
                return;
            }
        }
    }

    /// A proposal plus a quorum of precommits for it, in ANY round of this height.
    fn try_decide(&mut self, out: &mut Vec<Action>) -> bool {
        let candidates: Vec<(u32, Proposal)> = self
            .proposals
            .iter()
            .map(|(r, p)| (*r, p.clone()))
            .collect();
        for (r, p) in candidates {
            let id = block_id(&p.payload);
            if self
                .set
                .is_quorum(self.power_for(r, VoteKind::Precommit, Some(&id)))
                && self.app.validate_from(self.height, &p.proposer, &p.payload)
            {
                let precommits = self
                    .votes
                    .get(&(r, VoteKind::Precommit))
                    .map(|pool| {
                        pool.values()
                            .filter_map(|bv| bv.get(&id))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                self.decided = true;
                out.push(Action::Decide {
                    commit: Commit {
                        height: self.height,
                        round: r,
                        block_id: id,
                        precommits,
                    },
                    payload: p.payload,
                });
                return true;
            }
        }
        false
    }

    /// More than 1/3 of the power is already in a later round: join it.
    fn try_skip_round(&mut self, out: &mut Vec<Action>) -> bool {
        let mut by_round: BTreeMap<u32, BTreeSet<Vec<u8>>> = BTreeMap::new();
        for ((r, _), pool) in &self.votes {
            if *r > self.round {
                by_round.entry(*r).or_default().extend(pool.keys().cloned());
            }
        }
        for (r, p) in &self.proposals {
            if *r > self.round {
                by_round.entry(*r).or_default().insert(p.proposer.clone());
            }
        }
        for (r, who) in by_round {
            let power = who
                .iter()
                .filter_map(|k| self.set.power_of(k))
                .fold(0u64, u64::saturating_add);
            if self.set.is_one_third_plus(power) {
                self.start_round(r, out);
                return true;
            }
        }
        false
    }

    /// Tendermint lines 22 and 28: vote on the proposal of this round.
    fn rule_propose(&mut self, out: &mut Vec<Action>) -> bool {
        if self.step != Step::Propose {
            return false;
        }
        let Some(p) = self.proposals.get(&self.round).cloned() else {
            return false;
        };
        let id = block_id(&p.payload);
        let valid = self.app.validate_from(self.height, &p.proposer, &p.payload);
        let vote_for = if p.valid_round == -1 {
            valid && self.locked.as_ref().is_none_or(|(lid, _)| *lid == id)
        } else if p.valid_round >= 0
            && (p.valid_round as u32) < self.round
            && self.set.is_quorum(self.power_for(
                p.valid_round as u32,
                VoteKind::Prevote,
                Some(&id),
            ))
        {
            valid
                && self
                    .locked
                    .as_ref()
                    .is_none_or(|(lid, lr)| *lr <= p.valid_round as u32 || *lid == id)
        } else {
            return false; // line 28 not enabled yet (or malformed valid_round)
        };
        self.cast(VoteKind::Prevote, vote_for.then_some(id), out);
        self.step = Step::Prevote;
        true
    }

    /// Line 34: 2/3 of prevotes of any kind: start the prevote timeout once.
    fn rule_prevote_timeout(&mut self, out: &mut Vec<Action>) -> bool {
        if self.step == Step::Prevote
            && self
                .set
                .is_quorum(self.power_any(self.round, VoteKind::Prevote))
            && self.fired.insert((self.round, 1))
        {
            out.push(Action::Schedule {
                round: self.round,
                step: Step::Prevote,
                after_ms: timeout_ms(Step::Prevote, self.round),
            });
            return true;
        }
        false
    }

    /// Line 36: a valid proposal with 2/3 prevotes: lock on it and precommit it.
    fn rule_lock(&mut self, out: &mut Vec<Action>) -> bool {
        if self.step < Step::Prevote {
            return false;
        }
        let Some(p) = self.proposals.get(&self.round).cloned() else {
            return false;
        };
        let id = block_id(&p.payload);
        if !self
            .set
            .is_quorum(self.power_for(self.round, VoteKind::Prevote, Some(&id)))
            || !self.app.validate_from(self.height, &p.proposer, &p.payload)
            || !self.fired.insert((self.round, 2))
        {
            return false;
        }
        if self.step == Step::Prevote {
            self.locked = Some((id.clone(), self.round));
            self.cast(VoteKind::Precommit, Some(id.clone()), out);
            self.step = Step::Precommit;
        }
        self.valid = Some((id, p.payload, self.round));
        true
    }

    /// Line 44: 2/3 prevoted nil: precommit nil.
    fn rule_prevote_nil(&mut self, out: &mut Vec<Action>) -> bool {
        if self.step == Step::Prevote
            && self
                .set
                .is_quorum(self.power_for(self.round, VoteKind::Prevote, None))
        {
            self.cast(VoteKind::Precommit, None, out);
            self.step = Step::Precommit;
            return true;
        }
        false
    }

    /// Line 47: 2/3 of precommits of any kind: start the precommit timeout once.
    fn rule_precommit_timeout(&mut self, out: &mut Vec<Action>) -> bool {
        if self
            .set
            .is_quorum(self.power_any(self.round, VoteKind::Precommit))
            && self.fired.insert((self.round, 3))
        {
            out.push(Action::Schedule {
                round: self.round,
                step: Step::Precommit,
                after_ms: timeout_ms(Step::Precommit, self.round),
            });
            return true;
        }
        false
    }
}
