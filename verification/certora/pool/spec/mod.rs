/// Certora rules for the liquidity pool layer.
///
/// The controller verifier summarizes pool calls for tractability. These rules
/// verify the real pool implementation against the summary contracts consumed
/// by controller proofs.
pub mod additivity_rules;
pub mod integrity_rules;
pub mod summary_contract_rules;
