use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct CommonCertoraHarness;

#[contractimpl]
impl CommonCertoraHarness {
    pub fn ping(_env: Env) {}
}
