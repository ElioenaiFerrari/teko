use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_attestation::SignerSignature;
use teko_attestation::multisig::count_distinct_valid;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags};

use crate::error::ElectionError;
use crate::policy::GovernancePolicy;
use crate::text::sanitize_text;

/// Public ballots (ADR-0015, option 1). The ONLY kind allowed until the
/// product owner decides otherwise; kept as data so the format can evolve.
pub const BALLOT_KIND_PUBLIC: &str = "public";

const FORMAT_VERSION: u32 = 1;
const MAX_OPTIONS: usize = 64;

#[derive(Clone, PartialEq, Message)]
pub struct ElectionOption {
    /// Position in the ballot. Contiguous from 0.
    #[prost(uint32, tag = "1")]
    pub index: u32,
    #[prost(string, tag = "2")]
    pub name: String,
    /// Ballot number (e.g. "13"); may be empty.
    #[prost(string, tag = "3")]
    pub number: String,
}

/// Unix seconds. `registration_opens < voting_opens < voting_closes`.
#[derive(Clone, PartialEq, Message)]
pub struct Schedule {
    #[prost(uint64, tag = "1")]
    pub registration_opens: u64,
    #[prost(uint64, tag = "2")]
    pub voting_opens: u64,
    #[prost(uint64, tag = "3")]
    pub voting_closes: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ElectionManifest {
    #[prost(uint32, tag = "1")]
    pub format_version: u32,
    /// Lower-case `[a-z0-9-]`, 1..=64.
    #[prost(string, tag = "2")]
    pub election_id: String,
    #[prost(string, tag = "3")]
    pub title: String,
    #[prost(message, repeated, tag = "4")]
    pub options: Vec<ElectionOption>,
    #[prost(message, optional, tag = "5")]
    pub schedule: Option<Schedule>,
    #[prost(string, tag = "6")]
    pub ballot_kind: String,
    /// Voter-registry public key (ADR-0008). May be empty until decided.
    #[prost(bytes = "vec", tag = "7")]
    pub registry_key: Vec<u8>,
    /// Increases when the manifest is re-issued (a new manifest, never an edit).
    #[prost(uint32, tag = "8")]
    pub revision: u32,
    #[prost(message, repeated, tag = "9")]
    pub signatures: Vec<SignerSignature>,
}

impl ElectionManifest {
    /// Candidate name for the device screen; `None` if out of range.
    pub fn option_name(&self, index: u32) -> Option<&str> {
        self.options
            .iter()
            .find(|o| o.index == index)
            .map(|o| o.name.as_str())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// Structural validation (no signatures). Rejects anything a device should
/// not render: duplicates, gaps, spoofable text, impossible schedules.
pub fn validate_manifest(m: &ElectionManifest) -> Result<(), ElectionError> {
    let bad = ElectionError::InvalidManifest;
    if m.format_version != FORMAT_VERSION {
        return Err(bad("unsupported format version"));
    }
    if !valid_id(&m.election_id) {
        return Err(bad("election_id must be lower-case [a-z0-9-], 1..=64"));
    }
    if sanitize_text(&m.title, 120).is_none() {
        return Err(bad("invalid title"));
    }
    if m.ballot_kind != BALLOT_KIND_PUBLIC {
        return Err(bad("unsupported ballot_kind"));
    }
    if m.options.len() < 2 || m.options.len() > MAX_OPTIONS {
        return Err(bad("need 2..=64 options"));
    }
    for (i, o) in m.options.iter().enumerate() {
        if o.index as usize != i {
            return Err(bad("option indices must be contiguous from 0, in order"));
        }
        if sanitize_text(&o.name, 80).is_none() {
            return Err(bad("invalid option name"));
        }
        if !o.number.is_empty() && sanitize_text(&o.number, 8).is_none() {
            return Err(bad("invalid option number"));
        }
        let dup = m.options.iter().take(i).any(|p| {
            p.name.trim().eq_ignore_ascii_case(o.name.trim())
                || (!o.number.is_empty() && p.number == o.number)
        });
        if dup {
            return Err(bad("duplicate option name or number"));
        }
    }
    let s = m.schedule.as_ref().ok_or(bad("missing schedule"))?;
    if !(s.registration_opens < s.voting_opens && s.voting_opens < s.voting_closes) {
        return Err(bad(
            "schedule must be registration < voting opens < voting closes",
        ));
    }
    Ok(())
}

fn signing_bytes(m: &ElectionManifest) -> Vec<u8> {
    let mut unsigned = m.clone();
    unsigned.signatures = Vec::new();
    unsigned.encode_to_vec()
}

/// Identifier the quadro de avisos, transitions and the device bind to.
/// Independent of the signatures (adding a signature does not change it).
pub fn manifest_hash(m: &ElectionManifest) -> [u8; 32] {
    tagged_hash(tags::ELECTION_MANIFEST, &[&signing_bytes(m)])
}

pub fn sign_manifest(key: &SigningKey, m: &ElectionManifest) -> SignerSignature {
    SignerSignature {
        signer: public_key_bytes(key),
        signature: sign_tagged(key, tags::ELECTION_MANIFEST, &signing_bytes(m)),
    }
}

/// Validates the manifest AND requires M distinct valid governance signatures.
pub fn verify_manifest(
    policy: &GovernancePolicy,
    m: &ElectionManifest,
) -> Result<[u8; 32], ElectionError> {
    validate_manifest(m)?;
    let valid = count_distinct_valid(
        policy.signers(),
        tags::ELECTION_MANIFEST,
        &signing_bytes(m),
        &m.signatures,
    );
    if valid < policy.threshold() {
        return Err(ElectionError::InsufficientSignatures {
            valid,
            required: policy.threshold(),
        });
    }
    Ok(manifest_hash(m))
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::generate_signing_key;

    pub fn keys(n: usize, seed: u64) -> Vec<SigningKey> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..n).map(|_| generate_signing_key(&mut rng)).collect()
    }

