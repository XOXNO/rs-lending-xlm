# Controller

The controller is the user-facing contract of the lending protocol. It holds
account state, applies risk rules, and drives the pool that holds the money.
Users never call the pool directly.

It talks to four other contracts:

| Contract | Why |
| --- | --- |
| pool | Holds token custody and per-market share accounting. The controller owns it. |
| price-aggregator | Supplies USD prices. A failed price read reverts the call. |
| swap-aggregator | Executes swap routes for the strategy entrypoints. Treated as untrusted. |
| position-nft | Records account ownership. `account_id` equals the NFT `token_id`. See [`../position-nft/README.md`](../position-nft/README.md). |

## Authorization

There are three levels.

- **Owner.** Marked `#[only_owner]` in the source. After deployment the owner is
  the governance contract, so these calls run through a timelock.
- **Global pause.** Marked `#[when_not_paused]`. These calls revert while the
  contract is paused. Note that not every user entrypoint carries this
  attribute, so a global pause does not stop everything.
- **Caller.** Everything else authorizes `caller`. Most position entrypoints
  additionally require the caller to be the account owner or an opted-in
  delegate. `repay` is the exception: anyone may repay another account's debt.

## Halt flags

Each spoke asset carries three independent flags, set by
`set_spoke_asset_flags`.

| Flag | What it blocks |
| --- | --- |
| `paused` | Entries and exits for that spoke asset. |
| `frozen` | New entries; exits stay open. |
| `no_seize` | The liquidation seizure leg. This is the only flag that stops a seizure. |

`set_spoke_asset_flags` ratchets: an immediate GUARDIAN call may only tighten a
flag. Clearing a flag goes through the owner-only `edit_asset_in_spoke`, which
governance timelocks. See [`../governance/README.md`](../governance/README.md).

## Entrypoints

Signatures are copied from `contracts/controller/src/lib.rs`. The `Env` argument
is dropped by the generated client, so a client call takes one fewer argument
than the signature shows.

### Construction

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `__constructor` | `pub fn __constructor(env: Env, admin: Address)` | — | Initializes governance state: sets `admin` as the contract owner, sets supply/borrow position limits to their maximum and the minimum borrow collateral floor to its default, and pauses the contract until explicitly unpaused. |

### Positions

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `supply` | `fn supply( env: Env, caller: Address, account_id: u64, spoke_id: u32, assets: Vec<(HubAssetKey, i128)>, ) -> u64` | blocked by global pause | Supplies `assets` as collateral to `account_id` in spoke `spoke_id`, creating a new account when `account_id` is 0, and returns the account id. |
| `borrow` | `fn borrow( env: Env, caller: Address, account_id: u64, borrows: Vec<(HubAssetKey, i128)>, to: Option<Address>, )` | blocked by global pause | Borrows `borrows` against `account_id`'s collateral, sending the funds to `to` if provided or to the caller otherwise; reverts if the resulting position breaches the account's solvency limits. |
| `withdraw` | `fn withdraw( env: Env, caller: Address, account_id: u64, withdrawals: Vec<(HubAssetKey, i128)>, to: Option<Address>, ) -> Vec<(HubAssetKey, i128)>` | — | Withdraws `withdrawals` from `account_id`'s supplied collateral, sending the funds to `to` if provided or to the caller otherwise, and returns the amounts actually withdrawn; a zero amount for an asset withdraws the entire position. |
| `repay` | `fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>)` | — | Repays `payments` against `account_id`'s debt positions, pulling the funds from the caller. |
| `liquidate` | `fn liquidate( env: Env, liquidator: Address, account_id: u64, debt_payments: Vec<(HubAssetKey, i128)>, seize_mode: SeizeMode, ) -> u64` | — | Liquidates `account_id` by having `liquidator` repay `debt_payments` and seizing collateral at a bonus scaled by the account's health factor. |
| `clean_bad_debt` | `fn clean_bad_debt(env: Env, caller: Address, account_id: u64)` | — | Socializes `account_id`'s debt into the supply index and removes the account when it is insolvent and its remaining collateral value is at or below the dust threshold; reverts otherwise. |

