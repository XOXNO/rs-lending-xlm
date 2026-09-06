# Event reference

Use this reference to decode protocol events and interpret their amounts, shares and configuration snapshots. Match both the emitting contract and the ordered topic vector.

The contracts define 28 custom event types: controller 21, pool 3, governance 2, price aggregator 1 and DeFindex adapter 1. The position NFT, swap aggregator and XOXNO oracle define no custom events, but their inherited OpenZeppelin events remain observable. Subscribe to both controller and pool events to track account and market changes.

## Wire rules and units

The decoder must preserve the following encoding rules:

- Explicit custom topics below are ordered Soroban Symbols. Custom events have no dynamic topic fields. Inherited events do, as listed separately.
- `map` payloads use Symbol field-name keys sorted alphabetically. `vec` uses declaration order. `single-value` is the field value itself without a wrapper. Named contract structs are maps; tuple structs are vectors.
- Only integer enums with explicit discriminants encode as u32 here (`PositionAction`, `EventPositionMode`). Ordinary enum variants encode as `[Symbol(variant), payload...]`, including unit variants such as `["Market"]`. `Option<T>` is T for Some and Void for None.
- `i128` amounts/fees/cash use raw token decimals unless marked otherwise. `*_wad` and sanity/factor bounds use 10^18; indexes, annual rates, utilization and share balances use 10^27; risk parameters and fees marked BPS use 10,000. Pool timestamps use milliseconds. Oracle staleness and observation timestamps use seconds. Ledger deadlines are sequence numbers.
- For shares S, RAY index I and token decimals d, underlying RAY is the protocol-rounded `S * I / RAY`, then raw token units rescale by `10^(27-d)` with the path’s floor/ceil rounding. Multiplying shares by index alone does **not** produce raw token units. See [formulas](formulas.md).

## Custom events

Field lists use exact Rust types. Map key order is alphabetical, regardless of declaration order. The sections after the catalog define nested payloads.

### Controller events