    pub fn policy(keys: &[SigningKey], m: usize) -> GovernancePolicy {
        GovernancePolicy::new(keys.iter().map(public_key_bytes).collect(), m).unwrap()
    }

    pub fn manifest() -> ElectionManifest {
        ElectionManifest {
            format_version: 1,
            election_id: "municipal-2026".into(),
            title: "Eleição Municipal 2026 (exemplo)".into(),
            options: ["Candidata Onça", "Candidato Boto", "Candidata Arara"]
                .iter()
                .enumerate()
                .map(|(i, n)| ElectionOption {
                    index: i as u32,
                    name: (*n).into(),
                    number: alloc::format!("{}", 10 + i),
                })
                .collect(),
            schedule: Some(Schedule {
                registration_opens: 1_000,
                voting_opens: 2_000,
                voting_closes: 3_000,
            }),
            ballot_kind: BALLOT_KIND_PUBLIC.into(),
            registry_key: Vec::new(),
            revision: 1,
            signatures: Vec::new(),
        }
    }

    pub fn signed(keys: &[SigningKey], who: &[usize]) -> ElectionManifest {
        let mut m = manifest();
        m.signatures = who.iter().map(|i| sign_manifest(&keys[*i], &m)).collect();
        m
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn valid_manifest_passes_and_names_come_from_it() {
        let m = manifest();
        assert!(validate_manifest(&m).is_ok());
        assert_eq!(m.option_name(1), Some("Candidato Boto"));
        assert_eq!(m.option_name(3), None);
    }

    #[test]
    fn structural_rules_are_enforced() {
        let mut cases: Vec<(&str, ElectionManifest)> = Vec::new();
        let mut m = manifest();
        m.election_id = "Municipal 2026".into();
        cases.push(("id with uppercase/space", m));
        let mut m = manifest();
        m.options.truncate(1);
        cases.push(("single option", m));
        let mut m = manifest();
        m.options[1].index = 5;
        cases.push(("index gap", m));
        let mut m = manifest();
        m.options[1].name = "candidata onça".into();
        cases.push(("duplicate name (case-insensitive)", m));
        let mut m = manifest();
        m.options[2].number = "10".into();
        cases.push(("duplicate number", m));
        let mut m = manifest();
        m.options[0].name = "Fulano\u{202E}".into();
        cases.push(("bidi override in name", m));
        let mut m = manifest();
        m.title = "linha\nquebrada".into();
        cases.push(("newline in title", m));
        let mut m = manifest();
        m.schedule = Some(Schedule {
            registration_opens: 3,
            voting_opens: 2,
            voting_closes: 9,
        });
        cases.push(("schedule out of order", m));
        let mut m = manifest();
        m.schedule = None;
        cases.push(("no schedule", m));
        let mut m = manifest();
        m.ballot_kind = "encrypted".into();
        cases.push(("unsupported ballot kind", m));
        let mut m = manifest();
        m.format_version = 2;
        cases.push(("unknown format", m));
        for (name, m) in cases {
            assert!(
                matches!(
                    validate_manifest(&m),
                    Err(ElectionError::InvalidManifest(_))
                ),
                "should reject: {name}"
            );
        }
    }

    proptest! {
        /// Accepted iff >= M distinct, known signers signed (like firmware M-of-N).
        #[test]
        fn accepted_iff_m_distinct_valid_signers(seed in any::<u64>(), mask in 0u8..8, m_of in 1usize..=3) {
            let k = keys(3, seed);
            let pol = policy(&k, m_of);
            let who: Vec<usize> = (0..3).filter(|i| mask & (1 << i) != 0).collect();
            let m = signed(&k, &who);
            prop_assert_eq!(verify_manifest(&pol, &m).is_ok(), who.len() >= m_of);
        }

        #[test]
        fn duplicates_and_outsiders_count_zero(seed in any::<u64>(), copies in 2usize..6) {
            let k = keys(3, seed);
            let pol = policy(&k, 2);
            let mut m = manifest();
            m.signatures = vec![sign_manifest(&k[0], &m); copies];
            let e = verify_manifest(&pol, &m).unwrap_err();
            let one = matches!(e, ElectionError::InsufficientSignatures { valid: 1, .. });
            prop_assert!(one);
            let outsider = keys(1, seed ^ 0xABCD).remove(0);
            m.signatures = vec![sign_manifest(&k[0], &m), sign_manifest(&outsider, &m)];
            prop_assert!(verify_manifest(&pol, &m).is_err());
        }

        /// Any change to the content invalidates existing signatures.
        #[test]
        fn tampering_invalidates_signatures(seed in any::<u64>(), which in 0u8..5) {
            let k = keys(3, seed);
            let pol = policy(&k, 2);
            let mut m = signed(&k, &[0, 1, 2]);
            prop_assert!(verify_manifest(&pol, &m).is_ok());
            match which {
                0 => m.title.push('!'),
                1 => m.options[0].name = "Outro Nome".into(),
                2 => m.options.swap(0, 1),
                3 => m.schedule = Some(Schedule { registration_opens: 1, voting_opens: 2, voting_closes: 4_000 }),
                _ => m.revision += 1,
            }
            // Either structurally invalid (swap breaks indices) or signatures no longer match.
            prop_assert!(verify_manifest(&pol, &m).is_err());
        }

        #[test]
        fn hash_ignores_signatures_but_not_content(seed in any::<u64>()) {
            let k = keys(3, seed);
            let bare = manifest();
            let signed_m = signed(&k, &[0, 1]);
            prop_assert_eq!(manifest_hash(&bare), manifest_hash(&signed_m));
            let mut other = manifest();
            other.title.push('x');
            prop_assert_ne!(manifest_hash(&bare), manifest_hash(&other));
        }
    }
}
