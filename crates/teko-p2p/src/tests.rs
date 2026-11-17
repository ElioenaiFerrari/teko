#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rand::SeedableRng;
use teko_crypto::{SigningKey, generate_signing_key, public_key_bytes};

use crate::NetError;
use crate::handshake::{accept, connect, connect_with};
use crate::identity::{NetIdentity, PeerPolicy};
use crate::session::{Session, write_record};

const T: Duration = Duration::from_secs(5);
const ANY: fn(u32) -> usize = |_| 1 << 20;

fn key(seed: u64) -> SigningKey {
    generate_signing_key(&mut rand::rngs::StdRng::seed_from_u64(seed))
}

fn id(seed: u64, chain: &str, genesis: &[u8]) -> NetIdentity {
    NetIdentity::generate(key(seed), chain, genesis).unwrap()
}

/// Accepts exactly the listed consensus keys.
struct Set(Vec<Vec<u8>>);
impl PeerPolicy for Set {
    fn accepts(&self, k: &[u8]) -> bool {
        self.0.iter().any(|x| x == k)
    }
}

fn set(seeds: &[u64]) -> Set {
    Set(seeds.iter().map(|s| public_key_bytes(&key(*s))).collect())
}

/// A listener whose thread runs `accept` for ONE connection and hands the result back.
fn serve(
    me: NetIdentity,
    policy: Set,
) -> (
    std::net::SocketAddr,
    thread::JoinHandle<Result<Session, NetError>>,
) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let h = thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        accept(s, &me, &policy)
    });
    (addr, h)
}

fn pair() -> (Session, Session) {
    let (addr, h) = serve(id(1, "c", b"g"), set(&[2]));
    let c = connect(
        TcpStream::connect(addr).unwrap(),
        &id(2, "c", b"g"),
        &set(&[1]),
        None,
    )
    .unwrap();
    (c, h.join().unwrap().unwrap())
}

#[test]
fn two_validators_prove_who_they_are_and_talk_through_an_encrypted_link() {
    let (mut c, mut s) = pair();
    assert_eq!(c.peer_key(), public_key_bytes(&key(1)).as_slice());
    assert_eq!(s.peer_key(), public_key_bytes(&key(2)).as_slice());
    // an empty body, a small one and one far larger than a Noise record (several chunks)
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let t = thread::spawn(move || {
        for (k, b) in [(7u32, Vec::new()), (8, b"ola".to_vec()), (9, big.clone())] {
            c.send(k, &b, T).unwrap();
        }
        c
    });
    assert_eq!(s.recv(&ANY, T).unwrap(), (7, Vec::new()));
    assert_eq!(s.recv(&ANY, T).unwrap(), (8, b"ola".to_vec()));
    let (k, b) = s.recv(&ANY, T).unwrap();
    assert_eq!(
        (k, b.len(), b[250_000]),
        (9, 300_000, (250_000u32 % 251) as u8)
    );
    drop(t.join().unwrap());
}

#[test]
fn the_plaintext_never_appears_on_the_wire() {
    // a plain TCP middleman records everything the dialler sends
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut all = Vec::new();
        let mut buf = [0u8; 4096];
        s.set_read_timeout(Some(Duration::from_millis(400)))
            .unwrap();
        while let Ok(n) = std::io::Read::read(&mut s, &mut buf) {
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }
        all
    });
    let me = id(2, "c", b"g");
    // the middleman never answers: the handshake fails, and what it saw is only the first Noise message
    let _ = connect(TcpStream::connect(addr).unwrap(), &me, &set(&[1]), None);
    let seen = h.join().unwrap();
    let key = public_key_bytes(&key(2));
    assert!(
        !seen.windows(key.len()).any(|w| w == key.as_slice()),
        "the consensus key stays hidden"
    );
    assert!(!seen.is_empty());
}

