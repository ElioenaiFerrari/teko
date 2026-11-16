//! The civic identity credential held by the DEVICE (ADR-0038): an on-chain credential whose
//! holder keeps a `CivicBundle` (the claims, the salt and the signed transactions of the issuing
//! organ's quorum). The device checks the bundle ALONE (no clock, no chain): the commitment, the
//! credential id and that at least M distinct members of the organ signed the issuance. Veto,
//! contest window, expiry and revocation are checked by whoever USES the credential, reading the
//! chain (the same model the off-chain credential had: revocation was a verifier-side check).

use alloc::string::String;
use alloc::vec::Vec;

use prost::Message;

use crate::credential::{Claim, claim_digest};
use crate::keys::tagged_hash;
use crate::ledger::{
    CRED_OP_APPROVE, MODULE_CREDENTIALS, cred_id, cred_propose_payload, cred_vote_payload,
    tx_signing_bytes,
};
use crate::sign::verify_tagged;
use crate::tags;
use crate::{ledger_address, public_key_bytes};

/// The credential type of the civic identity.
pub const CIVIC_TYPE: &str = "civic";
/// At most this many transactions in a bundle (proposal plus approvals).
const MAX_TXS: usize = 16;
const MAX_CLAIMS_BYTES: usize = 2_048;

/// One transaction of the issuance, exactly as the member's device signed it.
#[derive(Clone, PartialEq, Message)]
pub struct BundleTx {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    #[prost(uint64, tag = "2")]
    pub nonce: u64,
    #[prost(uint64, tag = "3")]
    pub fee: u64,
    #[prost(bytes = "vec", tag = "4")]
    pub payload: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub sender: Vec<u8>,
    #[prost(bytes = "vec", tag = "6")]
    pub signature: Vec<u8>,
}

/// What the organ hands to the holder.
#[derive(Clone, PartialEq, Message)]
pub struct CivicBundle {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    #[prost(string, tag = "2")]
    pub ctype: String,
    /// `name=value;name=value`.
    #[prost(bytes = "vec", tag = "3")]
    pub claims: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub salt: Vec<u8>,
    #[prost(uint64, tag = "5")]
    pub valid_until: u64,
    /// The proposal first, then the approvals.
    #[prost(message, repeated, tag = "6")]
    pub txs: Vec<BundleTx>,
}

/// An opened claim presented to a verifier, with the digests of the claims NOT disclosed
/// (selective disclosure over the on-chain commitment).
#[derive(Clone, PartialEq, Message)]
pub struct CivicPresentation {
    #[prost(string, tag = "1")]
    pub chain_id: String,
    #[prost(string, tag = "2")]
    pub ctype: String,
    #[prost(bytes = "vec", tag = "3")]
    pub credential_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub subject: Vec<u8>,
    #[prost(uint64, tag = "5")]
    pub valid_until: u64,
    #[prost(message, repeated, tag = "6")]
    pub disclosed: Vec<Claim>,
    /// Digests of the claims that stay hidden.
    #[prost(bytes = "vec", repeated, tag = "7")]
    pub hidden: Vec<Vec<u8>>,
    #[prost(bytes = "vec", tag = "8")]
    pub nonce: Vec<u8>,
    #[prost(string, tag = "9")]
    pub verifier: String,
    #[prost(bytes = "vec", tag = "10")]
    pub holder_signature: Vec<u8>,
}

/// Who may issue the civic credential, as the device trusts them (`trust.txt`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CivicPolicy {
    pub chain_id: String,
    pub threshold: usize,
    /// 32-byte wallet addresses of the organ's members.
    pub members: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CivicError {
    Policy,
    Malformed,
    WrongChain,
    WrongType,
    Commitment,
    Subject,
    Transaction,
    Payload,
    Quorum,
}

impl core::fmt::Display for CivicError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            CivicError::Policy => "o dispositivo não confia em nenhum órgão emissor",
            CivicError::Malformed => "pacote malformado",
            CivicError::WrongChain => "pacote de outra rede",
            CivicError::WrongType => "não é uma credencial cívica",
            CivicError::Commitment => "atributos não conferem com o compromisso",
            CivicError::Subject => "a credencial é de outra chave",
            CivicError::Transaction => "assinatura de transação inválida",
            CivicError::Payload => "a transação não é a emissão desta credencial",
            CivicError::Quorum => "faltam aprovações de membros distintos do órgão",
        })
    }
}

