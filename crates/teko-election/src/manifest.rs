use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;
use teko_attestation::SignerSignature;
use teko_attestation::multisig::count_distinct_valid;
use teko_crypto::{SigningKey, public_key_bytes, sign_tagged, tagged_hash, tags};

use crate::eligibility::{Scope, ScopeLevel};
use crate::error::ElectionError;
use crate::policy::GovernancePolicy;
use crate::text::sanitize_text;

/// Public ballots (ADR-0015, option 1). The ONLY kind allowed until the
/// product owner decides otherwise; kept as data so the format can evolve.
pub const BALLOT_KIND_PUBLIC: &str = "public";

const FORMAT_VERSION: u32 = 2;
const MAX_OPTIONS: usize = 64;
const MAX_CONTESTS: usize = 32;

/// Who may see partial results while voting is open (ADR-0018). With PUBLIC
/// ballots this is an interface CONVENTION only: the log is in clear and anyone
/// can count it. Enforcing it needs encrypted ballots (ADR-0015 option 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum ResultsVisibility {
    Unspecified = 0,
    /// Distribution visible live.
    Live = 1,
    /// Only how many ballots; distribution after the voting closes.
    TurnoutOnly = 2,
    /// Nothing until the voting closes.
    AfterClose = 3,
}

/// What is being decided. Named offices must match their jurisdiction level
/// (a "Governor" contest with national scope is a misconfiguration, rejected).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum Office {
    Unspecified = 0,
    President = 1,
    Governor = 2,
    Senator = 3,
    FederalDeputy = 4,
    StateDeputy = 5,
    Mayor = 6,
    CityCouncillor = 7,
    /// Referendum, plebiscite or any other question; any scope level.
    Other = 8,
}

impl Office {
    /// The scope level a named office requires (`None` = any, for `Other`).
    pub fn required_level(self) -> Option<ScopeLevel> {
        match self {
            Office::President => Some(ScopeLevel::National),
            Office::Governor | Office::Senator | Office::FederalDeputy | Office::StateDeputy => {
                Some(ScopeLevel::State)
            }
            Office::Mayor | Office::CityCouncillor => Some(ScopeLevel::Municipality),
            Office::Other | Office::Unspecified => None,
        }
    }
}

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
/// EACH contest has its own: a municipal contest need not share the
/// President's dates.
#[derive(Clone, PartialEq, Message)]
pub struct Schedule {
    #[prost(uint64, tag = "1")]
    pub registration_opens: u64,
    #[prost(uint64, tag = "2")]
    pub voting_opens: u64,
    #[prost(uint64, tag = "3")]
    pub voting_closes: u64,
}

/// One race: an office, a jurisdiction, candidates and dates.
#[derive(Clone, PartialEq, Message)]
pub struct Contest {
    /// Lower-case `[a-z0-9-]`, 1..=64, unique within the election.
    #[prost(string, tag = "1")]
    pub contest_id: String,
    #[prost(enumeration = "Office", tag = "2")]
    pub office: i32,
    /// Shown on the device, e.g. "Governador do estado de São Paulo".
    #[prost(string, tag = "3")]
    pub title: String,
    #[prost(message, optional, tag = "4")]
    pub scope: Option<Scope>,
    #[prost(message, repeated, tag = "5")]
    pub options: Vec<ElectionOption>,
    #[prost(message, optional, tag = "6")]
    pub schedule: Option<Schedule>,
    /// RSA public key (SPKI DER) of the registration authority FOR THIS CONTEST
    /// (ADR-0008). Empty = registration NOT required (prototype/open): any key
    /// may vote. One key per contest, so a registration is never valid elsewhere.
    #[prost(bytes = "vec", tag = "7")]
    pub registration_key: Vec<u8>,
}

impl Contest {
    pub fn option_name(&self, index: u32) -> Option<&str> {
        self.options
            .iter()
            .find(|o| o.index == index)
            .map(|o| o.name.as_str())
    }
}