### Strategies and flash loans

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `flash_loan` | `fn flash_loan( env: Env, caller: Address, asset: HubAssetKey, amount: i128, receiver: Address, data: Bytes, )` | blocked by global pause | Flash-loans `amount` of `asset` to `receiver`, invoking its callback with `data`; the pool pulls back the principal plus fee before the call returns. |
| `multiply` | `fn multiply( env: Env, caller: Address, account_id: u64, spoke_id: u32, collateral: HubAssetKey, debt_to_flash_loan: i128, debt: HubAssetKey, mode: PositionMode, swap: Bytes, initial_payment: Option<(HubAssetKey, i128)>, convert_swap: Option<Bytes>, ) -> u64` | blocked by global pause | Opens or extends a leveraged position on `account_id`: borrows `debt_to_flash_loan` of `debt`, swaps it (plus any optional initial payment) into `collateral` via `swap`, and deposits the result as collateral. |
| `swap_debt` | `fn swap_debt( env: Env, caller: Address, account_id: u64, existing_debt: HubAssetKey, amount: i128, new_debt: HubAssetKey, swap: Bytes, )` | blocked by global pause | Replaces `account_id`'s `existing_debt` position with `new_debt` by borrowing `amount` of `new_debt`, swapping it to `existing_debt` via `swap`, and repaying the existing position with the proceeds. |
| `swap_collateral` | `fn swap_collateral( env: Env, caller: Address, account_id: u64, current: HubAssetKey, amount: i128, new: HubAssetKey, swap: Bytes, )` | blocked by global pause | Replaces `amount` of `account_id`'s `current` collateral with `new` by withdrawing it, swapping to `new` via `swap`, and depositing the proceeds as collateral. |
| `repay_debt_with_collateral` | `fn repay_debt_with_collateral( env: Env, caller: Address, account_id: u64, collateral: HubAssetKey, collateral_amount: i128, debt: HubAssetKey, swap: Bytes, close_position: bool, )` | blocked by global pause | Repays `account_id`'s `debt` position using `collateral_amount` of `collateral`, netting them directly when the two assets match or swapping via `swap` otherwise. |
| `migrate_from_blend` | `fn migrate_from_blend( env: Env, caller: Address, account_id: u64, spoke_id: u32, hub_id: u32, blend_pool: Address, collateral_assets: Vec<Address>, supply_assets: Vec<Address>, debt_caps: Vec<(Address, i128)>, ) -> u64` | blocked by global pause | Migrates the caller's position from `blend_pool` (which must be pre-approved) into `account_id`: borrows up to `debt_caps` to repay the caller's debt on Blend, sweeps `collateral_assets` and `supply_assets` from Blend into this pool as collateral, and repays any leftover borrowed amount. |

### Account management

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `renew_account` | `fn renew_account(env: Env, caller: Address, account_id: u64)` | — | Extends `account_id`'s storage TTL; the caller must be the account owner. |
| `add_delegate` | `fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address)` | blocked by global pause | Grants `delegate` authority to act on behalf of `account_id`'s owner; `delegate` must already be an active, governance-approved position manager. |
| `remove_delegate` | `fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address)` | — | Revokes `delegate`'s authority over `account_id`; the caller must be the account owner. |
| `update_account_threshold` | `fn update_account_threshold(env: Env, caller: Address, has_risks: bool, account_ids: Vec<u64>)` | blocked by global pause | Refreshes cached risk parameters for the supply positions of each account in `account_ids`. |

### Maintenance

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `update_indexes` | `fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>)` | blocked by global pause | Accrues the borrow and supply indexes for each hub asset in `assets` on the pool. |
| `claim_revenue` | `fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128>` | blocked by global pause | Claims accrued protocol revenue for each hub asset in `assets` from the pool and forwards it to the configured accumulator, returning the amount claimed per asset. |
| `recapitalize` | `fn recapitalize(env: Env, payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128` | — | Transfers `amount` of `hub_asset` from `payer` into the pool to cover a backing shortfall, applying only up to the shortfall and refunding any excess; returns the amount actually applied. |

