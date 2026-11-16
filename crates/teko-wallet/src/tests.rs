use super::*;
use proptest::prelude::*;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn ent(s: &str) -> [u8; 32] {
    unhex(s).try_into().unwrap()
}

/// Official SLIP-10 test vector 1 for nist256p1.
#[test]
fn slip10_nist256p1_vector_1() {
    let seed = unhex("000102030405060708090a0b0c0d0e0f");
    let m = slip10(&seed, &[]);
    assert_eq!(
        hex(&m.bytes[32..]),
        "beeb672fe4621673f722f38529c07392fecaa61015c80c34f29ce8b41b3cb6ea"
    );
    assert_eq!(
        hex(&m.bytes[..32]),
        "612091aaa12e22dd2abef664f8a01a82cae99ad7441b7ef8110424915c268bc2"
    );
    let m0 = slip10(&seed, &[0]);
    assert_eq!(
        hex(&m0.bytes[32..]),
        "3460cea53e6a6bb5fb391eeef3237ffd8724bf0a40e94943c98b83825342ee11"
    );
    assert_eq!(
        hex(&m0.bytes[..32]),
        "6939694369114c67917a182c59ddb8cafc3004e63ca5d3b84403ba8613debc0c"
    );
}

/// Official BIP-39 vectors (Trezor `vectors.json`, passphrase "TREZOR").
#[test]
fn bip39_official_vectors() {
    let cases = [
        (
            "0000000000000000000000000000000000000000000000000000000000000000",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
            "bda85446c68413707090a52022edd26a1c9462295029f2e60cd7c4f2bbd3097170af7a4d73245cafa9c3cca8d561a7c3de6f5d4a10be8ed2a5e608d68f92fcc8",
        ),
        (
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
            "dd48c104698c30cfe2b6142103248622fb7bb0ff692eebb00089b32d22484e1613912f0a5b694407be899ffd31ed3992c456cdf60f5d4564b8ba3f05a69890ad",
        ),
        (
            "066dca1a2bb7e8a1db2832148ce9933eea0f3ac9548d793112d9a95c9407efad",
            "all hour make first leader extend hole alien behind guard gospel lava path output census museum junior mass reopen famous sing advance salt reform",
            "26e975ec644423f4a4c4f4215ef09b4bd7ef924e85d1d17c4cf3f136c2863cf6df0a475045652c57eb5fb41513ca2a2d67722b77e954b4b3fc11f7590449191d",
        ),
    ];
    for (e, words, seed) in cases {
        let p = Phrase::from_entropy(&ent(e)).unwrap();
        assert_eq!(p.words().join(" "), words);
        let parsed = Phrase::parse(&words.split(' ').collect::<Vec<_>>()).unwrap();
        assert_eq!(parsed.words(), p.words());
        let m = mnemonic_of(p.words()).unwrap();
        assert_eq!(hex(&m.to_seed_normalized("TREZOR")), seed);
    }
}

#[test]
fn phrase_validation() {
    let p = Phrase::from_entropy(&[7u8; 32]).unwrap();
    let mut w: Vec<String> = p.words().to_vec();
    assert!(Phrase::parse(&w).is_ok());
    // wrong count
    assert_eq!(
        Phrase::parse(&w[..23]).unwrap_err(),
        WalletError::InvalidPhrase
    );
    // not in the list
    let mut bad = w.clone();
    bad[3] = "zzzzzz".into();
    assert_eq!(Phrase::parse(&bad).unwrap_err(), WalletError::InvalidPhrase);
    // checksum: swap two different words
    let i = (0..23).find(|i| w[*i] != w[*i + 1]).unwrap();
    w.swap(i, i + 1);
    assert_eq!(Phrase::parse(&w).unwrap_err(), WalletError::InvalidPhrase);
    // case and spaces are forgiven
    let loud: Vec<String> = p
        .words()
        .iter()
        .map(|x| format!("  {} ", x.to_uppercase()))
        .collect();
    assert!(Phrase::parse(&loud).is_ok());
}

#[test]
fn matches_and_autocomplete() {
    let p = Phrase::from_entropy(&[1u8; 32]).unwrap();
    let w = p.word(5).unwrap().to_string();
    assert!(p.matches(5, &w));
    assert!(p.matches(5, &format!(" {} ", w.to_uppercase())));
    assert!(!p.matches(6, &w) || p.word(6) == p.word(5));
    assert!(!p.matches(99, &w));
    assert!(is_word("abandon") && !is_word("teko"));
    assert!(complete("aba").contains(&"abandon"));
    assert!(complete("zzz").is_empty());
}

#[test]
fn passphrase_changes_the_wallet_and_is_validated() {
    let p = Phrase::from_entropy(&[9u8; 32]).unwrap();
    let a = p.master_key("").unwrap().address(0).unwrap();
    let a2 = p.master_key("").unwrap().address(0).unwrap();
    let b = p.master_key("outra senha").unwrap().address(0).unwrap();
    assert_eq!(a, a2);
    assert_ne!(a, b);
    assert_eq!(
        p.master_key("ação").unwrap_err(),
        WalletError::InvalidPassphrase
    );
    assert_eq!(
        p.master_key(&"x".repeat(MAX_PASSPHRASE + 1)).unwrap_err(),
        WalletError::InvalidPassphrase
    );
    assert_eq!(
        p.master_key("tab\there").unwrap_err(),
        WalletError::InvalidPassphrase
    );
}