/// An election = a container of contests (e.g. "Eleições Gerais 2026" holds
/// President, Governor of each state...). Signed M-of-N as a whole.
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
    pub contests: Vec<Contest>,
    #[prost(string, tag = "5")]
    pub ballot_kind: String,
    /// Voter-registry public key (ADR-0008). May be empty until decided.
    #[prost(bytes = "vec", tag = "6")]
    pub registry_key: Vec<u8>,
    /// 1 for the first publication; later revisions may only ADD contests.
    #[prost(uint32, tag = "7")]
    pub revision: u32,
    #[prost(message, repeated, tag = "8")]
    pub signatures: Vec<SignerSignature>,
    #[prost(enumeration = "ResultsVisibility", tag = "9")]
    pub results_visibility: i32,
    /// Network this election lives on (empty = unbound, dev only). Part of the
    /// signed manifest, so it cannot be moved to another network.
    #[prost(string, tag = "10")]
    pub chain_id: String,
}

impl ElectionManifest {
    pub fn contest(&self, contest_id: &str) -> Option<&Contest> {
        self.contests.iter().find(|c| c.contest_id == contest_id)
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

fn validate_contest(c: &Contest) -> Result<(), ElectionError> {
    let bad = ElectionError::InvalidManifest;
    if !valid_id(&c.contest_id) {
        return Err(bad("contest_id must be lower-case [a-z0-9-], 1..=64"));
    }
    if sanitize_text(&c.title, 120).is_none() {
        return Err(bad("invalid contest title"));
    }
    let office = c.office();
    if office == Office::Unspecified {
        return Err(bad("contest office missing"));
    }
    let scope = c.scope.as_ref().ok_or(bad("contest scope missing"))?;
    scope.validate()?;
    if let Some(required) = office.required_level()
        && scope.level() != required
    {
        return Err(bad("office does not match the jurisdiction level"));
    }
    if c.options.len() < 2 || c.options.len() > MAX_OPTIONS {
        return Err(bad("need 2..=64 options per contest"));
    }
    for (i, o) in c.options.iter().enumerate() {
        if o.index as usize != i {
            return Err(bad("option indices must be contiguous from 0, in order"));
        }
        if sanitize_text(&o.name, 80).is_none() {
            return Err(bad("invalid option name"));
        }
        if !o.number.is_empty() && sanitize_text(&o.number, 8).is_none() {
            return Err(bad("invalid option number"));
        }
        let dup = c.options.iter().take(i).any(|p| {
            p.name.trim().eq_ignore_ascii_case(o.name.trim())
                || (!o.number.is_empty() && p.number == o.number)
        });
        if dup {
            return Err(bad("duplicate option name or number"));
        }
    }
    if !c.registration_key.is_empty() && !(150..=1200).contains(&c.registration_key.len()) {
        return Err(bad(
            "registration_key must be an RSA SPKI (150..=1200 bytes)",
        ));
    }
    let s = c.schedule.as_ref().ok_or(bad("contest schedule missing"))?;
    if !(s.registration_opens < s.voting_opens && s.voting_opens < s.voting_closes) {
        return Err(bad(
            "schedule must be registration < voting opens < voting closes",
        ));
    }
    Ok(())
}

/// Structural validation (no signatures). Rejects anything a device should
/// not render: duplicates, gaps, spoofable text, impossible schedules,
/// offices that do not fit their jurisdiction.
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
    if m.revision == 0 {
        return Err(bad("revision starts at 1"));
    }
    if m.chain_id.len() > 64 || !m.chain_id.is_ascii() {
        return Err(bad("chain_id must be at most 64 ASCII chars"));
    }
    if m.results_visibility() == ResultsVisibility::Unspecified {
        return Err(bad("results_visibility must be declared"));
    }
    if m.contests.is_empty() || m.contests.len() > MAX_CONTESTS {
        return Err(bad("need 1..=32 contests"));
    }
    for (i, c) in m.contests.iter().enumerate() {
        validate_contest(c)?;
        if m.contests
            .iter()
            .take(i)
            .any(|p| p.contest_id == c.contest_id)
        {
            return Err(bad("duplicate contest_id"));
        }
    }
    Ok(())
}

fn signing_bytes(m: &ElectionManifest) -> Vec<u8> {
    let mut unsigned = m.clone();
    unsigned.signatures = Vec::new();
    unsigned.encode_to_vec()
}

/// Identifier of one manifest revision. Independent of the signatures.
pub fn manifest_hash(m: &ElectionManifest) -> [u8; 32] {
    tagged_hash(tags::ELECTION_MANIFEST, &[&signing_bytes(m)])
}

/// Identifier of ONE contest. Transitions bind to this (not to the whole
/// manifest), so adding a new contest in a later revision does not invalidate
/// the history of the existing ones.
pub fn contest_hash(c: &Contest) -> [u8; 32] {
    tagged_hash(tags::CONTEST, &[&c.encode_to_vec()])
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

/// A later revision may ADD contests (e.g. a run-off once round one is
/// tallied) but never alter or drop one that exists.
pub fn validate_revision(
    prev: &ElectionManifest,
    next: &ElectionManifest,
) -> Result<(), ElectionError> {
    let bad = ElectionError::InvalidRevision;
    if prev.election_id != next.election_id {
        return Err(bad("different election"));
    }
    if next.revision != prev.revision + 1 {
        return Err(bad("revision must increase by exactly one"));
    }
    if prev.ballot_kind != next.ballot_kind {
        return Err(bad("ballot_kind cannot change"));
    }
    if prev.chain_id != next.chain_id {
        return Err(bad("chain_id cannot change"));
    }
    if prev.results_visibility != next.results_visibility {
        return Err(bad("results_visibility cannot change"));
    }
    if !prev.registry_key.is_empty() && prev.registry_key != next.registry_key {
        return Err(bad("registry key cannot change once set"));
    }
    for old in &prev.contests {
        match next.contest(&old.contest_id) {
            Some(new) if contest_hash(new) == contest_hash(old) => {}
            Some(_) => return Err(bad("an existing contest was altered")),
            None => return Err(bad("an existing contest was dropped")),
        }
    }
    Ok(())
}

/// Verifies a whole history of manifest revisions (oldest first): every one
/// M-of-N signed and valid, each a legal revision of the previous. Returns the
/// latest, which is the one in force.
pub fn verify_manifest_chain<'a>(
    policy: &GovernancePolicy,
    revisions: &'a [ElectionManifest],
) -> Result<&'a ElectionManifest, ElectionError> {
    let (first, rest) = revisions
        .split_first()
        .ok_or(ElectionError::InvalidRevision("no manifest"))?;
    verify_manifest(policy, first)?;
    if first.revision != 1 {
        return Err(ElectionError::InvalidRevision("first revision must be 1"));
    }
    let mut prev = first;
    for next in rest {
        verify_manifest(policy, next)?;
        validate_revision(prev, next)?;
        prev = next;
    }
    Ok(prev)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use teko_crypto::generate_signing_key;

