use common::types::{AggregatorSwap, PositionMode, SwapHop, SwapPath, SwapVenue};
use soroban_sdk::{vec, Vec};

use crate::context::{AccountEntry, LendingTest};
use crate::helpers::f64_to_i128;

/// Default flash-loan fee in bps for every preset (`flashloan_fee_bps =
/// 9`). For strategies that flash-borrow (`multiply`, `swap_debt`), the
/// controller's actual swap `amount_in` is the requested borrow MINUS
/// this fee. Tests that build fixtures from the *requested* borrow
/// amount can call [`apply_flash_fee`] to land on the post-fee value
/// `validate_aggregator_swap` expects.
pub const DEFAULT_FLASHLOAN_FEE_BPS: i128 = 9;

/// `requested * (10_000 - DEFAULT_FLASHLOAN_FEE_BPS) / 10_000` — the
/// amount the controller actually receives from the flash strategy
/// borrow under the default preset config. Use this when sizing the
/// `amount_in` field of a fixture path for `multiply` / `swap_debt`.
pub fn apply_flash_fee(requested_raw: i128) -> i128 {
    requested_raw * (10_000 - DEFAULT_FLASHLOAN_FEE_BPS) / 10_000
}

/// Build a single-path single-hop `AggregatorSwap` whose token chain
/// matches the strategy's `(token_in, token_out)` and slippage floor
/// matches `min_out`. Used by every happy-path test that exercises
/// `swap_tokens` end-to-end.
///
/// Router paths are amount-free: the router computes per-path input from
/// `total_in * split_ppm / 1_000_000`. For single-path fixtures,
/// `split_ppm = 1_000_000`. The `_amount_in` parameter is retained for
/// call-site compatibility; harness strategy methods pass the authoritative
/// amount to the controller as `BatchSwap.total_in`.
///
/// The mock aggregator ignores `pool` and `fee_bps`, so the harness
/// aggregator address is used as a placeholder pool.
pub fn build_aggregator_swap(
    t: &LendingTest,
    token_in_name: &str,
    token_out_name: &str,
    _amount_in: i128,
    min_out: i128,
) -> AggregatorSwap {
    let env = &t.env;
    let in_addr = t.resolve_market(token_in_name).asset.clone();
    let out_addr = t.resolve_market(token_out_name).asset.clone();
    AggregatorSwap {
        paths: vec![
            env,
            SwapPath {
                split_ppm: 1_000_000,
                hops: vec![
                    env,
                    SwapHop {
                        fee_bps: 30,
                        pool: t.aggregator.clone(),
                        token_in: in_addr,
                        token_out: out_addr,
                        venue: SwapVenue::Soroswap,
                    },
                ],
            },
        ],
        total_min_out: min_out,
    }
}

impl LendingTest {
    /// Pre-fund the swap router with tokens so the mock router can transfer them.
    /// Call this before any strategy operation that involves a swap.
    pub fn fund_router(&self, asset_name: &str, amount: f64) {
        let market = self.resolve_market(asset_name);
        let raw = f64_to_i128(amount, market.decimals);
        market.token_admin.mint(&self.aggregator, &raw);
    }

    /// Pre-fund the swap router with a raw token amount.
    pub fn fund_router_raw(&self, asset_name: &str, amount: i128) {
        let market = self.resolve_market(asset_name);
        market.token_admin.mint(&self.aggregator, &amount);
    }

    /// Builds an empty `AggregatorSwap` for error-path tests that panic
    /// BEFORE reaching `swap_tokens`. For happy-path tests that exercise
    /// the swap, build a real `AggregatorSwap` per-test with at least one
    /// `SwapPath` containing one or more `SwapHop`s.
    ///
    /// `total_min_out = 1` satisfies the controller's `> 0` entry check
    /// without constraining behavior, since the router is never reached.
    pub fn mock_swap_steps(
        &self,
        _token_in: &str,
        _token_out: &str,
        _price_wad: i128,
    ) -> AggregatorSwap {
        AggregatorSwap {
            paths: Vec::new(&self.env),
            total_min_out: 1,
        }
    }