#[test]
fn another_chain_another_genesis_or_a_key_outside_the_set_is_refused_on_both_ends() {
    // (the dialler's chain, genesis, key seed, the keys the server accepts)
    let cases: [(&str, &[u8], u64, &[u64]); 4] = [
        ("other", b"g", 2, &[2]),
        ("c", b"other", 2, &[2]),
        ("c", b"g", 9, &[2]),
        ("c", b"g", 2, &[3]),
    ];
    for (chain, genesis, seed, accepted) in cases {
        let (addr, h) = serve(id(1, "c", b"g"), set(accepted));
        let c = connect(
            TcpStream::connect(addr).unwrap(),
            &id(seed, chain, genesis),
            &set(&[1]),
            None,
        );
        let srv = h.join().unwrap();
        assert!(
            srv.is_err(),
            "the server accepted {chain} {genesis:?} {seed}"
        );
        // the dialler is not left with a working link either (it learns on the first use)
        if let Ok(mut c) = c {
            let r = c
                .send(1, b"x", T)
                .and_then(|_| c.recv(&ANY, Duration::from_millis(500)).map(|_| ()));
            assert!(r.is_err());
        }
    }
}

#[test]
fn the_dialler_refuses_a_server_that_is_not_the_institution_it_dialled() {
    let (addr, h) = serve(id(1, "c", b"g"), set(&[2]));
    let want = public_key_bytes(&key(5));
    let r = connect(
        TcpStream::connect(addr).unwrap(),
        &id(2, "c", b"g"),
        &set(&[1, 5]),
        Some(&want),
    );
    assert!(matches!(r, Err(NetError::Handshake(_))));
    drop(h.join());
    // and a server outside the dialler's own policy
    let (addr, h) = serve(id(1, "c", b"g"), set(&[2]));
    let r = connect(
        TcpStream::connect(addr).unwrap(),
        &id(2, "c", b"g"),
        &set(&[7]),
        None,
    );
    assert!(matches!(r, Err(NetError::Handshake(_))));
    drop(h.join());
}

#[test]
fn a_proof_cannot_be_lent_to_another_noise_key_or_forged() {
    let victim = id(2, "c", b"g");
    let mallory = id(3, "c", b"g");
    // the server trusts the victim's institution
    for forged in [
        // the victim's REAL proof, replayed from Mallory's connection (bound to the victim's Noise key)
        victim.proof(1),
        // Mallory names the victim's key and signs with her own
        {
            let mut p = mallory.proof(1);
            p.consensus_key = victim.consensus_key();
            p
        },
        // the victim's proof for the OTHER role (reflection)
        victim.proof(2),
    ] {
        let (addr, h) = serve(id(1, "c", b"g"), set(&[2]));
        let _ = connect_with(
            TcpStream::connect(addr).unwrap(),
            &mallory,
            &set(&[1]),
            None,
            forged,
        );
        assert!(matches!(h.join().unwrap(), Err(NetError::Handshake(_))));
    }
}

#[test]
fn a_tampered_replayed_or_reordered_frame_never_decrypts() {
    // tampered
    let (mut c, mut s) = pair();
    let mut rec = c.seal_for_test(&[0, 0, 0, 5, 0, 0, 0, 1]);
    let n = rec.len();
    rec[n - 1] ^= 1;
    write_record(&mut c.stream().try_clone().unwrap(), &rec).unwrap();
    assert!(matches!(s.recv(&ANY, T), Err(NetError::BadFrame)));
    // replayed: the same ciphertext twice
    let (mut c, mut s) = pair();
    let head = c.seal_for_test(&[0, 0, 0, 6, 0, 0, 0, 0]);
    let mut w = c.stream().try_clone().unwrap();
    write_record(&mut w, &head).unwrap();
    write_record(&mut w, &head).unwrap();
    assert_eq!(s.recv(&ANY, T).unwrap(), (6, Vec::new()));
    assert!(matches!(s.recv(&ANY, T), Err(NetError::BadFrame)));
    // reordered: the second record before the first
    let (mut c, mut s) = pair();
    let a = c.seal_for_test(&[0, 0, 0, 6, 0, 0, 0, 0]);
    let b = c.seal_for_test(&[0, 0, 0, 7, 0, 0, 0, 0]);
    let mut w = c.stream().try_clone().unwrap();
    write_record(&mut w, &b).unwrap();
    write_record(&mut w, &a).unwrap();
    assert!(matches!(s.recv(&ANY, T), Err(NetError::BadFrame)));
}

