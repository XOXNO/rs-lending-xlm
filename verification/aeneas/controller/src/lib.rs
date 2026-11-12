//! The production controller liquidation code
//! (`contracts/controller/src/positions/liquidation/{curve,math,apply}.rs`,
//! `contracts/controller/src/risk/totals.rs`) and the
//! common fixed-point and rate code it calls (`common/src/{math,rates}`),
//! compiled verbatim through `#[path]` includes for extraction to Lean with
//! Charon and Aeneas. Nothing under `common/` or `contracts/` is copied or
//! edited. See `../docs/controller.md`.
//!
//! The controller files say `common::...` for the common crate and
//! `crate::...` for the controller crate. Both resolve to this crate:
//! `extern crate self as common` makes `common::x` mean `crate::x`, and the
//! modules below are laid out so that both path spellings exist.
#![no_std]
#![allow(dead_code)]

extern crate self as common;

pub mod constants;
pub mod errors;
pub mod math;
pub mod rates;
pub mod types;
pub mod validation;

pub mod positions;

// Controller modules: `risk/totals.rs` verbatim; the `Context` model; and
// stand-ins (see each file and `../docs/controller.md`).
pub mod account_store;
pub mod collections;
pub mod context;
pub mod events;
pub mod external;
pub mod payment_legs;
pub mod risk;
pub mod spoke_usage;
pub mod storage;

// `crate::account` and `crate::payments` are re-export modules: the items are
// defined in `account_store` / `payment_legs` because Aeneas would otherwise
// emit Lean names (`account.f`, `payments.f`) that the functions' own
// parameters named `account` / `payments` shadow.
pub mod account {
    pub(crate) use crate::account_store::{cleanup_account_if_empty, update_or_remove_supply_position};
}
pub mod payments {
    pub(crate) use crate::payment_legs::{aggregate_positive_payments, transfer_amount_measured};
}
