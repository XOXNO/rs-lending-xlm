# On-chain endpoint coverage — e2e (main 077d11e, XOXNO testnet RPC)

Sources: agg+stress (run `xoxnorpc-20260626-171834`), liq (run `dfxfix-20260626-173854`) — all three GREEN.
`tx` = state-changing call landed on-chain (tx hash in report.md); `read` = view; `xfail` = expected-revert verified; `sim` = fee-less budget probe.

## Controller — 70 public endpoints

| endpoint | on-chain coverage |
|---|---|
| `accept_ownership` | 2 tx |
| `account_exists` | 2 read |
| `add_asset_to_e_mode_category` | 3 tx, 1 xfail |
| `add_e_mode_category` | 2 tx |
| `add_rewards` | 1 tx |
| `approve_blend_pool` | **NOT covered** |
| `approve_token` | 34 tx |
| `borrow` | 9 tx, 3 xfail, 10 sim |
| `claim_revenue` | 2 tx |
| `clean_bad_debt` | 1 tx, 1 xfail |
| `create_liquidity_pool` | 33 tx |
| `deploy_pool` | 3 tx |
| `disable_token_oracle` | 1 tx, 2 xfail |
| `edit_asset_config` | 34 tx, 1 xfail |
| `edit_asset_in_e_mode_category` | 1 tx |
| `flash_loan` | 1 tx, 5 xfail |
| `get_account_attributes` | 1 read |
| `get_account_positions` | 1 read |
| `get_app_version` | 1 read |
| `get_borrow_amount` | 18 read |
| `get_collateral_amount` | 2 read |
| `get_e_mode_category` | 1 read |
| `get_health_factor` | 8 read |
| `get_liquidation_collateral` | 1 read |
| `get_liquidation_estimate` | 2 read |
| `get_ltv_collateral_usd` | 1 read |
| `get_market_config` | 4 read |
| `get_market_index` | **NOT covered** |
| `get_market_indexes_detailed` | 1 read |
| `get_markets_detailed` | 1 read |
| `get_min_borrow_collateral_usd` | 2 read |
| `get_pool_address` | 1 read |
| `get_total_borrow_usd` | 1 read |
| `get_total_collateral_usd` | 1 read |
| `is_blend_pool_approved` | **NOT covered** |
| `is_liquidatable` | 2 read |
| `liquidate` | 4 tx, 1 xfail |
| `max_borrow` | 1 read |
| `max_supply` | 1 read |
| `max_withdraw` | 1 read |
| `migrate` | 1 tx |
| `migrate_from_blend` | **NOT covered** |
| `multiply` | 2 tx |
| `pause` | 2 tx |
| `remove_asset_from_e_mode` | 1 tx |
| `remove_e_mode_category` | 1 tx |
| `renew_account` | 1 tx |
| `repay` | 3 tx |
| `repay_debt_with_collateral` | 2 tx |
| `revoke_blend_pool` | **NOT covered** |
| `revoke_token` | 1 tx |
| `set_accumulator` | 3 tx |
| `set_aggregator` | 3 tx |
| `set_liquidity_pool_template` | 3 tx |
| `set_market_oracle_config` | 33 tx |
| `set_min_borrow_collateral_usd` | 2 tx, 1 xfail |
| `set_oracle_tolerance` | 1 tx, 1 xfail |
| `set_position_limits` | 1 tx |
| `supply` | 17 tx, 5 xfail, 5 sim |
| `swap_collateral` | 1 tx |
| `swap_debt` | 1 tx |
| `transfer_ownership` | 2 tx |
| `unpause` | 6 tx, 1 xfail |
| `update_account_threshold` | 2 tx |
| `update_indexes` | 2 tx |
| `update_pool_caps` | 4 tx |
| `upgrade` | 1 tx |
| `upgrade_liquidity_pool_params` | 1 tx |
| `upgrade_pool` | **NOT covered** |
| `withdraw` | 4 tx, 4 xfail |

## Governance — 16 public endpoints

| endpoint | on-chain coverage |
|---|---|
| `accept_ownership` | 2 tx |
| `cancel` | 1 tx |
| `controller` | 1 read |
| `deploy_controller` | 3 tx, 1 xfail |
| `execute` | 1 tx, 1 xfail |
| `execute_self` | **NOT covered** |
| `get_min_delay` | **NOT covered** |
| `get_operation_ledger` | **NOT covered** |
| `get_operation_state` | 3 read |
| `has_role` | **NOT covered** |
| `hash_operation` | **NOT covered** |
| `pause` | 2 tx |
| `propose` | 2 tx, 2 xfail |
| `resolve_market_oracle_config` | 33 read |
| `resolve_oracle_tolerance` | 2 read |
| `unpause` | 6 tx, 1 xfail |

## DeFindex strategy — 5 public endpoints

| endpoint | on-chain coverage |
|---|---|
| `asset` | 1 read |
| `balance` | 5 read |
| `deposit` | 2 tx, 1 xfail |
| `harvest` | 1 tx |
| `withdraw` | 4 tx, 4 xfail |

## Pool — 24 public endpoints

| endpoint | on-chain coverage |
|---|---|
| `add_rewards` | 1 tx |
| `borrow` | 9 tx, 3 xfail, 10 sim |
| `claim_revenue` | 2 tx |
| `create_market` | _transitive (driven via controller)_ |
| `create_strategy` | _transitive (driven via controller)_ |
| `flash_loan` | 1 tx, 5 xfail |
| `get_borrow_rate` | 1 read |
| `get_borrowed_amount` | **NOT covered** |
| `get_bulk_indexes` | **NOT covered** |
| `get_delta_time` | **NOT covered** |
| `get_deposit_rate` | **NOT covered** |
| `get_reserves` | **NOT covered** |
| `get_revenue` | 2 read |
| `get_supplied_amount` | **NOT covered** |
| `get_sync_data` | _transitive (driven via controller)_ |
| `get_utilisation` | 1 read |
| `repay` | 3 tx |
| `seize_position` | _transitive (driven via controller)_ |
| `supply` | 17 tx, 5 xfail, 5 sim |
| `update_caps` | _transitive (driven via controller)_ |
| `update_indexes` | 2 tx |
| `update_params` | _transitive (driven via controller)_ |
| `upgrade` | 1 tx |
| `withdraw` | 4 tx, 4 xfail |

## Summary

| contract | covered (direct or transitive) | total |
|---|---|---|
| Controller | 64 | 70 |
| Governance | 11 | 16 |
| DeFindex strategy | 5 | 5 |
| Pool | 18 | 24 |
| **All** | **98** | **115** (85%) |