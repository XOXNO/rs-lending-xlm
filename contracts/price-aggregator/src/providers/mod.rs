//! Provider reads. Two entry points, one per wire ABI:
//! [`reflector::read_reflector_source`] (SEP-40) and
//! [`multi_feed::read_multi_feed_source`] (RedStone/XOXNO adapters). The engine
//! calls them directly; there is no dispatch layer, because the composable
//! model already decided which one to call when it stored a `ProviderRef`.
//!
//! Both take a `soft` flag. Soft turns per-asset read problems into `None`; it
//! is not panic-free. Failures that revert under either discipline: a TWAP
//! record count `validate_twap_records` rejects, a Reflector asset ref
//! `to_reflector_asset` cannot express, a Reflector contract that reverts at
//! read time (the SEP-40 client calls are not `try_`), and a scaled reprice
//! whose `Wad::mul` overflows. `compose`'s callers gate each source before the
//! next is read, so a caller whose verdict is settled never reverts inside a
//! source it would discard.
//!
//! Under `--features certora` both are summarized — see
//! `crate::spec::summaries`.

pub(crate) mod multi_feed;
pub(crate) mod reflector;
