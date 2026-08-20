# Event reference

The protocol publishes Soroban contract events from six of its nine contracts. Every event is a Rust struct annotated with `#[contractevent]`; the annotation fixes the event's topic vector, and the struct's fields become the event's data payload. A decoder reads the topics to route the event and then reads the data payload according to the struct's `data_format`. This page lists all **26** events, their topics, their data format, every field with its numeric scale, and the entrypoints that emit them.

## Reading these tables

**Topics.** The `topics = [...]` list in `#[contractevent]` is the literal topic vector. Each entry is a Soroban `Symbol`. No event in this codebase promotes a struct field into the topic vector, so the topic vector is a constant per event type and carries no data. Match on it exactly, in order.

**Data format.** The `#[contractevent]` macro supports three payload encodings:

- `map` (the default when `data_format` is not given) — the payload is an `ScMap`. Keys are the field names as `Symbol`s, **sorted alphabetically by field name**, not in declaration order. Read fields by key.
- `vec` — the payload is an `ScVec`. Entries are the fields in **declaration order**. Read fields by position.
- `single-value` — the struct has exactly one data field, and the payload *is* that field's value, with no wrapping map or vector.

Every event below states its format. Only `UpdatePositionBatchEvent` uses `vec`; only `PoolMarketStateBatchEvent` and `PoolMarketParamsBatchEvent` use `single-value`; everything else uses the default `map`.

**Nested types.** A `#[contracttype]` struct with **named** fields encodes as a map (keys sorted alphabetically). A `#[contracttype]` struct with **unnamed** fields (a tuple struct) encodes as a vector in declaration order. A `#[contracttype]` enum whose variants are all unit variants encodes as its `u32` discriminant.

**Scales and units.** Numeric fields are fixed-point integers. Never assume a scale; use the one in the table.

| Convention | Meaning |
| --- | --- |
| raw asset units | Integer in the token's own decimals. Divide by `10^asset_decimals` to get a human amount. The decimals are not carried in the event; read them from the token contract or from `CreateMarketEvent`'s market. |
| RAY (1e27) | `common/src/constants/shared.rs:5` — `RAY = 1_000_000_000_000_000_000_000_000_000`. Used for interest indexes, interest rates, utilization points, and scaled (share) balances. |
| WAD (1e18) | `common/src/constants/shared.rs:8` — `WAD = 1_000_000_000_000_000_000`. Used for USD values, health factors, and oracle sanity bounds. |
| bps | Basis points, `BPS = 10_000` (`common/src/constants/shared.rs:11`). 10000 = 100%. |
| ms | Milliseconds since the Unix epoch. Pool market timestamps are milliseconds, not seconds: `contracts/pool/src/time.rs:15` multiplies the Soroban ledger timestamp (seconds) by `MS_PER_SECOND`. |
| s | Seconds. Used only for oracle staleness windows. |

**Scaled vs actual amounts.** A "scaled amount" is a share balance in RAY. Multiply it by the matching RAY index and divide by RAY to get the actual asset amount in raw asset units. The events that carry a scaled amount always carry the index needed to convert it in the same record.

## PositionAction values

`PositionAction` is defined at `contracts/controller/src/events/mod.rs:54`. It is a unit-only `#[contracttype]` enum with `#[repr(u32)]`, so it appears on the wire as a plain `u32`. It tags each leg inside `UpdatePositionBatchEvent`.

