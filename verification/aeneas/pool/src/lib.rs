//! The production pool accounting code in `contracts/pool/src` (the market
//! cache, guards, interest booking and the operation legs in `ops/`) together with the
//! `common` math, rate, constant, error and type modules it calls, compiled
//! verbatim through `#[path]` includes for extraction to Lean with Charon and
//! Aeneas. Nothing in `common/` or `contracts/` is copied or edited.
//!
//! Layout: the crate root mirrors **both** crates at once. The `common`
//! modules sit at the root (`crate::math`, `crate::rates`, ...), exactly where
//! `common/src/lib.rs` puts them, so their own `crate::` paths resolve; and
//! `extern crate self as common` makes the pool files' `common::...` paths
//! resolve to the same modules. The pool modules (`crate::cache`,
//! `crate::guards`, ...) sit where `contracts/pool/src/lib.rs` puts them.
//!
//! Stand-ins (hand-written, documented in `docs/extraction.md`): `types` (wraps the
//! verbatim `types/pool.rs` and `types/shared.rs`), `oracle`, `ownable`,
//! `storage`, `time` and `events`. `storage`, `time` and `events` are marked
//! `opaque` for Charon (signatures only; their meaning is in
//! `lean/PoolAeneas/FunsExternal.lean`); `oracle` and `ownable` are never
//! reachable from the extraction roots.
#![no_std]
#![allow(dead_code, unused_imports)]

extern crate self as common;
// `ops/revenue.rs` says `use stellar_access::ownable;` (stand-in `ownable`).
extern crate self as stellar_access;

// ---- `common` (verbatim) -------------------------------------------------
#[path = "../../../../common/src/constants/mod.rs"]
pub mod constants;
#[path = "../../../../common/src/errors.rs"]
pub mod errors;
#[path = "../../../../common/src/math/mod.rs"]
pub mod math;
#[path = "../../../../common/src/rates/mod.rs"]
pub mod rates;
#[path = "../../../../common/src/validation.rs"]
pub mod validation;
#[path = "../../../../common/src/ttl.rs"]
pub mod ttl;

// ---- `common` (stand-ins) ------------------------------------------------
pub mod oracle;
pub mod types;

// ---- `stellar_access` (stand-in) -----------------------------------------
pub mod ownable;

// ---- `contracts/pool` (verbatim) -----------------------------------------
// Mounted as `market_cache` and re-exported as `cache`, so the production
// paths (`crate::cache::Cache`) resolve unchanged. The extra name avoids an
// Aeneas name clash: generated Lean refers to `cache.Cache.…` relative to the
// crate namespace, which a function parameter named `cache` (guards.rs,
// interest.rs, flash.rs) shadows.
#[path = "../../../../contracts/pool/src/cache/mod.rs"]
mod market_cache;
use market_cache as cache;
#[path = "../../../../contracts/pool/src/guards.rs"]
mod guards;
#[path = "../../../../contracts/pool/src/interest.rs"]
mod interest;
// `ops/mod.rs` and through it every operation leg (`supply.rs`, `borrow.rs`,
// `withdraw.rs`, `repay.rs`, `net_settle.rs`, `seize.rs`, `recapitalize.rs`,
// `strategy.rs`, `revenue.rs`, `market.rs`, `flash.rs`).
#[path = "../../../../contracts/pool/src/ops/mod.rs"]
mod ops;

// ---- `contracts/pool` (stand-ins, opaque to Charon: signatures only) ------
mod events;
mod storage;
mod time;
