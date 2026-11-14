use std::path::PathBuf;

pub const DEFAULT_ADDR: &str = "127.0.0.1:7878";

/// Finds the folder made by `issuer-cli dev-setup`.
/// Order: `$TEKO_DEV_DIR`, `./dev`, then `<repo>/dev` relative to this crate.
pub fn resolve_dev_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TEKO_DEV_DIR") {
        return PathBuf::from(d);
    }
    let local = PathBuf::from("dev");
    if local.join("trust.txt").exists() {
        return local;
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev");
    if repo.join("trust.txt").exists() {
        return repo;
    }
    local
}