| Event / ordered topics | Format and fields | Emission / interpretation |
| --- | --- | --- |
| `AccountDelegateEvent`<br>`["account", "delegate"]` | map: `account_id: u64, owner: Address, delegate: Address, granted: bool` | add_delegate/remove_delegate, only when list changes. |
| `CleanBadDebtEvent`<br>`["debt", "bad_debt"]` | map: `account_id: u64, total_borrow_usd_wad: i128, total_collateral_usd_wad: i128` | clean_bad_debt, force_socialize_bad_debt or liquidation residual cleanup. Values are before cleanup; no cleanup position batch. NFT Burn and pool snapshots still emit. |
| `UpdatePositionBatchEvent`<br>`["position", "batch_update"]` | vec: `account_id: u64, account_attributes: EventAccountAttributes, deposits: Vec<EventDepositDelta>, borrows: Vec<EventBorrowDelta>` | Supply/borrow/withdraw/repay/liquidation, all account strategies including flash_position, and threshold refresh. Suppressed when both leg vectors are empty. |
| `LiquidationEvent`<br>`["position", "liquidation"]` | map: `liquidator: Address, account_id: u64, repaid_usd_wad: i128, bonus_bps: i128` | liquidate: repayment USD is measured and capped per planned leg; bonus is BPS. Seizure and fee figures are in position batches. |
| `FlashLoanEvent`<br>`["position", "flash_loan"]` | map: `hub_id: u32, asset: Address, receiver: Address, caller: Address, amount: i128, fee: i128` | flash_loan: requested principal and charged fee, token units. |
| `FlashPositionEvent`<br>`["position", "flash_position"]` | map: `account_id: u64, hub_id: u32, asset: Address, receiver: Address, caller: Address, amount: i128, amount_received: i128, fee: i128` | flash_position: requested amount, measured receiver receipt; fee is zero. |
| `ClaimRevenueEvent`<br>`["revenue", "claim"]` | map: `hub_id: u32, asset: Address, caller: Address, accumulator: Address, amount: i128` | claim_revenue: positive measured controller receipt sent onward to accumulator. Does not measure accumulator receipt. |
| `InitialMultiplyPaymentEvent`<br>`["strategy", "initial_payment"]` | map: `token: Address, amount: i128, account_id: u64` | multiply with initial payment: requested original payment before conversion, not measured receipt. |
| `BlendMigrationEvent`<br>`["strategy", "blend_migration"]` | map: `account_id: u64, blend_pool: Address, collateral_count: u32, supply_count: u32, debt_count: u32` | migrate_from_blend: completed input entry counts. |
| `UpdateSpokeEvent`<br>`["config", "spoke"]` | map: `spoke: EventSpoke` | add_spoke, remove_spoke, set_spoke_liquidation_curve: post-change snapshot. |
| `UpdateSpokeAssetEvent`<br>`["config", "spoke_asset"]` | map: `asset: Address, config: SpokeAssetConfig, spoke_id: u32, hub_id: u32` | add_asset_to_spoke, edit_asset_in_spoke, set_spoke_asset_flags: full post-change listing. |
| `RemoveSpokeAssetEvent`<br>`["config", "remove_spoke_asset"]` | map: `asset: Address, spoke_id: u32, hub_id: u32` | remove_asset_from_spoke after zero-usage check. |
| `ApproveBlendPoolEvent`<br>`["config", "approve_blend_pool"]` | map: `pool: Address, approved: bool` | approve_blend_pool/revoke_blend_pool; approval bool. |
| `UpdateSwapAggregatorEvent`<br>`["config", "swap_aggregator"]` | map: `swap_aggregator: Address` | set_swap_aggregator. |
| `UpdatePriceAggregatorEvent`<br>`["config", "price_aggregator"]` | map: `price_aggregator: Address` | set_price_aggregator, including governance deployment wiring. |
| `UpdateAccumulatorEvent`<br>`["config", "accumulator"]` | map: `accumulator: Address` | set_accumulator. |
| `UpdatePositionLimitsEvent`<br>`["config", "position_limits"]` | map: `max_supply_positions: u32, max_borrow_positions: u32` | set_position_limits and controller constructor. |
| `UpdateMinBorrowCollateralEvent`<br>`["config", "min_borrow_collateral"]` | map: `min_borrow_collateral_usd_wad: i128` | set_min_borrow_collateral_usd and controller constructor: LTV-weighted USD floor. |
| `CreateHubEvent`<br>`["config", "hub"]` | map: `hub_id: u32` | create_hub. |
| `CreateMarketEvent`<br>`["market", "create"]` | map: `hub_id: u32, base_asset: Address, max_borrow_rate: i128, base_borrow_rate: i128, slope1: i128, slope2: i128, slope3: i128, mid_utilization: i128, optimal_utilization: i128, max_utilization: i128, reserve_factor: u32, market_address: Address` | create_liquidity_pool: flattened curve fields; omits decimals/flash settings. |
| `UpdateMarketParamsEvent`<br>`["market", "params_update"]` | map: `hub_id: u32, asset: Address, max_borrow_rate: i128, base_borrow_rate: i128, slope1: i128, slope2: i128, slope3: i128, mid_utilization: i128, optimal_utilization: i128, max_utilization: i128, reserve_factor: u32` | upgrade_liquidity_pool_params: flattened curve fields; omits flash settings. |

### Pool events

| Event / ordered topics | Format and fields | Emission / interpretation |
| --- | --- | --- |
| `PoolMarketStateBatchEvent`<br>`["market", "batch_state_update"]` | single-value: `updates: Vec<PoolMarketStateEvent>` | Pool supply/borrow/withdraw/repay/seize_positions, update_indexes, recapitalize, flash_loan, create_strategy, net_settle, claim_revenue. Suppressed for empty snapshots. |
| `PoolMarketParamsBatchEvent`<br>`["market", "batch_params_update"]` | single-value: `updates: Vec<PoolMarketParamsEvent>` | create_market/update_params: one full parameter row. |
| `StrategyFeeEvent`<br>`["strategy", "fee"]` | map: `hub_id: u32, asset: Address, amount: i128, fee: i128, amount_sent: i128` | create_strategy only when fee != 0; amount_sent = requested amount minus fee, not receiver receipt. |

### Governance events

| Event / ordered topics | Format and fields | Emission / interpretation |
| --- | --- | --- |
| `DeployControllerEvent`<br>`["governance", "deploy_controller"]` | map: `controller: Address, wasm_hash: BytesN<32>` | governance deploy_controller. |
| `DeployPriceAggregatorEvent`<br>`["governance", "deploy_price_aggregator"]` | map: `price_aggregator: Address, wasm_hash: BytesN<32>` | governance deploy_price_aggregator. |

