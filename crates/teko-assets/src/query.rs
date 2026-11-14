use prost::Message;
use teko_ledger::{Address, Ledger};

use crate::module::{ASSETS_MODULE, k_asset, k_class};
use crate::types::{AssetRec, ClassRec};

pub fn class_info(l: &Ledger, class: &[u8]) -> Option<ClassRec> {
    ClassRec::decode(l.read(ASSETS_MODULE, &k_class(class))?).ok()
}

pub fn asset(l: &Ledger, class: &[u8], id: &[u8]) -> Option<AssetRec> {
    AssetRec::decode(l.read(ASSETS_MODULE, &k_asset(class, id))?).ok()
}

/// Every (class, asset id) owned by `owner`, in key order.
pub fn assets_of(l: &Ledger, owner: &Address) -> Vec<(Vec<u8>, Vec<u8>)> {
    let prefix = [b"o:".as_slice(), owner].concat();
    l.scan(ASSETS_MODULE, &prefix)
        .into_iter()
        .filter_map(|(k, _)| {
            let rest = k.get(prefix.len()..)?;
            let (&n, rest) = rest.split_first()?;
            let (class, id) = rest.split_at_checked(n as usize)?;
            Some((class.to_vec(), id.to_vec()))
        })
        .collect()
}