/// Splits `name=value;name=value` (a trailing `;` is fine). STRICT: every part has a non-empty
/// name and an `=`, and names are unique. `None` otherwise.
pub fn parse_claims(text: &[u8]) -> Option<Vec<(String, String)>> {
    if text.len() > MAX_CLAIMS_BYTES {
        return None;
    }
    let text = core::str::from_utf8(text).ok()?;
    let mut out: Vec<(String, String)> = Vec::new();
    for part in text.split(';').filter(|p| !p.trim().is_empty()) {
        let (k, v) = part.split_once('=')?;
        let (k, v) = (k.trim(), v.trim());
        if k.is_empty() || out.iter().any(|(n, _)| n == k) {
            return None;
        }
        out.push((String::from(k), String::from(v)));
    }
    (!out.is_empty()).then_some(out)
}

/// The salt of one claim, derived from the credential salt and the claim name (so the holder
/// keeps ONE salt and every claim still has its own).
pub fn claim_salt(salt: &[u8; 16], name: &str) -> [u8; 16] {
    let h = tagged_hash(tags::CRED_CLAIM_SALT, &[salt.as_slice(), name.as_bytes()]);
    let mut out = [0u8; 16];
    out.copy_from_slice(&h[..16]);
    out
}

/// The claims with their salts. LENIENT on input that `parse_claims` would refuse (it then holds
/// what could be read): callers that need a valid list check `parse_claims` first.
pub fn claim_list(salt: &[u8; 16], claims: &[u8]) -> Vec<Claim> {
    let text = core::str::from_utf8(claims).unwrap_or("");
    text.split(';')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| Claim {
            name: String::from(k),
            value: String::from(v),
            salt: claim_salt(salt, k).to_vec(),
        })
        .collect()
}

/// The commitment: a hash over the SORTED digests of the claims.
pub fn commitment_of(claims: &[Claim]) -> [u8; 32] {
    let mut digests: Vec<[u8; 32]> = claims.iter().map(claim_digest).collect();
    digests.sort_unstable();
    commitment_of_digests(&digests)
}

/// The same commitment from digests (a verifier of a presentation has only some claims).
pub fn commitment_of_digests(sorted: &[[u8; 32]]) -> [u8; 32] {
    let parts: Vec<&[u8]> = sorted.iter().map(<[u8; 32]>::as_slice).collect();
    tagged_hash(tags::CRED_CLAIMS_PRIVATE, &parts)
}

/// Checks a bundle ALONE and returns the credential id (the key on the chain).
///
/// * the claims, with their salt, give the commitment; the id binds type, holder, commitment
///   and expiry;
/// * the first transaction is the member's PROPOSAL of exactly that credential; the others are
///   APPROVALS of its id; every one is signed (`LEDGER_TX`) by a key whose address is a member;
/// * at least `threshold` DISTINCT members signed.
pub fn verify_civic_bundle(
    b: &CivicBundle,
    policy: &CivicPolicy,
    subject: &[u8],
) -> Result<[u8; 32], CivicError> {
    if policy.members.is_empty()
        || policy.threshold == 0
        || policy.threshold > policy.members.len()
        || policy.chain_id.is_empty()
    {
        return Err(CivicError::Policy);
    }
    if b.chain_id != policy.chain_id {
        return Err(CivicError::WrongChain);
    }
    if b.ctype != CIVIC_TYPE {
        return Err(CivicError::WrongType);
    }
    if b.txs.is_empty() || b.txs.len() > MAX_TXS {
        return Err(CivicError::Malformed);
    }
    let salt: [u8; 16] = b
        .salt
        .as_slice()
        .try_into()
        .map_err(|_| CivicError::Malformed)?;
    parse_claims(&b.claims).ok_or(CivicError::Malformed)?;
    let commitment = commitment_of(&claim_list(&salt, &b.claims));
    let id = cred_id(
        CIVIC_TYPE.as_bytes(),
        subject,
        &commitment,
        &[],
        b.valid_until,
    );
    let propose = cred_propose_payload(
        CIVIC_TYPE.as_bytes(),
        subject,
        &commitment,
        &[],
        b.valid_until,
    );
    let approve = cred_vote_payload(CRED_OP_APPROVE, CIVIC_TYPE.as_bytes(), &id, "");
    let mut signers: Vec<[u8; 32]> = Vec::new();
    for (i, t) in b.txs.iter().enumerate() {
        if t.chain_id != policy.chain_id {
            return Err(CivicError::WrongChain);
        }
        let msg = tx_signing_bytes(
            &t.chain_id,
            t.nonce,
            t.fee,
            MODULE_CREDENTIALS,
            &t.payload,
            &t.sender,
        );
        verify_tagged(&t.sender, tags::LEDGER_TX, &msg, &t.signature)
            .map_err(|_| CivicError::Transaction)?;
        let want = if i == 0 { &propose } else { &approve };
        if &t.payload != want {
            return Err(CivicError::Payload);
        }
        let who = ledger_address(&t.sender);
        if policy.members.iter().any(|m| m.as_slice() == who) && !signers.contains(&who) {
            signers.push(who);
        }
    }
    if signers.len() < policy.threshold {
        return Err(CivicError::Quorum);
    }
    Ok(id)
}

