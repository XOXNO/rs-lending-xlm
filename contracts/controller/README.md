# Controller

The only user-facing lending contract. It holds account state, applies risk
rules and drives the pool that holds the tokens. Users never call the pool
directly. The swap aggregator stays callable on its own for plain swaps.

| Calls | Why |
| --- | --- |
| pool | Token custody and per-market share accounting. The controller owns it. See [`../pool/README.md`](../pool/README.md) |
| price-aggregator | USD prices. A failed price read reverts every state-changing call; `get_market_indexes_detailed` reports it as `valid: false` instead |
| swap-aggregator | Swap routes for the strategy entrypoints. Untrusted |
| position-nft | Account ownership. `account_id` equals the NFT `token_id`. See [`../position-nft/README.md`](../position-nft/README.md) |

| | |
| --- | --- |
| Owner | Governance, after deployment |
| Client | [`interfaces/controller`](../../interfaces/controller) |

Full signatures are in `contracts/controller/src/lib.rs`; the generated client
drops the `Env` argument.

## Authorization

**Owner** (`#[only_owner]`). After deployment the owner is the governance
contract. Most owner calls go through the governance timelock. A GUARDIAN can
call `pause`, `set_spoke_asset_flags`, `create_hub` and `add_spoke` through
governance with no delay
([`timelock/immediate.rs`](../governance/src/timelock/immediate.rs)).

**Global pause** (`#[when_not_paused]`). Every user entrypoint stops during a
pause except these, which stay open so users can exit and the protocol can
clean up: `withdraw`, `repay`, `liquidate`, `clean_bad_debt`, `recapitalize`,
`renew_account`, `remove_delegate`.

**Caller.** Other state-changing entrypoints require auth from their `caller`,
`liquidator` or `payer` argument. In addition:

- `borrow`, `withdraw` and every strategy entrypoint except `flash_loan`
  require the account owner or an active delegate the owner added.
- `supply` by a third party may only top up hub assets the account already
  supplies. `account_id` 0 creates a new account owned by the caller.
- Permissionless on any account: `repay`; `liquidate` when health factor < 1
  (the owner too); `clean_bad_debt` when the account is insolvent and its
  collateral is at or below the dust threshold.

## Halt flags

Each spoke asset has three independent flags:

| Flag | Blocks |
| --- | --- |
| `paused` | Entries and exits for that spoke asset |
| `frozen` | New entries; exits stay open |
| `no_seize` | The liquidation seizure leg. The only flag that stops a seizure |

`set_spoke_asset_flags` and `edit_asset_in_spoke` can only keep or tighten a
flag; clearing one reverts with `SpokeAssetFlagRelaxation`.
`remove_asset_from_spoke` retains any set flag, and `add_asset_to_spoke` for
that asset and spoke reverts the same way if it clears one. To clear, use the
owner-only, timelocked `relax_spoke_asset_flags`, which reverts unless
`expected_epoch` equals the listing's flags epoch
(`get_spoke_asset_flags_epoch`). See [`../governance/README.md`](../governance/README.md).

## Entrypoints

Amount lists are `Vec<(HubAssetKey, i128)>`. An `account_id` of 0 creates a
new account where noted; those entrypoints return the account id.

### Positions

| Entrypoint | Does |
| --- | --- |
| `supply(caller, account_id, spoke_id, assets) -> u64` | Supplies collateral in spoke `spoke_id`. Account 0 creates one |
| `borrow(caller, account_id, borrows, to)` | Borrows against the account's collateral; sends to `to` or the caller. Reverts if the position breaks solvency limits |
| `withdraw(caller, account_id, withdrawals, to) -> amounts` | Withdraws collateral to `to` or the caller. **Amount 0 withdraws the whole position.** Returns the amounts withdrawn |
| `repay(caller, account_id, payments)` | Pulls funds from the caller, repays debt, refunds the excess |
| `liquidate(liquidator, account_id, debt_payments, seize_mode) -> u64` | Repays debt and seizes collateral at a bonus set by the health factor. Returns the `Credit` receiver's account id, or 0 for `Transfer` |
| `clean_bad_debt(caller, account_id)` | For an insolvent account with dust collateral: socializes the debt into the supply index and removes the account |

