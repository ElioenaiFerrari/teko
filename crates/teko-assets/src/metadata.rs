//! Off-chain store of metadata records (ADR-0032): plain files for the devnet,
//! `<dir>/<class>/<hex(asset id)>.rec`. The chain keeps only the HASH of each record;
//! whoever holds the record can prove it matches (`get_verified`).

use std::fs;
use std::path::{Path, PathBuf};

use teko_crypto::{asset_meta, hex};

#[derive(Debug, Clone)]
pub struct MetadataStore {
    dir: PathBuf,
}

fn safe_class(class: &str) -> bool {
    !class.is_empty()
        && class.len() <= 32
        && class
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl MetadataStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The asset id is hex-encoded in the file name: no path from an id can escape.
    fn path(&self, class: &str, id: &str) -> Option<PathBuf> {
        safe_class(class).then(|| {
            self.dir
                .join(class)
                .join(format!("{}.rec", hex::encode(id.as_bytes())))
        })
    }

    /// Validates `record`, derives its asset id and saves it. Refuses to overwrite a
    /// different record for the same asset. Returns the asset id.
    pub fn put(&self, class: &str, record: &[u8]) -> Result<String, String> {
        let id = asset_meta::asset_id(class, record).map_err(|e| e.to_string())?;
        let path = self.path(class, &id).ok_or("classe inválida")?;
        if let Ok(old) = fs::read(&path) {
            return if old == record {
                Ok(id)
            } else {
                Err("já existe outro registro de metadados para este bem".into())
            };
        }
        let parent = path.parent().ok_or("caminho inválido")?;
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        fs::write(&path, record).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(id)
    }

    /// The saved record, if any (not checked against a hash).
    pub fn get(&self, class: &str, id: &str) -> Option<Vec<u8>> {
        fs::read(self.path(class, id)?).ok()
    }

    /// The saved record only if it is valid and its hash equals `expected` (the hash on
    /// the chain).
    pub fn get_verified(&self, class: &str, id: &str, expected: &[u8]) -> Option<Vec<u8>> {
        let rec = self.get(class, id)?;
        let ok = asset_meta::asset_id(class, &rec).ok().as_deref() == Some(id)
            && asset_meta::hash(class, &rec).as_slice() == expected;
        ok.then_some(rec)
    }
}
