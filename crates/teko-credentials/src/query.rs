use prost::Message;
use teko_ledger::Ledger;

use crate::module::{CREDENTIALS_MODULE, k_cred, k_type};
use crate::types::{CredRec, Status, TypeRec};

pub fn type_info(l: &Ledger, ctype: &[u8]) -> Option<TypeRec> {
    TypeRec::decode(l.read(CREDENTIALS_MODULE, &k_type(ctype))?).ok()
}

pub fn credential(l: &Ledger, ctype: &[u8], id: &[u8]) -> Option<CredRec> {
    CredRec::decode(l.read(CREDENTIALS_MODULE, &k_cred(ctype, id))?).ok()
}

/// What the credential is worth at block time `time`; `None` if unknown.
pub fn status(l: &Ledger, ctype: &[u8], id: &[u8], time: u64) -> Option<Status> {
    Some(credential(l, ctype, id)?.status_at(time))
}

/// Every credential type (organ) of the chain, in key order.
pub fn types(l: &Ledger) -> Vec<(Vec<u8>, TypeRec)> {
    l.scan(CREDENTIALS_MODULE, b"t:")
        .into_iter()
        .filter_map(|(k, v)| Some((k.get(2..)?.to_vec(), TypeRec::decode(v.as_slice()).ok()?)))
        .collect()
}

/// Every credential of one type: `(id, record)`, in key order.
pub fn credentials_of_type(l: &Ledger, ctype: &[u8]) -> Vec<(Vec<u8>, CredRec)> {
    let prefix = k_cred(ctype, &[]);
    l.scan(CREDENTIALS_MODULE, &prefix)
        .into_iter()
        .filter_map(|(k, v)| {
            Some((
                k.get(prefix.len()..)?.to_vec(),
                CredRec::decode(v.as_slice()).ok()?,
            ))
        })
        .collect()
}

/// Every `(type, id)` issued to a credential key, in key order.
pub fn credentials_of(l: &Ledger, subject: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut prefix = b"s:".to_vec();
    prefix.push(subject.len() as u8);
    prefix.extend_from_slice(subject);
    l.scan(CREDENTIALS_MODULE, &prefix)
        .into_iter()
        .filter_map(|(k, _)| {
            let rest = k.get(prefix.len()..)?;
            let (&n, rest) = rest.split_first()?;
            let (ctype, id) = rest.split_at_checked(n as usize)?;
            Some((ctype.to_vec(), id.to_vec()))
        })
        .collect()
}