### Strategies and flash loans

All of these stop during a global pause.

| Entrypoint | Does |
| --- | --- |
| `flash_loan(caller, asset, amount, receiver, data)` | Lends `amount` to `receiver`, calls its callback with `data`; the pool pulls back principal + fee before return |
| `flash_position(caller, account_id, spoke_id, mode, debt, amount, receiver, data, collaterals, refund_assets) -> u64` | Mints `debt` onto the account with no fee, forwards the measured tokens to `receiver`, calls `execute_flash_position`, then deposits the measured balance increase of `collaterals`. The account must be solvent after. Account 0 creates one |
| `multiply(caller, account_id, spoke_id, collateral, debt_to_flash_loan, debt, mode, swap, initial_payment, convert_swap) -> u64` | Opens or grows a leveraged position: borrows `debt`, swaps to `collateral`, deposits. An `initial_payment` in `collateral` joins the deposit, in `debt` joins the swap, in a third asset needs `convert_swap`. Account 0 creates one |
| `swap_debt(caller, account_id, existing_debt, amount, new_debt, swap)` | Borrows `new_debt`, swaps to `existing_debt`, repays it |
| `swap_collateral(caller, account_id, current, amount, new, swap)` | Withdraws `current`, swaps to `new`, deposits it |
| `repay_debt_with_collateral(caller, account_id, collateral, collateral_amount, debt, swap, close_position)` | Repays `debt` with collateral: nets directly for the same asset, swaps otherwise |
| `migrate_from_blend(caller, account_id, spoke_id, hub_id, blend_pool, collateral_assets, supply_assets, debt_caps) -> u64` | Moves a position from an approved Blend pool: borrows each `debt_caps` amount, repays Blend, repays the unused borrow, then sweeps the Blend collateral and supply into the pool. Account 0 creates one |

### Account management

| Entrypoint | Does |
| --- | --- |
| `renew_account(caller, account_id)` | Extends the account's storage TTL. Owner only |
| `add_delegate(caller, account_id, delegate)` | Lets `delegate` act for the owner. `delegate` must be an active, governance-approved position manager |
| `remove_delegate(caller, account_id, delegate)` | Revokes a delegate. Owner only |
| `update_account_threshold(caller, has_risks, account_ids)` | Refreshes the LTV of each supply position. With `has_risks`, also refreshes liquidation parameters and requires a final health factor of at least 1.05 |

### Maintenance

| Entrypoint | Does |
| --- | --- |
| `update_indexes(caller, assets)` | Accrues pool indexes for each hub asset |
| `claim_revenue(caller, assets) -> Vec<i128>` | Claims protocol revenue from the pool to the accumulator; returns the amount per asset |
| `recapitalize(payer, hub_asset, amount) -> i128` | Pays into the pool up to the backing shortfall, refunds the excess, returns the amount applied |

### Views

Values in USD are WAD; indexes are RAY; asset amounts use the asset's
decimals.

| View | Returns |
| --- | --- |
| `get_health_factor(account_id)` | Liquidation-weighted collateral / debt; `i128::MAX` with no debt or no account |
| `is_liquidatable(account_id)` | Health factor < 1 |
| `get_total_collateral_usd`, `get_total_borrow_usd` | Total supplied / borrowed value |
| `get_liquidation_collateral` | Threshold-weighted collateral (the health-factor numerator); 0 for no account |
| `get_ltv_collateral_usd` | LTV-weighted collateral (the borrow ceiling) |
| `get_collateral_amount`, `get_borrow_amount` `(account_id, hub_asset)` | Position amount, or 0 |
| `get_account_positions(account_id)` | Supply and debt position maps, keyed by hub asset |
| `get_account_attributes(account_id)` | Spoke id and position mode |
| `account_exists(account_id)` | Whether the account exists |
| `get_liquidation_estimate(account_id, debt_payments, seize_mode)` | Simulated liquidation, no state change |
| `get_market_index(hub_asset)` | Current supply and borrow index |
| `get_market_indexes_detailed(hub_assets)` | Indexes plus oracle price status; reverts above `MAX_VIEW_INPUTS` |
| `get_spoke(spoke_id)` | Spoke configuration |
| `get_spoke_asset(spoke_id, hub_asset)` | Asset configuration in the spoke; reverts with `AssetNotInSpoke` if unlisted |
| `get_spoke_usage(spoke_id, hub_asset)` | Supplied and borrowed usage, or zero |
| `get_spoke_asset_flags_epoch(spoke_id, hub_asset)` | The `expected_epoch` for `relax_spoke_asset_flags`; 0 if no flag was written |
| `get_min_borrow_collateral_usd` | Minimum LTV-weighted collateral an account with debt must keep after a borrow, withdrawal or strategy call; 0 disables it |
| `get_pool_address`, `price_aggregator` | Contract addresses |
| `is_blend_pool_approved(pool)` | Whether `pool` is an approved migration source |
| `get_app_version` | Stored app version |

