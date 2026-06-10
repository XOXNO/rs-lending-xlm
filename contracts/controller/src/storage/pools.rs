//! Pool registry and listed asset storage.

use super::renew_protocol_shared_key;
use common::constants::MAX_POOLS_LIST_ENTRIES;
use common::errors::GenericError;
use common::types::ControllerKey;
use soroban_sdk::{assert_with_error, Address, Env, Vec};

/// Returns listed asset addresses from the controller pool registry.
pub(crate) fn get_pools_list(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&ControllerKey::PoolsList)
        .unwrap_or_else(|| Vec::new(env))
}

/// Adds an asset to `PoolsList`; duplicate inserts are no-ops.
pub(crate) fn add_to_pools_list(env: &Env, asset: &Address) {
    let mut list = get_pools_list(env);
    if list.iter().any(|existing| &existing == asset) {
        return;
    }
    assert_with_error!(
        env,
        list.len() < MAX_POOLS_LIST_ENTRIES,
        GenericError::InvalidPositionLimits
    );
    list.push_back(asset.clone());
    let key = ControllerKey::PoolsList;
    env.storage().persistent().set(&key, &list);
    renew_protocol_shared_key(env, &key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Controller;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    #[test]
    fn test_add_to_pools_list_duplicate_is_noop() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(Controller, (admin,));
        env.as_contract(&contract_id, || {
            let asset = Address::generate(&env);
            add_to_pools_list(&env, &asset);
            assert_eq!(get_pools_list(&env).len(), 1);
            add_to_pools_list(&env, &asset);
            assert_eq!(get_pools_list(&env).len(), 1);
        });
    }

    #[test]
    #[should_panic]
    fn test_add_to_pools_list_rejects_beyond_max_entries() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(Controller, (admin,));
        env.as_contract(&contract_id, || {
            for _ in 0..MAX_POOLS_LIST_ENTRIES {
                add_to_pools_list(&env, &Address::generate(&env));
            }
            add_to_pools_list(&env, &Address::generate(&env));
        });
    }
}