    /// Execute a multiply strategy.
    pub fn multiply(
        &mut self,
        user: &str,
        collateral_asset: &str,
        debt_amount: f64,
        debt_asset: &str,
        mode: PositionMode,
        steps: &AggregatorSwap,
    ) -> u64 {
        let debt_decimals = self.resolve_market(debt_asset).decimals;
        let raw_debt = f64_to_i128(debt_amount, debt_decimals);
        let caller_addr = self.get_or_create_user(user);
        let collateral_addr = self.resolve_asset(collateral_asset);
        let debt_addr = self.resolve_asset(debt_asset);

        let ctrl = self.ctrl_client();
        let account_id = ctrl.multiply(
            &caller_addr,
            &0u64, // account_id: 0 = create new
            &0u32, // e_mode_category
            &collateral_addr,
            &raw_debt,
            &debt_addr,
            &mode,
            steps,
            &None, // initial_payment
            &None, // convert_steps
        );
        let attrs = ctrl.get_account_attributes(&account_id);

        // Register account in harness state
        let user_state = self.users.get_mut(user).expect("user exists");
        user_state.accounts.push(AccountEntry {
            account_id,
            e_mode_category: attrs.e_mode_category_id,
            mode: attrs.mode,
            is_isolated: attrs.is_isolated,
        });
        if user_state.default_account_id.is_none() {
            user_state.default_account_id = Some(account_id);
        }

        account_id
    }

