use common::types::{Account, AccountPosition, AccountPositionType};

pub fn update_or_remove_position(account: &mut Account, position: &AccountPosition) -> bool {
    let map = if position.position_type == AccountPositionType::Deposit {
        &mut account.supply_positions
    } else if position.position_type == AccountPositionType::Borrow {
        &mut account.borrow_positions
    } else {
        unreachable!()
    };

    if position.scaled_amount_ray == 0 {
        map.remove(position.asset.clone());
        true
    } else {
        map.set(position.asset.clone(), position.clone());
        false
    }
}

pub fn store_position(account: &mut Account, position: &AccountPosition) {
    let map = if position.position_type == AccountPositionType::Deposit {
        &mut account.supply_positions
    } else if position.position_type == AccountPositionType::Borrow {
        &mut account.borrow_positions
    } else {
        unreachable!()
    };
    map.set(position.asset.clone(), position.clone());
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::types::{AccountPositionType, PositionMode};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Map};

    fn empty_account(env: &Env) -> Account {
        Account {
            owner: Address::generate(env),
            is_isolated: false,
            e_mode_category_id: 0,
            mode: PositionMode::Normal,
            isolated_asset: None,
            supply_positions: Map::new(env),
            borrow_positions: Map::new(env),
        }
    }

    fn position(
        _env: &Env,
        asset: Address,
        position_type: AccountPositionType,
        scaled_amount_ray: i128,
    ) -> AccountPosition {
        AccountPosition {
            position_type,
            asset,
            scaled_amount_ray,
            account_id: 1,
            liquidation_threshold_bps: 8_000,
            liquidation_bonus_bps: 500,
            liquidation_fees_bps: 100,
            loan_to_value_bps: 7_500,
        }
    }

    #[test]
    fn test_update_or_remove_position_sets_and_removes_deposit_positions() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let mut account = empty_account(&env);
        let deposit = position(&env, asset.clone(), AccountPositionType::Deposit, 500);

        assert!(!update_or_remove_position(&mut account, &deposit));
        assert_eq!(
            account
                .supply_positions
                .get(asset.clone())
                .unwrap()
                .scaled_amount_ray,
            500
        );

        let zero_deposit = position(&env, asset.clone(), AccountPositionType::Deposit, 0);
        assert!(update_or_remove_position(&mut account, &zero_deposit));
        assert!(account.supply_positions.get(asset).is_none());
    }

    #[test]
    fn test_update_or_remove_position_sets_borrow_positions() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let mut account = empty_account(&env);
        let borrow = position(&env, asset.clone(), AccountPositionType::Borrow, 700);

        assert!(!update_or_remove_position(&mut account, &borrow));
        assert_eq!(
            account
                .borrow_positions
                .get(asset)
                .unwrap()
                .scaled_amount_ray,
            700
        );
    }

    #[test]
    fn test_store_position_overwrites_existing_entries() {
        let env = Env::default();
        let deposit_asset = Address::generate(&env);
        let borrow_asset = Address::generate(&env);
        let mut account = empty_account(&env);

        let first_deposit = position(
            &env,
            deposit_asset.clone(),
            AccountPositionType::Deposit,
            100,
        );
        let second_deposit = position(
            &env,
            deposit_asset.clone(),
            AccountPositionType::Deposit,
            250,
        );
        let borrow = position(&env, borrow_asset.clone(), AccountPositionType::Borrow, 333);

        store_position(&mut account, &first_deposit);
        store_position(&mut account, &second_deposit);
        store_position(&mut account, &borrow);

        assert_eq!(
            account
                .supply_positions
                .get(deposit_asset)
                .unwrap()
                .scaled_amount_ray,
            250
        );
        assert_eq!(
            account
                .borrow_positions
                .get(borrow_asset)
                .unwrap()
                .scaled_amount_ray,
            333
        );
    }
}