| Variant | Value | Meaning |
| --- | --- | --- |
| `Supply` | 0 | Collateral added by a plain supply, or by the deposit leg of a strategy. Set at `contracts/controller/src/positions/supply.rs:361`. |
| `Borrow` | 1 | Debt taken by a plain borrow. Set at `contracts/controller/src/positions/debt.rs:147`. |
| `Withdraw` | 2 | Collateral removed by a plain withdraw. Set at `contracts/controller/src/positions/supply.rs:231`. |
| `Repay` | 3 | Debt repaid by a plain repay. Set at `contracts/controller/src/positions/debt.rs:96`. |
| `LiqRepay` | 4 | Debt retired on the liquidated account during a liquidation. Set at `contracts/controller/src/positions/liquidation/apply.rs:82`. |
| `LiqSeize` | 5 | Collateral debited from the liquidated account. **Gross of the protocol fee**, in both seize modes. Set at `contracts/controller/src/positions/liquidation/apply.rs:121` and `:184`. |
| `Multiply` | 6 | Debt borrowed by the leverage leg of `multiply`. Set at `contracts/controller/src/strategies/multiply.rs:79`. |
| `ParamUpd` | 7 | Supply position rewritten by a risk-parameter refresh; no funds move. Set at `contracts/controller/src/keepers.rs:193`. |
| `SwDebtR` | 8 | Both legs of `swap_debt`: the new borrow and the repay of the old debt. Set at `contracts/controller/src/strategies/swap_debt.rs:65` and `:87`. |
| `SwColWd` | 9 | Collateral withdrawn to be swapped by `swap_collateral`. Set at `contracts/controller/src/strategies/swap_collateral.rs:65`. |
| `RpColWd` | 10 | Collateral withdrawn to be swapped by `repay_debt_with_collateral`. Set at `contracts/controller/src/strategies/repay_debt_with_collateral.rs:134`. |
| `RpColR` | 11 | Debt repaid from the swap proceeds in `repay_debt_with_collateral`. Set at `contracts/controller/src/strategies/repay_debt_with_collateral.rs:146`. |
| `CloseWd` | 12 | Full withdrawal of a collateral position when a position is closed out. Set at `contracts/controller/src/strategies/legs.rs:138`. |
| `Migrate` | 13 | Position moved in from an external Blend pool. Set at `contracts/controller/src/strategies/migrate_blend.rs:150` and `:379`. |
| `RpColNet` | 14 | Collateral netted directly against same-asset debt, with no swap and no token movement. Set at `contracts/controller/src/strategies/repay_debt_with_collateral.rs:104`. |
| `LiqCredit` | 15 | Collateral credited to a share-credit liquidator's receiving account. **Net of the protocol fee.** Set at `contracts/controller/src/positions/liquidation/apply.rs:297`. |

`LiqSeize` and `LiqCredit` are deliberately distinct. In share-credit mode, the protocol fee equals `LiqSeize.amount - LiqCredit.amount`. In transfer mode the fee is withheld from the outbound token transfer and there is no `LiqCredit` leg. Reading a `LiqSeize` amount as the liquidator's proceeds overstates them by the fee.

## Shared payload types

These `#[contracttype]` types are not events themselves. They appear as fields inside events, so a decoder needs their wire shape.

### `EventPositionMode`

Defined at `contracts/controller/src/events/mod.rs:15`. Unit-only enum with `#[repr(u32)]`; encodes as a `u32`. It is the wire form of the internal `PositionMode`, with `PositionMode::Normal` mapped to `None`.

| Variant | Value | Meaning |
| --- | --- | --- |
| `None` | 0 | A normal, non-strategy position. |
| `Multiply` | 1 | A leveraged multiply position. |
| `Long` | 2 | A long position. |
| `Short` | 3 | A short position. |

### `EventAccountAttributes`

Defined at `contracts/controller/src/events/mod.rs:39`. A tuple struct, so it encodes as a **3-entry vector in this order**:

| Index | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| 0 | `Address` | — | The account's owner. |
| 1 | `u32` | — | The spoke id the account belongs to. |
| 2 | `EventPositionMode` | `u32` enum | The account's position mode. |

### `EventDepositDelta`

Defined at `contracts/controller/src/events/mod.rs:100`. A tuple struct, so it encodes as a **10-entry vector in this order**. Built by `EventDepositDelta::new` at `contracts/controller/src/events/mod.rs:117`.

| Index | Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- | --- |
| 0 | action | `PositionAction` | `u32` enum | Which operation produced this supply-side leg. |
| 1 | hub_id | `u32` | — | The hub the asset belongs to. |
| 2 | asset | `Address` | — | The asset's token contract. |
| 3 | scaled_amount | `i128` | RAY (1e27) | The account's supply position **after** the change, as a scaled share balance. |
| 4 | index_ray | `i128` | RAY (1e27) | The market's supply index at the moment of the change. Multiply `scaled_amount` by this and divide by RAY for the actual balance. |
| 5 | amount | `i128` | raw asset units | The size of this account's own movement. Never a counterparty's receipt. |
| 6 | liquidation_threshold | `u32` | bps | The position's stamped liquidation threshold. |
| 7 | liquidation_bonus | `u32` | bps | The position's stamped liquidation bonus. |
| 8 | loan_to_value | `u32` | bps | The position's stamped loan-to-value. |
| 9 | liquidation_fees | `u32` | bps | The position's stamped protocol liquidation fee. |