### Views (read-only)

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `is_liquidatable` | `fn is_liquidatable(env: Env, account_id: u64) -> bool` | — | Returns whether `account_id`'s health factor is below one (WAD-scaled), meaning it is eligible for liquidation. |
| `get_health_factor` | `fn get_health_factor(env: Env, account_id: u64) -> i128` | — | Returns `account_id`'s health factor as a WAD-scaled ratio of liquidation-weighted collateral to debt, or `i128::MAX` when the account has no debt or does not exist. |
| `get_total_collateral_usd` | `fn get_total_collateral_usd(env: Env, account_id: u64) -> i128` | — | Returns the USD value (WAD) of `account_id`'s total supplied collateral. |
| `get_total_borrow_usd` | `fn get_total_borrow_usd(env: Env, account_id: u64) -> i128` | — | Returns the USD value (WAD) of `account_id`'s total borrowed debt. |
| `get_collateral_amount` | `fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128` | — | Returns `account_id`'s supplied amount of `hub_asset` in the asset's own decimals, or zero if it holds no such position. |
| `get_borrow_amount` | `fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128` | — | Returns `account_id`'s borrowed amount of `hub_asset` in the asset's own decimals, or zero if it holds no such position. |
| `get_account_positions` | `fn get_account_positions( env: Env, account_id: u64, ) -> ( Map<HubAssetKey, AccountPositionRaw>, Map<HubAssetKey, DebtPositionRaw>, )` | — | Returns `account_id`'s supply and debt positions, keyed by hub asset. |
| `get_account_attributes` | `fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes` | — | Returns `account_id`'s spoke id and position mode. |
| `account_exists` | `fn account_exists(env: Env, account_id: u64) -> bool` | — | Returns whether `account_id` has been created. |
| `get_liquidation_estimate` | `fn get_liquidation_estimate( env: Env, account_id: u64, debt_payments: Vec<(HubAssetKey, i128)>, seize_mode: SeizeMode, ) -> LiquidationEstimate` | — | Simulates liquidating `account_id` with `debt_payments` under `seize_mode` without changing state. |
| `get_liquidation_collateral` | `fn get_liquidation_collateral(env: Env, account_id: u64) -> i128` | — | Returns `account_id`'s liquidation-threshold-weighted collateral value (WAD), the ceiling on collateral seizable during liquidation. |
| `get_ltv_collateral_usd` | `fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128` | — | Returns `account_id`'s LTV-weighted collateral value (WAD), the ceiling on its borrowing power. |
| `get_pool_address` | `fn get_pool_address(env: Env) -> Address` | — | Returns the address of the deployed liquidity pool contract. |
| `get_market_index` | `fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw` | — | Returns the current supply and borrow indexes (RAY) for `hub_asset`. |
| `get_market_indexes_detailed` | `fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>` | — | Returns the supply/borrow indexes and current oracle price status for each hub asset in `hub_assets`; reverts if more assets than the configured maximum are requested. |
| `get_spoke` | `fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig` | — | Returns the configuration of spoke `spoke_id`. |
| `get_spoke_asset` | `fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig` | — | Returns the configuration of `hub_asset` within spoke `spoke_id`; panics if the asset is not listed there. |
| `get_spoke_usage` | `fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw` | — | Returns the current supplied and borrowed usage of `hub_asset` within spoke `spoke_id`, or a zeroed value if none is recorded. |
| `price_aggregator` | `fn price_aggregator(env: Env) -> Address` | — | Returns the configured price aggregator contract address. |
| `get_min_borrow_collateral_usd` | `fn get_min_borrow_collateral_usd(env: Env) -> i128` | — | Returns the minimum collateral value (WAD) required to open a new borrow position. |
| `is_blend_pool_approved` | `fn is_blend_pool_approved(env: Env, pool: Address) -> bool` | — | Returns whether `pool` is approved as a Blend migration source. |
| `get_app_version` | `fn get_app_version(env: Env) -> u32` | — | Returns the contract's current app version. |
| `accept_ownership` | `fn accept_ownership(env: Env)` | — | Completes a pending ownership transfer, making the caller the new owner. |

