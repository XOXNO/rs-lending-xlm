# Error code reference

Every fallible entry point in this protocol fails by panicking with a numeric
contract error. Soroban surfaces that number to the caller as a contract error
(`Error(Contract, #<code>)` in the RPC response, `Err(Ok(soroban_sdk::Error))`
from a `try_` client call), so the code is all an integrator gets back — there
is no message. The codes are stable: they are explicit discriminants on the
enums in [`common/src/errors.rs`](../../common/src/errors.rs) and are never
renumbered, so gaps in the sequence are expected and deliberate. There are
**121 codes** across six enums, grouped by domain.

Read a row like this: **Code** is the number on the wire, **Name** is the Rust
variant, **Raised when** is the condition the contract actually checks,
**What to do** is the caller's remedy, and **Raised by** lists the public entry
points that can surface it. "Public entry point" means a method on one of the
contract interface traits in `interfaces/` or a `pub fn` in a
`contracts/*/src/lib.rs` — the functions an external account or contract can
invoke.

Terms used below:

- **Hub asset** identifies a market; **spoke** identifies an account's risk
  regime (its caps, collateral flags, and liquidation curve).
- **WAD** is 10^18, **RAY** is 10^27, **BPS** is 10,000.
- **Scaled shares** are index-relative position units; an amount too small to
  move the index by one unit rounds to zero shares and is rejected.
- **TWAP** is a time-weighted average price computed over N stored oracle
  observations.
- **Attestation** is the one-time validation the price aggregator runs against
  a live provider contract when an oracle configuration is registered.

## How error codes are grouped

| Enum | Code range | Domain | Source file |
|---|---|---|---|
| `GenericError` | 1–55 | Contract setup, registry, accounts, timelock, roles, arithmetic | `common/src/errors.rs` |
| `CollateralError` | 100–135 | Collateral, positions, interest-rate curves, liquidation | `common/src/errors.rs` |
| `OracleError` | 201–235 | Oracle configuration, price feeds, staleness, sanity bounds | `common/src/errors.rs` |
| `SpokeError` | 300–318 | Spoke registration and per-spoke asset listings | `common/src/errors.rs` |
| `FlashLoanError` | 400–412 | Flash-loan execution and repayment | `common/src/errors.rs` |
| `StrategyError` | 500–505 | Strategy conversion, swap routing, flash-position | `common/src/errors.rs` |

## All error codes

### `GenericError` (41 codes)

