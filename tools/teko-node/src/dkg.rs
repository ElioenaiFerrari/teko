//! The key ceremony, distributed across the validator PROCESSES: each node makes its
//! own secret polynomial, publishes only commitments, and gets one share from every
//! peer straight over the node-to-node connection. The coordinator (the `dkg`
//! command) only relays PUBLIC data and never sees a share.
//!
//! Local test network: loopback, unauthenticated. A real ceremony needs
//! authenticated, encrypted channels between institutions (ADR-0019).

use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use teko_ballot_crypto::dkg::{KeyShare, Participant, Round1, public_outputs, verify_packages};
use teko_ballot_crypto::dkg_wire::{DkgRecord, Round1Msg};
use teko_ballot_crypto::{point_from_bytes, point_to_bytes, scalar_from_bytes, scalar_to_bytes};
use teko_bulletin::{Entry, EntryKind};
use teko_crypto::hex;

use crate::client::{NodeClient, call};
use crate::error::NodeError;
use crate::node::DkgState;
use crate::wire::{self, DkgPackages, DkgReply, DkgShareReq, DkgStart};

const T: Duration = Duration::from_secs(10);

pub(crate) fn reply(ok: bool, info: &str) -> DkgReply {
    DkgReply {
        ok,
        info: info.to_string(),
        round1: None,
        share: Vec::new(),
        group_key: Vec::new(),
    }
}

pub(crate) fn dkg_msg(r: DkgReply) -> wire::Msg {
    wire::msg(wire::DKG_REPLY, &r)
}

/// Round 1 at a node: make the polynomial, keep it, publish the commitments.
pub(crate) fn start(state: &mut Option<DkgState>, my_index: usize, m: &DkgStart) -> DkgReply {
    if m.n < 2 || m.threshold == 0 || m.threshold > m.n || my_index as u32 >= m.n {
        return reply(false, "bad parameters");
    }
    let mut rng = UnwrapErr(SysRng);
    match Participant::new(
        &mut rng,
        &m.session,
        my_index as u32 + 1,
        m.n,
        m.threshold as usize,
    ) {
        Ok((p, r1)) => {
            *state = Some(DkgState {
                participant: p,
                session: m.session.clone(),
                n: m.n,
                threshold: m.threshold as usize,
            });
            let mut out = reply(true, "round 1");
            out.round1 = Some(Round1Msg::from(&r1));
            out
        }
        Err(_) => reply(false, "bad parameters"),
    }
}

/// A peer asks for the share meant for it. Released only while a ceremony is running.
pub(crate) fn share_for(state: &Option<DkgState>, requester: u32) -> DkgReply {
    match state
        .as_ref()
        .and_then(|s| s.participant.share_for(requester))
    {
        Some(sh) => {
            let mut out = reply(true, "share");
            out.share = scalar_to_bytes(&sh).to_vec();
            out
        }
        None => reply(false, "no ceremony or bad index"),
    }
}

/// Round 2 at a node: verify every package, collect the shares from the peers, finish.
pub(crate) fn finish(
    st: &DkgState,
    my_index: usize,
    peers: &[SocketAddr],
    m: &DkgPackages,
) -> Result<KeyShare, String> {
    if m.session != st.session || m.threshold as usize != st.threshold {
        return Err("another ceremony".into());
    }
    let packages: Vec<Round1> = m
        .packages
        .iter()
        .map(Round1Msg::to_round1)
        .collect::<Option<Vec<_>>>()
        .ok_or("malformed package")?;
    verify_packages(&st.session, st.n, st.threshold, &packages).map_err(|e| format!("{e:?}"))?;
    let me = my_index as u32 + 1;
    let mut received = vec![(me, st.participant.share_for(me).ok_or("own share")?)];
    for (i, addr) in peers.iter().enumerate() {
        if i == my_index {
            continue;
        }
        let req = wire::msg(wire::DKG_SHARE, &DkgShareReq { requester: me });
        let r = call(*addr, &req, T).map_err(|e| e.to_string())?;
        let rep: DkgReply = wire::decode(&r).map_err(|e| e.to_string())?;
        let sh = scalar_from_bytes(&rep.share).ok_or("bad share encoding")?;
        received.push((i as u32 + 1, sh));
    }
    st.participant
        .finish(&st.session, st.threshold, &packages, &received)
        .map_err(|e| format!("{e:?}"))
}

