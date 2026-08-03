use common::errors::GenericError;
use common::types::{HubAssetKey, PriceKey};

use soroban_sdk::{assert_with_error, contractimpl, Address, Env, Symbol};

use stellar_macros::only_owner;

use crate::access::{self, GUARDIAN_ROLE, ORACLE_ROLE};
use crate::timelock::*;
use crate::{Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl Governance {
    pub fn pause(env: Env, caller: Address) {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).pause();
    }

    pub fn set_spoke_asset_flags(
        env: Env,
        caller: Address,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    ) {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).set_spoke_asset_flags(&spoke_id, &hub_asset, &paused, &frozen);
    }

    pub fn set_sanity_band(env: Env, caller: Address, key: PriceKey, min_wad: i128, max_wad: i128) {
        begin_immediate(&env, &caller, ORACLE_ROLE);
        price_aggregator_client(&env).set_sanity_band(&key, &min_wad, &max_wad);
    }

    pub fn create_hub(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).create_hub()
    }

    pub fn add_spoke(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).add_spoke()
    }

    #[only_owner]
    pub fn revoke_role_immediate(env: Env, account: Address, role: Symbol) {
        assert_with_error!(
            &env,
            role == Symbol::new(&env, GUARDIAN_ROLE) || role == Symbol::new(&env, ORACLE_ROLE),
            GenericError::InvalidRole
        );
        access::apply_revoke_role(&env, &account, &role);
    }
}
