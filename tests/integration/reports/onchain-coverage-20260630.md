# Integration On-Chain Coverage - 2026-06-30

Scope: integration coverage after explicit create_hub/create_spoke refactor.

Run gates executed:
- `bash -n` for every `tests/integration/**/*.sh`
- `RUN_TS=syntax-probe` source probe for all integration libs and flows
- `RUN_TS=20260630-045422-stressfix PHASES='deploy stress'` live testnet stress lane: GREEN (`ok=230`, `read=20`, `sim-ok=15`)

Architecture checks:
- Hub IDs are created via `create_hub`; integration helpers no longer synthesize hub 0.
- Spoke IDs are created via `add_spoke`; user flows no longer use spoke 0.
- Hub-aware helpers: `hub_key <hub> <asset>`, `hub_vec <hub> ...`, `pay_vec <hub> ...`, `spoke_args <hub> <asset> <spoke> ...`.
- Real Blend migration remains opt-in with `BLEND_MIGRATION_LIVE=1`; allow-list endpoints are covered by default.

## Endpoint Matrix

| contract | endpoint | integration status | source |
|---|---|---|---|
| Controller | `__constructor` | constructor-only | `contracts/controller/src/governance/access.rs:63` |
| Controller | `accept_ownership` | direct live call | `contracts/controller/src/governance/access.rs:155` |
| Controller | `account_exists` | covered by helper/assertion | `contracts/controller/src/views/mod.rs:80` |
| Controller | `add_asset_to_spoke` | direct live call | `contracts/controller/src/config/mod.rs:82` |
| Controller | `add_delegate` | direct live call | `contracts/controller/src/pool_ops/mod.rs:50` |
| Controller | `add_rewards` | direct live call | `contracts/controller/src/pool_ops/mod.rs:127` |
| Controller | `add_spoke` | direct live call | `contracts/controller/src/config/mod.rs:70` |
| Controller | `approve_blend_pool` | direct live call | `contracts/controller/src/config/mod.rs:114` |
| Controller | `approve_token` | direct live call | `contracts/controller/src/config/mod.rs:100` |
| Controller | `borrow` | direct live call | `contracts/controller/src/positions/borrow.rs:30` |
| Controller | `claim_revenue` | direct live call | `contracts/controller/src/pool_ops/mod.rs:120` |
| Controller | `clean_bad_debt` | direct live call | `contracts/controller/src/positions/liquidation/mod.rs:39` |
| Controller | `create_hub` | direct live call | `contracts/controller/src/config/mod.rs:64` |
| Controller | `create_liquidity_pool` | direct live call | `contracts/controller/src/pool_ops/mod.rs:89` |
| Controller | `deploy_pool` | direct live call | `contracts/controller/src/pool_ops/mod.rs:68` |
| Controller | `disable_token_oracle` | direct live call | `contracts/controller/src/config/mod.rs:138` |
| Controller | `edit_asset_in_spoke` | direct live call | `contracts/controller/src/config/mod.rs:88` |
| Controller | `flash_loan` | direct live call | `contracts/controller/src/strategies/flash_loan.rs:20` |
| Controller | `get_account_attributes` | direct live call | `contracts/controller/src/views/mod.rs:75` |
| Controller | `get_account_positions` | direct live call | `contracts/controller/src/views/mod.rs:65` |
| Controller | `get_app_version` | direct live call | `contracts/controller/src/governance/access.rs:121` |
| Controller | `get_borrow_amount` | direct live call | `contracts/controller/src/views/mod.rs:61` |
| Controller | `get_collateral_amount` | direct live call | `contracts/controller/src/views/mod.rs:57` |
| Controller | `get_health_factor` | direct live call | `contracts/controller/src/views/mod.rs:45` |
| Controller | `get_liquidation_collateral` | direct live call | `contracts/controller/src/views/mod.rs:120` |
| Controller | `get_liquidation_estimate` | direct live call | `contracts/controller/src/views/mod.rs:112` |
| Controller | `get_ltv_collateral_usd` | direct live call | `contracts/controller/src/views/mod.rs:124` |
| Controller | `get_market_index` | direct live call | `contracts/controller/src/views/mod.rs:146` |
| Controller | `get_market_indexes_detailed` | direct live call | `contracts/controller/src/views/mod.rs:105` |
| Controller | `get_markets_detailed` | direct live call | `contracts/controller/src/views/mod.rs:98` |
| Controller | `get_min_borrow_collateral_usd` | covered by helper/assertion | `contracts/controller/src/config/mod.rs:59` |
| Controller | `get_pool_address` | direct live call | `contracts/controller/src/views/mod.rs:94` |
| Controller | `get_spoke` | direct live call | `contracts/controller/src/views/mod.rs:89` |
| Controller | `get_spoke_asset` | direct live call | `contracts/controller/src/views/mod.rs:84` |
| Controller | `get_total_borrow_usd` | direct live call | `contracts/controller/src/views/mod.rs:53` |
| Controller | `get_total_collateral_usd` | direct live call | `contracts/controller/src/views/mod.rs:49` |
| Controller | `is_blend_pool_approved` | direct live call | `contracts/controller/src/config/mod.rs:109` |
| Controller | `is_liquidatable` | covered by helper/assertion | `contracts/controller/src/views/mod.rs:41` |
| Controller | `liquidate` | direct live call | `contracts/controller/src/positions/liquidation/mod.rs:30` |
| Controller | `max_borrow` | direct live call | `contracts/controller/src/views/mod.rs:141` |
| Controller | `max_supply` | direct live call | `contracts/controller/src/views/mod.rs:134` |
| Controller | `max_withdraw` | direct live call | `contracts/controller/src/views/mod.rs:129` |
| Controller | `migrate` | direct live call | `contracts/controller/src/governance/access.rs:104` |
| Controller | `migrate_from_blend` | direct live call | `contracts/controller/src/strategies/migrate_blend.rs:49` |
| Controller | `multiply` | direct live call | `contracts/controller/src/strategies/multiply.rs:37` |
| Controller | `pause` | direct live call | `contracts/controller/src/governance/access.rs:129` |
| Controller | `remove_asset_from_spoke` | direct live call | `contracts/controller/src/config/mod.rs:94` |
| Controller | `remove_delegate` | direct live call | `contracts/controller/src/pool_ops/mod.rs:58` |
| Controller | `remove_spoke` | direct live call | `contracts/controller/src/config/mod.rs:76` |
| Controller | `renew_account` | direct live call | `contracts/controller/src/pool_ops/mod.rs:43` |
| Controller | `repay` | direct live call | `contracts/controller/src/positions/repay.rs:34` |
| Controller | `repay_debt_with_collateral` | direct live call | `contracts/controller/src/strategies/repay_debt_with_collateral.rs:32` |
| Controller | `revoke_blend_pool` | direct live call | `contracts/controller/src/config/mod.rs:120` |
| Controller | `revoke_token` | direct live call | `contracts/controller/src/config/mod.rs:105` |
| Controller | `set_accumulator` | direct live call | `contracts/controller/src/config/mod.rs:36` |
| Controller | `set_aggregator` | direct live call | `contracts/controller/src/config/mod.rs:30` |
| Controller | `set_liquidity_pool_template` | direct live call | `contracts/controller/src/config/mod.rs:42` |
| Controller | `set_market_oracle_config` | direct live call | `contracts/controller/src/config/mod.rs:126` |
| Controller | `set_min_borrow_collateral_usd` | direct live call | `contracts/controller/src/config/mod.rs:54` |
| Controller | `set_oracle_tolerance` | direct live call | `contracts/controller/src/config/mod.rs:132` |
| Controller | `set_position_limits` | direct live call | `contracts/controller/src/config/mod.rs:48` |
| Controller | `set_position_manager` | direct live call | `contracts/controller/src/config/mod.rs:144` |
| Controller | `supply` | direct live call | `contracts/controller/src/positions/supply.rs:29` |
| Controller | `swap_collateral` | direct live call | `contracts/controller/src/strategies/swap_collateral.rs:34` |
| Controller | `swap_debt` | direct live call | `contracts/controller/src/strategies/swap_debt.rs:31` |
| Controller | `transfer_ownership` | direct live call | `contracts/controller/src/governance/access.rs:141` |
| Controller | `unpause` | direct live call | `contracts/controller/src/governance/access.rs:135` |
| Controller | `update_account_threshold` | direct live call | `contracts/controller/src/pool_ops/mod.rs:137` |
| Controller | `update_indexes` | direct live call | `contracts/controller/src/pool_ops/mod.rs:35` |
| Controller | `update_pool_caps` | direct live call | `contracts/controller/src/pool_ops/mod.rs:108` |
| Controller | `upgrade` | direct live call | `contracts/controller/src/governance/access.rs:96` |
| Controller | `upgrade_liquidity_pool_params` | direct live call | `contracts/controller/src/pool_ops/mod.rs:99` |
| Controller | `upgrade_pool` | direct live call | `contracts/controller/src/pool_ops/mod.rs:113` |
| Controller | `withdraw` | direct live call | `contracts/controller/src/positions/withdraw.rs:50` |
| Governance | `__constructor` | constructor-only | `contracts/governance/src/access.rs:151` |
| Governance | `accept_ownership` | direct live call | `contracts/governance/src/access.rs:163` |
| Governance | `cancel` | direct live call | `contracts/governance/src/timelock.rs:202` |
| Governance | `controller` | direct live call | `contracts/governance/src/deploy.rs:48` |
| Governance | `deploy_controller` | direct live call | `contracts/governance/src/deploy.rs:21` |
| Governance | `execute` | direct live call | `contracts/governance/src/timelock.rs:152` |
| Governance | `execute_immediate` | direct live call | `contracts/governance/src/timelock.rs:281` |
| Governance | `execute_self` | direct live call | `contracts/governance/src/timelock.rs:182` |
| Governance | `get_min_delay` | direct live call | `contracts/governance/src/timelock.rs:224` |
| Governance | `get_operation_ledger` | direct live call | `contracts/governance/src/timelock.rs:234` |
| Governance | `get_operation_state` | direct live call | `contracts/governance/src/timelock.rs:229` |
| Governance | `has_role` | direct live call | `contracts/governance/src/access.rs:171` |
| Governance | `hash_operation` | direct live call | `contracts/governance/src/timelock.rs:239` |
| Governance | `pause` | direct live call | `contracts/governance/src/timelock.rs:211` |
| Governance | `propose` | direct live call | `contracts/governance/src/timelock.rs:131` |
| Governance | `resolve_market_oracle_config` | direct live call | `contracts/governance/src/timelock.rs:260` |
| Governance | `resolve_oracle_tolerance` | direct live call | `contracts/governance/src/timelock.rs:270` |
| Governance | `set_controller` | direct live call | `contracts/governance/src/deploy.rs:56` |
| Governance | `unpause` | direct live call | `contracts/governance/src/timelock.rs:218` |

## Unresolved Exclusions

- `__constructor`: deployment only, not called as post-deploy endpoint.
- `execute_immediate` and `set_controller`: testing-only governance entrypoints; production integration asserts absence instead of live positive calls.
- `migrate_from_blend`: default run records environment-blocked unless `BLEND_MIGRATION_LIVE=1` and real Blend position assets are supplied.