### Price-aggregator events

| Event / ordered topics | Format and fields | Emission / interpretation |
| --- | --- | --- |
| `UpdateAssetOracleEvent`<br>`["config", "asset_oracle"]` | map: `key: PriceKey, oracle: AssetOracle` | set_oracle, set_sanity_band, set_tolerance: full stored oracle snapshot. |

### DeFindex-strategy events

| Event / ordered topics | Format and fields | Emission / interpretation |
| --- | --- | --- |
| `HarvestEvent`<br>`["strategy", "harvest"]` | map: `from: Address, amount: i128, price_per_share: i128` | harvest: from authorizes, amount = 0; supply index floor-rescaled to 12 decimals. |

## Position batches

`UpdatePositionBatchEvent` encodes as `[account_id, account_attributes, deposits, borrows]`. The last two entries are vectors of tuple records. `scaled_amount` is the resulting share balance; `amount` is the account’s movement in raw token units. Deposit risk parameters use BPS.

| Tuple type | Ordered elements with types |
| --- | --- |
| EventAccountAttributes | `owner: Address, spoke_id: u32, mode: EventPositionMode` |
| EventDepositDelta | `action: PositionAction, hub_id: u32, asset: Address, scaled_amount: i128, index_ray: i128, amount: i128, liquidation_threshold: u32, liquidation_bonus: u32, loan_to_value: u32, liquidation_fees: u32` |
| EventBorrowDelta | `action: PositionAction, hub_id: u32, asset: Address, scaled_amount: i128, index_ray: i128, amount: i128` |

| PositionAction u32 | Meaning |
| --- | --- |
| `Supply = 0` | ordinary supply and strategy deposits |
| `Borrow = 1` | ordinary debt mint |
| `Withdraw = 2` | ordinary collateral withdrawal |
| `Repay = 3` | ordinary debt repayment |
| `LiqRepay = 4` | liquidated account repayment |
| `LiqSeize = 5` | gross collateral debit, both seize modes |
| `Multiply = 6` | multiply debt mint |
| `ParamUpd = 7` | risk-parameter rewrite; no token movement |
| `SwDebtR = 8` | both new borrow and old repay in swap_debt |
| `SwColWd = 9` | swap_collateral withdrawal |
| `RpColWd = 10` | repay_debt_with_collateral withdrawal |
| `RpColR = 11` | repay from converted collateral |
| `CloseWd = 12` | remaining collateral withdrawn on close |
| `Migrate = 13` | Blend migration debt/repayment legs |
| `RpColNet = 14` | same-market net settlement, no cash movement |
| `LiqCredit = 15` | net supply credit to seizure receiver |
| `FlashPos = 16` | flash_position debt mint |

`EventPositionMode` is u32: None=0 (internal Normal), Multiply=1, Long=2, Short=3.

Liquidation emits controller events in this order: `LiquidationEvent`, the target position batch, an optional Credit receiver batch, then optional bad-debt cleanup. Pool, NFT and token events can occur between these controller events. The receiver batch contains only supply credits and omits zero-net credits.

`LiqSeize.amount` is gross and `LiqCredit.amount` is net. Both use raw token units, including in Credit mode. Their difference can include conversion rounding; compute the exact share fee from share deltas and the [seizure calculation](formulas.md).

`Credit(0)` emits an inherited NFT `Mint` when it creates an account. There is no controller AccountCreated event. Track NFT lifecycle events as well as position batches, which can be absent when no nonzero deltas remain. Account deletion burns the NFT; an ordinary repayment does not always delete an emptied account.

## Market and configuration records

`PoolMarketStateBatchEvent` data is directly `Vec<PoolMarketStateEvent>`. Each row is the following ordered tuple:

```text
[hub_id: u32, asset: Address, timestamp: u64, supply_index: i128,
 borrow_index: i128, cash: i128, supplied: i128, borrowed: i128, revenue: i128]
```

Timestamp uses milliseconds and cash uses raw token units. Indexes and supplied, borrowed and revenue shares use RAY. Cash is the market's recorded allocation of the pool's tokens. Revenue is outstanding unclaimed supply shares and decreases when claimed.