| Code | Name | Raised when | What to do | Raised by |
|---|---|---|---|---|
| 1 | `AssetNotSupported` | Never constructed in the codebase. | — | Unused; see [Unused variants](#unused-variants) |
| 2 | `AssetAlreadySupported` | A market already exists for this hub asset (`contracts/pool/src/ops/market.rs:28`). | Use the existing market. | pool `create_market`; controller `create_liquidity_pool` |
| 3 | `InvalidTicker` | Never constructed in contract code; only the RedStone test mock returns it (`tests/test-harness/src/mock_redstone.rs:31`). | — | Unused in production; see [Unused variants](#unused-variants) |
| 5 | `PoolAlreadyDeployed` | The singleton pool, controller, or price aggregator address is already recorded (`contracts/controller/src/markets.rs:25`, `contracts/governance/src/deploy.rs:37`). | Nothing; deployment already happened. | controller `deploy_pool`; governance `deploy_controller`, `deploy_price_aggregator` |
| 6 | `InvalidAsset` | The token reports no decimals or symbol, or the declared decimals do not match the token (`contracts/governance/src/validate/asset.rs:15`). | Pass a real token and its true decimals. | governance `propose`, `execute`, `execute_self` for `CreateLiquidityPool` and `AddAssetToSpoke` |
| 7 | `AssetsAreTheSame` | The source and destination assets of a swap or leverage leg are the same address (`contracts/controller/src/strategies/swap_collateral.rs:45`, `contracts/controller/src/strategies/swap_debt.rs:46`, `contracts/controller/src/strategies/multiply.rs:164`). | Pick two distinct assets. | controller `swap_collateral`, `swap_debt`, `multiply`, `migrate_from_blend` |
| 8 | `WrongToken` | `params.asset_id` does not equal the asset the market is being created for (`contracts/controller/src/markets.rs:77`, `contracts/governance/src/validate/asset.rs:48`). | Make `params.asset_id` match the asset. | controller `create_liquidity_pool`; governance `CreateLiquidityPool` |
| 10 | `InvalidWasmHash` | The supplied Wasm hash is all zero bytes (`contracts/governance/src/validate/mod.rs:27`). | Pass the hash of an uploaded Wasm. | governance `deploy_controller`, `deploy_price_aggregator` |
| 11 | `InvalidExchangeSrc` | Never constructed in the codebase. | — | Unused; see [Unused variants](#unused-variants) |
| 12 | `PairNotActive` | Never constructed in the codebase. | — | Unused; see [Unused variants](#unused-variants) |
| 13 | `AccountNotInMarket` | The caller does not own the account's position NFT, or the account has no metadata record (`contracts/controller/src/account.rs:112`, `contracts/controller/src/storage/account.rs:50`). | Use an account you own. | controller `add_delegate`, `remove_delegate`, `renew_account`, and any entry point that loads an account |
| 14 | `AmountMustBePositive` | An amount argument is zero or negative, or a measured transfer delta is not positive (`common/src/validation.rs:20`, `common/src/token.rs:33`). | Send a strictly positive amount. | All controller mutating entry points (`supply`, `borrow`, `withdraw`, `repay`, `multiply`, swaps, `recapitalize`) |
| 16 | `InvalidPayments` | A payments or swap-route list is empty where one is required, non-empty where forbidden, or longer than the view limit (`common/src/validation.rs:39`, `contracts/controller/src/strategies/swap.rs:24`, `contracts/controller/src/views.rs:20`). | Fix the length of the payments or route list. | controller `supply`, `repay`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend`, batch view getters |
| 18 | `NotSmartContract` | The supplied address is not a deployed contract (`contracts/governance/src/validate/mod.rs:21`). | Pass a contract address, not an account. | governance `propose`, `execute_self` for `ApproveBlendPool`, `RevokeBlendPool`, `TransferCtrlOwnership` |
| 24 | `AccountNotFound` | The account id has no stored state, or its position-NFT owner cannot be resolved (`contracts/controller/src/storage/account.rs:39`, `contracts/controller/src/external/position_nft.rs:15`). | Open a position first. | Every controller entry point that takes an `account_id` |
| 25 | `AccountModeMismatch` | The account's position mode differs from the mode the call requires, or a liquidation receiver is not in normal mode (`contracts/controller/src/account.rs:72`, `contracts/controller/src/positions/liquidation/mod.rs:210`). | Use an account in the matching mode. | controller `multiply`, `liquidate` |
| 27 | `AggregatorNotSet` | The swap aggregator or price aggregator address has not been configured (`contracts/controller/src/storage/protocol.rs:46`). | Wait for governance to set the aggregator. | controller strategy entry points and any entry point that reads prices |
| 29 | `PositionLimitsNotSet` | No position limits are stored in controller instance storage (`contracts/controller/src/storage/protocol.rs:88`). | Wait for governance to set position limits. | All controller entry points that open a position |
| 30 | `PoolNotInitialized` | The pool or controller address is unset, or the market's params or state record does not exist (`contracts/controller/src/storage/protocol.rs:31`, `contracts/pool/src/storage.rs:27`). | Wait for the market to be created. | Nearly all controller and pool entry points |
| 32 | `OwnerNotSet` | Contract ownership has never been assigned (`contracts/pool/src/ops/revenue.rs:28`, `contracts/governance/src/access.rs:67`). | Initialize ownership first. | pool `claim_revenue`; controller and governance owner-gated entry points |
| 33 | `MathOverflow` | A checked arithmetic operation overflows, a narrowing conversion fails, or a decimal rescale exceeds `i128` (`common/src/math/fp.rs:15`, `common/src/math/fp_core.rs:119`). | Use a smaller amount. | All controller and pool mutating entry points |
| 34 | `InternalError` | An internal invariant fails: an expected value is absent, liquidation math is inconsistent, a timelock target is wrong, or a migration version does not increase (`common/src/validation.rs:33`, `contracts/controller/src/positions/liquidation/math.rs:56`, `contracts/governance/src/timelock/lifecycle.rs:78`). | Report it; the inputs are inconsistent. | controller `migrate`, `liquidate`; governance `execute`, `execute_self`; pool revenue paths |
| 36 | `InvalidPositionLimits` | A supply or borrow position limit is zero or above `POSITION_LIMIT_MAX` (`contracts/controller/src/config/registry.rs:54`). | Use limits inside the allowed range. | controller `set_position_limits`; governance position-limit and `AddAssetToSpoke` operations |
| 38 | `SpotOnlyNotProductionSafe` | Every configured oracle source reads an unsmoothed spot market, including a single-source oracle whose only leg is one (`contracts/price-aggregator/src/validation.rs:59`). | Add a TWAP or otherwise smoothed source. | price-aggregator `set_oracle` |
| 39 | `InvalidTimelockDelay` | The delay is zero, below the current minimum, or above `TIMELOCK_MAX_DELAY_LEDGERS` (`contracts/governance/src/timelock/mod.rs:61`). | Choose a delay inside the allowed range. | governance `execute_self` (`AdminOperation::UpdateGovDelay`), governance constructor |
| 40 | `TimelockOperationExpired` | The scheduled operation's grace period has already elapsed (`contracts/governance/src/timelock/mod.rs:96`). | Propose the operation again. | governance `execute`, `execute_self`, `execute_canceller_reset` |
| 41 | `InvalidRole` | The role symbol is not a known governance role, the grant would combine executor and canceller, or the role is not held on revoke (`contracts/governance/src/access.rs:47`, `:161`, `:210`). | Use a valid role that the account holds. | governance `propose`, `execute_self` (grant/revoke role), `revoke_role_immediate` |
| 42 | `BlendPoolNotApproved` | The target Blend pool is not on the controller's approved list (`contracts/controller/src/strategies/migrate_blend.rs:196`). | Ask governance to approve the pool. | controller `migrate_from_blend` |
| 43 | `HubNotActive` | The hub id does not exist or has been deactivated (`contracts/controller/src/config/spoke.rs:93`). | Use an active hub. | All controller position and strategy entry points, plus `create_liquidity_pool` |
| 44 | `NotAuthorized` | The caller is neither the account owner nor an active delegate, or a governance action illegally targets itself (`contracts/controller/src/account.rs:104`, `contracts/governance/src/timelock/lifecycle.rs:35`). | Call as the owner or an approved delegate. | controller `supply`, `borrow`, `withdraw`, `multiply`, `liquidate`, `add_delegate`, `remove_delegate`; governance `propose` and revoke-role operations |
| 45 | `RegistryCapReached` | The account already has `MAX_DELEGATES` delegates (`contracts/controller/src/storage/account.rs:212`). | Remove a delegate first. | controller `add_delegate` |
| 46 | `OperationNotCancellable` | The operation is a recovery operation, or the canceller is the account the operation would revoke (`contracts/governance/src/timelock/lifecycle.rs:120`). | Have a different canceller cancel it. | governance `cancel` |
| 47 | `BorrowRoundsToZeroShares` | A positive borrow amount mints zero scaled debt shares (`contracts/pool/src/ops/borrow.rs:70`). | Borrow a larger amount. | controller `borrow`, `multiply`, `swap_debt`; pool `borrow` |
| 48 | `CannotRemoveLastProposer` | Revoking the role would leave the proposer role with no holders (`contracts/governance/src/access.rs:218`). | Grant another proposer first. | governance `execute_self` (`RevokeGovRole`) |
| 49 | `WithdrawRoundsToZeroShares` | A positive withdrawal burns zero scaled supply shares (`contracts/pool/src/ops/withdraw.rs:83`). | Withdraw a larger amount. | controller `withdraw`, `swap_collateral`, strategy closes; pool `withdraw` |
| 50 | `NetSettleRoundsToZeroShares` | A positive net settlement burns zero supply or debt shares (`contracts/pool/src/ops/net_settle.rs:38`). | Settle a larger amount. | controller `repay_debt_with_collateral` (same-asset); pool `net_settle` |
| 51 | `SupplyRoundsToZeroShares` | A positive supply amount mints zero scaled shares (`contracts/pool/src/ops/supply.rs:29`). | Supply a larger amount. | controller `supply`, `multiply`, strategies; pool `supply` |
| 52 | `RepayRoundsToZeroShares` | A positive net repayment burns zero scaled debt shares (`contracts/pool/src/ops/repay.rs:49`). | Repay a larger amount. | controller `repay`, `swap_debt`, `repay_debt_with_collateral`; pool `repay` |
| 53 | `PositionNftNotSet` | The position-NFT contract address is unset in controller storage (`contracts/controller/src/storage/protocol.rs:120`). | Wait for governance to deploy the position NFT. | Every controller entry point that resolves an account owner |
| 54 | `PositionNftAlreadyDeployed` | A position-NFT contract address is already recorded (`contracts/controller/src/markets.rs:52`). | Nothing; deployment already happened. | controller `deploy_position_nft` |
| 55 | `DivisionByZero` | A fixed-point multiply-divide received a zero denominator (`common/src/math/fp_core.rs:29`). Distinct from `MathOverflow`, which the same operations raise when the result does not fit `i128`. | Report it; a zero index or denominator is an internal inconsistency. | Any path that scales by a market index or a configured ratio |

### `CollateralError` (30 codes)

| Code | Name | Raised when | What to do | Raised by |
|---|---|---|---|---|
| 100 | `InsufficientCollateral` | After the operation, LTV-weighted collateral falls below total debt or the health factor drops below 1 WAD (`contracts/controller/src/risk/validation.rs:46`). | Supply more collateral or repay debt. | controller `borrow`, `withdraw`, `supply`, `repay`, `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend`, `flash_loan` |
| 101 | `HealthFactorTooHigh` | The target account has no debt, or its health factor is at least 1 WAD (`contracts/controller/src/positions/liquidation/plan.rs:26`, `:48`). | Only liquidate unhealthy accounts. | controller `liquidate`, `get_liquidation_estimate` |
| 102 | `HealthFactorTooLow` | After refreshing thresholds the health factor is below the minimum required to accept the update (`contracts/controller/src/keepers.rs:226`). | Improve the health factor first. | controller `update_account_threshold` |
| 104 | `NotCollateral` | The spoke listing does not allow the asset to be used as collateral (`contracts/controller/src/positions/mod.rs:285`). | Choose a collateral-enabled asset. | controller `supply`, `multiply`, `swap_collateral`, `migrate_from_blend` |
| 107 | `AssetNotBorrowable` | The spoke listing does not allow the asset to be borrowed (`contracts/controller/src/positions/mod.rs:268`). | Choose a borrow-enabled asset. | controller `borrow`, `multiply`, `swap_debt`, `migrate_from_blend`, `flash_loan` |
| 109 | `PositionLimitExceeded` | The new supply or borrow position would push the account past its configured position count (`contracts/controller/src/risk/validation.rs:107`). | Close or consolidate positions. | controller `supply`, `borrow`, strategies, `liquidate` |
| 110 | `PositionNotFound` | Never constructed in contract code. | — | Unused; see [Unused variants](#unused-variants) |
| 111 | `InvalidPositionMode` | The requested position mode is not Multiply, Long, or Short (`contracts/controller/src/strategies/multiply.rs:174`). | Use Multiply, Long, or Short. | controller `multiply` |
| 112 | `InsufficientLiquidity` | Market cash is below the requested draw, or the draw would break the liquidation buffer (`contracts/pool/src/cache/cash.rs:18`, `contracts/pool/src/guards.rs:40`). | Reduce the amount or wait for liquidity. | pool `borrow`, `withdraw`, `flash_loan`; controller `borrow`, `withdraw`, `flash_loan`, strategies |
| 113 | `InvalidLiqThreshold` | The liquidation fee is at or above BPS, the threshold is at or below the LTV or above BPS, or threshold times (BPS plus bonus) exceeds BPS squared (`common/src/validation.rs:91`, `:105`, `:110`). | Fix the risk bounds in the proposal. | controller `add_asset_to_spoke`, `edit_asset_in_spoke`; governance `propose`, `execute`, `execute_self` |
| 114 | `CannotCleanBadDebt` | The account does not pass the bad-debt gate: its leftover debt is not dust-capped, or debt does not exceed collateral (`contracts/controller/src/positions/liquidation/mod.rs:259`). | Only clean genuinely bad debt. | controller `clean_bad_debt`, `force_socialize_bad_debt` |
| 115 | `WithdrawLessThanFee` | The liquidation protocol fee is larger than the gross amount withdrawn (`contracts/pool/src/ops/withdraw.rs:123`). | Seize a larger amount. | pool `withdraw`, reached through controller `liquidate` |
| 116 | `InvalidBorrowParams` | A cap or floor is negative, a cap exceeds the asset's decimal domain, or the flash-loan fee is above the maximum (`common/src/validation.rs:69`, `contracts/governance/src/validate/asset.rs:70`). | Use non-negative, in-range parameters. | controller `add_asset_to_spoke`, `edit_asset_in_spoke`, `set_min_borrow_collateral_usd`, `create_liquidity_pool`, `upgrade_liquidity_pool_params`; pool `create_market`, `update_params`; governance `propose`, `execute` |
| 117 | `InvalidUtilRange` | `mid_utilization` is zero, `optimal_utilization` is not above it, or `max_utilization` is below optimal or above RAY (`InterestRateModel::verify` in `common/src/types/pool.rs`). | Order the utilization breakpoints correctly. | pool `create_market`, `update_params`; controller `create_liquidity_pool`, `upgrade_liquidity_pool_params`; governance `execute` |
| 118 | `OptUtilTooHigh` | Optimal utilization is at or above RAY, that is 100% (`common/src/types/pool.rs:200`). | Set optimal utilization below 100%. | Same rate-model entry points as code 117 |
| 119 | `InvalidReserveFactor` | The reserve factor is at or above BPS, that is 100% (`common/src/types/pool.rs:208`). | Set the reserve factor below 100%. | Same rate-model entry points as code 117 |
| 120 | `DebtPositionNotFound` | The account has no debt position for the referenced hub asset (`contracts/controller/src/positions/mod.rs:377`). | Reference an asset the account owes. | controller `repay`, `liquidate`, `swap_debt`, `repay_debt_with_collateral`, `clean_bad_debt`, `force_socialize_bad_debt` |
| 121 | `CollateralPositionNotFound` | The account has no supply position for the referenced hub asset (`contracts/controller/src/positions/mod.rs:363`). | Reference an asset the account supplied. | controller `withdraw`, `swap_collateral`, `repay_debt_with_collateral`, `liquidate` |
| 122 | `CannotCloseWithRemainingDebt` | `close_position` is requested while borrow positions remain open (`contracts/controller/src/strategies/repay_debt_with_collateral.rs:150`). | Repay all debt before closing. | controller `repay_debt_with_collateral` |
| 123 | `PoolInsolvent` | Supplier claims exceed cash plus outstanding debt, or supply is zero while debt remains (`contracts/pool/src/guards.rs:51`, `:66`). | Recapitalize or clean bad debt first. | pool `supply`, `withdraw`, `net_settle`, `claim_revenue`; controller `supply`, `withdraw`, `repay`, `claim_revenue` |
| 126 | `MinBorrowCollateralNotMet` | LTV-weighted collateral is below the configured USD floor while the account still has debt (`contracts/controller/src/risk/validation.rs:59`). | Supply more collateral or repay in full. | Same entry points as code 100 |
| 127 | `UtilizationAboveMax` | Utilization after the operation exceeds the market's `max_utilization` (`contracts/pool/src/guards.rs:26`). | Reduce the borrow or withdrawal size. | pool `borrow`, `withdraw`, `claim_revenue`; controller `borrow`, `withdraw`, `flash_loan`, strategies |
| 128 | `BaseRateNegative` | The base borrow rate is negative (`common/src/types/pool.rs:168`). | Use a non-negative base rate. | Same rate-model entry points as code 117 |
| 129 | `SlopeNonMonotonic` | The rate slopes do not increase monotonically from base up to the max borrow rate (`common/src/types/pool.rs:176`). | Order the slopes non-decreasing. | Same rate-model entry points as code 117 |
| 130 | `MaxRateBelowBase` | The max borrow rate is not strictly above the base rate (`common/src/types/pool.rs:180`). | Raise the max above the base rate. | Same rate-model entry points as code 117 |
| 131 | `MaxBorrowRateTooHigh` | The max borrow rate exceeds `MAX_BORROW_RATE_RAY` (`common/src/types/pool.rs:185`). | Lower the max borrow rate. | Same rate-model entry points as code 117 |
| 132 | `AssetDecimalsTooHigh` | The asset's decimals exceed the WAD/RAY decimal domain (`common/src/types/pool.rs:67`, `common/src/validation.rs:64`). | List a token with fewer decimals. | pool `create_market`; controller `create_liquidity_pool`, `add_asset_to_spoke`, `edit_asset_in_spoke`; governance `execute` |
| 133 | `SelfLiquidationNotAllowed` | The seizure receiver account is the account being liquidated (`contracts/controller/src/positions/liquidation/mod.rs:197`). | Use a different receiver account. | controller `liquidate` |
| 134 | `InvalidLiquidationCurve` | `target_hf` is outside its allowed range, `hf_for_max_bonus` is not below the target, or the bonus factor is outside (0, BPS] (`common/src/validation.rs:127`, `:132`, `:137`). | Fix the curve bounds in the proposal. | controller `set_spoke_liquidation_curve`; governance `propose`, `execute` |
| 135 | `FullCloseRequired` | A partial repayment stays below the ideal amount while the health-factor-preserving bonus cap binds (`contracts/controller/src/positions/liquidation/math.rs:198`). | Repay the full debt instead. | controller `liquidate`, `get_liquidation_estimate` |

### `OracleError` (27 codes)

Price failures reach users indirectly: price-aggregator `prices` and
`price_spread` panic on an unusable price, and so does every controller entry
point that values an account. Price-aggregator `quotes` never panics — it
returns a `PriceStatus` with `valid: false` instead
(`contracts/price-aggregator/src/engine.rs:230`). Configuration failures are
raised by `set_oracle`, `set_sanity_band`, and `set_tolerance`.

| Code | Name | Raised when | What to do | Raised by |
|---|---|---|---|---|
| 201 | `InvalidAggregator` | The proposed swap or price aggregator address is not a deployed contract (`contracts/governance/src/op.rs:166`, `:174`, `:415`). | Pass a deployed contract address. | governance `propose`, `execute`, `execute_self` for `SetSwapAggregator` and `SetPriceAggregator` |
| 204 | `InvalidOracleTokenType` | A Reflector feed references its asset by string; Reflector accepts only an address or a symbol (`common/src/oracle/providers/reflector.rs:105`). | Reference the asset by address or symbol. | price-aggregator `set_oracle`, `prices`, `price_spread` |
| 205 | `UnsafePriceNotAllowed` | The two oracle legs disagree beyond the configured tolerance, or only one of two legs produced a reading (`contracts/price-aggregator/src/engine.rs:136`). | Retry once the feeds reconverge. | price-aggregator `prices`, `price_spread`; every controller entry point that prices an account |
| 206 | `PriceFeedStale` | The feed timestamp is older than `max_stale_seconds` (`contracts/price-aggregator/src/engine.rs:133`, `common/src/oracle/observation.rs:55`). A future-dated timestamp does not raise this error: the observation is dropped by `is_future_at`, so the read fails as `NoLastPrice` or `UnsafePriceNotAllowed` — see INV-ORACLE-04. | Wait for a fresh feed update. | price-aggregator `prices`, `price_spread`; every controller entry point that prices an account |
| 208 | `BadLastTolerance` | The tolerance is outside `MIN_TOLERANCE`..`MAX_TOLERANCE`, or the lower ratio is not the exact reciprocal of the upper ratio (`common/src/validation.rs:153`, `:160`, `contracts/governance/src/validate/tolerance.rs:37`). | Use an in-range tolerance with its derived lower bound. | price-aggregator `set_tolerance`, `set_oracle`; governance `resolve_oracle_tolerance`, `propose` |
| 210 | `NoLastPrice` | An Aquarius LP read fails: wrong pool type, the key is not a token, decimals do not match, or reserves and shares cannot be read (`contracts/price-aggregator/src/providers/aquarius.rs:66`, `:68`, `:74`, `:79`). | Check the LP oracle configuration against the pool. | price-aggregator `prices`, `quotes`, `price_spread` for LP keys; controller entry points pricing LP collateral |
| 211 | `NoAccumulator` | No revenue accumulator address is configured on the controller (`contracts/controller/src/keepers.rs:106`). | Wait for governance to set the accumulator. | controller `claim_revenue` |
| 212 | `ReflectorHistoryEmpty` | Reflector returns no price history for the asset (`contracts/price-aggregator/src/providers/reflector.rs:128`, `:131`). | Wait for the Reflector feed to publish. | price-aggregator `prices`, `price_spread`, `set_oracle` |
| 216 | `OracleNotConfigured` | No oracle is registered for the price key, or the aggregator response omits a requested asset (`contracts/price-aggregator/src/engine.rs:130`, `contracts/controller/src/external/price_aggregator.rs:27`). | Register an oracle for the asset. | price-aggregator `prices`, `price_spread`, `set_sanity_band`, `set_tolerance`; every controller entry point that prices an account |
| 217 | `InvalidPrice` | The resolved price is zero or negative, or LP fair-value math produces a non-representable result (`contracts/price-aggregator/src/engine.rs:139`, `common/src/oracle/lp.rs:69`, `common/src/oracle/lp_stable.rs:42`). | Report it; the feed returned an unusable value. | price-aggregator `prices`, `price_spread`; every controller entry point that prices an account |
| 218 | `InvalidStalenessConfig` | `max_stale_seconds` is outside the allowed range, or it is shorter than the source's own guaranteed freshness (`contracts/price-aggregator/src/validation.rs:52`, `:161`, `contracts/price-aggregator/src/admin.rs:63`). | Choose a `max_stale_seconds` inside the range. | price-aggregator `set_oracle` |
| 219 | `TwapInsufficientObservations` | The TWAP record count is zero or below the smoothing minimum, or the returned history is too short, too long, or spaced closer than the feed resolution (`common/src/validation.rs:217`, `contracts/price-aggregator/src/validation.rs:167`, `contracts/price-aggregator/src/providers/reflector.rs:135`, `:138`, `:156`). | Raise the record count, or wait for more observations. | price-aggregator `set_oracle`, `prices`, `price_spread` |
| 220 | `InvalidOracleBase` | A Reflector feed is not quoted in USD, or an LP source's price keys and tokens do not match the pool (`contracts/price-aggregator/src/providers/reflector.rs:29`, `contracts/price-aggregator/src/validation.rs:133`, `contracts/price-aggregator/src/providers/aquarius.rs:28`). | Point at a USD-quoted feed, or fix the LP keys. | price-aggregator `set_oracle` |
| 221 | `InvalidOracleDecimals` | The declared decimals do not match what the provider reports, or they fall outside the allowed range (`contracts/price-aggregator/src/providers/reflector.rs:34`, `contracts/price-aggregator/src/admin.rs:50`, `contracts/price-aggregator/src/admin.rs:56`, `contracts/price-aggregator/src/validation.rs:125`). | Declare the provider's real decimals. | price-aggregator `set_oracle` |
| 222 | `InvalidOracleResolution` | The Reflector resolution is below the minimum, above `max_stale_seconds`, or the TWAP span it implies exceeds `max_stale_seconds` (`contracts/price-aggregator/src/providers/reflector.rs:40`, `:48`, `:143`). | Match records and `max_stale_seconds` to the feed resolution. | price-aggregator `set_oracle`, `prices`, `price_spread` |
| 223 | `SanityBoundViolated` | The resolved price falls outside the oracle's stored min/max sanity band (`contracts/price-aggregator/src/engine.rs:145`). | Wait for the price to return to the band, or ask governance to rebase it. | price-aggregator `prices`, `price_spread`; every controller entry point that prices an account |
| 224 | `InvalidSanityBounds` | Bounds are non-positive, inverted, above the maximum reasonable price, or narrower than `MIN_SANITY_BAND_BPS`; a scaled source's min/max factor bounds fail the same checks (`common/src/validation.rs:173`, `:180`, `contracts/price-aggregator/src/validation.rs:196`). | Pass an ordered, in-range band at least `MIN_SANITY_BAND_BPS` wide. | price-aggregator `set_sanity_band`, `set_oracle` |
| 225 | `OracleCycleDetected` | The price key is already being resolved higher up the composition chain (`contracts/price-aggregator/src/session.rs:88`, `contracts/price-aggregator/src/engine.rs:333`). | Remove the circular oracle reference. | price-aggregator `set_oracle`, `prices`, `quotes`, `price_spread` |
| 226 | `SanityBandTooWideForSingleSource` | A single-source oracle's sanity band is wider than the single-source limit, or an LP band is wider than the LP limit (`common/src/validation.rs:197`, `:208`). | Narrow the band, or add a second source. | price-aggregator `set_oracle`, `set_sanity_band` |
| 227 | `SanityBandMustTighten` | The immediate `set_sanity_band` call would widen the stored band; only tightening is allowed on that path (`contracts/price-aggregator/src/admin.rs:200`). | Widen the band through the timelocked `ConfigureAssetOracle` operation. | price-aggregator `set_sanity_band` |
| 228 | `TwapRecordsOutOfRange` | The requested TWAP record count is above `MAX_TWAP_RECORDS` (`common/src/validation.rs:221`). | Request fewer TWAP records. | price-aggregator `set_oracle` |
| 229 | `OracleDepthExceeded` | The oracle composition nests deeper than `MAX_RESOLUTION_DEPTH` (`contracts/price-aggregator/src/validation.rs:35`, `contracts/price-aggregator/src/engine.rs:330`). | Flatten the oracle composition. | price-aggregator `set_oracle`, `prices`, `quotes`, `price_spread` |
| 230 | `FactorOutOfBounds` | A scaled source's factor price falls outside its configured min/max factor (`contracts/price-aggregator/src/engine.rs:646`). | Wait for the factor to recover, or widen its bounds. | price-aggregator `prices`, `price_spread`; controller entry points pricing scaled assets |
| 231 | `SourceCountOutOfRange` | The oracle has zero or more than two sources, or an Aquarius LP source is combined with any other source (`contracts/price-aggregator/src/validation.rs:27`, `contracts/price-aggregator/src/engine.rs:509`, `contracts/price-aggregator/src/admin.rs:168`, `:221`). | Configure one or two sources; LP oracles take exactly one. | price-aggregator `set_oracle`, `set_tolerance`, `prices`, `price_spread` |
| 232 | `IndependenceNotDeclared` | Two sources share a provider contract while the policy requires disjoint sources, or the declared shared set does not match the actual one (`contracts/price-aggregator/src/validation.rs:81`, `:86`). | Declare the shared contracts, or use independent sources. | price-aggregator `set_oracle` |
| 234 | `UnsupportedAquariusPool` | The Aquarius pool is not the expected type, has a zero reserve, or has zero total shares (`contracts/price-aggregator/src/providers/aquarius.rs:41`, `:46`). | Point at a live pool of the expected type. | price-aggregator `set_oracle`, `prices`, `price_spread` |
| 235 | `InsufficientAquariusLiquidity` | The Aquarius pool's total value is below the source's `min_pool_value_wad` floor (`contracts/price-aggregator/src/providers/aquarius.rs:118`). | Wait for deeper pool liquidity. | price-aggregator `set_oracle`, `prices`, `price_spread` |

### `SpokeError` (12 codes)

| Code | Name | Raised when | What to do | Raised by |
|---|---|---|---|---|
| 300 | `SpokeNotFound` | The spoke id is zero, or no configuration is stored for it (`contracts/controller/src/account.rs:24`, `contracts/controller/src/storage/spoke.rs:13`). | Use an existing spoke id. | controller `supply` (new account), `get_spoke`, `add_asset_to_spoke`, `edit_asset_in_spoke`, `remove_asset_from_spoke`, `remove_spoke`, `set_spoke_liquidation_curve` |
| 301 | `SpokeDeprecated` | The spoke is flagged deprecated, or it is already deprecated when `remove_spoke` runs (`contracts/controller/src/context/spoke.rs:99`, `contracts/controller/src/config/spoke.rs:41`). | Use an active spoke. | controller position entry points, `add_asset_to_spoke`, `remove_spoke` |
| 307 | `AssetNotInSpoke` | The hub asset is not listed on that spoke (`contracts/controller/src/context/spoke.rs:72`, `contracts/controller/src/config/asset.rs:66`, `:112`, `:158`). | List the asset on the spoke first. | controller position entry points, `edit_asset_in_spoke`, `set_spoke_asset_flags`, `remove_asset_from_spoke`, `get_spoke_asset` |
| 308 | `AssetAlreadyInSpoke` | The hub asset is already listed on that spoke (`contracts/controller/src/config/asset.rs:58`). | Use `edit_asset_in_spoke` instead. | controller `add_asset_to_spoke` |
| 309 | `SpokeAssetInUse` | The listing still has non-zero supplied or borrowed usage (`contracts/controller/src/config/asset.rs:164`). | Wait until all positions unwind. | controller `remove_asset_from_spoke` |
| 310 | `SpokeMismatch` | The account's spoke id differs from the spoke the call targets, or a liquidation receiver sits on a different spoke (`contracts/controller/src/account.rs:119`, `contracts/controller/src/positions/liquidation/mod.rs:205`). | Use an account on the same spoke. | controller `supply`, `borrow`, `withdraw`, `liquidate`, strategies |
| 311 | `SpokeSupplyCapReached` | The supply would push the spoke's tracked supply above its configured cap (`contracts/controller/src/spoke_usage.rs:55`, `:158`). | Wait for cap headroom, or supply less. | controller `supply`, `multiply`, `swap_collateral`, `migrate_from_blend` |
| 312 | `SpokeBorrowCapReached` | The borrow would push the spoke's tracked borrows above its configured cap (`contracts/controller/src/spoke_usage.rs:56`, `:158`). | Borrow less, or wait for cap headroom. | controller `borrow`, `multiply`, `swap_debt`, `migrate_from_blend` |
| 315 | `SpokeAssetPaused` | The listing's `paused` flag is set; this blocks both entry and exit (`contracts/controller/src/positions/mod.rs:327`, `:331`). | Wait for governance to unpause the asset. | All controller position and strategy entry points |
| 316 | `SpokeAssetFrozen` | The listing's `frozen` flag is set; this blocks only entry (`contracts/controller/src/positions/mod.rs:328`). | Exit only; `withdraw` and `repay` still work. | controller `supply`, `borrow`, `multiply`, `migrate_from_blend` |
| 317 | `SpokeAssetFlagRelaxation` | The immediate guardian call tries to clear `paused`, `frozen`, or `no_seize` (`contracts/controller/src/config/asset.rs:147`). | Clear flags through the timelocked `edit_asset_in_spoke`. | controller `set_spoke_asset_flags`; governance `set_spoke_asset_flags` |
| 318 | `SpokeAssetSeizureHalted` | The listing's `no_seize` flag is set, so the asset cannot be taken as liquidation collateral (`contracts/controller/src/positions/mod.rs:334`). | Seize a different collateral asset. | controller `liquidate`, `get_liquidation_estimate` |

### `FlashLoanError` (5 codes)

| Code | Name | Raised when | What to do | Raised by |
|---|---|---|---|---|
| 400 | `FlashLoanOngoing` | A flash loan is already in progress in this transaction; re-entrant protocol calls are refused (`contracts/controller/src/risk/validation.rs:22`). | Do not call back into the protocol from a flash-loan callback. | All controller mutating entry points except `flash_loan` itself, including `liquidate`, `clean_bad_debt`, `force_socialize_bad_debt`, `update_indexes`, `claim_revenue`, `recapitalize`, `update_account_threshold` |
| 401 | `FlashloanNotEnabled` | The market's `is_flashloanable` parameter is false (`contracts/pool/src/ops/flash.rs:89`). | Choose a flash-loan-enabled market. | pool `flash_loan`; controller `flash_loan` |
| 402 | `InvalidFlashloanRepay` | The receiver's allowance to the pool is below principal plus fee, or the pool balance after the callback is not the expected amount (`contracts/pool/src/ops/flash.rs:180`, `:191`). | Approve and return principal plus fee before returning. | pool `flash_loan`; controller `flash_loan` |
| 409 | `StrategyFeeExceeds` | The computed flash-loan fee is larger than the borrowed amount (`contracts/pool/src/ops/strategy.rs:99`). | Report it; the market fee parameter is misconfigured. | pool `create_strategy`, reached through controller `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend` |
| 412 | `InvalidFlashloanReceiver` | The receiver address is not a deployed Wasm contract (`common/src/validation.rs:79`). | Pass a contract address as the receiver. | pool `flash_loan`; controller `flash_loan` |

### `StrategyError` (6 codes)

| Code | Name | Raised when | What to do | Raised by |
|---|---|---|---|---|
| 500 | `ConvertStepsRequired` | The initial payment asset is neither the collateral nor the debt asset, and no conversion swap was supplied (`contracts/controller/src/strategies/multiply.rs:214`). | Supply conversion swap steps, or pay in the collateral or debt asset. | controller `multiply` |
| 501 | `RouterOverspend` | The controller's balance of the input token rose during the swap, or the router spent more than `amount_in` (`contracts/controller/src/strategies/swap.rs:46`, `:51`). | Use a route that respects the declared input amount. | controller `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend` |
| 502 | `NoSwapOutput` | The swap increased the controller's output-token balance by zero (`contracts/controller/src/strategies/swap.rs:105`). | Check that the route actually produces output. | controller `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_from_blend` |
| 503 | `CollateralRequired` | The declared collateral list is empty, or every declared minimum is zero (`contracts/controller/src/strategies/flash_position.rs:201`). | Declare at least one collateral asset with a positive minimum. | controller `flash_position` |
| 504 | `CollateralMinimumNotMet` | The measured collateral push is below the caller-declared minimum (`contracts/controller/src/strategies/flash_position.rs:338`, `:347`). | Lower the declared minimum, or supply more collateral. | controller `flash_position` |
| 505 | `FlashPositionClosed` | The `flash_position` callback finished debt-free or without supply (`contracts/controller/src/strategies/flash_position.rs:360`, `:363`, `:368`). | Leave a live borrow and supply position open at the end of the callback. | controller `flash_position` |

## Unused variants

These codes are declared but constructed nowhere in contract code. They are
reserved: the numbers stay allocated so existing codes never shift. An
integrator will not observe them from a deployed contract today.

| Code | Name | Enum | Status |
|---|---|---|---|
| 1 | `AssetNotSupported` | `GenericError` | No occurrence anywhere in the repository outside the enum declaration. |
| 3 | `InvalidTicker` | `GenericError` | Constructed only by the RedStone test mock (`tests/test-harness/src/mock_redstone.rs:31`), which is not deployed. |
| 11 | `InvalidExchangeSrc` | `GenericError` | No occurrence anywhere in the repository outside the enum declaration. |
| 12 | `PairNotActive` | `GenericError` | No occurrence anywhere in the repository outside the enum declaration. |
| 110 | `PositionNotFound` | `CollateralError` | Referenced only by the test harness (`tests/test-harness/src/errors.rs:31`, `tests/test-harness/src/ops/account.rs:126`), never by a contract. |
