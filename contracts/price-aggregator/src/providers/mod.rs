//! Provider reads. Two entry points, one per wire ABI:
//! [`reflector::read_reflector_source`] (SEP-40) and
//! [`multi_feed::read_multi_feed_source`] (RedStone / XOXNO adapters). The
//! engine calls them directly; there is no intermediate dispatch layer.
//!
//! Market-data problems return `None` (including Reflector host traps via
//! `try_lastprice` / `try_prices`). Hard path maps miss → unreadable →
//! `force` → `NoLastPrice`. Paths that still trap under either discipline:
//! a TWAP record count rejected by `validate_twap_records`, and a Reflector
//! asset ref `to_reflector_asset` cannot express. Scaled product overflow is
//! typed (`InvalidPrice`) in the engine, not a host trap.
//!
//! Under `--features certora` both reads are summarized; see
//! `crate::spec::summaries`.

pub(crate) mod multi_feed;
pub(crate) mod reflector;