#[test]
fn a_body_larger_than_its_kind_allows_is_refused_before_it_is_read() {
    let (mut c, mut s) = pair();
    let t = thread::spawn(move || {
        c.send(3, &vec![1u8; 100_000], T).unwrap();
        c
    });
    let small: &dyn Fn(u32) -> usize = &|k| if k == 3 { 1_000 } else { 1 << 20 };
    assert!(matches!(s.recv(small, T), Err(NetError::TooLarge)));
    drop(t.join().unwrap());
}

#[test]
fn a_peer_that_drips_bytes_is_cut_by_the_deadline_of_the_whole_message() {
    let (c, mut s) = pair();
    let mut w = c.stream().try_clone().unwrap();
    let mut c = c;
    // a valid header, then the body record announced but never finished: a length of 1000 and 3 bytes
    let head = c.seal_for_test(&[0, 0, 0, 4, 0, 0, 0, 50]);
    write_record(&mut w, &head).unwrap();
    std::io::Write::write_all(&mut w, &[0x03, 0xE8, 1, 2, 3]).unwrap();
    let started = std::time::Instant::now();
    let r = s.recv(&ANY, Duration::from_millis(400));
    assert!(matches!(r, Err(NetError::Timeout)), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn the_keys_change_in_lockstep_after_many_records() {
    let (mut c, mut s) = pair();
    let t = thread::spawn(move || {
        for i in 0..40_000u32 {
            c.send(1, &i.to_be_bytes(), T).unwrap();
        }
        c
    });
    for i in 0..40_000u32 {
        let (k, b) = s.recv(&ANY, T).unwrap();
        assert_eq!((k, b), (1, i.to_be_bytes().to_vec()));
    }
    drop(t.join().unwrap());
}

#[test]
fn the_noise_key_file_is_made_once_and_reloaded() {
    let dir = std::env::temp_dir().join(format!("teko-p2p-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("noise.key");
    let _ = std::fs::remove_file(&path);
    let a = NetIdentity::load_or_create(key(1), "c", b"g", &path).unwrap();
    let b = NetIdentity::load_or_create(key(1), "c", b"g", &path).unwrap();
    assert_eq!(a.noise_public, b.noise_public);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::write(&path, "zz").unwrap();
    assert!(NetIdentity::load_or_create(key(1), "c", b"g", &path).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = Arc::new(0);
}

mod fuzz {
    use super::*;
    use proptest::prelude::*;
    use std::io::Write;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// Whatever bytes arrive at the door, the handshake ends in an error, never a panic and never a session.
        #[test]
        fn random_bytes_never_open_a_session(junk in proptest::collection::vec(any::<u8>(), 0..2_000)) {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = l.local_addr().unwrap();
            let me = id(1, "c", b"g");
            let h = thread::spawn(move || {
                let (s, _) = l.accept().unwrap();
                accept(s, &me, &set(&[2])).is_err()
            });
            let mut c = TcpStream::connect(addr).unwrap();
            let _ = c.write_all(&junk);
            drop(c);
            let refused = h.join().unwrap();
            prop_assert!(refused);
        }

        /// After a good handshake, garbage records are refused (or time out), never panic, and never
        /// make the receiver allocate more than the kind's limit.
        #[test]
        fn random_records_after_a_handshake_are_refused(junk in proptest::collection::vec(any::<u8>(), 2..3_000)) {
            let (c, mut s) = pair();
            let mut w = c.stream().try_clone().unwrap();
            let _ = w.write_all(&junk);
            let small: &dyn Fn(u32) -> usize = &|_| 1_000;
            let r = s.recv(small, Duration::from_millis(300));
            prop_assert!(r.is_err());
        }
    }
}