Fields 6-9 are the position's risk parameters, read from the stored position and truncated from `i128` to `u32`.

### `EventBorrowDelta`

Defined at `contracts/controller/src/events/mod.rs:144`. A tuple struct, so it encodes as a **6-entry vector in this order**. Built by `EventBorrowDelta::new` at `contracts/controller/src/events/mod.rs:156`.

| Index | Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- | --- |
| 0 | action | `PositionAction` | `u32` enum | Which operation produced this borrow-side leg. |
| 1 | hub_id | `u32` | — | The hub the asset belongs to. |
| 2 | asset | `Address` | — | The asset's token contract. |
| 3 | scaled_amount | `i128` | RAY (1e27) | The account's debt position **after** the change, as a scaled share balance. |
| 4 | index_ray | `i128` | RAY (1e27) | The market's borrow index at the moment of the change. |
| 5 | amount | `i128` | raw asset units | The size of this account's own debt movement. |

### `EventSpoke`

Defined at `contracts/controller/src/events/config.rs:13`. A named-field struct, so it encodes as a map with keys sorted alphabetically.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| spoke_id | `u32` | — | The spoke this snapshot describes. |
| is_deprecated | `bool` | — | True when the spoke accepts no new positions. |
| liquidation_target_hf_wad | `i128` | WAD (1e18) | The health factor a liquidation aims to restore the account to. |
| hf_for_max_bonus_wad | `i128` | WAD (1e18) | The health factor at or below which the liquidation bonus is maximal. |
| liquidation_bonus_factor_bps | `u32` | bps | Scaling factor applied to the liquidation bonus curve. |

### `SpokeAssetConfig`

Defined at `common/src/types/controller.rs:121`. Named-field struct; encodes as a map with alphabetically sorted keys.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| is_collateralizable | `bool` | — | True when the asset may be used as collateral in this spoke. |
| is_borrowable | `bool` | — | True when the asset may be borrowed in this spoke. |
| paused | `bool` | — | True blocks every user verb on the asset. |
| frozen | `bool` | — | True blocks entry (supply, borrow) but still allows exit. |
| no_seize | `bool` | — | True blocks only the liquidation seizure leg for this asset. |
| loan_to_value | `u32` | bps | Maximum borrowing power granted per unit of this collateral. |
| liquidation_threshold | `u32` | bps | Collateral ratio at which the position becomes liquidatable. |
| liquidation_bonus | `u32` | bps | Base discount a liquidator receives on this collateral. |
| liquidation_fees | `u32` | bps | Share of the liquidation bonus taken by the protocol. |
| supply_cap | `i128` | raw asset units | Maximum total supply of this asset in this spoke. Validated against the asset's decimals at `contracts/controller/src/config/asset.rs:84`. |
| borrow_cap | `i128` | raw asset units | Maximum total borrow of this asset in this spoke. |

### `MarketParamsRaw`

Defined at `common/src/types/pool.rs:16`. Named-field struct; encodes as a map with alphabetically sorted keys. All rate and utilization fields are RAY-scaled (`MarketParams` converts each one through `Ray::from`, `common/src/types/pool.rs:145`).

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| max_borrow_rate | `i128` | RAY (1e27) | Borrow rate at maximum utilization; the top of the curve. |
| base_borrow_rate | `i128` | RAY (1e27) | Borrow rate at zero utilization. |
| slope1 | `i128` | RAY (1e27) | Rate at the mid utilization breakpoint. |
| slope2 | `i128` | RAY (1e27) | Rate at the optimal utilization breakpoint. |
| slope3 | `i128` | RAY (1e27) | Rate at the max utilization breakpoint. |
| mid_utilization | `i128` | RAY (1e27) | First utilization breakpoint, as a fraction of 1 RAY. |
| optimal_utilization | `i128` | RAY (1e27) | Second utilization breakpoint. |
| max_utilization | `i128` | RAY (1e27) | Utilization ceiling enforced after borrows. |
| reserve_factor | `u32` | bps | Share of borrow interest booked as protocol revenue. Converted with `Bps::from` at `common/src/types/pool.rs:96`. |
| is_flashloanable | `bool` | — | True when flash loans are enabled for this market. |
| flashloan_fee | `u32` | bps | Flash-loan fee. Capped at `MAX_FLASHLOAN_FEE_BPS = 500` (`common/src/constants/shared.rs:39`). |
| asset_id | `Address` | — | The market's underlying token contract. |
| asset_decimals | `u32` | decimal places | The token's decimals. Use this to interpret every raw-asset-unit amount for this market. |

