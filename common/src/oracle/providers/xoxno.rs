//! Cross-contract client trait and call helper for reading configured
//! staleness limits from a XOXNO oracle adapter contract.

use soroban_sdk::{contractclient, Address, Env};

/// Client interface for a XOXNO oracle adapter contract.
#[contractclient(name = "XoxnoOracleAdapterClient")]
#[allow(dead_code)]
pub trait XoxnoOracleAdapter {
    /// Returns the maximum age, in seconds, allowed for a submitted price.
    fn max_submission_age_seconds(env: Env) -> u64;
}

/// Calls `contract`'s `max_submission_age_seconds` function directly and returns the result.
pub fn max_submission_age(env: &Env, contract: &Address) -> u64 {
    XoxnoOracleAdapterClient::new(env, contract).max_submission_age_seconds()
}

#[cfg(test)]
#[path = "../../../tests/oracle/providers/xoxno.rs"]
mod tests;
