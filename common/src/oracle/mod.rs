//! Oracle price sourcing and derivation: external price-provider
//! integrations ([`providers`]), price-observation and unit-conversion
//! helpers ([`observation`]), and fair-value pricing for constant-product
//! ([`lp`]) and StableSwap-style ([`lp_stable`]) LP tokens.

pub mod lp;
pub mod lp_stable;
pub mod observation;
pub mod providers;