### `AssetOracle`

Defined at `common/src/types/composable_oracle.rs:186`. Named-field struct; encodes as a map with alphabetically sorted keys.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| asset_decimals | `u32` | decimal places | The decimal scale of the price this oracle returns. |
| max_price_stale_seconds | `u64` | s | Maximum age a blended price may have before it is rejected. |
| sources | `Vec<PriceSource>` | — | One or two inputs composed into the price. `PriceSource` (`common/src/types/composable_oracle.rs:180`) is an enum with variants `Feed`, `Scaled`, `AquariusLp`, and `AquariusStableLp`, each carrying a payload. |
| tolerance | `OracleTolerance` | — | The agreement band checked between two sources. |
| independence | `IndependencePolicy` | — | Enum at `common/src/types/composable_oracle.rs:200`: `RequireDisjoint`, or `AllowShared(Vec<Address>)`. |
| min_sanity_price_wad | `i128` | WAD (1e18) | Lower bound a resolved price must fall within. |
| max_sanity_price_wad | `i128` | WAD (1e18) | Upper bound a resolved price must fall within. |

### `PriceKey`

Defined at `common/src/types/composable_oracle.rs:22`. An enum with payload-carrying variants: `Token(Address)` names a token contract, and `Ref(Symbol)` names a synthetic reference used as an intermediate quote.

## Controller events

The controller defines **19** events, in `contracts/controller/src/events/`.

### `UpdatePositionBatchEvent`

- **Topics:** `["position", "batch_update"]`
- **Data format:** `vec` — the payload is a 4-entry vector in declaration order, **not** a map.
- **Defined at:** `contracts/controller/src/events/position.rs:17`
- **Emitted by:** `supply`, `withdraw`, `borrow`, `repay`, `liquidate`, `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend`, and `update_account_threshold`. Published from `Cache::emit_position_batch` at `contracts/controller/src/context/events.rs:53`, reached through `finalize_position_flow` (`contracts/controller/src/positions/mod.rs:257`) and from the keeper path at `contracts/controller/src/keepers.rs:220`.

| Index | Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- | --- |
| 0 | account_id | `u64` | — | The account whose positions changed. |
| 1 | account_attributes | `EventAccountAttributes` | 3-entry vector | Owner, spoke id, and position mode. See the shared-types section. |
| 2 | deposits | `Vec<EventDepositDelta>` | vector of 10-entry vectors | One entry per supply-side leg in this operation. May be empty. |
| 3 | borrows | `Vec<EventBorrowDelta>` | vector of 6-entry vectors | One entry per borrow-side leg in this operation. May be empty. |

**How to iterate.** Read the payload as a vector. Index 2 is itself a vector; each of its entries is a 10-entry vector matching the `EventDepositDelta` table above, so read element `k` of each inner vector by position. Index 3 is a vector of 6-entry vectors matching the `EventBorrowDelta` table. Both lists may be empty, but the event is not published when both are empty (`contracts/controller/src/context/events.rs:50`).

**One operation can publish more than one batch.** A `SeizeMode::Credit` liquidation writes two accounts and publishes the liquidated account's batch first and the receiving account's batch second (`contracts/controller/src/positions/liquidation/mod.rs:126` and `:140`). Key on `account_id`; do not assume one operation yields one batch.

### `LiquidationEvent`

- **Topics:** `["position", "liquidation"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/position.rs:42`
- **Emitted by:** `liquidate`, at `contracts/controller/src/positions/liquidation/mod.rs:107`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| liquidator | `Address` | — | The caller performing the liquidation. |
| account_id | `u64` | — | The account being liquidated. |
| repaid_usd_wad | `i128` | WAD (1e18) USD | The repayment the pool actually received, valued after the tokens moved: net of any refunded overpayment and net of any shortfall from an under-delivering debt token. It matches the debt actually retired, which also appears as the `LiqRepay` legs of the accompanying batch. |
| bonus_bps | `i128` | bps | The liquidation bonus applied. Sourced from `repayment.bonus.raw()`, a `Bps` value (`contracts/controller/src/positions/liquidation/math.rs:82`). |

This event carries no seizure or protocol-fee figure. Those live in the accompanying batch's `LiqSeize` legs (gross of fee) and, in share-credit mode, its `LiqCredit` legs (net of fee).

