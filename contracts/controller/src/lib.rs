#![no_std]
#![allow(clippy::too_many_arguments)]

//! Lending Controller — the single user-facing contract for the XOXNO
//! multi-asset lending protocol on Stellar Soroban.
//!
//! # Responsibilities
//!
//! The controller owns:
//! - All user account state (metadata + per-side supply/borrow positions).
//! - Market configuration, asset risk parameters, e-mode categories,
//!   isolation debt ceilings, position limits.
//! - Oracle price resolution, validation, and policy (see `oracle`).
//! - Risk checks (LTV, health factor, caps, dust, isolation, e-mode).
//! - All mutating flows: supply/borrow/repay/withdraw/liquidate,
//!   plus the four account-bound strategies and flash loans.
//! - Access control (owner + KEEPER / REVENUE / ORACLE roles), pause,
//!   and contract upgrade.
//! - Deployment and ownership of per-asset `pool` contracts.
//!
//! Pools (in the sibling `pool` crate) own only per-asset custody and
//! interest accounting; the controller is their sole authorized caller
//! for mutations (see ADR 0001).
//!
//! # Module Organization & Rationale
//!
//! The crate is deliberately split to make security boundaries, storage
//! access patterns, and verification scope obvious:
//!
//! - `router`, `config`, `access`: the public `#[contractimpl]` surface.
//!   Router hosts the bulk of user + privileged entrypoints and delegates
//!   to the other modules. Config contains the heavier owner/oracle-role
//!   setters. Access owns roles, ownership transfer, pause.
//!
//! - `positions/`: one file per core operation (supply, borrow, repay,
//!   withdraw, liquidation, plus liquidation_math). Each file implements
//!   the staged pipeline (auth → cache+policy → validation → pool calls →
//!   risk re-check → storage write + batch events). Splitting by side and
//!   operation keeps hot paths from touching unrelated storage keys
//!   (ADR 0002) and makes the exact risk gates per flow easy to audit.
//!
//! - `strategies/`: the four aggregator-routed strategies
//!   (`multiply`, `swap_collateral`, `swap_debt`, `repay_debt_with_collateral`)
//!   plus `flash_loan` and shared helpers. These flows are grouped because
//!   they all cross the untrusted router boundary and therefore share the
//!   balance-delta verification discipline (ADR 0005). The top-level
//!   `flash_loan` API lives here for historical/implementation affinity with
//!   the guarded execution pattern, not because it is a "strategy".
//!
//! - `storage/`: thin, key-type-aware accessors + TTL renewal helpers.
//!   Split into `account`, `debt`, `emode`, `instance`, `market`, `pools`,
//!   `ttl` so that each concern can be reasoned about independently and
//!   so that TTL policy (user vs shared vs instance) is localized. Never
//!   perform direct storage access from business logic; go through cache
//!   or these helpers.
//!
//! - `cache/`: the *mandatory* in-transaction context object. It provides
//!   memoized reads for prices, market configs, indexes, pool sync data,
//!   and e-mode data, plus deferred batch event emission for positions and
//!   markets. It is the single place that receives an `OraclePolicy` and
//!   therefore decides which oracle failure modes are tolerated (ADR 0004).
//!   Views use `new_view` (no instance TTL bump); mutating paths use `new`.
//!
//! - `oracle/`: price subsystem with clean provider boundaries (the model
//!   established by the recent oracle improvements). Client surfaces live
//!   under `providers/*/client.rs`; consumption + dispatch under provider
//!   modules; pure tolerance math in `tolerance`; live probing validation
//!   split from pure config validation; heavy paths replaced by certora
//!   harnesses. See `oracle/mod.rs` for the design principles.
//!
//! - `cross_contract/`: the only place that constructs Soroban clients for
//!   external contracts (pools and SAC tokens). Under `certora` the modules
//!   are path-replaced by harnesses so the prover never reasons about real
//!   cross-contract state. Business logic must never call `Client::new`
//!   directly.
//!
//! - `validation.rs`: centralized, pure-ish guard functions used by every
//!   flow (owner match, market active, healthy account, position limits,
//!   asset config shape, etc.). Kept separate so the checks are easy to
//!   enumerate for audit and certora rules.
//!
//! - `helpers/`: pure math helpers (position value, health factor, LTV
//!   collateral, dust checks, …) that do not perform storage or oracle
//!   calls themselves. Under certora this module is also harnessed.
//!
//! - `emode.rs`, `utils.rs`: small focused utilities for e-mode application
//!   and common payment aggregation / event context helpers.
//!
//! - `views/`: read-only aggregates and liquidation estimation. Uses
//!   `ControllerCache::new_view` and a certora harness replacement.
//!
//! # Key Mental Models & Non-Obvious Points
//!
//! - **OraclePolicy is per-entrypoint, not per-price.** Every risk-bearing
//!   decision picks its policy when it constructs the cache (see table in
//!   ADR 0004 and usage sites in `positions/*`, `strategies/*`). The policy
//!   then flows into the oracle resolution path. This is the mechanism that
//!   lets repay and supply continue under degraded or disabled-market
//!   conditions while borrow/liquidate stay strict.
//!
//! - **Cache vs. direct storage.** In almost all mutating business logic you
//!   go through `ControllerCache`. Direct `storage::` calls are limited to
//!   TTL renewal, isolated debt counters, flash-loan guard flag, and a few
//!   initialization paths. The cache guarantees consistent policy application
//!   and coalesces events.
//!
//! - **E-mode vs. isolation (ADR 0008).** Isolation is per-account state
//!   that restricts the whole account. E-mode is a category that can be
//!   applied per-supply operation and that can be deprecated without
//!   touching live accounts. The two features compose but have different
//!   lifecycle and storage shapes.
//!
//! - **Strategies are not privileged.** They are subject to exactly the
//!   same risk model and oracle policy as the primitive operations they
//!   compose. The only extra surface is the untrusted router boundary,
//!   which is defended by pre/post balance delta checks, not by trusting
//!   any reported numbers from the aggregator.
//!
//! - **Flash-loan guard is a transaction-singleton.** The `FlashLoanOngoing`
//!   session key prevents re-entrancy into any controller mutation from a
//!   flash-loan callback (or from a strategy router callback that itself
//!   uses the flash-loan machinery internally).
//!
//! - **Certora harness strategy.** Expensive or non-deterministic modules
//!   (price resolution, tolerance math, helpers, views aggregates, all
//!   cross-contract surfaces, and selected storage paths) are replaced at
//!   compile time via `#[path = "..."]` when the `certora` feature is on.
//!   The rest of the crate continues to compile against the original
//!   signatures, keeping the proof surface small and stable.
//!
//! # External References
//!
//! - High-level architecture & topology: `SCF_BUILD_ARCHITECTURE.md` (root)
//! - Protocol invariants & verification mapping: `architecture/INVARIANTS.md`
//! - Design decisions (especially 0001–0009): `architecture/decisions/`
//! - Recent oracle boundary example: `oracle/mod.rs` and its submodules
//!
//! All numeric domains (BPS / WAD / RAY / asset-native) and rounding rules
//! are defined in the `common` crate.

mod access;
pub(crate) mod cache;
mod config;
pub(crate) mod cross_contract;
#[cfg(not(feature = "certora"))]
pub(crate) mod helpers;
#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/harness/helpers.rs"]
pub(crate) mod helpers;
// ^ Full module replacement: helpers is the shared-primitives bucket
//   (see its own header and helpers/mod.rs:114). Most invasive harness
//   override. All other controller overrides are narrower (price/tolerance
//   for cost, aggregates/views for iteration, cross-contract for external).
pub(crate) mod emode;
pub(crate) mod oracle;
pub(crate) mod positions;
mod router;
mod storage;
mod strategies;
mod utils;
mod validation;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../verification/certora/controller/spec/mod.rs"]
pub mod spec;
// ^ Includes the entire CVLR rule suite + summaries (see spec/mod.rs and
//   verification/certora/README.md "Production Boundary"). No rules live
//   in prod source.

use soroban_sdk::{contract, contractmeta};

contractmeta!(key = "name", val = "Lending Controller");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Controller;