    pub const PRESIDENT: &str = "presidente";
    pub const GOVERNOR_SP: &str = "governador-sp";
    pub const MAYOR_SP: &str = "prefeito-3550308";

    pub fn keys(n: usize, seed: u64) -> Vec<SigningKey> {
        let mut rng = StdRng::seed_from_u64(seed);
        (0..n).map(|_| generate_signing_key(&mut rng)).collect()
    }

    pub fn policy(keys: &[SigningKey], m: usize) -> GovernancePolicy {
        GovernancePolicy::new(keys.iter().map(public_key_bytes).collect(), m).unwrap()
    }

    fn options(names: &[&str], first_number: u32) -> Vec<ElectionOption> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| ElectionOption {
                index: i as u32,
                name: (*n).into(),
                number: alloc::format!("{}", first_number + i as u32),
            })
            .collect()
    }

    /// Three contests with DIFFERENT jurisdictions and DIFFERENT dates.
    pub fn manifest() -> ElectionManifest {
        ElectionManifest {
            format_version: FORMAT_VERSION,
            election_id: "eleicoes-2026".into(),
            title: "Eleições de exemplo 2026".into(),
            contests: alloc::vec![
                Contest {
                    contest_id: PRESIDENT.into(),
                    office: Office::President as i32,
                    title: "Presidente da República".into(),
                    scope: Some(Scope::national()),
                    options: options(&["Candidata Onça", "Candidato Boto", "Candidata Arara"], 10),
                    registration_key: Vec::new(),
                    schedule: Some(Schedule {
                        registration_opens: 1_000,
                        voting_opens: 2_000,
                        voting_closes: 3_000
                    }),
                },
                Contest {
                    contest_id: GOVERNOR_SP.into(),
                    office: Office::Governor as i32,
                    title: "Governador do estado de São Paulo".into(),
                    scope: Some(Scope::state("SP")),
                    options: options(&["Candidato Guará", "Candidata Vitória-Régia"], 20),
                    registration_key: Vec::new(),
                    schedule: Some(Schedule {
                        registration_opens: 1_500,
                        voting_opens: 2_500,
                        voting_closes: 3_500
                    }),
                },
                Contest {
                    contest_id: MAYOR_SP.into(),
                    office: Office::Mayor as i32,
                    title: "Prefeito de São Paulo".into(),
                    scope: Some(Scope::municipality("3550308")),
                    options: options(&["Candidata Jaguatirica", "Candidato Tucano"], 30),
                    registration_key: Vec::new(),
                    schedule: Some(Schedule {
                        registration_opens: 2_000,
                        voting_opens: 4_000,
                        voting_closes: 5_000
                    }),
                },
            ],
            ballot_kind: BALLOT_KIND_PUBLIC.into(),
            registry_key: Vec::new(),
            revision: 1,
            signatures: Vec::new(),
            results_visibility: ResultsVisibility::Live as i32,
            chain_id: String::new(),
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
        let gov = m.contest(GOVERNOR_SP).unwrap();
        assert_eq!(gov.option_name(1), Some("Candidata Vitória-Régia"));
        assert_eq!(gov.option_name(2), None);
        assert!(m.contest("nao-existe").is_none());
    }

    #[test]
    fn each_contest_has_its_own_dates() {
        let m = manifest();
        let opens: Vec<u64> = m
            .contests
            .iter()
            .map(|c| c.schedule.as_ref().unwrap().voting_opens)
            .collect();
        assert_eq!(opens, [2_000, 2_500, 4_000]);
    }

    #[test]
    fn structural_rules_are_enforced() {
        type Edit = fn(&mut ElectionManifest);
        let cases: [(&str, Edit); 17] = [
            ("id with uppercase/space", |m| {
                m.election_id = "Eleicoes 2026".into()
            }),
            ("no contests", |m| m.contests.clear()),
            ("duplicate contest id", |m| {
                m.contests[1].contest_id = m.contests[0].contest_id.clone()
            }),
            ("bad contest id", |m| {
                m.contests[0].contest_id = "Presidente!".into()
            }),
            ("single option", |m| m.contests[0].options.truncate(1)),
            ("index gap", |m| m.contests[0].options[1].index = 5),
            ("duplicate name (case-insensitive)", |m| {
                m.contests[0].options[1].name = "candidata onça".into()
            }),
            ("duplicate number", |m| {
                m.contests[0].options[2].number = "10".into()
            }),
            ("bidi override in name", |m| {
                m.contests[0].options[0].name = "Fulano\u{202E}".into()
            }),
            ("newline in contest title", |m| {
                m.contests[0].title = "linha\nquebrada".into()
            }),
            ("schedule out of order", |m| {
                m.contests[1].schedule = Some(Schedule {
                    registration_opens: 3,
                    voting_opens: 2,
                    voting_closes: 9,
                })
            }),
            ("no schedule", |m| m.contests[1].schedule = None),
            ("president with a state scope", |m| {
                m.contests[0].scope = Some(Scope::state("SP"))
            }),
            ("mayor with a state scope", |m| {
                m.contests[2].scope = Some(Scope::state("SP"))
            }),
            ("unsupported ballot kind", |m| {
                m.ballot_kind = "encrypted".into()
            }),
            ("revision zero", |m| m.revision = 0),
            ("results visibility undeclared", |m| {
                m.results_visibility = 0
            }),
        ];
        for (name, edit) in cases {
            let mut m = manifest();
            edit(&mut m);
            assert!(validate_manifest(&m).is_err(), "should reject: {name}");
        }
        // jurisdiction codes are validated too
        let mut m = manifest();
        m.contests[1].scope = Some(Scope::state("sp"));
        assert!(matches!(
            validate_manifest(&m),
            Err(ElectionError::InvalidJurisdiction(_))
        ));
        // an `Other` question (plebiscite) may use any scope level
        let mut m = manifest();
        m.contests[0].office = Office::Other as i32;
        m.contests[0].scope = Some(Scope::municipality("3550308"));
        assert!(validate_manifest(&m).is_ok());
    }

    #[test]
    fn revisions_may_add_contests_but_never_change_or_drop_one() {
        let prev = manifest();
        let extra = Contest {
            contest_id: "prefeito-3550308-2turno".into(),
            office: Office::Mayor as i32,
            title: "Prefeito de São Paulo (2º turno)".into(),
            scope: Some(Scope::municipality("3550308")),
            options: prev.contests[2].options.clone(),
            registration_key: Vec::new(),
            schedule: Some(Schedule {
                registration_opens: 6_000,
                voting_opens: 7_000,
                voting_closes: 8_000,
            }),
        };
        let mut next = prev.clone();
        next.revision = 2;
        next.contests.push(extra);
        assert!(validate_manifest(&next).is_ok());
        assert!(validate_revision(&prev, &next).is_ok());
        // the existing contests keep the same hash, so their history stays valid
        for c in &prev.contests {
            assert_eq!(
                contest_hash(c),
                contest_hash(next.contest(&c.contest_id).unwrap())
            );
        }

        let mut altered = next.clone();
        altered.contests[0].options[0].name = "Outro Nome".into();
        assert!(validate_revision(&prev, &altered).is_err(), "altered");
        let mut dropped = next.clone();
        dropped.contests.remove(1);
        assert!(validate_revision(&prev, &dropped).is_err(), "dropped");
        let mut skipped = next.clone();
        skipped.revision = 3;
        assert!(validate_revision(&prev, &skipped).is_err(), "revision jump");
        let mut other = next.clone();
        other.election_id = "outra-eleicao".into();
        assert!(validate_revision(&prev, &other).is_err(), "other election");
        let mut kind = next.clone();
        kind.ballot_kind = "x".into();
        assert!(validate_revision(&prev, &kind).is_err(), "ballot kind");
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

        /// Any change to any contest invalidates existing signatures.
        #[test]
        fn tampering_invalidates_signatures(seed in any::<u64>(), which in 0u8..6) {
            let k = keys(3, seed);
            let pol = policy(&k, 2);
            let mut m = signed(&k, &[0, 1, 2]);
            prop_assert!(verify_manifest(&pol, &m).is_ok());
            match which {
                0 => m.title.push('!'),
                1 => m.contests[0].options[0].name = "Outro Nome".into(),
                2 => m.contests[1].schedule = Some(Schedule { registration_opens: 1, voting_opens: 2, voting_closes: 4_000 }),
                3 => m.contests[2].scope = Some(Scope::municipality("3304557")),
                4 => m.contests.swap(0, 1),
                _ => m.revision += 1,
            }
            prop_assert!(verify_manifest(&pol, &m).is_err());
        }

        #[test]
        fn hash_ignores_signatures_but_not_content(seed in any::<u64>()) {
            let k = keys(3, seed);
            let bare = manifest();
            let signed_m = signed(&k, &[0, 1]);
            prop_assert_eq!(manifest_hash(&bare), manifest_hash(&signed_m));
            let mut other = manifest();
            other.contests[1].title.push('x');
            prop_assert_ne!(manifest_hash(&bare), manifest_hash(&other));
            prop_assert_ne!(contest_hash(&bare.contests[0]), contest_hash(&bare.contests[1]));
        }
    }
}
