//! Locates the release WASM artifacts that tests upload.
//!
//! Lookup order for `file` (for example `controller.wasm`):
//! 1. the artifact's override variable (`POOL_WASM_PATH`, `CONTROLLER_WASM_PATH`,
//!    `PRICE_AGGREGATOR_WASM_PATH`, `POSITION_NFT_WASM_PATH`,
//!    `SWAP_AGGREGATOR_WASM_PATH`);
//! 2. `$CARGO_TARGET_DIR/wasm32v1-none/release/<file>`, where CI's mutation job
//!    builds (`target-mutants`);
//! 3. `target/wasm32v1-none/release/<file>`, where `make build` leaves it.
//!
//! A relative path is tried from the test's working directory (the crate
//! directory) and from up to two levels above it, where the workspace root is.

use std::env::var as env_var;
use std::fs;
use std::path::Path;

const RELEASE_DIR: &str = "wasm32v1-none/release";

fn override_var(file: &str) -> Option<&'static str> {
    match file {
        "pool.wasm" => Some("POOL_WASM_PATH"),
        "controller.wasm" => Some("CONTROLLER_WASM_PATH"),
        "price_aggregator.wasm" => Some("PRICE_AGGREGATOR_WASM_PATH"),
        "position_nft.wasm" => Some("POSITION_NFT_WASM_PATH"),
        "swap_aggregator.wasm" => Some("SWAP_AGGREGATOR_WASM_PATH"),
        _ => None,
    }
}

/// The paths `release_wasm` tries for `file`, in order.
pub fn release_wasm_candidates(file: &str) -> Vec<String> {
    let mut bases = Vec::new();
    if let Some(path) = override_var(file).and_then(|name| env_var(name).ok()) {
        bases.push(path);
    }
    if let Ok(dir) = env_var("CARGO_TARGET_DIR") {
        bases.push(format!("{dir}/{RELEASE_DIR}/{file}"));
    }
    bases.push(format!("target/{RELEASE_DIR}/{file}"));

    let mut candidates = Vec::new();
    for base in bases {
        if Path::new(&base).is_absolute() {
            candidates.push(base);
        } else {
            for prefix in ["", "../", "../../"] {
                candidates.push(format!("{prefix}{base}"));
            }
        }
    }
    candidates
}

/// Reads the release WASM `file`, or panics naming every path it tried.
pub fn release_wasm(file: &str) -> Vec<u8> {
    let candidates = release_wasm_candidates(file);
    candidates
        .iter()
        .find_map(|path| fs::read(path).ok())
        .unwrap_or_else(|| {
            panic!("{file} not found (tried {candidates:?}); run `make build` first")
        })
}