/// Bytes the holder signs in a presentation (the identity key; the verifier knows the subject).
pub fn presentation_signing_bytes(p: &CivicPresentation) -> Vec<u8> {
    let mut unsigned = p.clone();
    unsigned.holder_signature = Vec::new();
    unsigned.encode_to_vec()
}

/// The holder's presentation signature: over the verifier's nonce and name, bound to the id.
pub fn presentation_signed_by(p: &CivicPresentation, subject: &[u8]) -> bool {
    verify_tagged(
        subject,
        tags::ATTR_PROOF,
        &presentation_signing_bytes(p),
        &p.holder_signature,
    )
    .is_ok()
}

/// What a verifier reads from the chain about a civic credential (owned: it comes from a ledger).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CivicRecord {
    pub subject: Vec<u8>,
    pub commitment: Vec<u8>,
    pub valid_until: u64,
    /// ACTIVE at the verifier's time: past the contest window, before the expiry, not vetoed or revoked.
    pub active: bool,
}

/// What a verifier needs from the chain's record of the credential.
pub struct ChainRecord<'a> {
    pub subject: &'a [u8],
    pub commitment: &'a [u8],
    pub valid_until: u64,
    /// The credential is ACTIVE at `now` according to the chain.
    pub active: bool,
}

/// Verifies a presentation against the chain's record: the commitment rebuilt from the disclosed
/// claims and the hidden digests, the holder's signature, the nonce and verifier, and the status.
pub fn verify_civic_presentation(
    p: &CivicPresentation,
    rec: &ChainRecord<'_>,
    nonce: &[u8],
    verifier: &str,
) -> Result<(), CivicError> {
    if p.nonce != nonce || p.verifier != verifier {
        return Err(CivicError::Malformed);
    }
    if p.subject != rec.subject || p.valid_until != rec.valid_until || !rec.active {
        return Err(CivicError::Subject);
    }
    if !presentation_signed_by(p, rec.subject) {
        return Err(CivicError::Transaction);
    }
    let mut digests: Vec<[u8; 32]> = p.disclosed.iter().map(claim_digest).collect();
    for h in &p.hidden {
        digests.push(h.as_slice().try_into().map_err(|_| CivicError::Malformed)?);
    }
    digests.sort_unstable();
    if commitment_of_digests(&digests).as_slice() != rec.commitment {
        return Err(CivicError::Commitment);
    }
    // the id on the presentation must be the chain's id for this subject
    let id = cred_id(
        p.ctype.as_bytes(),
        rec.subject,
        rec.commitment,
        &[],
        p.valid_until,
    );
    if p.credential_id != id {
        return Err(CivicError::Commitment);
    }
    Ok(())
}