### `FlashLoanEvent`

- **Topics:** `["position", "flash_loan"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/position.rs:53`
- **Emitted by:** `flash_loan`, at `contracts/controller/src/strategies/flash_loan.rs:39`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| hub_id | `u32` | — | The hub the loaned asset belongs to. |
| asset | `Address` | — | The loaned token contract. |
| receiver | `Address` | — | The contract that received the funds and ran the callback. |
| caller | `Address` | — | The account that initiated the loan. |
| amount | `i128` | raw asset units | Principal lent out. |
| fee | `i128` | raw asset units | Fee charged on top of the principal, returned by the pool call. |

### `AccountDelegateEvent`

- **Topics:** `["account", "delegate"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/account.rs:8`
- **Emitted by:** `add_delegate` and `remove_delegate`, at `contracts/controller/src/account.rs:245`. Published only when the delegate list actually changed.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| account_id | `u64` | — | The account whose delegate list changed. |
| owner | `Address` | — | The account owner making the change. |
| delegate | `Address` | — | The delegate being granted or revoked. |
| granted | `bool` | — | True when the delegate was added, false when removed. |

### `CleanBadDebtEvent`

- **Topics:** `["debt", "bad_debt"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/debt.rs:10`
- **Emitted by:** `clean_bad_debt`, and by `liquidate` when the post-liquidation account still holds bad debt. Published at `contracts/controller/src/positions/liquidation/bad_debt.rs:54`.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| account_id | `u64` | — | The account being wound down and removed. |
| total_borrow_usd_wad | `i128` | WAD (1e18) USD | The account's total debt value before cleanup. |
| total_collateral_usd_wad | `i128` | WAD (1e18) USD | The account's total collateral value before cleanup. |

This event records no position deltas. The positions are seized on the pool and the account entry is removed; no `UpdatePositionBatchEvent` accompanies the cleanup.

### `CreateMarketEvent`

- **Topics:** `["market", "create"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/market.rs:14`
- **Emitted by:** `create_liquidity_pool`, at `contracts/controller/src/markets.rs:87`

The interest-rate fields are copied flat from `MarketParamsRaw` by `CreateMarketEvent::from_params` (`contracts/controller/src/events/market.rs:34`). The flash-loan flag, flash-loan fee, and asset decimals are **not** copied.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| hub_id | `u32` | — | The hub the new market belongs to. |
| base_asset | `Address` | — | The market's underlying token contract. |
| max_borrow_rate | `i128` | RAY (1e27) | Borrow rate at maximum utilization. |
| base_borrow_rate | `i128` | RAY (1e27) | Borrow rate at zero utilization. |
| slope1 | `i128` | RAY (1e27) | Rate at the mid utilization breakpoint. |
| slope2 | `i128` | RAY (1e27) | Rate at the optimal utilization breakpoint. |
| slope3 | `i128` | RAY (1e27) | Rate at the max utilization breakpoint. |
| mid_utilization | `i128` | RAY (1e27) | First utilization breakpoint. |
| optimal_utilization | `i128` | RAY (1e27) | Second utilization breakpoint. |
| max_utilization | `i128` | RAY (1e27) | Utilization ceiling. |
| reserve_factor | `u32` | bps | Share of borrow interest booked as protocol revenue. |
| market_address | `Address` | — | The pool contract address serving this market. |

### `UpdateMarketParamsEvent`

- **Topics:** `["market", "params_update"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/market.rs:63`
- **Emitted by:** `upgrade_liquidity_pool_params`, at `contracts/controller/src/markets.rs:108`

Built from an `InterestRateModel` by `UpdateMarketParamsEvent::from_rate_model` (`contracts/controller/src/events/market.rs:81`). The flash-loan flag and flash-loan fee are **not** copied.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| hub_id | `u32` | — | The hub the market belongs to. |
| asset | `Address` | — | The market's underlying token contract. |
| max_borrow_rate | `i128` | RAY (1e27) | Borrow rate at maximum utilization. |
| base_borrow_rate | `i128` | RAY (1e27) | Borrow rate at zero utilization. |
| slope1 | `i128` | RAY (1e27) | Rate at the mid utilization breakpoint. |
| slope2 | `i128` | RAY (1e27) | Rate at the optimal utilization breakpoint. |
| slope3 | `i128` | RAY (1e27) | Rate at the max utilization breakpoint. |
| mid_utilization | `i128` | RAY (1e27) | First utilization breakpoint. |
| optimal_utilization | `i128` | RAY (1e27) | Second utilization breakpoint. |
| max_utilization | `i128` | RAY (1e27) | Utilization ceiling. |
| reserve_factor | `u32` | bps | Share of borrow interest booked as protocol revenue. |

