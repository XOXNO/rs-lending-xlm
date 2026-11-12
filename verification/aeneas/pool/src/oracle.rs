//! Stand-in for `common::oracle`: the three constants of
//! `oracle/observation.rs` that `validation.rs` imports, copied with their
//! production values. None of them reaches the extracted Lean.

pub mod observation {
    pub const MAX_TWAP_RECORDS: u32 = 12;
    pub const MAX_SINGLE_SOURCE_SANITY_BAND_BPS: i128 = 1_000;
    pub const MAX_LP_SANITY_BAND_BPS: i128 = 8_182;
}