`PoolMarketParamsBatchEvent` data is directly a vector of named maps `{hub_id: u32, asset: Address, params: MarketParamsRaw}`. Decimals omitted from controller market events are available here or from pool sync data.

| Named map type | Complete fields and units |
| --- | --- |
| `EventSpoke` | `spoke_id: u32, is_deprecated: bool, liquidation_target_hf_wad: i128, hf_for_max_bonus_wad: i128, liquidation_bonus_factor_bps: u32` |
| `SpokeAssetConfig` | `is_collateralizable: bool, is_borrowable: bool, paused: bool, frozen: bool, no_seize: bool, loan_to_value: u32, liquidation_threshold: u32, liquidation_bonus: u32, liquidation_fees: u32, supply_cap: i128, borrow_cap: i128` |
| `MarketParamsRaw` | `max_borrow_rate: i128, base_borrow_rate: i128, slope1: i128, slope2: i128, slope3: i128, mid_utilization: i128, optimal_utilization: i128, max_utilization: i128, reserve_factor: u32, is_flashloanable: bool, flashloan_fee: u32, asset_id: Address, asset_decimals: u32` |
| `AssetOracle` | `asset_decimals: u32, max_price_stale_seconds: u64, sources: Vec<PriceSource>, tolerance: OracleTolerance, independence: IndependencePolicy, min_sanity_price_wad: i128, max_sanity_price_wad: i128` |
| `ReflectorFeedRef` | `contract: Address, asset: OracleAssetRef, read_mode: OracleReadMode` |
| `MultiFeedRef` | `contract: Address, feed_id: String, nature: FeedNature` |
| `FeedSource` | `provider: ProviderRef, decimals: u32, max_stale_seconds: u64` |
| `ScaledSource` | `factor: FeedSource, quote: PriceKey, min_factor_wad: i128, max_factor_wad: i128` |
| `AquariusLpSource` | `pool: Address, token_a: Address, token_b: Address, key_a: PriceKey, key_b: PriceKey, reserve_a_decimals: u32, reserve_b_decimals: u32, min_pool_value_wad: i128` |
| `OracleTolerance` | `upper_ratio_bps: u32, lower_ratio_bps: u32` |

Rate fields `base_borrow_rate`, `max_borrow_rate` and `slope1/2/3` use annual RAY. Slopes are additive segment increments; `max_utilization` is an operation limit. See [formulas](formulas.md) for the curve and rounding. Reserve factor and flash-loan fee use BPS; asset decimals describe raw token units. Spoke caps use raw token units; position limits and entry counts are counts.

