//! Cross-contract client trait for a XOXNO oracle adapter contract.

use soroban_sdk::{contractclient, Env};

/// Client interface for a XOXNO oracle adapter contract.
#[contractclient(name = "XoxnoOracleAdapterClient")]
#[allow(dead_code)]
pub trait XoxnoOracleAdapter {
    /// Returns the maximum age, in seconds, allowed for a submitted price.
    fn max_submission_age_seconds(env: Env) -> u64;
}
