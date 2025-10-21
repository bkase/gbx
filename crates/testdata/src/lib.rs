//! Test ROM data accessors for gbx development and CI.

mod types;

pub use types::{Expected, RomKind, RomMeta, RomModel};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

#[cfg(feature = "embed")]
use std::collections::HashMap;
use std::sync::Arc;

use once_cell::sync::{Lazy, OnceCell};
use sha2::{Digest, Sha256};

static DATA_DIR: Lazy<std::path::PathBuf> =
    Lazy::new(|| std::path::PathBuf::from(env!("TESTDATA_DATA_DIR")));

struct Entry {
    #[cfg(feature = "embed")]
    embed_data: Option<&'static [u8]>,
    cache: OnceCell<Arc<[u8]>>,
}

#[cfg(feature = "embed")]
fn embed_lookup() -> HashMap<&'static str, &'static [u8]> {
    generated::EMBED_DATA.iter().copied().collect()
}

static ENTRIES: Lazy<Vec<Entry>> = Lazy::new(|| {
    #[cfg(feature = "embed")]
    let embed = embed_lookup();

    generated::ROMS
        .iter()
        .map(|meta| {
            #[cfg(not(feature = "embed"))]
            let _ = meta;

            Entry {
                #[cfg(feature = "embed")]
                embed_data: embed.get(meta.path).copied(),
                cache: OnceCell::new(),
            }
        })
        .collect()
});

static BY_PATH: Lazy<std::collections::HashMap<&'static str, usize>> = Lazy::new(|| {
    generated::ROMS
        .iter()
        .enumerate()
        .map(|(idx, meta)| (meta.path, idx))
        .collect()
});

/// Returns metadata for every ROM in the vendored bundle.
pub fn list() -> &'static [RomMeta] {
    generated::ROMS
}

/// Returns the upstream bundle identifier (e.g. `c-sp-v7.0`).
pub fn source_bundle() -> &'static str {
    generated::SOURCE_BUNDLE
}

/// Returns the upstream vendor identifier.
pub fn source_vendor() -> &'static str {
    generated::SOURCE_VENDOR
}

/// Returns the pinned upstream release URL.
pub fn source_url() -> &'static str {
    generated::SOURCE_URL
}

/// Looks up ROM metadata by normalized path.
pub fn metadata(path: &str) -> Option<&'static RomMeta> {
    BY_PATH.get(path).map(|&idx| &generated::ROMS[idx])
}

/// Loads ROM bytes into an `Arc` either from the embed map or from the copied OUT_DIR cache.
pub fn bytes(path: &str) -> Arc<[u8]> {
    let idx = BY_PATH
        .get(path)
        .copied()
        .unwrap_or_else(|| panic!("unknown ROM path {path}"));
    load_entry(idx)
}

fn load_entry(idx: usize) -> Arc<[u8]> {
    ENTRIES[idx]
        .cache
        .get_or_init(|| {
            #[cfg(feature = "embed")]
            if let Some(data) = ENTRIES[idx].embed_data {
                return Arc::from(data);
            }

            let meta = &generated::ROMS[idx];
            let file_path = DATA_DIR.join(meta.path);
            let bytes = std::fs::read(&file_path).unwrap_or_else(|err| {
                panic!("failed to read ROM {path:?}: {err}", path = file_path)
            });

            let digest = Sha256::digest(&bytes);
            let digest_hex = hex::encode(digest);
            assert_eq!(
                digest_hex, meta.sha256,
                "ROM bytes for {} do not match manifest digest",
                meta.path
            );

            Arc::from(bytes.into_boxed_slice())
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_returns_entries() {
        assert!(
            !list().is_empty(),
            "expected testdata to expose at least one ROM"
        );
    }

    #[test]
    fn bytes_roundtrip_read() {
        let meta = list()
            .iter()
            .find(|m| {
                m.path
                    .ends_with("mooneye-test-suite/acceptance/ei_sequence.gb")
            })
            .expect("expected mooneye acceptance rom to exist");
        let data = bytes(meta.path);
        assert_eq!(data.len() as u64, meta.size);
    }
}