/// SIMULATION AND TESTS: builds the bundle an organ would hand over, signing the proposal and the
/// approvals with the given member keys (the first proposes, the others approve). Real issuance
/// is done by the members' DEVICES, one transaction each (`apps/issuer`).
pub fn build_civic_bundle(
    chain_id: &str,
    claims: &[u8],
    salt: [u8; 16],
    valid_until: u64,
    subject: &[u8],
    members: &[&p256::ecdsa::SigningKey],
) -> CivicBundle {
    let commitment = commitment_of(&claim_list(&salt, claims));
    let id = cred_id(
        CIVIC_TYPE.as_bytes(),
        subject,
        &commitment,
        &[],
        valid_until,
    );
    let txs = members
        .iter()
        .enumerate()
        .map(|(n, k)| {
            let payload = if n == 0 {
                cred_propose_payload(
                    CIVIC_TYPE.as_bytes(),
                    subject,
                    &commitment,
                    &[],
                    valid_until,
                )
            } else {
                cred_vote_payload(CRED_OP_APPROVE, CIVIC_TYPE.as_bytes(), &id, "")
            };
            let sender = public_key_bytes(k);
            let msg = tx_signing_bytes(chain_id, 0, 0, MODULE_CREDENTIALS, &payload, &sender);
            BundleTx {
                chain_id: chain_id.into(),
                nonce: 0,
                fee: 0,
                payload,
                sender,
                signature: crate::sign::sign_tagged(k, tags::LEDGER_TX, &msg),
            }
        })
        .collect();
    CivicBundle {
        chain_id: chain_id.into(),
        ctype: CIVIC_TYPE.into(),
        claims: claims.to_vec(),
        salt: salt.to_vec(),
        valid_until,
        txs,
    }
}

/// What the chain would hold for a bundle's credential: its id and record.
pub fn civic_record_of(
    bundle: &CivicBundle,
    subject: &[u8],
    active: bool,
) -> ([u8; 32], CivicRecord) {
    let salt: [u8; 16] = bundle.salt.as_slice().try_into().unwrap_or([0; 16]);
    let commitment = commitment_of(&claim_list(&salt, &bundle.claims));
    let id = cred_id(
        CIVIC_TYPE.as_bytes(),
        subject,
        &commitment,
        &[],
        bundle.valid_until,
    );
    (
        id,
        CivicRecord {
            subject: subject.to_vec(),
            commitment: commitment.to_vec(),
            valid_until: bundle.valid_until,
            active,
        },
    )
}