### Administration (owner only)

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `set_swap_aggregator` | `fn set_swap_aggregator(env: Env, addr: Address)` | owner-only | Sets the swap aggregator contract address used by strategy swaps. |
| `set_price_aggregator` | `fn set_price_aggregator(env: Env, addr: Address)` | owner-only | Sets the price aggregator contract address used for oracle lookups. |
| `set_accumulator` | `fn set_accumulator(env: Env, addr: Address)` | owner-only | Sets the accumulator address that receives claimed protocol revenue. |
| `set_position_limits` | `fn set_position_limits(env: Env, limits: PositionLimits)` | owner-only | Sets the maximum number of concurrent supply and borrow positions an account may hold. |
| `set_min_borrow_collateral_usd` | `fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128)` | owner-only | Sets the minimum collateral value (WAD) required to open a new borrow position. |
| `set_position_manager` | `fn set_position_manager(env: Env, manager: Address, is_active: bool)` | owner-only | Activates or deactivates `manager` as a position manager eligible to be granted delegate access on accounts. |
| `approve_blend_pool` | `fn approve_blend_pool(env: Env, pool: Address)` | owner-only | Approves `pool` as a Blend migration source. |
| `revoke_blend_pool` | `fn revoke_blend_pool(env: Env, pool: Address)` | owner-only | Revokes `pool` as an approved Blend migration source. |
| `create_hub` | `fn create_hub(env: Env) -> u32` | owner-only | Creates a new active hub and returns its id. |
| `add_spoke` | `fn add_spoke(env: Env) -> u32` | owner-only | Creates a new spoke with the default liquidation curve and returns its id. |
| `remove_spoke` | `fn remove_spoke(env: Env, id: u32)` | owner-only | Marks spoke `id` as deprecated, blocking new positions in it. |
| `set_spoke_liquidation_curve` | `fn set_spoke_liquidation_curve( env: Env, id: u32, target_hf_wad: i128, hf_for_max_bonus_wad: i128, liquidation_bonus_factor_bps: u32, )` | owner-only | Sets spoke `id`'s liquidation curve: the target health factor, the health factor at which the bonus is maximal, and the bonus scaling factor (BPS). |
| `add_asset_to_spoke` | `fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs)` | owner-only | Lists a new asset in a spoke with its risk parameters and caps, validating them against the asset's pool decimals. |
| `edit_asset_in_spoke` | `fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs)` | owner-only | Updates an already-listed spoke asset's risk parameters and caps, revalidating them against the asset's pool decimals. |
| `set_spoke_asset_flags` | `fn set_spoke_asset_flags( env: Env, spoke_id: u32, hub_asset: HubAssetKey, paused: bool, frozen: bool, no_seize: bool, )` | owner-only | Sets the paused, frozen, and no-seize flags for `hub_asset` within spoke `spoke_id`. |
| `remove_asset_from_spoke` | `fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32)` | owner-only | Removes `hub_asset` from spoke `spoke_id`. |
| `deploy_pool` | `fn deploy_pool(env: Env, wasm_hash: BytesN<32>) -> Address` | owner-only | Deploys the liquidity pool contract from `wasm_hash` and records its address. |
| `deploy_position_nft` | `fn deploy_position_nft( env: Env, wasm_hash: BytesN<32>, uri: String, name: String, symbol: String, ) -> Address` | owner-only | Deploys the position-NFT contract that anchors account ownership. |
| `create_liquidity_pool` | `fn create_liquidity_pool( env: Env, hub_id: u32, asset: Address, params: MarketParamsRaw, ) -> Address` | owner-only | Creates a new market for `asset` under hub `hub_id` on the pool using `params`. |
| `upgrade_liquidity_pool_params` | `fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel)` | owner-only | Accrues `hub_asset`'s indexes, then updates its interest rate model to `params` on the pool. |
| `upgrade_pool` | `fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>)` | owner-only | Upgrades the liquidity pool contract to `new_wasm_hash`. |
| `upgrade_position_nft` | `fn upgrade_position_nft(env: Env, new_wasm_hash: BytesN<32>)` | owner-only | Upgrades the position-NFT contract's Wasm bytecode to `new_wasm_hash`. |
| `force_socialize_bad_debt` | `fn force_socialize_bad_debt(env: Env, account_id: u64)` | owner-only | Force-socializes `account_id`'s debt into the supply index when the account is insolvent. |
| `pause` | `fn pause(env: Env)` | owner-only | Pauses the contract. |
| `unpause` | `fn unpause(env: Env)` | owner-only | Unpauses the contract. |
| `upgrade` | `fn upgrade(env: Env, new_wasm_hash: BytesN<32>)` | owner-only | Pauses the contract if it is not already paused, then upgrades it to `new_wasm_hash`. |
| `migrate` | `fn migrate(env: Env, new_version: u32)` | owner-only | Sets the stored app version to `new_version`. |
| `transfer_ownership` | `fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32)` | owner-only | Begins a two-step transfer of ownership to `new_owner`. |

## Errors and events

Error codes are listed in [`../../docs/reference/errors.md`](../../docs/reference/errors.md).
Event topics, fields and their scales are listed in
[`../../docs/reference/events.md`](../../docs/reference/events.md).

## Further reading

- Shared model: [`../../skills/lending-protocol-fundamentals/SKILL.md`](../../skills/lending-protocol-fundamentals/SKILL.md)
- Protocol math: [`../../docs/reference/formulas.md`](../../docs/reference/formulas.md)
- Client ABI: [`../../interfaces/controller`](../../interfaces/controller)