### `InitialMultiplyPaymentEvent`

- **Topics:** `["strategy", "initial_payment"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/strategy.rs:7`
- **Emitted by:** `multiply`, at `contracts/controller/src/strategies/multiply.rs:235`. Published only when the caller supplied an initial payment.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| token | `Address` | — | The token contract the caller paid in. |
| amount | `i128` | raw asset units | The amount paid in, before conversion into the position's collateral asset. |
| account_id | `u64` | — | The account the multiply position belongs to. |

### `BlendMigrationEvent`

- **Topics:** `["strategy", "blend_migration"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/strategy.rs:18`
- **Emitted by:** `migrate_from_blend`, at `contracts/controller/src/strategies/migrate_blend.rs:107`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| account_id | `u64` | — | The account the positions were migrated into. |
| blend_pool | `Address` | — | The external Blend pool migrated from. |
| collateral_count | `u32` | count | Number of collateral positions moved. |
| supply_count | `u32` | count | Number of non-collateral supply positions moved. |
| debt_count | `u32` | count | Number of debt positions moved. |

### `UpdateSpokeEvent`

- **Topics:** `["config", "spoke"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:38`
- **Emitted by:** `add_spoke` (`contracts/controller/src/config/spoke.rs:29`), `remove_spoke` (`:46`), and `set_spoke_liquidation_curve` (`:75`)

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| spoke | `EventSpoke` | map | The spoke's post-change configuration snapshot. See the shared-types section. |

### `UpdateSpokeAssetEvent`

- **Topics:** `["config", "spoke_asset"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:46`
- **Emitted by:** `add_asset_to_spoke` and `edit_asset_in_spoke` (both via `store_spoke_asset`, `contracts/controller/src/config/asset.rs:116`), and `set_spoke_asset_flags` (`contracts/controller/src/config/asset.rs:145`)

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| asset | `Address` | — | The asset's token contract. |
| config | `SpokeAssetConfig` | map | The asset's full post-change configuration. See the shared-types section. |
| spoke_id | `u32` | — | The spoke the listing belongs to. |
| hub_id | `u32` | — | The hub the asset belongs to. |

### `RemoveSpokeAssetEvent`

- **Topics:** `["config", "remove_spoke_asset"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:56`
- **Emitted by:** `remove_asset_from_spoke`, at `contracts/controller/src/config/asset.rs:190`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| asset | `Address` | — | The asset's token contract. |
| spoke_id | `u32` | — | The spoke the listing was removed from. |
| hub_id | `u32` | — | The hub the asset belongs to. |

### `ApproveBlendPoolEvent`

- **Topics:** `["config", "approve_blend_pool"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:65`
- **Emitted by:** `approve_blend_pool` and `revoke_blend_pool`, both via `set_blend_pool_approval` at `contracts/controller/src/config/registry.rs:48`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| pool | `Address` | — | The Blend pool contract. |
| approved | `bool` | — | True when the pool is approved as a migration source, false when revoked. |

### `UpdateSwapAggregatorEvent`

- **Topics:** `["config", "swap_aggregator"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:73`
- **Emitted by:** `set_swap_aggregator`, at `contracts/controller/src/config/registry.rs:16`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| swap_aggregator | `Address` | — | The new swap aggregator contract used by strategy swaps. |

### `UpdatePriceAggregatorEvent`

- **Topics:** `["config", "price_aggregator"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:80`
- **Emitted by:** `set_price_aggregator`, at `contracts/controller/src/config/registry.rs:26`. Governance also calls this entrypoint from `deploy_price_aggregator` (`contracts/governance/src/deploy.rs:83`).

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| price_aggregator | `Address` | — | The new price aggregator contract used for oracle lookups. |

### `UpdateAccumulatorEvent`

- **Topics:** `["config", "accumulator"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:87`
- **Emitted by:** `set_accumulator`, at `contracts/controller/src/config/registry.rs:35`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| accumulator | `Address` | — | The new address that receives claimed protocol revenue. |

### `UpdatePositionLimitsEvent`