pub(crate) fn save_share(dir: &Path, ks: &KeyShare) -> std::io::Result<()> {
    let mut text = format!(
        "# Teko election key share (SECRET, test keys only)\nindex={}\nsecret={}\ngroup_key={}\n",
        ks.index,
        hex::encode(&scalar_to_bytes(&ks.secret)),
        hex::encode(&point_to_bytes(&ks.group_public_key))
    );
    for (j, vk) in &ks.verification_keys {
        text.push_str(&format!("vk{j}={}\n", hex::encode(&point_to_bytes(vk))));
    }
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(dir.join("tally-share.key"))?
        .write_all(text.as_bytes())
}

pub fn load_share(dir: &Path) -> Option<KeyShare> {
    let text = fs::read_to_string(dir.join("tally-share.key")).ok()?;
    let (mut index, mut secret, mut group_key) = (None, None, None);
    let mut vks = Vec::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "index" => index = v.parse::<u32>().ok(),
            "secret" => secret = scalar_from_bytes(&hex::decode(v).ok()?),
            "group_key" => group_key = point_from_bytes(&hex::decode(v).ok()?),
            k if k.starts_with("vk") => {
                let j: u32 = k[2..].parse().ok()?;
                vks.push((j, point_from_bytes(&hex::decode(v).ok()?)?));
            }
            _ => {}
        }
    }
    vks.sort_by_key(|x| x.0);
    Some(KeyShare {
        index: index?,
        secret: secret?,
        group_public_key: group_key?,
        verification_keys: vks,
    })
}

/// Runs the ceremony across the running validator nodes and records it on the
/// chain. Returns the on-chain record (public). `dir` gets `tally-key.pub`.
pub fn run_ceremony(
    client: &NodeClient,
    dir: &Path,
    session: &[u8],
    threshold: usize,
) -> Result<DkgRecord, NodeError> {
    let addrs = client.addrs().to_vec();
    let n = addrs.len() as u32;
    let bad = |m: &str| NodeError::Refused(format!("key ceremony: {m}"));
    // round 1: every node publishes its commitments
    let start_msg = wire::msg(
        wire::DKG_START,
        &DkgStart {
            session: session.to_vec(),
            n,
            threshold: threshold as u32,
        },
    );
    let mut packages = Vec::new();
    for a in &addrs {
        let r: DkgReply = wire::decode(&call(*a, &start_msg, T)?)?;
        if !r.ok {
            return Err(bad(&r.info));
        }
        packages.push(r.round1.ok_or_else(|| bad("no round-1 package"))?);
    }
    // the coordinator checks the public data too
    let r1: Vec<Round1> = packages
        .iter()
        .map(Round1Msg::to_round1)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| bad("malformed package"))?;
    verify_packages(session, n, threshold, &r1).map_err(|e| bad(&format!("{e:?}")))?;
    let (y, _) = public_outputs(n, &r1).map_err(|e| bad(&format!("{e:?}")))?;
    // round 2: each node takes its shares from the peers and finishes
    let pk_msg = wire::msg(
        wire::DKG_PACKAGES,
        &DkgPackages {
            session: session.to_vec(),
            threshold: threshold as u32,
            packages: packages.clone(),
        },
    );
    for a in &addrs {
        let r: DkgReply = wire::decode(&call(*a, &pk_msg, T)?)?;
        if !r.ok {
            return Err(bad(&r.info));
        }
        if r.group_key != point_to_bytes(&y) {
            return Err(bad("a node derived a different election key"));
        }
    }
    let record = DkgRecord {
        session: session.to_vec(),
        threshold: threshold as u32,
        n,
        packages,
        group_key: point_to_bytes(&y).to_vec(),
    };
    record.verify().map_err(|e| bad(&format!("{e:?}")))?;
    let entry = Entry::new(EntryKind::DKG, prost::Message::encode_to_vec(&record));
    client.submit(std::slice::from_ref(&entry))?;
    // The record is only real once the network DECIDED it (BFT, ADR-0034): an entry still in a
    // mempool is lost if the nodes restart. Wait until a verified replica carries it.
    let profile = crate::setup::load_network(dir)?.profile;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let included = client.sync_chain(&profile).is_ok_and(|chain| {
            (0..teko_bulletin::BulletinBoard::len(chain.board()))
                .any(|i| teko_bulletin::BulletinBoard::entry(chain.board(), i) == Some(&entry))
        });
        if included {
            break;
        }
        if std::time::Instant::now() > deadline {
            return Err(bad(
                "the network did not decide the key ceremony record in time",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    fs::write(
        dir.join("tally-key.pub"),
        format!("{}\n", hex::encode(&point_to_bytes(&y))),
    )?;
    Ok(record)
}