#[test]
fn addresses_are_distinct_per_index_and_match_the_ledger() {
    let m = Phrase::from_entropy(&[3u8; 32])
        .unwrap()
        .master_key("")
        .unwrap();
    let a0 = m.address(0).unwrap();
    assert_ne!(a0, m.address(1).unwrap());
    assert_eq!(a0, teko_ledger::address_of(&m.public_key(0).unwrap()));
    let s = m.address_string(0).unwrap();
    assert!(s.starts_with("tekow1"));
    assert_eq!(teko_crypto::parse_wallet_address(&s).unwrap(), a0);
    // persisted form round-trips
    let again = MasterKey::from_bytes(m.as_bytes()).unwrap();
    assert_eq!(again.address(0).unwrap(), a0);
    assert!(MasterKey::from_bytes(&[0u8; 64]).is_none());
    assert!(MasterKey::from_bytes(&[1u8; 10]).is_none());
}

#[test]
fn the_wallet_is_independent_of_the_identity_derivation() {
    // Same bytes used as an identity root and as wallet entropy give unrelated keys.
    let root = [5u8; 32];
    let id = teko_crypto::derive_signing_key(&root, teko_crypto::tags::IDENTITY).unwrap();
    let w = Phrase::from_entropy(&root).unwrap().master_key("").unwrap();
    assert_ne!(teko_crypto::public_key_bytes(&id), w.public_key(0).unwrap());
}

#[test]
fn debug_output_never_shows_secrets() {
    let p = Phrase::from_entropy(&[2u8; 32]).unwrap();
    let m = p.master_key("").unwrap();
    let d = format!("{p:?} {m:?}");
    assert!(!d.contains(p.word(0).unwrap()) || p.word(0).unwrap().len() < 3);
    assert!(d.contains("redacted"));
}

proptest! {
    #[test]
    fn entropy_to_words_to_entropy_roundtrip(e in proptest::array::uniform32(any::<u8>())) {
        let p = Phrase::from_entropy(&e).unwrap();
        prop_assert_eq!(p.len(), 24);
        let parsed = Phrase::parse(p.words()).unwrap();
        let m1 = p.master_key("").unwrap();
        let m2 = parsed.master_key("").unwrap();
        prop_assert_eq!(m1.as_bytes(), m2.as_bytes());
    }

    #[test]
    fn replacing_one_word_is_caught_or_changes_the_wallet(e in proptest::array::uniform32(any::<u8>()), pos in 0usize..24, other in 0usize..2048) {
        let p = Phrase::from_entropy(&e).unwrap();
        let mut w = p.words().to_vec();
        let repl = Language::English.word_list()[other].to_string();
        prop_assume!(w[pos] != repl);
        w[pos] = repl;
        match Phrase::parse(&w) {
            Err(_) => {}
            Ok(q) => prop_assert_ne!(q.master_key("").unwrap().address(0).unwrap(), p.master_key("").unwrap().address(0).unwrap()),
        }
    }

    #[test]
    fn wallet_addresses_detect_single_typos(addr in proptest::array::uniform32(any::<u8>()), pos in any::<usize>(), repl in 0usize..32) {
        const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
        let s = teko_crypto::wallet_address(&addr).unwrap();
        prop_assert_eq!(teko_crypto::parse_wallet_address(&s).unwrap(), addr);
        let sep = s.rfind('1').unwrap();
        let idx = sep + 1 + pos % (s.len() - sep - 1);
        let mut bytes = s.clone().into_bytes();
        prop_assume!(bytes[idx] != CHARSET[repl]);
        bytes[idx] = CHARSET[repl];
        prop_assert!(teko_crypto::parse_wallet_address(&String::from_utf8(bytes).unwrap()).is_err());
    }
}

#[test]
fn credential_keys_are_per_type_deterministic_and_unrelated_to_the_wallet_addresses() {
    let phrase = Phrase::from_entropy(&[7u8; 32]).unwrap();
    let m = phrase.master_key("").unwrap();
    let a = public_key_bytes(&m.credential_key("cnh-b").unwrap());
    assert_eq!(a, public_key_bytes(&m.credential_key("cnh-b").unwrap()));
    assert_ne!(a, public_key_bytes(&m.credential_key("cnh-a").unwrap()));
    assert_ne!(a, m.public_key(0).unwrap());
    // another passphrase is another wallet: it finds none of the credentials
    let other = phrase.master_key("x").unwrap();
    assert_ne!(a, public_key_bytes(&other.credential_key("cnh-b").unwrap()));
    // restoring from the same words and passphrase gives the same keys
    let again = Phrase::parse(phrase.words())
        .unwrap()
        .master_key("")
        .unwrap();
    assert_eq!(a, public_key_bytes(&again.credential_key("cnh-b").unwrap()));
}
