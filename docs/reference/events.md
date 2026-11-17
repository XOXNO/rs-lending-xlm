# Event reference

Match emitting contract and ordered topics. Controller, pool, governance, curator, price aggregator, swap aggregator and DeFindex adapter define custom events; position NFT and XOXNO oracle emit only inherited OpenZeppelin events. Track accounts and markets from both controller and pool events.

## Wire rules and units

- Custom topics are ordered Symbols, never dynamic; inherited events have dynamic topics.
- `map`: Symbol field keys, alphabetical regardless of declaration order. `vec`: declaration order. `single-value`: the bare value. Named structs are maps; tuple structs vectors.
- Only `PositionAction` and `EventPositionMode` (explicit discriminants) encode as u32. Other enum variants encode as `[Symbol(variant), payload...]`, unit variants included (`["Market"]`). `Option<T>` is T for Some and Void for None.
- `i128` amounts/fees/cash: raw token units unless marked. `*_wad`: 10^18. Indexes, annual rates, utilization, shares: 10^27. BPS: 10,000. Pool timestamps: milliseconds; oracle staleness and observation timestamps: seconds; ledger deadlines: sequence numbers.
- Shares × index is a RAY value, **not** token units ([conversion](formulas.md#shares-and-token-amounts)).

## Custom events

Fields use exact Rust types; nested payloads are defined below.

### Controller events

| Event / topics | Format and fields | Emitted by / meaning |
| --- | --- | --- |
| `AccountDelegateEvent`<br>`["account", "delegate"]` | map: `account_id: u64, owner: Address, delegate: Address, granted: bool` | add_delegate/remove_delegate, when the list changes. |
| `CleanBadDebtEvent`<br>`["debt", "bad_debt"]` | map: `account_id: u64, total_borrow_usd_wad: i128, total_collateral_usd_wad: i128` | clean_bad_debt, force_socialize_bad_debt, liquidation residual cleanup. Pre-cleanup totals: debt ceil (unlike half-up `get_total_borrow_usd`), unweighted collateral half-up. No position batch; NFT `Burn` and pool snapshots emit. |
| `UpdatePositionBatchEvent`<br>`["position", "batch_update"]` | vec: `account_id: u64, account_attributes: EventAccountAttributes, deposits: Vec<EventDepositDelta>, borrows: Vec<EventBorrowDelta>` | Supply/borrow/withdraw/repay/liquidation, every account strategy incl. flash_position, update_account_threshold. Skipped when both leg vectors are empty. |
| `LiquidationEvent`<br>`["position", "liquidation"]` | map: `liquidator: Address, account_id: u64, repaid_usd_wad: i128, bonus_bps: i128` | liquidate: repayment measured, capped per planned leg; bonus BPS. Seizure and fees: position batches. |
| `FlashLoanEvent`<br>`["position", "flash_loan"]` | map: `hub_id: u32, asset: Address, receiver: Address, caller: Address, amount: i128, fee: i128` | flash_loan: requested principal, charged fee. |
| `FlashPositionEvent`<br>`["position", "flash_position"]` | map: `account_id: u64, hub_id: u32, asset: Address, receiver: Address, caller: Address, amount: i128, amount_received: i128, fee: i128` | flash_position: requested amount, measured receipt; fee zero. |
| `ClaimRevenueEvent`<br>`["revenue", "claim"]` | map: `hub_id: u32, asset: Address, caller: Address, accumulator: Address, amount: i128` | claim_revenue: positive measured controller receipt forwarded (accumulator receipt unmeasured). |
| `InitialMultiplyPaymentEvent`<br>`["strategy", "initial_payment"]` | map: `token: Address, amount: i128, account_id: u64` | multiply initial payment: requested, pre-conversion, unmeasured. |
| `BlendMigrationEvent`<br>`["strategy", "blend_migration"]` | map: `account_id: u64, blend_pool: Address, collateral_count: u32, supply_count: u32, debt_count: u32` | migrate_from_blend: completed entry counts. |
| `UpdateSpokeEvent`<br>`["config", "spoke"]` | map: `spoke: EventSpoke` | add_spoke, remove_spoke, set_spoke_liquidation_curve; post-change snapshot. |
| `UpdateSpokeAssetEvent`<br>`["config", "spoke_asset"]` | map: `asset: Address, config: SpokeAssetConfig, spoke_id: u32, hub_id: u32` | add_asset_to_spoke, edit_asset_in_spoke, set_spoke_asset_flags, relax_spoke_asset_flags; full post-change listing. |
| `RemoveSpokeAssetEvent`<br>`["config", "remove_spoke_asset"]` | map: `asset: Address, spoke_id: u32, hub_id: u32` | remove_asset_from_spoke, after zero-usage check. |
| `ApproveBlendPoolEvent`<br>`["config", "approve_blend_pool"]` | map: `pool: Address, approved: bool` | approve_blend_pool/revoke_blend_pool. |
| `UpdateSwapAggregatorEvent`<br>`["config", "swap_aggregator"]` | map: `swap_aggregator: Address` | set_swap_aggregator. |
| `UpdatePriceAggregatorEvent`<br>`["config", "price_aggregator"]` | map: `price_aggregator: Address` | set_price_aggregator, including governance deployment. |
| `UpdateAccumulatorEvent`<br>`["config", "accumulator"]` | map: `accumulator: Address` | set_accumulator. |
| `UpdatePositionLimitsEvent`<br>`["config", "position_limits"]` | map: `max_supply_positions: u32, max_borrow_positions: u32` | set_position_limits and constructor. |
| `UpdateMinBorrowCollateralEvent`<br>`["config", "min_borrow_collateral"]` | map: `min_borrow_collateral_usd_wad: i128` | set_min_borrow_collateral_usd and constructor: LTV-weighted USD floor. |
| `CreateHubEvent`<br>`["config", "hub"]` | map: `hub_id: u32` | create_hub. |
| `CreateMarketEvent`<br>`["market", "create"]` | map: `hub_id: u32, base_asset: Address, market_address: Address`, plus the nine curve fields `max_borrow_rate`…`reserve_factor` of [`MarketParamsRaw`](#market-and-configuration-records), same types | create_liquidity_pool; omits decimals and flash settings. |
| `UpdateMarketParamsEvent`<br>`["market", "params_update"]` | map: `hub_id: u32, asset: Address`, plus the same nine curve fields | upgrade_liquidity_pool_params; omits flash settings. |

### Pool events

| Event / topics | Format and fields | Emitted by / meaning |
| --- | --- | --- |
| `PoolMarketStateBatchEvent`<br>`["market", "batch_state_update"]` | single-value: `updates: Vec<PoolMarketStateEvent>` | supply/borrow/withdraw/repay/seize_positions, update_indexes (one single-row event per market), recapitalize, flash_loan, create_strategy, net_settle, claim_revenue. Skipped when empty. |
| `PoolMarketParamsBatchEvent`<br>`["market", "batch_params_update"]` | single-value: `updates: Vec<PoolMarketParamsEvent>` | create_market/update_params; one full row. |
| `StrategyFeeEvent`<br>`["strategy", "fee"]` | map: `hub_id: u32, asset: Address, amount: i128, fee: i128, amount_sent: i128` | create_strategy when fee != 0; amount_sent = requested amount minus fee, not receiver receipt. |
| `MarketValueCeilingEvent`<br>`["market", "value_ceiling"]` | single-value: `hub_asset: HubAssetKey`, a map `{asset: Address, hub_id: u32}` | An accrual that held interest because supply or debt value hit `MAX_MARKET_VALUE_RAY` ([INV-IDX-06](invariants.md#inv-idx-06)). At most one per market per accrual, before the state snapshot; repeats while held. |

### Governance events

| Event / topics | Format and fields | Emitted by / meaning |
| --- | --- | --- |
| `DeployControllerEvent`<br>`["governance", "deploy_controller"]` | map: `controller: Address, wasm_hash: BytesN<32>` | governance deploy_controller. |
| `DeployPriceAggregatorEvent`<br>`["governance", "deploy_price_aggregator"]` | map: `price_aggregator: Address, wasm_hash: BytesN<32>` | governance deploy_price_aggregator. |

### Curator events

Each registry write emits one event, from its setter, carrying the stored value. Forwarded calls (`create_hub`, `add_spoke`, `create_market`, `tighten_flags`) appear only as the governance and controller events they cause. `ProposedEvent` alone records which registrar queued an operation; `CancelledEvent` names the canceller's registrar.

| Event / topics | Format and fields | Emitted by / meaning |
| --- | --- | --- |
| `RegistrarAdminEvent`<br>`["curator", "registrar_admin"]` | map: `registrar: u32, admin: Address` | register_registrar, set_admin, force_set_admin. Admin change silently voids the registrar's roles and grants (no `RolesEvent`/`CapGrantEvent`); indexers drop them here. |
| `RolesEvent`<br>`["curator", "roles"]` | map: `registrar: u32, account: Address, bits: u32` | set_roles; `bits = 0` removes the account. |
| `SuspendedEvent`<br>`["curator", "suspended"]` | map: `registrar: u32, suspended: bool` | set_registrar_suspended. |
| `HubRegistrarEvent`<br>`["curator", "hub_registrar"]` | map: `hub_id: u32, registrar: Option<u32>` | create_hub, reassign_hub; `None` unbinds the hub. |
| `SpokeRegistrarEvent`<br>`["curator", "spoke_registrar"]` | map: `spoke_id: u32, registrar: Option<u32>` | add_spoke, reassign_spoke; `None` unbinds the spoke. |
| `CapGrantEvent`<br>`["curator", "cap_grant"]` | map: `grantor: u32, grantee: u32, hub_id: u32, spoke_id: u32, asset: Address, grant: Option<CapGrant>` | set_cap_grant (`Some`, stored grant), revoke_cap_grant (`None`; `grantor` = revoking hub owner). |
| `ProposedEvent`<br>`["curator", "proposed"]` | map: `registrar: u32, op_id: BytesN<32>` | propose, after governance queues. |
| `CancelledEvent`<br>`["curator", "cancelled"]` | map: `registrar: u32, op_id: BytesN<32>` | cancel_proposal, after governance cancels; `registrar` owns the operation's hub or spoke. |

### Price-aggregator events

| Event / topics | Format and fields | Emitted by / meaning |
| --- | --- | --- |
| `UpdateAssetOracleEvent`<br>`["config", "asset_oracle"]` | map: `key: PriceKey, oracle: AssetOracle` | set_oracle, set_sanity_band, set_tolerance; full stored snapshot. |

### Swap-aggregator events

| Event / topics | Format and fields | Emitted by / meaning |
| --- | --- | --- |
| `SwapEvent`<br>`["router", "swap"]` | map: `sender: Address, token_in: Address, token_out: Address, amount_in: i128, amount_out: i128, referral_id: u64, fee_token: Address, static_fee: i128, referral_fee: i128` | execute_strategy, after the payout. `amount_in`: measured router receipt. `amount_out`: paid to `sender` after fees (receipt unmeasured). `fee_token` is `token_in` or `token_out`; both fees are 0 without an active referral. Controller strategy swaps emit it with the controller as `sender`. |

### DeFindex-strategy events

| Event / topics | Format and fields | Emitted by / meaning |
| --- | --- | --- |
| `HarvestEvent`<br>`["strategy", "harvest"]` | map: `from: Address, amount: i128, price_per_share: i128` | harvest: from authorizes; amount 0; price_per_share = supply index floored to 12 decimals. |

## Position batches

`deposits`/`borrows` are tuple vectors. `scaled_amount` is the resulting share balance; `amount` the movement in raw token units. Deposit risk parameters are BPS.

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
| `ParamUpd = 7` | risk-parameter rewrite, no token movement: `update_account_threshold`, or a borrow, withdrawal or account strategy restamping a supply leg's stored LTV or a liquidation tuple that does not favor liquidators |
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

Liquidation order: `LiquidationEvent`, target batch, optional Credit receiver batch (nonzero supply credits only), optional bad-debt cleanup; pool, NFT and token events can interleave.

`LiqSeize.amount` (gross) and `LiqCredit.amount` (net) are raw token units even in Credit mode; their difference can include rounding. Derive the exact share fee from share deltas ([seizure](formulas.md#seizure-and-fees)).

`Credit(0)` account creation emits only the NFT `Mint`; deletion burns the NFT, so track NFT lifecycle too. No deltas, no batch. `repay` never deletes an account, even clearing the last debt.

## Market and configuration records

`PoolMarketStateBatchEvent` data is `Vec<PoolMarketStateEvent>` of ordered tuples:

```text
[hub_id: u32, asset: Address, timestamp: u64, supply_index: i128,
 borrow_index: i128, cash: i128, supplied: i128, borrowed: i128, revenue: i128]
```

Timestamp: ms. Cash: the market's booked pool tokens, raw units. Indexes and supplied, borrowed, revenue shares: RAY. Revenue is unclaimed supply shares, falling when claimed.

`PoolMarketParamsBatchEvent` data is a vector of maps `{hub_id: u32, asset: Address, params: MarketParamsRaw}`, carrying the decimals controller market events omit (also in pool sync data).

| Named map type | Complete fields and units |
| --- | --- |
| `EventSpoke` | `spoke_id: u32, is_deprecated: bool, liquidation_target_hf_wad: i128, hf_for_max_bonus_wad: i128, liquidation_bonus_factor_bps: u32` |
| `SpokeAssetConfig` | `is_collateralizable: bool, is_borrowable: bool, paused: bool, frozen: bool, no_seize: bool, loan_to_value: u32, liquidation_threshold: u32, liquidation_bonus: u32, liquidation_fees: u32, supply_cap: i128, borrow_cap: i128` |
| `MarketParamsRaw` | `max_borrow_rate: i128, base_borrow_rate: i128, slope1: i128, slope2: i128, slope3: i128, mid_utilization: i128, optimal_utilization: i128, max_utilization: i128, reserve_factor: u32, is_flashloanable: bool, flashloan_fee: u32, asset_id: Address, asset_decimals: u32` |
| `AssetOracle` | `asset_decimals: u32, max_price_stale_seconds: u64, sources: Vec<PriceSource>, tolerance: OracleTolerance, independence: IndependencePolicy, min_sanity_price_wad: i128, max_sanity_price_wad: i128` |
| `ReflectorFeedRef` | `contract: Address, asset: OracleAssetRef, read_mode: OracleReadMode` |
| `ChainlinkFeedRef` | `contract: Address, data_id: BytesN<32>` |
| `MultiFeedRef` | `contract: Address, feed_id: String, nature: FeedNature` |
| `FeedSource` | `provider: ProviderRef, decimals: u32, max_stale_seconds: u64` |
| `ScaledSource` | `factor: FeedSource, quote: PriceKey, min_factor_wad: i128, max_factor_wad: i128` |
| `AquariusLpSource` | `pool: Address, token_a: Address, token_b: Address, key_a: PriceKey, key_b: PriceKey, reserve_a_decimals: u32, reserve_b_decimals: u32, min_pool_value_wad: i128` |
| `CurveStableLpSource` | `pool: Address, coins: Vec<Address>, wasm_hash: BytesN<32>, min_pool_value_wad: i128` |
| `OracleTolerance` | `upper_ratio_bps: u32, lower_ratio_bps: u32` |

Rate fields and slopes use annual RAY ([curve](formulas.md#utilization-and-annual-rates)); `max_utilization` is an operation limit. Reserve factor and flash-loan fee use BPS. Spoke caps use raw token units.

Flag and deprecation effects: [pause flags](endpoints.md#risk-checks-and-pause-flags), [Credit liquidation](endpoints.md#liquidation-results).

`CapGrant` is a map: `supply_cap: i128, borrow_cap: i128` (hub-asset raw units), `can_collateral: bool, can_borrow: bool`, and `ltv: u32, threshold: u32, bonus: u32, liquidation_fees: u32` (BPS ceilings a grantee listing may not exceed).

## Oracle enum payloads

| Type | Exact variant shape |
| --- | --- |
| PriceKey | `["Token", Address]`, `["Ref", Symbol]` |
| PriceSource | `["Feed", FeedSource]`, `["Scaled", ScaledSource]`, `["AquariusLp", AquariusLpSource]`, `["AquariusStableLp", AquariusLpSource]`, `["CurveStableLp", CurveStableLpSource]` |
| ProviderRef | `["Reflector", ReflectorFeedRef]`, `["RedStone", MultiFeedRef]`, `["Xoxno", MultiFeedRef]`, `["Chainlink", ChainlinkFeedRef]` |
| OracleAssetRef | `["Stellar", Address]`, `["Symbol", Symbol]`, `["String", String]` |
| OracleReadMode | `["Spot"]`, `["Twap", u32]`; Twap is the equal-weight mean of records |
| FeedNature | `["Market"]`, `["Fundamental"]` |
| IndependencePolicy | `["RequireDisjoint"]`, `["AllowShared", Vec<Address>]` |

Feed `decimals` describe provider output; reserve/asset decimals token amounts. Reads use only the upper OracleTolerance ratio; configuration checks the lower is its reciprocal.

## Inherited events

OpenZeppelin 0.8.0 and Soroban SDK 28.0.0 determine these shapes. Topics default to snake_case struct names. Payloads are maps, empty for topic-only events. Angle brackets mark dynamic topics.

| Emitter / event | Ordered topics | Map fields | Trigger |
| --- | --- | --- | --- |
| Position NFT: `Transfer` | `["transfer", <from: Address>, <to: Address>]` | `token_id: u32` | transfer/transfer_from |
| Position NFT: `Approve` | `["approve", <approver: Address>, <token_id: u32>]` | `approved: Address, live_until_ledger: u32` | approve |
| Position NFT: `ApproveForAll` | `["approve_for_all", <owner: Address>]` | `operator: Address, live_until_ledger: u32` | approve_for_all |
| Position NFT: `Mint` | `["mint", <to: Address>]` | `token_id: u32` | mint |
| Position NFT: `Burn` | `["burn", <from: Address>]` | `token_id: u32` | controller-authorized burn |
| Controller, governance, router, XOXNO: OwnershipTransfer | `["ownership_transfer"]` | `old_owner: Address, new_owner: Address, live_until_ledger: u32` | transfer_ownership; for governance and controller via TransferGovOwnership / TransferCtrlOwnership |
| Controller, governance, router, XOXNO; price aggregator constructor: OwnershipTransferCompleted | `["ownership_transfer_completed"]` | `new_owner: Address` | accept_ownership; controller, governance, price-aggregator constructors |
| Router, XOXNO: OwnershipRenounced | `["ownership_renounced"]` | `old_owner: Address` | renounce_ownership, only where still exported; builds from this source do not export it |
| Governance: RoleGranted | `["role_granted", <role: Symbol>, <account: Address>]` | `caller: Address` | constructor, GrantGovRole, accept_ownership role sync, canceller reset |
| Governance: RoleRevoked | `["role_revoked", <role: Symbol>, <account: Address>]` | `caller: Address` | RevokeGovRole, revoke_role_immediate, accept_ownership role sync, canceller reset |
| Governance: AdminTransferInitiated | `["admin_transfer_initiated", <current_admin: Address>]` | `new_admin: Address, live_until_ledger: u32` | TransferGovOwnership |
| Governance: AdminTransferCompleted | `["admin_transfer_completed", <new_admin: Address>]` | `previous_admin: Address` | constructor and accepted ownership |
| Controller: `Paused` | `["paused"]` | `{}` | pause, constructor and upgrade when not already paused |
| Controller: `Unpaused` | `["unpaused"]` | `{}` | unpause |
| Governance: MinDelayChanged | `["min_delay_changed"]` | `old_delay: u32, new_delay: u32` | constructor, UpdateGovDelay |
| Governance: OperationScheduled | `["operation_scheduled", <id: BytesN<32>>, <target: Address>]` | `function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>, delay: u32` | propose/propose_canceller_reset |
| Governance: OperationExecuted | `["operation_executed", <id: BytesN<32>>, <target: Address>]` | `function: Symbol, args: Vec<Val>, predecessor: BytesN<32>, salt: BytesN<32>` | execute/execute_self/execute_canceller_reset |
| Governance: OperationCancelled | `["operation_cancelled", <id: BytesN<32>>]` | `{}` | cancel |

RoleAdminChanged and AdminRenounced never emit (not exposed). Ownable `set_owner` is silent; router, XOXNO oracle and pool constructors use it. `update_current_contract` upgrades emit no custom event.

Decode token events under the token's standard; NFT events are not SEP-41. XOXNO submissions and router fee claims have no custom event ([endpoints](endpoints.md)).

## Source map

- [Controller events](../../contracts/controller/src/events/mod.rs), [config events](../../contracts/controller/src/events/config.rs), [market events](../../contracts/controller/src/events/market.rs), [liquidation application](../../contracts/controller/src/positions/liquidation/apply.rs), [cleanup](../../contracts/controller/src/positions/liquidation/bad_debt.rs).
- [Pool events](../../contracts/pool/src/events.rs), [governance events](../../contracts/governance/src/events.rs), [curator events](../../contracts/curator/src/events.rs), [swap-aggregator events](../../contracts/swap-aggregator/src/events.rs), [price registry](../../contracts/price-aggregator/src/registry.rs), [adapter](../../contracts/defindex-strategy/src/lib.rs), [NFT lifecycle](../../contracts/position-nft/src/contract.rs).
- [Shared types](../../common/src/types/mod.rs), [rate curve](../../common/src/rates/curve.rs), [dependency pins](../../Cargo.toml).
