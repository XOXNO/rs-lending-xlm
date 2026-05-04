use common::types::{Account, AccountPosition, AccountPositionType};
use soroban_sdk::Address;

/// Upserts or removes the position from the appropriate side map on
/// `account`. Removes the entry when `scaled_amount_ray == 0`; returns
/// `true` when removed.
///
/// `side` and `asset` are taken from the caller because
/// [`AccountPosition`] no longer carries them in its stored form — the
/// side is implied by which map the value lives in and the asset is the
/// map key.
pub fn update_or_remove_position(
    account: &mut Account,
    side: AccountPositionType,
    asset: &Address,
    position: &AccountPosition,
) -> bool {
    let map = match side {
        AccountPositionType::Deposit => &mut account.supply_positions,
        AccountPositionType::Borrow => &mut account.borrow_positions,
    };

    if position.scaled_amount_ray == 0 {
        map.remove(asset.clone());
        true
    } else {
        map.set(asset.clone(), position.clone());
        false
    }
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

    fn position(scaled_amount_ray: i128) -> AccountPosition {
        AccountPosition {
            scaled_amount_ray,
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
        let deposit = position(500);

        assert!(!update_or_remove_position(
            &mut account,
            AccountPositionType::Deposit,
            &asset,
            &deposit,
        ));
        assert_eq!(
            account
                .supply_positions
                .get(asset.clone())
                .unwrap()
                .scaled_amount_ray,
            500
        );

        let zero_deposit = position(0);
        assert!(update_or_remove_position(
            &mut account,
            AccountPositionType::Deposit,
            &asset,
            &zero_deposit,
        ));
        assert!(account.supply_positions.get(asset).is_none());
    }

    #[test]
    fn test_update_or_remove_position_sets_borrow_positions() {
        let env = Env::default();
        let asset = Address::generate(&env);
        let mut account = empty_account(&env);
        let borrow = position(700);

        assert!(!update_or_remove_position(
            &mut account,
            AccountPositionType::Borrow,
            &asset,
            &borrow,
        ));
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
    fn test_update_or_remove_position_overwrites_existing_entries() {
        let env = Env::default();
        let deposit_asset = Address::generate(&env);
        let borrow_asset = Address::generate(&env);
        let mut account = empty_account(&env);

        let first_deposit = position(100);
        let second_deposit = position(250);
        let borrow = position(333);

        assert!(!update_or_remove_position(
            &mut account,
            AccountPositionType::Deposit,
            &deposit_asset,
            &first_deposit,
        ));
        assert!(!update_or_remove_position(
            &mut account,
            AccountPositionType::Deposit,
            &deposit_asset,
            &second_deposit,
        ));
        assert!(!update_or_remove_position(
            &mut account,
            AccountPositionType::Borrow,
            &borrow_asset,
            &borrow,
        ));

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