- **Topics:** `["config", "position_limits"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:95`
- **Emitted by:** `set_position_limits`, at `contracts/controller/src/config/registry.rs:63`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| max_supply_positions | `u32` | count | Maximum concurrent supply positions one account may hold. |
| max_borrow_positions | `u32` | count | Maximum concurrent borrow positions one account may hold. |

### `UpdateMinBorrowCollateralEvent`

- **Topics:** `["config", "min_borrow_collateral"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:104`
- **Emitted by:** `set_min_borrow_collateral_usd`, at `contracts/controller/src/config/registry.rs:76`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| min_borrow_collateral_usd_wad | `i128` | WAD (1e18) USD | Minimum collateral value required to open a new borrow position. |

### `CreateHubEvent`

- **Topics:** `["config", "hub"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/controller/src/events/config.rs:111`
- **Emitted by:** `create_hub`, at `contracts/controller/src/config/spoke.rs:87`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| hub_id | `u32` | — | The id of the newly created hub. |

## Pool events

The pool defines **3** events, in `contracts/pool/src/events.rs`.

### `PoolMarketStateBatchEvent`

- **Topics:** `["market", "batch_state_update"]`
- **Data format:** `single-value` — the struct has one field, `updates`, so the payload **is** the vector of rows. There is no wrapping map and no `updates` key.
- **Defined at:** `contracts/pool/src/events.rs:48`
- **Emitted by:** `supply`, `borrow`, `withdraw`, `repay`, and `seize_positions` (batched, via `run_batch`/`run_batch_without_result` at `contracts/pool/src/ops/mod.rs:70` and `:89`); `update_indexes` (`contracts/pool/src/ops/market.rs:78`); `claim_revenue` (`contracts/pool/src/ops/revenue.rs:27` and `:37`); `flash_loan` (`contracts/pool/src/ops/flash.rs:143`); `recapitalize` (`contracts/pool/src/ops/recapitalize.rs:36`); `create_strategy` (`contracts/pool/src/ops/strategy.rs:49`); and `net_settle` (`contracts/pool/src/lib.rs:250`). Not published when the snapshot list is empty (`contracts/pool/src/events.rs:84`).

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| updates | `Vec<PoolMarketStateEvent>` | vector of 9-entry vectors | One row per market touched by the operation. |

`PoolMarketStateEvent` (`contracts/pool/src/events.rs:16`) is a tuple struct, so each row is a **9-entry vector in this order**. Values come from `Cache::snapshot` at `contracts/pool/src/cache/report.rs:35`.

| Index | Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- | --- |
| 0 | hub_id | `u32` | — | The hub the market belongs to. |
| 1 | asset | `Address` | — | The market's underlying token contract. |
| 2 | timestamp | `u64` | ms | Ledger time stamped on this snapshot. Milliseconds, from `time::now_ms` (`contracts/pool/src/time.rs:15`). |
| 3 | supply_index | `i128` | RAY (1e27) | Supply exchange-rate index after the operation. |
| 4 | borrow_index | `i128` | RAY (1e27) | Borrow exchange-rate index after the operation. |
| 5 | cash | `i128` | raw asset units | Underlying tokens the pool holds for this market. |
| 6 | supplied | `i128` | RAY (1e27) | Total supply **shares** outstanding. Multiply by `supply_index` and divide by RAY for the actual supplied amount. Stored as `Ray` (`contracts/pool/src/cache/mod.rs:33`). |
| 7 | borrowed | `i128` | RAY (1e27) | Total debt **shares** outstanding. Multiply by `borrow_index` and divide by RAY for the actual borrowed amount. |
| 8 | revenue | `i128` | RAY (1e27) | Accrued protocol revenue, held as supply shares. Convert with `supply_index`. |

### `PoolMarketParamsBatchEvent`

- **Topics:** `["market", "batch_params_update"]`
- **Data format:** `single-value` — the payload **is** the vector of rows.
- **Defined at:** `contracts/pool/src/events.rs:64`
- **Emitted by:** `create_market` (`contracts/pool/src/ops/market.rs:46`) and `update_params` (`contracts/pool/src/ops/market.rs:59`), both via `emit_market_params` (`contracts/pool/src/events.rs:101`). Both paths publish exactly one row.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| updates | `Vec<PoolMarketParamsEvent>` | vector of maps | One row per market configured. |

