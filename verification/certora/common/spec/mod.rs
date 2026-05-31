/// Certora rules for shared protocol math.
///
/// These rules verify the library layer independently from controller and pool
/// state. Higher-level proofs may then summarize or reuse the same math without
/// re-proving fixed-point and rate-model properties on every path.
pub mod harness;
pub mod math_rules;
pub mod rates_rules;
