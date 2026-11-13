//! The production LP pricing and observation helpers, verbatim.
//! `providers/` (contract clients) is not included.

#[path = "../../../../common/src/oracle/lp.rs"]
pub mod lp;
#[path = "../../../../common/src/oracle/lp_stable.rs"]
pub mod lp_stable;
#[path = "../../../../common/src/oracle/observation.rs"]
pub mod observation;