### Administration

Owner only, except `accept_ownership` (pending owner).

| Entrypoint | Does |
| --- | --- |
| `__constructor(admin)` | Sets the owner, both position limits to `POSITION_LIMIT_MAX`, the borrow collateral floor to `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`, the version to `INITIAL_APP_VERSION`. **Starts paused** |
| `set_swap_aggregator(addr)`, `set_price_aggregator(addr)`, `set_accumulator(addr)` | Set the swap router, the price source and the revenue receiver |
| `set_position_limits(limits)` | Max supply and borrow positions per account, each in `1..=POSITION_LIMIT_MAX` |
| `set_min_borrow_collateral_usd(floor_wad)` | Minimum LTV-weighted collateral for an account with debt; 0 disables |
| `set_position_manager(manager, is_active)` | Allow or disallow `manager` as a delegate |
| `approve_blend_pool(pool)`, `revoke_blend_pool(pool)` | Manage Blend migration sources |
| `create_hub() -> u32` | New active hub |
| `add_spoke() -> u32` | New spoke with the default liquidation curve |
| `remove_spoke(id)` | Deprecates a spoke; blocks new positions |
| `set_spoke_liquidation_curve(id, target_hf_wad, hf_for_max_bonus_wad, liquidation_bonus_factor_bps)` | Target health factor, health factor at max bonus, bonus factor |
| `add_asset_to_spoke(input)`, `edit_asset_in_spoke(input)` | List or update a spoke asset's risk parameters and caps, validated against the pool decimals |
| `set_spoke_asset_flags(spoke_id, hub_asset, paused, frozen, no_seize)` | Tighten flags only |
| `relax_spoke_asset_flags(spoke_id, hub_asset, expected_epoch, paused, frozen, no_seize)` | Set any flag combination, clearing included, at the matching epoch |
| `remove_asset_from_spoke(hub_asset, spoke_id)` | Unlist an asset with no usage in the spoke |
| `deploy_pool(wasm_hash)`, `deploy_position_nft(wasm_hash, uri, name, symbol)` | Deploy and record the pool and the position NFT |
| `create_liquidity_pool(hub_id, asset, params)` | Create a market on the pool |
| `upgrade_liquidity_pool_params(hub_asset, params)` | Accrue, then replace the interest rate model |
| `upgrade_pool(new_wasm_hash)`, `upgrade_position_nft(new_wasm_hash)` | Upgrade the pool or the position NFT |
| `force_socialize_bad_debt(account_id)` | Socialize debt when debt > collateral, with no dust threshold |
| `pause()`, `unpause()` | Global pause |
| `upgrade(new_wasm_hash)` | **Pauses first**, then upgrades. Call `unpause` after |
| `migrate(new_version)` | Set the app version; must be higher than the current one |
| `transfer_ownership(new_owner, live_until_ledger)`, `accept_ownership()` | Two-step ownership transfer |

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events, fields and scales: [`docs/reference/events.md`](../../docs/reference/events.md)
- Formulas: [`docs/reference/formulas.md`](../../docs/reference/formulas.md)
- Integration model: [`skills/xoxno-lending/SKILL.md`](../../skills/xoxno-lending/SKILL.md)
- Formal proofs: [`certora/controller/spec`](../../certora/controller/spec/README.md)
