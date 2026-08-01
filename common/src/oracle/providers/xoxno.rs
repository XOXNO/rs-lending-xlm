use soroban_sdk::{contractclient, Address, Env};

#[contractclient(name = "XoxnoOracleAdapterClient")]
#[allow(dead_code)]
pub trait XoxnoOracleAdapter {
    fn max_submission_age_seconds(env: Env) -> u64;
    fn max_stale_seconds(env: Env) -> u64;
    fn max_relative_skew_seconds(env: Env) -> u64;
}

pub fn max_submission_age(env: &Env, contract: &Address) -> u64 {
    XoxnoOracleAdapterClient::new(env, contract).max_submission_age_seconds()
}