    /// Try multiply with category -- returns Result.
    #[allow(clippy::too_many_arguments)]
    pub fn try_multiply_with_category(
        &mut self,
        user: &str,
        category: u32,
        collateral_asset: &str,
        debt_amount: f64,
        debt_asset: &str,
        mode: PositionMode,
        steps: &AggregatorSwap,
    ) -> Result<u64, soroban_sdk::Error> {
        let debt_decimals = self.resolve_market(debt_asset).decimals;
        let raw_debt = f64_to_i128(debt_amount, debt_decimals);
        let caller_addr = self.get_or_create_user(user);
        let collateral_addr = self.resolve_asset(collateral_asset);
        let debt_addr = self.resolve_asset(debt_asset);

        let ctrl = self.ctrl_client();
        match ctrl.try_multiply(
            &caller_addr,
            &0u64, // account_id: 0 = create new
            &category,
            &collateral_addr,
            &raw_debt,
            &debt_addr,
            &mode,
            steps,
            &None, // initial_payment
            &None, // convert_steps
        ) {
            Ok(Ok(id)) => {
                let attrs = ctrl.get_account_attributes(&id);
                let user_state = self.users.get_mut(user).expect("user exists");
                user_state.accounts.push(AccountEntry {
                    account_id: id,
                    e_mode_category: attrs.e_mode_category_id,
                    mode: attrs.mode,
                    is_isolated: attrs.is_isolated,
                });
                if user_state.default_account_id.is_none() {
                    user_state.default_account_id = Some(id);
                }
                Ok(id)
            }
            Ok(Err(err)) => Err(err),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Try multiply -- returns Result.
    pub fn try_multiply(
        &mut self,
        user: &str,
        collateral_asset: &str,
        debt_amount: f64,
        debt_asset: &str,
        mode: PositionMode,
        steps: &AggregatorSwap,
    ) -> Result<u64, soroban_sdk::Error> {
        self.try_multiply_with_category(
            user,
            0,
            collateral_asset,
            debt_amount,
            debt_asset,
            mode,
            steps,
        )
    }

    /// Swap an existing debt position from one token to another.
    pub fn swap_debt(
        &mut self,
        user: &str,
        existing_debt: &str,
        new_amount: f64,
        new_debt: &str,
        steps: &AggregatorSwap,
    ) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let existing_addr = self.resolve_asset(existing_debt);
        let new_addr = self.resolve_asset(new_debt);
        let decimals = self.resolve_market(new_debt).decimals;
        let raw = f64_to_i128(new_amount, decimals);

        self.ctrl_client()
            .swap_debt(&addr, &account_id, &existing_addr, &raw, &new_addr, steps);
    }

    /// Try swap debt -- returns Result.
    pub fn try_swap_debt(
        &mut self,
        user: &str,
        existing_debt: &str,
        new_amount: f64,
        new_debt: &str,
        steps: &AggregatorSwap,
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let existing_addr = self.resolve_asset(existing_debt);
        let new_addr = self.resolve_asset(new_debt);
        let decimals = self.resolve_market(new_debt).decimals;
        let raw = f64_to_i128(new_amount, decimals);

        match self.ctrl_client().try_swap_debt(
            &addr,
            &account_id,
            &existing_addr,
            &raw,
            &new_addr,
            steps,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Swap an existing collateral position from one token to another.
    pub fn swap_collateral(
        &mut self,
        user: &str,
        current_collateral: &str,
        amount: f64,
        new_collateral: &str,
        steps: &AggregatorSwap,
    ) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let current_addr = self.resolve_asset(current_collateral);
        let new_addr = self.resolve_asset(new_collateral);
        let decimals = self.resolve_market(current_collateral).decimals;
        let raw = f64_to_i128(amount, decimals);

        self.ctrl_client().swap_collateral(
            &addr,
            &account_id,
            &current_addr,
            &raw,
            &new_addr,
            steps,
        );
    }

    /// Try swap collateral -- returns Result.
    pub fn try_swap_collateral(
        &mut self,
        user: &str,
        current_collateral: &str,
        amount: f64,
        new_collateral: &str,
        steps: &AggregatorSwap,
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let current_addr = self.resolve_asset(current_collateral);
        let new_addr = self.resolve_asset(new_collateral);
        let decimals = self.resolve_market(current_collateral).decimals;
        let raw = f64_to_i128(amount, decimals);

        match self.ctrl_client().try_swap_collateral(
            &addr,
            &account_id,
            &current_addr,
            &raw,
            &new_addr,
            steps,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Repay debt with collateral.
    pub fn repay_debt_with_collateral(
        &mut self,
        user: &str,
        collateral_asset: &str,
        collateral_amount: f64,
        debt_asset: &str,
        steps: &AggregatorSwap,
        close_position: bool,
    ) {
        let account_id = self.resolve_account_id(user);
        let addr = self.users.get(user).unwrap().address.clone();
        let collateral_addr = self.resolve_asset(collateral_asset);
        let debt_addr = self.resolve_asset(debt_asset);
        let decimals = self.resolve_market(collateral_asset).decimals;
        let raw = f64_to_i128(collateral_amount, decimals);

        self.ctrl_client().repay_debt_with_collateral(
            &addr,
            &account_id,
            &collateral_addr,
            &raw,
            &debt_addr,
            steps,
            &close_position,
        );
    }

    /// Try repay debt with collateral -- returns Result.
    pub fn try_repay_debt_with_collateral(
        &mut self,
        user: &str,
        collateral_asset: &str,
        collateral_amount: f64,
        debt_asset: &str,
        steps: &AggregatorSwap,
        close_position: bool,
    ) -> Result<(), soroban_sdk::Error> {
        let account_id = self.try_resolve_account_id(user)?;
        let addr = self.users.get(user).unwrap().address.clone();
        let collateral_addr = self.resolve_asset(collateral_asset);
        let debt_addr = self.resolve_asset(debt_asset);
        let decimals = self.resolve_market(collateral_asset).decimals;
        let raw = f64_to_i128(collateral_amount, decimals);

        match self.ctrl_client().try_repay_debt_with_collateral(
            &addr,
            &account_id,
            &collateral_addr,
            &raw,
            &debt_addr,
            steps,
            &close_position,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
}