/// The identity public key of a signing key (convenience for tests and tools).
pub fn subject_of(key: &p256::ecdsa::SigningKey) -> Vec<u8> {
    public_key_bytes(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_signing_key;
    use crate::ledger::cred_vote_payload as vote;
    use crate::sign::sign_tagged;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    const CHAIN: &str = "teko-devnet-1-economy";

    struct Fixture {
        members: Vec<p256::ecdsa::SigningKey>,
        subject: Vec<u8>,
        claims: Vec<u8>,
        salt: [u8; 16],
    }

    fn fixture() -> Fixture {
        let mut rng = StdRng::seed_from_u64(5);
        Fixture {
            members: (0..3).map(|_| generate_signing_key(&mut rng)).collect(),
            subject: public_key_bytes(&generate_signing_key(&mut rng)),
            claims: b"nome=Ana Souza;birth_date=1990-05-17;age_over_18=true;electoral_state=SP"
                .to_vec(),
            salt: [7; 16],
        }
    }

    fn tx(chain: &str, k: &p256::ecdsa::SigningKey, nonce: u64, payload: Vec<u8>) -> BundleTx {
        let sender = public_key_bytes(k);
        let msg = tx_signing_bytes(chain, nonce, 0, MODULE_CREDENTIALS, &payload, &sender);
        BundleTx {
            chain_id: chain.into(),
            nonce,
            fee: 0,
            payload,
            sender,
            signature: sign_tagged(k, tags::LEDGER_TX, &msg),
        }
    }

    fn bundle(f: &Fixture, signers: &[usize]) -> CivicBundle {
        let c = cred_private_commitment_of(f);
        let id = cred_id(CIVIC_TYPE.as_bytes(), &f.subject, &c, &[], 9_000);
        let mut txs = Vec::new();
        for (n, i) in signers.iter().enumerate() {
            let payload = if n == 0 {
                cred_propose_payload(CIVIC_TYPE.as_bytes(), &f.subject, &c, &[], 9_000)
            } else {
                vote(CRED_OP_APPROVE, CIVIC_TYPE.as_bytes(), &id, "")
            };
            txs.push(tx(CHAIN, &f.members[*i], 0, payload));
        }
        CivicBundle {
            chain_id: CHAIN.into(),
            ctype: CIVIC_TYPE.into(),
            claims: f.claims.clone(),
            salt: f.salt.to_vec(),
            valid_until: 9_000,
            txs,
        }
    }

    fn cred_private_commitment_of(f: &Fixture) -> [u8; 32] {
        commitment_of(&claim_list(&f.salt, &f.claims))
    }

    fn policy(f: &Fixture, threshold: usize) -> CivicPolicy {
        CivicPolicy {
            chain_id: CHAIN.into(),
            threshold,
            members: f
                .members
                .iter()
                .map(|k| ledger_address(&public_key_bytes(k)).to_vec())
                .collect(),
        }
    }

    #[test]
    fn a_bundle_with_the_quorum_verifies_and_gives_the_chain_id() {
        let f = fixture();
        let b = bundle(&f, &[0, 1]);
        let id = verify_civic_bundle(&b, &policy(&f, 2), &f.subject).unwrap();
        let c = cred_private_commitment_of(&f);
        assert_eq!(
            id,
            cred_id(CIVIC_TYPE.as_bytes(), &f.subject, &c, &[], 9_000)
        );
    }

    #[test]
    fn a_short_repeated_or_foreign_quorum_is_refused() {
        let f = fixture();
        assert_eq!(
            verify_civic_bundle(&bundle(&f, &[0]), &policy(&f, 2), &f.subject),
            Err(CivicError::Quorum)
        );
        // the same member twice is still one member
        assert_eq!(
            verify_civic_bundle(&bundle(&f, &[0, 0]), &policy(&f, 2), &f.subject),
            Err(CivicError::Quorum)
        );
        // members the device does not trust
        let mut p = policy(&f, 2);
        p.members = vec![vec![1; 32], vec![2; 32]];
        assert_eq!(
            verify_civic_bundle(&bundle(&f, &[0, 1]), &p, &f.subject),
            Err(CivicError::Quorum)
        );
    }

    #[test]
    fn any_change_to_the_claims_the_salt_the_holder_or_a_transaction_is_refused() {
        let f = fixture();
        let p = policy(&f, 2);
        let good = bundle(&f, &[0, 1]);
        let mut b = good.clone();
        b.claims =
            b"nome=Ana Souza;birth_date=1990-05-17;age_over_18=true;electoral_state=RJ".to_vec();
        assert_eq!(
            verify_civic_bundle(&b, &p, &f.subject),
            Err(CivicError::Payload)
        );
        let mut b = good.clone();
        b.salt = vec![8; 16];
        assert_eq!(
            verify_civic_bundle(&b, &p, &f.subject),
            Err(CivicError::Payload)
        );
        // another holder
        let other = public_key_bytes(&generate_signing_key(&mut StdRng::seed_from_u64(99)));
        assert_eq!(
            verify_civic_bundle(&good, &p, &other),
            Err(CivicError::Payload)
        );
        // a tampered signature, payload, chain, expiry or type
        let mut b = good.clone();
        b.txs[1].signature[0] ^= 1;
        assert_eq!(
            verify_civic_bundle(&b, &p, &f.subject),
            Err(CivicError::Transaction)
        );
        let mut b = good.clone();
        b.txs[1].payload.push(0);
        assert_eq!(
            verify_civic_bundle(&b, &p, &f.subject),
            Err(CivicError::Transaction)
        );
        let mut b = good.clone();
        b.chain_id = "outra-rede".into();
        assert_eq!(
            verify_civic_bundle(&b, &p, &f.subject),
            Err(CivicError::WrongChain)
        );
        let mut b = good.clone();
        b.valid_until = 9_001;
        assert_eq!(
            verify_civic_bundle(&b, &p, &f.subject),
            Err(CivicError::Payload)
        );
        let mut b = good;
        b.ctype = "cnh-b".into();
        assert_eq!(
            verify_civic_bundle(&b, &p, &f.subject),
            Err(CivicError::WrongType)
        );
    }

    #[test]
    fn a_device_with_no_trusted_organ_accepts_nothing() {
        let f = fixture();
        let mut p = policy(&f, 2);
        p.members.clear();
        assert_eq!(
            verify_civic_bundle(&bundle(&f, &[0, 1]), &p, &f.subject),
            Err(CivicError::Policy)
        );
    }

    #[test]
    fn claims_are_parsed_strictly_and_commit_independently_of_their_order() {
        assert!(parse_claims(b"a=1;b=2;").is_some());
        assert!(parse_claims(b"a=1;a=2").is_none()); // duplicate name
        assert!(parse_claims(b"a=1;b").is_none()); // no '='
        assert!(parse_claims(b"=1").is_none());
        assert!(parse_claims(b"").is_none());
        assert!(parse_claims(&[0xff, 0xfe]).is_none());
        let s = [3u8; 16];
        assert_eq!(
            commitment_of(&claim_list(&s, b"a=1;b=2")),
            commitment_of(&claim_list(&s, b"b=2;a=1"))
        );
        assert_ne!(
            commitment_of(&claim_list(&s, b"a=1;b=2")),
            commitment_of(&claim_list(&s, b"a=1;b=3"))
        );
        assert_ne!(
            commitment_of(&claim_list(&s, b"a=1;b=2")),
            commitment_of(&claim_list(&[4u8; 16], b"a=1;b=2"))
        );
    }

    #[test]
    fn a_presentation_discloses_one_claim_and_the_chain_commitment_still_checks() {
        let f = fixture();
        let holder = generate_signing_key(&mut StdRng::seed_from_u64(5)); // not the subject: built below
        let _ = holder;
        let key = generate_signing_key(&mut StdRng::seed_from_u64(1234));
        let subject = public_key_bytes(&key);
        let claims = claim_list(&f.salt, &f.claims);
        let commitment = commitment_of(&claims);
        let id = cred_id(CIVIC_TYPE.as_bytes(), &subject, &commitment, &[], 9_000);
        let shown: Vec<Claim> = claims
            .iter()
            .filter(|c| c.name == "age_over_18")
            .cloned()
            .collect();
        let hidden: Vec<Vec<u8>> = claims
            .iter()
            .filter(|c| c.name != "age_over_18")
            .map(|c| claim_digest(c).to_vec())
            .collect();
        let mut p = CivicPresentation {
            chain_id: CHAIN.into(),
            ctype: CIVIC_TYPE.into(),
            credential_id: id.to_vec(),
            subject: subject.clone(),
            valid_until: 9_000,
            disclosed: shown,
            hidden,
            nonce: vec![9; 16],
            verifier: "loja".into(),
            holder_signature: Vec::new(),
        };
        p.holder_signature = sign_tagged(&key, tags::ATTR_PROOF, &presentation_signing_bytes(&p));
        fn rec<'a>(subject: &'a [u8], c: &'a [u8], active: bool) -> ChainRecord<'a> {
            ChainRecord {
                subject,
                commitment: c,
                valid_until: 9_000,
                active,
            }
        }
        verify_civic_presentation(&p, &rec(&subject, &commitment, true), &[9; 16], "loja").unwrap();
        // the hidden claims are NOT in the presentation
        assert!(p.disclosed.iter().all(|c| c.name == "age_over_18"));
        // not active, other nonce or verifier, other commitment: refused
        assert!(
            verify_civic_presentation(&p, &rec(&subject, &commitment, false), &[9; 16], "loja")
                .is_err()
        );
        assert!(
            verify_civic_presentation(&p, &rec(&subject, &commitment, true), &[1; 16], "loja")
                .is_err()
        );
        assert!(
            verify_civic_presentation(&p, &rec(&subject, &commitment, true), &[9; 16], "outra")
                .is_err()
        );
        assert!(
            verify_civic_presentation(&p, &rec(&subject, &[0; 32], true), &[9; 16], "loja")
                .is_err()
        );
        // a changed disclosed value breaks the commitment
        let mut forged = p.clone();
        forged.disclosed[0].value = "false".into();
        forged.holder_signature =
            sign_tagged(&key, tags::ATTR_PROOF, &presentation_signing_bytes(&forged));
        assert_eq!(
            verify_civic_presentation(&forged, &rec(&subject, &commitment, true), &[9; 16], "loja"),
            Err(CivicError::Commitment)
        );
        // a signature by another key is refused
        let other = generate_signing_key(&mut StdRng::seed_from_u64(77));
        let mut bad = p.clone();
        bad.holder_signature =
            sign_tagged(&other, tags::ATTR_PROOF, &presentation_signing_bytes(&bad));
        assert!(
            verify_civic_presentation(&bad, &rec(&subject, &commitment, true), &[9; 16], "loja")
                .is_err()
        );
    }
}