`PoolMarketParamsEvent` (`contracts/pool/src/events.rs:55`) has named fields, so each row is a map with alphabetically sorted keys.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| hub_id | `u32` | — | The hub the market belongs to. |
| asset | `Address` | — | The market's underlying token contract. |
| params | `MarketParamsRaw` | map | The market's full post-change parameters, including flash-loan settings and asset decimals. See the shared-types section. |

### `StrategyFeeEvent`

- **Topics:** `["strategy", "fee"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/pool/src/events.rs:71`
- **Emitted by:** `create_strategy`, at `contracts/pool/src/ops/strategy.rs:40`. Published only when the fee is non-zero (`contracts/pool/src/events.rs:123`).

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| hub_id | `u32` | — | The hub the asset belongs to. |
| asset | `Address` | — | The asset's token contract. |
| amount | `i128` | raw asset units | Gross strategy principal, before the fee. |
| fee | `i128` | raw asset units | Protocol fee withheld. |
| amount_sent | `i128` | raw asset units | Net amount transferred to the receiver, equal to `amount - fee`. |

## Governance events

Governance defines **2** events, in `contracts/governance/src/events.rs`.

### `DeployControllerEvent`

- **Topics:** `["governance", "deploy_controller"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/governance/src/events.rs:10`
- **Emitted by:** `deploy_controller` (`contracts/governance/src/api.rs:26`), published at `contracts/governance/src/deploy.rs:46`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| controller | `Address` | — | The address the controller was deployed to. |
| wasm_hash | `BytesN<32>` | 32 raw bytes | The wasm hash the controller was deployed from. |

### `DeployPriceAggregatorEvent`

- **Topics:** `["governance", "deploy_price_aggregator"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/governance/src/events.rs:19`
- **Emitted by:** `deploy_price_aggregator` (`contracts/governance/src/api.rs:40`), published at `contracts/governance/src/deploy.rs:87`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| price_aggregator | `Address` | — | The address the price aggregator was deployed to. |
| wasm_hash | `BytesN<32>` | 32 raw bytes | The wasm hash the price aggregator was deployed from. |

## Price-aggregator events

The price aggregator defines **1** event.

### `UpdateAssetOracleEvent`

- **Topics:** `["config", "asset_oracle"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/price-aggregator/src/registry.rs:96`
- **Emitted by:** `set_oracle` (via `registry::emit`, `contracts/price-aggregator/src/admin.rs:67`), `set_sanity_band` (via `registry::commit`, `contracts/price-aggregator/src/admin.rs:191`), and `set_tolerance` (via `registry::commit`, `contracts/price-aggregator/src/admin.rs:210`). All three publish from `registry::emit` at `contracts/price-aggregator/src/registry.rs:87`.

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| key | `PriceKey` | enum with payload | The asset whose oracle configuration changed. See the shared-types section. |
| oracle | `AssetOracle` | map | The full post-change oracle configuration. See the shared-types section. |

## DeFindex-strategy events

The DeFindex strategy adapter defines **1** event.

### `HarvestEvent`

- **Topics:** `["strategy", "harvest"]`
- **Data format:** `map` (default)
- **Defined at:** `contracts/defindex-strategy/src/lib.rs:25`
- **Emitted by:** `harvest`, via `emit_harvest` at `contracts/defindex-strategy/src/lib.rs:311`

| Field | Type | Scale/unit | Meaning |
| --- | --- | --- | --- |
| from | `Address` | — | The caller that invoked `harvest`. |
| amount | `i128` | raw asset units | Always `0`. `harvest` moves no funds; the call site passes a literal zero (`contracts/defindex-strategy/src/lib.rs:311`). |
| price_per_share | `i128` | 12 decimals (1e12) | Current price per share for the configured hub asset. Computed by floor-rescaling the RAY supply index down to `PPS_DECIMALS = 12` (`contracts/defindex-strategy/src/lib.rs:42` and `:192`). |

## Contracts that emit no events

These three contracts contain zero `#[contractevent]` definitions and publish no contract events:

- **position-nft** (`contracts/position-nft/`) — position ownership changes are observable through the standard SEP-41/non-fungible token transfer semantics of its base library, not through any event this repository defines.
- **swap-aggregator** (`contracts/swap-aggregator/`) — swap routing and execution emit no protocol events; observe the underlying token transfers instead.
- **xoxno-oracle** (`contracts/xoxno-oracle/`) — price submissions and reads emit no events.
