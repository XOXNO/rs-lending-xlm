# Endpoint reference

Interfaces and authorization for the nine contracts. See [architecture](architecture.md), [errors](errors.md), and [events](events.md).

Signatures use Rust ABI types, omit `Env`, and exclude test, mock, and Certora hooks. Named structs encode as maps, tuple structs as vectors. Amounts are raw token units. USD values and HF use WAD. Shares, indexes, and annual rates use RAY. Ratios use BPS ([units](formulas.md#units-and-arithmetic-domain)).

`HubAssetKey` is `{hub_id: u32, asset: Address}`. The same token in two hubs is two markets.

"Owner" is the contract owner. "NFT owner or delegate" is the position's current NFT owner or an active registered manager that the owner delegated. Open views need no signature but can renew storage TTL and call external contracts.

## Controller

Every user mutation needs the auth of the named caller, payer, or liquidator. In the Pause column, "gated" needs an unpaused controller and "open" works during global pause. Both enforce asset-level flags. Monetary calls fail inside a flash callback. Renewal and delegate writes are unguarded.

| Endpoint (parameters and return) | Additional authority | Pause | Effect |
| --- | --- | --- | --- |
| `supply(caller: Address, account_id: u64, spoke_id: u32, assets: Vec<(HubAssetKey, i128)>) -> u64` | Third parties can supply existing assets only | gated | Measured deposits. Id 0 creates a Normal account. A new leg on an indebted account reads its price. |
| `borrow(caller: Address, account_id: u64, borrows: Vec<(HubAssetKey, i128)>, to: Option<Address>)` | NFT owner or delegate | gated | Recipient defaults to caller. |
| `withdraw(caller: Address, account_id: u64, withdrawals: Vec<(HubAssetKey, i128)>, to: Option<Address>) -> Vec<(HubAssetKey, i128)>` | NFT owner or delegate | open | Amounts must be positive. `i128::MAX` withdraws the full position. Returns amounts paid. |
| `repay(caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>)` | None | open | Excess returns to caller. |
| `liquidate(liquidator: Address, account_id: u64, debt_payments: Vec<(HubAssetKey, i128)>, seize_mode: SeizeMode) -> u64` | None. Credit receiver owner or delegate | open | Pro-rata seizure. Returns 0 (Transfer) or receiver id (Credit). |
| `clean_bad_debt(caller: Address, account_id: u64)` | None | open | Needs debt > collateral and collateral <= $5. It [socializes](formulas.md#bad-debt) and burns the NFT. |
| `flash_loan(caller: Address, asset: HubAssetKey, amount: i128, receiver: Address, data: Bytes)` | None | gated | Wasm callback. Pool pulls exact principal plus fee. |
| `flash_position(caller: Address, account_id: u64, spoke_id: u32, mode: PositionMode, debt: HubAssetKey, amount: i128, receiver: Address, data: Bytes, collaterals: Vec<(HubAssetKey, i128)>, refund_assets: Vec<Address>) -> u64` | NFT owner or delegate for existing id | gated | Fee-free debt. Deposits declared collateral the callback delivers. |
| `multiply(caller: Address, account_id: u64, spoke_id: u32, collateral: HubAssetKey, debt_to_flash_loan: i128, debt: HubAssetKey, mode: PositionMode, swap: Bytes, initial_payment: Option<(HubAssetKey, i128)>, convert_swap: Option<Bytes>) -> u64` | NFT owner or delegate for existing id | gated | Borrow, swap, and supply. Optional initial capital. |
| `swap_debt(caller: Address, account_id: u64, existing_debt: HubAssetKey, amount: i128, new_debt: HubAssetKey, swap: Bytes)` | NFT owner or delegate | gated | Borrow new, repay existing with swap output. New borrow must fit its cap and the borrow-position limit before repayment. |
| `swap_collateral(caller: Address, account_id: u64, current: HubAssetKey, amount: i128, new: HubAssetKey, swap: Bytes)` | NFT owner or delegate | gated | Withdraw, convert, and redeposit. |
| `repay_debt_with_collateral(caller: Address, account_id: u64, collateral: HubAssetKey, collateral_amount: i128, debt: HubAssetKey, swap: Bytes, close_position: bool)` | NFT owner or delegate | gated | Same-market netting or swap. Optional full close. |
| `migrate_from_blend(caller: Address, account_id: u64, spoke_id: u32, hub_id: u32, blend_pool: Address, collateral_assets: Vec<Address>, supply_assets: Vec<Address>, debt_caps: Vec<(Address, i128)>) -> u64` | NFT owner or delegate for existing id | gated | From an approved Blend pool. |
| `update_indexes(caller: Address, assets: Vec<HubAssetKey>)` | None | gated | Accrue the markets. |
| `claim_revenue(caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128>` | None | gated | Pays only the accumulator. Returns controller receipts. |
| `update_account_threshold(caller: Address, has_risks: bool, account_ids: Vec<u64>)` | None | gated | Refresh LTV. Optional risk refresh must not drop HF below both 1.05 and its prior value. |
| `recapitalize(payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128` | None | open | Measured backing injection. Refunds surplus. Returns amount applied. |
| `renew_account(caller: Address, account_id: u64)` | NFT owner | open | Renew account and NFT ownership storage. |
| `add_delegate(caller: Address, account_id: u64, delegate: Address)` | NFT owner | gated | Active approved manager only. Max 16. Replaces a previous owner's dormant list. |
| `remove_delegate(caller: Address, account_id: u64, delegate: Address)` | NFT owner | open | From a new owner, deletes the previous owner's dormant list without an event. |

### Accounts and authorization

Id `0` creates an account on `supply`, `multiply`, `flash_position`, `migrate_from_blend`, and `Credit(0)` liquidation. Spoke binding is permanent. `multiply` and `flash_position` need Multiply, Long, or Short mode, matching an existing account. `supply` into an existing account checks only the spoke, so it accepts an account of any mode. Blend migration creates a Normal account. An existing destination need not be Normal.

Delegates belong to the granting owner. An NFT transfer disables them. A transfer back reactivates them unless a later owner replaced or deleted the list. Revoke grants before transferring. Ownership, with collateral control and the debt obligation, transfers atomically.

Payment lists sum duplicate markets in first-appearance order and reject negatives. `supply`, `borrow`, `repay`, and `withdraw` reject zero. In `withdraw`, an amount at or above the position, such as `i128::MAX`, withdraws all of it. Two such legs for one market overflow the sum and fail with `MathOverflow`. Flash-position declarations reject duplicates.

### Risk checks and pause flags

After pool accounting, `borrow`, `withdraw`, and the six account strategies require LTV-weighted collateral >= debt, HF >= 1, and, with debt left, the minimum-borrow floor. With debt left, each supply leg in an asset below `MIN_BORROWABLE_ASSET_DECIMALS` (3) must also hold at least `MIN_WHOLE_UNIT_COLLATERAL` (2) whole units, else `MinBorrowCollateralNotMet`. `supply` and `repay` skip these checks. Repayment loads debt positions only.

A supply leg below 3 decimals cannot sit beside another supply position. A new supply leg that would create such a pair fails with `PositionLimitExceeded`, whether it comes from `supply`, a strategy, Blend migration, or a `Credit` seizure. Liquidation has its own rules. See [formulas](formulas.md#valuation-and-health).

| Listing flag | Restriction |
| --- | --- |
| `paused` | Blocks ordinary entry and exit, including liquidation debt repayment |
| `frozen` | Blocks entry |
| `no_seize` | Blocks collateral seizure |

Seizure checks only `no_seize`. Liquidators cannot pick collateral. Global pause leaves withdrawal, repayment, liquidation, bad-debt cleanup, and recapitalization callable.

### Liquidation results

Credit liquidation moves supply shares, not tokens. The receiver must differ from the target, share its spoke, and be Normal. `Credit(0)` can create the receiver in a deprecated spoke. `Credit(0)` is exempt from position limits and an existing receiver is not, so a lowered limit cannot block seizure. Every receiver keeps the rule that a leg below 3 decimals cannot sit beside another supply position. Collateral permissions and supply caps do not gate the credit.

Estimates give gross seizure before fees. The unit is token units in Transfer and RAY shares in Credit. `refunds` use debt-token units. Prices, indexes, and measured delivery can move execution off the estimate. Same-call dust cleanup can write down a `Credit` receiver's shares ([INV-LIQ-02](invariants.md#inv-liq-02)).

### Strategy payments and callbacks

`multiply` adds a collateral initial payment to supply and a debt payment to the main swap. Other assets need `convert_swap`. Same-market `repay_debt_with_collateral` needs empty swap bytes. `close_position` fails if debt remains. Otherwise it withdraws all collateral to the caller.

`flash_position` needs a flash-loan-enabled debt market and a deployed Wasm receiver other than the controller or the pool. Collateral declarations must:

- be nonempty and at most the maximum supply-position count.
- have unique markets and underlying tokens.
- have nonnegative minimums, at least one positive.
- be met by measured controller receipts from the callback.

Before any token moves, each declared asset must be listed, not paused or frozen, and usable as collateral, and the account's position limits apply. The controller measures the transfer of each receipt into the pool again before the pool credits it. Caps are checked as each pool result merges. Supply and the declared debt position must stay open after finalization.

Refund assets must be unique, listed in the debt hub and account spoke, not declared as collateral, and at most the maximum supply-position count. They need an active spoke and existing listing. The collateralizable, borrowable, paused, and frozen flags are ignored. Only gains above pre-callback balances return to the caller. Refunding the debt token does not repay the minted debt.

Undeclared callback assets get neither credit nor refund. No sweep endpoint exists. Refunds emit only token transfer events.

### Revenue and recapitalization

`claim_revenue` forwards measured receipts. A taxed onward transfer can deliver less. Pool revenue is outstanding claims, not cumulative earnings. Recapitalization applies at most the [backing shortfall](formulas.md#backing-and-cash-constraints) and refunds the excess.

### Controller views

USD values are WAD.

| View | Result and semantics |
| --- | --- |
| `is_liquidatable(account_id: u64) -> bool` | HF < 1 WAD. `AccountNotFound` if missing |
| `get_health_factor(account_id: u64) -> i128` | WAD. i128::MAX if debt-free. `AccountNotFound` if missing |
| `get_total_collateral_usd(account_id: u64) -> i128` | Unweighted, half-up. The bad-debt `C` |
| `get_total_borrow_usd(account_id: u64) -> i128` | Display debt, half-up. Cleanup and HF use ceil risk debt |
| `get_collateral_amount(account_id: u64, hub_asset: HubAssetKey) -> i128` | Token units. Zero without position |
| `get_borrow_amount(account_id: u64, hub_asset: HubAssetKey) -> i128` | Token units. Zero without position |
| `get_account_positions(account_id: u64) -> ( Map<HubAssetKey, AccountPositionRaw>, Map<HubAssetKey, DebtPositionRaw>, )` | Supply and debt maps. Empty if missing |
| `get_account_attributes(account_id: u64) -> AccountAttributes` | Stored spoke and mode |
| `account_exists(account_id: u64) -> bool` | Metadata existence |
| `get_liquidation_estimate(account_id: u64, debt_payments: Vec<(HubAssetKey, i128)>, seize_mode: SeizeMode) -> LiquidationEstimate` | Same plan as execution. Max 256 payments. A `Credit(id)` receiver must pass execution's spoke, mode, position-limit, and isolation gates (no auth) |
| `get_liquidation_collateral(account_id: u64) -> i128` | Threshold-weighted. Not a seizure ceiling |
| `get_ltv_collateral_usd(account_id: u64) -> i128` | LTV-weighted |
| `get_pool_address() -> Address` | Configured pool |
| `get_market_index(hub_asset: HubAssetKey) -> MarketIndexRaw` | Simulated accrued RAY indexes |
| `get_market_indexes_detailed(hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>` | Accrued indexes and oracle status. Max 256 inputs |
| `get_spoke(spoke_id: u32) -> SpokeConfig` | Spoke config |
| `get_spoke_asset(spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig` | Risk config. Fails if missing |
| `get_spoke_usage(spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw` | RAY shares. Zero if absent |
| `get_spoke_asset_flags_epoch(spoke_id: u32, hub_asset: HubAssetKey) -> u64` | Listing flags epoch. Zero if never written |
| `price_aggregator() -> Address` | Configured price aggregator |
| `get_min_borrow_collateral_usd() -> i128` | LTV-weighted collateral floor |
| `is_blend_pool_approved(pool: Address) -> bool` | Migration allowlist |

### Controller administration

The constructor `(admin: Address)` starts the controller paused. Every endpoint in the next list needs the owner, normally governance, except `accept_ownership` (pending owner) and the open `get_app_version`. No other ownership methods are exported.

- `set_swap_aggregator(addr: Address)`
- `set_price_aggregator(addr: Address)`
- `set_accumulator(addr: Address)`
- `set_position_limits(limits: PositionLimits)`
- `set_min_borrow_collateral_usd(floor_wad: i128)`
- `set_position_manager(manager: Address, is_active: bool)`
- `approve_blend_pool(pool: Address)`
- `revoke_blend_pool(pool: Address)`
- `create_hub() -> u32`
- `add_spoke() -> u32`
- `remove_spoke(id: u32)`
- `set_spoke_liquidation_curve(id: u32, target_hf_wad: i128, hf_for_max_bonus_wad: i128, liquidation_bonus_factor_bps: u32)`
- `add_asset_to_spoke(input: SpokeAssetArgs)`
- `edit_asset_in_spoke(input: SpokeAssetArgs)`
- `set_spoke_asset_flags(spoke_id: u32, hub_asset: HubAssetKey, paused: bool, frozen: bool, no_seize: bool)`
- `relax_spoke_asset_flags(spoke_id: u32, hub_asset: HubAssetKey, expected_epoch: u64, paused: bool, frozen: bool, no_seize: bool)`
- `remove_asset_from_spoke(hub_asset: HubAssetKey, spoke_id: u32)`
- `deploy_pool(wasm_hash: BytesN<32>) -> Address`
- `deploy_position_nft(wasm_hash: BytesN<32>, uri: String, name: String, symbol: String) -> Address`
- `create_liquidity_pool(hub_id: u32, asset: Address, params: MarketParamsRaw) -> Address`
- `upgrade_liquidity_pool_params(hub_asset: HubAssetKey, params: InterestRateModel)`
- `upgrade_pool(new_wasm_hash: BytesN<32>)`
- `upgrade_position_nft(new_wasm_hash: BytesN<32>)`
- `force_socialize_bad_debt(account_id: u64)`
- `pause()`
- `unpause()`
- `upgrade(new_wasm_hash: BytesN<32>)`
- `migrate(new_version: u32)`
- `get_app_version() -> u32`
- `transfer_ownership(new_owner: Address, live_until_ledger: u32)`
- `accept_ownership()`

`force_socialize_bad_debt` requires debt greater than unweighted collateral and bypasses the $5 collateral cap. Follow the [force-socialize runbook](runbooks/force-socialize-bad-debt.md) before scheduling this irreversible operation.

`set_spoke_asset_flags` and `edit_asset_in_spoke` set but never clear flags. `set_spoke_asset_flags` rejects a no-op (`SpokeAssetFlagsUnchanged`). `relax_spoke_asset_flags` writes any combination only when `expected_epoch` matches. The epoch advances on listing creation, each successful `set_spoke_asset_flags` or `relax_spoke_asset_flags`, and a flag-changing edit. Removal keeps the epoch. Upgrade pauses before replacing code. Migration needs a strictly increasing version.

## Pool

The constructor `(admin: Address)` sets the owner, normally the controller. Every mutator requires the owner. No ownership transfer is exported.

The pool has these mutators:

- `create_market(hub_id: u32, params: MarketParamsRaw)`
- `update_params(hub_asset: HubAssetKey, model: InterestRateModel)`
- `update_indexes(hub_assets: Vec<HubAssetKey>)`
- `supply(entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>`
- `borrow(receiver: Address, entries: Vec<PoolBorrowEntry>) -> Vec<PoolPositionMutation>`
- `withdraw(receiver: Address, is_liquidation: bool, entries: Vec<PoolWithdrawEntry>) -> Vec<PoolPositionMutation>`
- `repay(payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>`
- `net_settle(entry: PoolNetSettleEntry) -> PoolNetSettleResult`
- `seize_positions(entries: Vec<PoolSeizeEntry>)`
- `flash_loan(hub_asset: HubAssetKey, initiator: Address, receiver: Address, amount: i128, data: Bytes) -> i128`
- `create_strategy(receiver: Address, action: PoolAction, charge_fee: bool) -> PoolStrategyMutation`
- `recapitalize(hub_asset: HubAssetKey, payer: Address, amount: i128) -> PoolAmountMutation`
- `claim_revenue(hub_asset: HubAssetKey) -> PoolAmountMutation`
- `upgrade(new_wasm_hash: BytesN<32>)`

Open views take `hub_asset: HubAssetKey` and read stored indexes without accruing, except `get_bulk_indexes`, which simulates accrual:

| Open view | Units / behavior |
| --- | --- |
| `get_utilisation(hub_asset) -> i128` | RAY ratio |
| `get_reserves(hub_asset) -> i128` | Booked cash, token units |
| `get_deposit_rate(hub_asset) -> i128` | Annual RAY APR at stored utilization |
| `get_borrow_rate(hub_asset) -> i128` | Annual RAY APR at stored utilization |
| `get_revenue(hub_asset) -> i128` | Outstanding revenue, token units |
| `get_supplied_amount(hub_asset) -> i128` | Token units |
| `get_borrowed_amount(hub_asset) -> i128` | Token units |
| `get_delta_time(hub_asset) -> u64` | Milliseconds since accrual |
| `get_sync_data(hub_asset) -> PoolSyncData` | Stored params and state |
| `get_bulk_indexes(hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw>` | Simulated accrued indexes, RAY |

## Governance

The constructor `(admin: Address, min_delay: u32)` sets the owner, the access-control admin, five operational roles, and a nonzero minimum delay. Scheduled operations are `AdminOperation` variants.

`execute` must match a scheduled hash and predecessor within its window. Anyone can execute a ready operation. The call checks no caller and no role.

The window closes 120,960 ledgers after the ready ledger. A later `execute` fails with `TimelockOperationExpired` (40). `get_operation_state` has no expired value. An expired operation still reads `Ready` and stays in storage. To detect expiry, compare `get_operation_ledger(id) + 120960` with the current ledger. A CANCELLER can cancel an expired operation, except a recovery operation.

| Endpoint | Authority and effect |
| --- | --- |
| `deploy_controller(wasm_hash: BytesN<32>) -> Address` | Owner. One-time deployment |
| `deploy_price_aggregator(wasm_hash: BytesN<32>) -> Address` | Owner. One-time deployment |
| `execute(target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>) -> Val` | Ready scheduled operation. Permissionless |
| `cancel(canceller: Address, operation_id: BytesN<32>)` | CANCELLER_ROLE. Recovery ops cannot be cancelled. Target cannot cancel own revocation |
| `propose(proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32>` | PROPOSER_ROLE. The proposer must also be the current owner for ownership transfers, code upgrades (`UpgradeGov`, `UpgradeController`, `UpgradePool`, `UpgradePositionNft`, `UpgradePriceAggregator`, `UpgradeCurator`, `MigrateController`), curator registrar control (`RegisterCuratorRegistrar`, `SuspendCuratorRegistrar`, `ReassignCuratorHub`, `ReassignCuratorSpoke`, `SetCuratorRegistrarAdmin`), the timelock minimum delay (`UpdateGovDelay`), price and swap sources (`SetPriceAggregator`, `ConfigureAssetOracle`, `EditOracleTolerance`, `SetSwapAggregator`), `ApproveBlendPool`, `SetAccumulator`, `GrantGovRole`, and position-manager activation (`SetPositionManager(_, true)`). `RevokeGovRole` cannot target the proposer or the owner |
| `pause(caller: Address)` | GUARDIAN_ROLE. Immediate |
| `set_spoke_asset_flags(caller: Address, spoke_id: u32, hub_asset: HubAssetKey, paused: bool, frozen: bool, no_seize: bool)` | GUARDIAN_ROLE or CURATOR_ROLE. Immediate tightening only |
| `set_sanity_band(caller: Address, key: PriceKey, min_wad: i128, max_wad: i128)` | ORACLE_ROLE. Immediate tightening only |
| `create_hub(caller: Address) -> u32` | CURATOR_ROLE. Immediate |
| `add_spoke(caller: Address) -> u32` | CURATOR_ROLE. Immediate |
| `create_liquidity_pool(caller: Address, hub_id: u32, params: MarketParamsRaw) -> Address` | CURATOR_ROLE. Immediate. Registers `params.asset_id` on the deployed pool |
| `suspend_curator_registrar(caller: Address, curator: Address, registrar: u32)` | GUARDIAN_ROLE. Immediate, suspension only. `curator` must hold CURATOR_ROLE |
| `deactivate_position_manager(caller: Address, manager: Address)` | GUARDIAN_ROLE. Immediate, deactivation only |
| `revoke_blend_pool(caller: Address, pool: Address)` | GUARDIAN_ROLE. Immediate, revocation only |
| `revoke_role_immediate(account: Address, role: Symbol)` | Owner. Only the guardian, oracle, and curator roles |
| `execute_self(op: AdminOperation, salt: BytesN<32>)` | Ready scheduled self-operation. Permissionless |
| `propose_canceller_reset(new_cancellers: Vec<Address>, salt: BytesN<32>) -> BytesN<32>` | Owner. Schedule uncancellable recovery. The list is not checked when scheduled |
| `execute_canceller_reset(new_cancellers: Vec<Address>, salt: BytesN<32>)` | Ready recovery. Permissionless. Revokes every CANCELLER except the owner, then grants the list. An empty list leaves the owner as the only canceller. Include the curator to keep its `cancel_proposal` path |
| `accept_ownership()` | Pending owner. Synchronizes access-control admin and roles |

The open views and resolvers are:

- `controller() -> Address`
- `price_aggregator() -> Address`
- `get_min_delay() -> u32`
- `get_operation_state(operation_id: BytesN<32>) -> OperationState`
- `get_operation_ledger(operation_id: BytesN<32>) -> u32`
- `hash_operation(target: Address, function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>) -> BytesN<32>`
- `resolve_oracle_tolerance(tolerance: u32) -> OracleTolerance`
- `resolve_asset_oracle(key: PriceKey, oracle: AssetOracle) -> AssetOracle`
- `has_role(account: Address, role: Symbol) -> bool`

No generic `grant_role`, `revoke_role`, `renounce_ownership`, `get_owner`, `schedule`, or `update_delay` is exported. Role, owner, delay, and upgrade changes use `AdminOperation`, except `revoke_role_immediate`, the canceller reset, and `accept_ownership`.

`AdminOperation::RelaxSpokeAssetFlags(RelaxSpokeAssetFlagsArgs)` schedules `relax_spoke_asset_flags` on the Standard tier. `propose` and execution both reject a stale `expected_epoch` with `SpokeFlagsEpochMismatch`.

## Position NFT

Account ids are token ids. They are `u32` here and `u64` in the controller. The constructor `(controller: Address, uri: String, name: String, symbol: String)` reserves id 0 and stores the controller and the metadata. NonFungibleToken and NonFungibleEnumerable defaults are exported.

| Endpoint | Authority / effect |
| --- | --- |
| `mint(to: Address) -> u32` | Controller. Sequential id. Renews owner and balance TTL |
| `burn(token_id: u32)` | Controller. Removes ownership and enumeration, emits inherited Burn, no holder auth |
| `renew(token_id: u32)` | Permissionless. Checks existence, renews owner, balance, and instance TTL |
| `upgrade(new_wasm_hash: BytesN<32>)` | Controller |
| `transfer(from: Address, to: Address, token_id: u32)` | from signature and token ownership |
| `transfer_from(spender: Address, from: Address, to: Address, token_id: u32)` | spender signature and approval |
| `approve(approver: Address, approved: Address, token_id: u32, live_until_ledger: u32)` | approver signature and ownership or operator authority |
| `approve_for_all(owner: Address, operator: Address, live_until_ledger: u32)` | owner signature |

The open views are `balance(account: Address) -> u32`, `owner_of(token_id: u32) -> Address`, `get_approved(token_id: u32) -> Option<Address>` (fails with `NonExistentToken` (200) for an id never minted or burned), `is_approved_for_all(owner: Address, operator: Address) -> bool`, `name() -> String`, `symbol() -> String`, `token_uri(token_id: u32) -> String`, `total_supply() -> u32`, `get_owner_token_id(owner: Address, index: u32) -> u32`, `get_token_id(index: u32) -> u32`.

`token_uri` needs an existing token and returns base URI + id + `?isStatic=true&chain=STELLAR`. No metadata setter, ownership setter, or Burnable burn_from is exported. A nonzero approval expiry must lie from the current ledger to the maximum live-until ledger. `0` revokes.

## Swap aggregator

The constructor `(admin: Address)` sets the owner. The XDR payload of `execute_strategy` is a validated, packed stream of swaps and Aquarius LP burns and mints. A Curve leg mints LP when the pool is its output. A Curve leg burns LP to one coin when the pool is its input. The router applies [fees](formulas.md#swap-router-fees-and-splits), then checks output against the route minimum. It opens a `G...` sender's missing output trustline when the output is a SAC. It then transfers, publishes `SwapEvent`, and returns the payout.

Recipient receipt is not remeasured, so taxed tokens can deliver less than returned. Controller strategies measure positive receipts but do not enforce the route minimum. Fees need an active nonzero referral. The whitelist picks the fee side, not allowed tokens. Lending governance cannot upgrade the router. The router owner does.

- `execute_strategy(sender: Address, total_in: i128, swap_xdr: Bytes) -> i128`: sender signature.
- `claim_referral_fees(id: u64, tokens: Vec<Address>)`: permissionless. Pays the fixed referral owner.
- Owner: `set_static_fee(fee_bps: u32)`, `add_to_whitelist(token: Address)`, `remove_from_whitelist(token: Address)`, `upgrade(new_wasm_hash: BytesN<32>)`, `add_referral(owner: Address, fee_bps: u32, protocol_fee_bps: Option<u32>) -> u64`, `set_referral_fee(id: u64, fee_bps: u32, protocol_fee_bps: Option<u32>)`, `set_referral_active(id: u64, active: bool)`, `set_referral_owner(id: u64, new_owner: Address)`, `claim_admin_fees(recipient: Address, tokens: Vec<Address>)`, `sweep_balance(recipient: Address, tokens: Vec<Address>)`. `sweep_balance` excludes reserved admin and referral fee balances.
- Open views: `admin() -> Address`, `static_fee_bps() -> u32`, `referral(id: u64) -> Option<ReferralConfig>`, `referral_counter() -> u64`, `is_whitelisted(token: Address) -> bool`, `whitelisted_tokens() -> Vec<Address>`, `admin_fee_balance(token: Address) -> i128`, `referral_fee_balance(id: u64, token: Address) -> i128`.

### Router and XOXNO oracle ownership

Both export `get_owner() -> Option<Address>` (open), `transfer_ownership(new_owner: Address, live_until_ledger: u32)` (owner), and `accept_ownership()` (pending owner). They do not export `renounce_ownership`, so ownership cannot be cleared. A zero deadline cancels the pending transfer.

## Price aggregator

The constructor `(owner: Address)` sets the owner and emits OwnershipTransferCompleted. No ownership-transfer ABI is exported.

| Endpoint | Authority and semantics |
| --- | --- |
| `get_owner() -> Option<Address>` | Open view |
| `prices(keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw>` | Open. Fails on an unusable price |
| `quotes(keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus>` | Open. Resolution failures return as status. Host, storage, budget, and cross-contract traps still abort |
| `price_spread(key: PriceKey) -> (i128, i128)` | Open. Fails on an unusable price |
| `oracle(key: PriceKey) -> Option<AssetOracle>` | Open view |
| `set_oracle(key: PriceKey, oracle: AssetOracle)` | Owner. A replacement must carry the stored `asset_decimals`, else `InvalidOracleDecimals` |
| `set_sanity_band(key: PriceKey, min_wad: i128, max_wad: i128)` | Owner. Tightens only |
| `set_tolerance(key: PriceKey, tolerance: OracleTolerance)` | Owner |
| `upgrade(new_wasm_hash: BytesN<32>)` | Owner |

## XOXNO oracle

Constructor `(admin: Address, signers: Vec<Address>, threshold: u32, resolution: u32) -> Result<(), Error>` sets the owner and quorum. Each submission authenticates one registered signer. A quorum of fresh stored submissions within the timestamp cluster yields the lower-median aggregate. A cluster under `2 * (signers - threshold) + 1` entries must also satisfy `max * 10000 <= min * (10000 + max_cluster_spread_bps)`, or the quorum check fails. A successful submission need not aggregate.

Prices have 8 decimals. Package and aggregate-write timestamps use milliseconds. Freshness, resolution, and Reflector timestamps use seconds.

In the next list, every endpoint except `upgrade` returns `Result<_, Error>`. The types shown are `Ok` values.

- Owner: `add_signer(signer: Address)`, `remove_signer(signer: Address)`, `set_threshold(threshold: u32)`, `set_max_stale_seconds(seconds: u64)`, `set_max_submission_age_seconds(seconds: u64)`, `set_max_relative_skew_seconds(seconds: u64)`, `set_max_cluster_spread_bps(bps: u32)` (`1 <= bps <= 10000`), `recompute_feeds(feed_ids: Vec<String>)`, `register_feed(feed_id: String)`, `add_feed(feed_id: String, asset: ReflectorAsset)`, `remove_feed(asset: ReflectorAsset)`, `set_resolution(resolution: u32)`, `purge_feed(feed_id: String)`, and `upgrade(new_wasm_hash: BytesN<32>)`.
- Registered signer signature: `submit_price(signer: Address, feed_id: String, price: i128, package_timestamp: u64)`, `submit_prices(signer: Address, feed_ids: Vec<String>, prices: Vec<i128>, package_timestamp: u64)`.
- Open: `read_price_data_for_feed(feed_id: String) -> RedStonePriceData`, `read_price_data(feed_ids: Vec<String>) -> Vec<RedStonePriceData>`, `read_price_history(feed_id: String, limit: u32) -> Vec<RedStonePriceData>`.

The open Reflector-compatible and configuration reads return plain values, not `Result`: `max_submission_age_seconds() -> u64`, `max_stale_seconds() -> u64`, `max_relative_skew_seconds() -> u64`, `max_cluster_spread_bps() -> u32` (200 when unset), `base() -> ReflectorAsset`, `decimals() -> u32`, `resolution() -> u32`, `assets() -> Vec<ReflectorAsset>`, `feeds() -> Vec<String>`, `lastprice(asset: ReflectorAsset) -> Option<ReflectorPriceData>`, `price(asset: ReflectorAsset, timestamp: u64) -> Option<ReflectorPriceData>`, `prices(asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>`.

The [ownership methods](#router-and-xoxno-oracle-ownership) are also exported. The Ownable owner administers the oracle, and lending governance has no XOXNO variants.

The history keeps one entry per `resolution`-second bucket of the write time. Each entry is the newest aggregate of that bucket. The history holds at most 12 entries. `prices` and `price` serve each entry as a round stamped with its bucket start, in seconds. Round timestamps are distinct and at least one resolution apart. `price(asset, timestamp)` serves the newest round written at or before `timestamp`.

Threshold, submission-age, relative-skew, and cluster-spread setters do not recompute. The owner calls `recompute_feeds` in bounded batches. `remove_signer` deletes the signer's submissions and recomputes affected feeds. Without quorum, submissions keep the previous aggregate. Recomputation or signer removal clears the previous aggregate. Reads check aggregate freshness without reevaluating quorum. Package timestamps allow 60 seconds of future skew after flooring to seconds. Equal timestamps from one signer are accepted.

## DeFindex strategy

The constructor takes `asset: Address` and `init_args: Vec<Val> = [controller: Address, hub_id: u32, spoke_id: u32]`, resolves the pool, and checks the market index. Each vault has one controller account. The contract has no upgrade or admin endpoint. Every endpoint returns `Result<_, DeFindexStrategyError>`.

| Endpoint (`Ok` type) | Authority and behavior |
| --- | --- |
| `asset() -> Address` | Open. Configured token |
| `deposit(amount: i128, from: Address) -> i128` | from signature. Positive measured supply. Returns collateral balance |
| `harvest(from: Address, data: Option<Bytes>) -> ()` | from signature. Emits price-per-share. No funds move |
| `balance(from: Address) -> i128` | Open. Vault collateral balance, zero without a live account |
| `withdraw(amount: i128, from: Address, to: Address) -> i128` | from signature. 0 < amount <= balance, paid to `to`. Returns remaining balance |

## Curator

The constructor `(governance: Address)` stores the owning governance. The curator holds governance `CURATOR`, `PROPOSER`, and `CANCELLER`. Registrar calls reach the controller through these roles. The [curator README](../../contracts/curator/README.md) covers cap grants and the control table.

Registrar calls need `caller` auth as the registrar admin, who holds every bit, or as a holder of a named role bit. An unknown registrar fails with `UnknownRegistrar`. A suspended registrar fails with `RegistrarSuspended`.

| Role bit | Value | Grants |
| --- | --- | --- |
| `ROLE_MARKET` | 1 | Markets, cap grants, listing and rate proposals, cancellation |
| `ROLE_SPOKE` | 2 | Hub and spoke creation |
| `ROLE_FLAGS` | 4 | Flag tightening, flag-relaxation proposals, cancellation |

Registrar endpoints act immediately unless noted:

| Registrar endpoint | Authority | Effect |
| --- | --- | --- |
| `set_admin(caller: Address, registrar: u32, new_admin: Address)` | Registrar admin | Replace admin. Voids the registrar's roles and cap grants |
| `set_roles(caller: Address, registrar: u32, account: Address, bits: u32)` | Registrar admin | Replace `account`'s bits. Zero removes it. Bits outside the mask fail with `RoleBits` |
| `create_hub(caller: Address, registrar: u32) -> u32` | `ROLE_SPOKE` | Create a hub via governance, bound to `registrar` |
| `add_spoke(caller: Address, registrar: u32) -> u32` | `ROLE_SPOKE` | Create a spoke via governance, bound to `registrar` |
| `create_market(caller: Address, registrar: u32, hub_id: u32, params: MarketParamsRaw) -> Address` | `ROLE_MARKET`. Owned hub | Register `params.asset_id` on the pool under `hub_id` |
| `tighten_flags(caller: Address, registrar: u32, spoke_id: u32, hub_asset: HubAssetKey, paused: bool, frozen: bool, no_seize: bool)` | `ROLE_FLAGS` | Tightening only. See the flag rules below |
| `set_cap_grant(caller: Address, registrar: u32, grantee: u32, hub_id: u32, spoke_id: u32, asset: Address, grant: CapGrant)` | `ROLE_MARKET`. Owned hub | Store grant. Grantee must be active and own `spoke_id`. Negative caps fail with `GrantExceeded` |
| `revoke_cap_grant(caller: Address, registrar: u32, grantee: u32, hub_id: u32, spoke_id: u32, asset: Address)` | `ROLE_MARKET`. Owned hub | Delete the grant |
| `propose(caller: Address, registrar: u32, op: AdminOperation, salt: BytesN<32>) -> BytesN<32>` | `ROLE_MARKET`, or `ROLE_FLAGS` for `RelaxSpokeAssetFlags` | Queue on the governance timelock. Returns op id. See proposal rules |
| `cancel_proposal(caller: Address, registrar: u32, op_id: BytesN<32>)` | `ROLE_MARKET` or `ROLE_FLAGS`. Owned hub or spoke | Cancel an operation this contract queued |

Governance endpoints run only from governance. Each named operation needs the owner as proposer and waits the Sensitive delay tier. Suspension can also run at once through `suspend_curator_registrar`, which needs `GUARDIAN_ROLE`.

| Governance endpoint | Operation | Effect |
| --- | --- | --- |
| `register_registrar(admin: Address) -> u32` | `RegisterCuratorRegistrar` | Open a registrar. Returns id |
| `force_set_admin(registrar: u32, admin: Address)` | `SetCuratorRegistrarAdmin` | Replace admin. Voids the registrar's roles and cap grants |
| `set_registrar_suspended(registrar: u32, suspended: bool)` | `SuspendCuratorRegistrar`, or immediate `suspend_curator_registrar` for suspension | Suspend or resume a registrar |
| `reassign_hub(hub_id: u32, registrar: Option<u32>)` | `ReassignCuratorHub` | Bind to an active registrar, or unbind with `None` |
| `reassign_spoke(spoke_id: u32, registrar: Option<u32>)` | `ReassignCuratorSpoke` | Bind to an active registrar, or unbind with `None` |
| `upgrade(wasm_hash: BytesN<32>)` | `UpgradeCurator` | Replace Wasm. Zero hash fails with `InvalidWasmHash` |

| Open view | Result |
| --- | --- |
| `governance() -> Address` | Owning governance contract |
| `registrar_admin(registrar: u32) -> Option<Address>` | Admin, suspended or not |
| `registrar_suspended(registrar: u32) -> bool` | Suspension state |
| `roles(registrar: u32, account: Address) -> u32` | Stored role bits. The admin holds every bit even when this is zero |
| `hub_registrar(hub_id: u32) -> Option<u32>` | Owning registrar |
| `spoke_registrar(spoke_id: u32) -> Option<u32>` | Owning registrar |
| `cap_grant(grantee: u32, hub_id: u32, spoke_id: u32, asset: Address) -> Option<CapGrant>` | Live grant. `None` after the grantor's admin changes |

### Curator flag rules

- The spoke owner sets any flag on a listing in its spoke.
- The spoke owner cannot add `paused` to a listing of a hub asset that another registrar owns (`ForeignPause`).
- The hub owner sets only `frozen` on a listing of its asset in another registrar's spoke. The listing keeps its stored `paused` and `no_seize`.
- The controller rejects clearing a set flag and a call that sets no new flag.

### Curator proposal rules

`propose` accepts only `AddAssetToSpoke`, `EditAssetInSpoke`, `UpgradeLiquidityPoolParams`, and `RelaxSpokeAssetFlags`. Other variants fail with `Unsupported`.

| Operation | Condition |
| --- | --- |
| `AddAssetToSpoke`, `EditAssetInSpoke` in an owned spoke | An owned hub passes. A foreign hub asset needs a live grant, and the listing must fit it (`GrantExceeded`). A foreign hub listing cannot gain `paused` (`ForeignPause`) |
| `EditAssetInSpoke` in a spoke the registrar does not own | The registrar owns the hub. The edit only lowers the supply or borrow cap. Every other field equals the live listing (`NotCapLowering`) |
| `UpgradeLiquidityPoolParams` | The registrar owns the hub |
| `RelaxSpokeAssetFlags` | The registrar owns the spoke. A foreign hub asset needs a live grant. A foreign hub listing cannot gain `paused` (`ForeignPause`) |

A live grant's grantor still owns the hub, is unsuspended, and keeps the admin that gave it. The curator stores each operation's hub and spoke. `cancel_proposal` needs the caller's registrar to own one of them (`ProposalForeign`). It fails with `ProposalUnknown` for operations the curator did not queue. Grants, ownership, and suspension are checked at proposal only. Cancel a queued operation to apply a later revocation, reassignment, or admin change.

## Source map

- [Controller ABI](../../interfaces/controller/src/lib.rs), [administration](../../interfaces/controller/src/admin.rs), [implementation](../../contracts/controller/src/lib.rs), [account auth](../../contracts/controller/src/account.rs), [risk gates](../../contracts/controller/src/risk/validation.rs).
- [Pool ABI](../../interfaces/pool/src/lib.rs), [implementation](../../contracts/pool/src/lib.rs). [Governance ABI](../../interfaces/governance/src/lib.rs), [implementation](../../contracts/governance/src/api.rs), [immediate authority](../../contracts/governance/src/timelock/immediate.rs).
- [NFT implementation](../../contracts/position-nft/src/contract.rs). [Router ABI](../../interfaces/swap-aggregator/src/lib.rs), [implementation](../../contracts/swap-aggregator/src/lib.rs). [Price ABI](../../interfaces/price-aggregator/src/lib.rs).
- [XOXNO oracle](../../contracts/xoxno-oracle/src/lib.rs), [admin](../../contracts/xoxno-oracle/src/admin.rs), [reads](../../contracts/xoxno-oracle/src/reads.rs), [submit](../../contracts/xoxno-oracle/src/submit.rs). [Adapter](../../contracts/defindex-strategy/src/lib.rs). [Curator entrypoints](../../contracts/curator/src/api.rs), [registrar rules](../../contracts/curator/src/ops.rs).
- Generated NFT and default ownership semantics use OpenZeppelin stellar-contracts release 0.8.0, pinned in [Cargo.toml](../../Cargo.toml).