Spoke flags and deprecation affect admission differently for entry, exit and seizure. See [risk checks and pause flags](endpoints.md#risk-checks-and-pause-flags) and [Credit liquidation](endpoints.md#liquidation-results) for these restrictions.

## Oracle enum payloads

| Type | Exact variant shape |
| --- | --- |
| PriceKey | `["Token", Address]`, `["Ref", Symbol]` |
| PriceSource | `["Feed", FeedSource]`, `["Scaled", ScaledSource]`, `["AquariusLp", AquariusLpSource]`, `["AquariusStableLp", AquariusLpSource]` |
| ProviderRef | `["Reflector", ReflectorFeedRef]`, `["RedStone", MultiFeedRef]`, `["Xoxno", MultiFeedRef]` |
| OracleAssetRef | `["Stellar", Address]`, `["Symbol", Symbol]`, `["String", String]` |
| OracleReadMode | `["Spot"]`, `["Twap", u32]`; Twap is the equal-weight mean of records |
| FeedNature | `["Market"]`, `["Fundamental"]` |
| IndependencePolicy | `["RequireDisjoint"]`, `["AllowShared", Vec<Address>]` |

All `*_seconds` fields are seconds; `*_wad` prices/factors are WAD. Feed `decimals` describes provider output; reserve/asset decimals describe token amounts. OracleTolerance ratios use BPS; only upper ratio is consulted at read time, with lower reciprocal validated at configuration.

## Inherited events

OpenZeppelin revision `fbfde388e1b72afa93d6b1c922067879b20e81db` and Soroban SDK 27.0.6 determine these shapes. Topic names default to snake_case struct names. All payloads here are maps, including empty maps for topic-only events. Angle-bracket entries below are dynamic topic values.

| Emitter / event | Ordered topics | Map fields | Trigger |
| --- | --- | --- | --- |
| Position NFT: `Transfer` | `["transfer", <from: Address>, <to: Address>]` | `token_id: u32` | transfer/transfer_from |
| Position NFT: `Approve` | `["approve", <approver: Address>, <token_id: u32>]` | `approved: Address, live_until_ledger: u32` | approve |
| Position NFT: `ApproveForAll` | `["approve_for_all", <owner: Address>]` | `operator: Address, live_until_ledger: u32` | approve_for_all |
| Position NFT: `Mint` | `["mint", <to: Address>]` | `token_id: u32` | mint |
| Position NFT: `Burn` | `["burn", <from: Address>]` | `token_id: u32` | controller-authorized burn |
| Controller, governance, router, XOXNO: OwnershipTransfer | `["ownership_transfer"]` | `old_owner: Address, new_owner: Address, live_until_ledger: u32` | ownership transfer initiation where exported/routed |
| Controller, governance, router, XOXNO; price aggregator constructor: OwnershipTransferCompleted | `["ownership_transfer_completed"]` | `new_owner: Address` | accept_ownership; controller, governance and price-aggregator constructors emit explicitly |
| Router, XOXNO: OwnershipRenounced | `["ownership_renounced"]` | `old_owner: Address` | router/XOXNO renounce_ownership only |
| Governance: RoleGranted | `["role_granted", <role: Symbol>, <account: Address>]` | `caller: Address` | constructor, role grants, ownership synchronization/reset |
| Governance: RoleRevoked | `["role_revoked", <role: Symbol>, <account: Address>]` | `caller: Address` | role revocation, immediate revoke, ownership/reset |
| Governance: AdminTransferInitiated | `["admin_transfer_initiated", <current_admin: Address>]` | `new_admin: Address, live_until_ledger: u32` | scheduled governance ownership transfer |
| Governance: AdminTransferCompleted | `["admin_transfer_completed", <new_admin: Address>]` | `previous_admin: Address` | constructor and accepted ownership |
| Controller: `Paused` | `["paused"]` | `{}` | pause, constructor and upgrade when not already paused |
| Controller: `Unpaused` | `["unpaused"]` | `{}` | unpause |
| Governance: MinDelayChanged | `["min_delay_changed"]` | `old_delay: u32, new_delay: u32` | constructor, UpdateGovDelay |
| Governance: OperationScheduled | `["operation_scheduled", <id: BytesN<32>>, <target: Address>]` | `function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>, delay: u32` | propose/propose_canceller_reset |
| Governance: OperationExecuted | `["operation_executed", <id: BytesN<32>>, <target: Address>]` | `function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>` | execute/execute_self/execute_canceller_reset |
| Governance: OperationCancelled | `["operation_cancelled", <id: BytesN<32>>]` | `{}` | cancel |

The governance ABI does not expose role-admin changes or admin renunciation, so it does not emit RoleAdminChanged or AdminRenounced. Ownable `set_owner` emits nothing: router, XOXNO oracle and pool constructors use this silent path. Controller, price-aggregator and governance constructors explicitly emit ownership events. Direct `update_current_contract_wasm` calls define no custom upgrade event.

Underlying token contracts emit their own transfer and approval events. Decode them under the token's standard; NFT events are not SEP-41 events, and token transfers are not protocol custom events.

XOXNO price submissions have no custom submission event. Router swaps have no custom route or fee event, although the router emits inherited ownership events. Use [endpoint semantics](endpoints.md) when an action has no dedicated event.

## Source map

- [Controller events](../../contracts/controller/src/events/mod.rs), [config events](../../contracts/controller/src/events/config.rs), [market events](../../contracts/controller/src/events/market.rs), [liquidation application](../../contracts/controller/src/positions/liquidation/apply.rs), [cleanup](../../contracts/controller/src/positions/liquidation/bad_debt.rs).
- [Pool events](../../contracts/pool/src/events.rs), [governance events](../../contracts/governance/src/events.rs), [price registry](../../contracts/price-aggregator/src/registry.rs), [adapter](../../contracts/defindex-strategy/src/lib.rs), [NFT lifecycle](../../contracts/position-nft/src/contract.rs).
- [Shared types](../../common/src/types/mod.rs), [rate curve](../../common/src/rates/curve.rs), [dependency pins](../../Cargo.toml).
