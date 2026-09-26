# Error reference

Use the failing contract and its numeric code together to identify an error. Codes overlap between shared lending errors, contract-local errors and inherited helpers.

SDK `try_` calls distinguish contract errors from host, authorization, storage, budget and conversion failures. Some APIs return `Result` or `Option` instead of panicking. Downstream token, venue and callback errors can propagate. The [endpoint reference](endpoints.md) lists each call's authorization and input requirements.

## Shared lending errors

[Shared error definitions](../../common/src/errors.rs) group lending failures by domain. Gaps in the numeric ranges are reserved. The tables describe failure conditions; the checks a call reaches depend on its execution path.

### Generic errors (1–55)

| Code / variant | Condition | Response |
| --- | --- | --- |
| 2 `AssetAlreadySupported` | A market already exists for this hub asset. | Use the existing market. |
| 5 `PoolAlreadyDeployed` | The singleton pool, controller, or price aggregator address is already recorded. | Use the recorded deployment. |
| 6 `InvalidAsset` | The token reports no decimals or symbol, or the declared decimals fall outside 0–18 (`MIN_ASSET_DECIMALS`..=`MAX_ASSET_DECIMALS`) or differ from the listed decimals: the stored oracle's `asset_decimals` when the price aggregator holds an oracle for the token, else the token's live decimals. | Pass a real token and its listed decimals. |
| 7 `AssetsAreTheSame` | A prohibited same-market pair is used (Long/Short also reject the same underlying token across hubs), or a Blend migration repeats a debt asset. | Choose an allowed distinct market/token pair. |
| 8 `WrongToken` | `params.asset_id` does not equal the asset the market is being created for. | Make `params.asset_id` match the asset. |
| 10 `InvalidWasmHash` | The supplied Wasm hash is all zero bytes. | Pass the hash of an uploaded Wasm. |
| 13 `AccountNotInMarket` | Account metadata is missing or caller fails the explicit NFT-owner check. | Use a live account; owner must authorize renewal/delegate writes. |
| 14 `AmountMustBePositive` | Negative input, forbidden zero, or nonpositive measured receipt. Withdrawal zero and zero flash collateral minima are allowed exceptions. | Use valid amounts and a token that delivers funds. |
| 16 `InvalidPayments` | Required list/route empty, forbidden route nonempty, input bound exceeded, duplicate/overlapping flash declarations. Empty flash collateral list reaches this code, as does a liquidation whose trimmed plan keeps no repayment leg. | Correct request shape and declared asset sets. |
| 18 `NotSmartContract` | The supplied address is not a deployed Wasm contract. | Pass a contract address, not an account. |
| 24 `AccountNotFound` | Required account or its NFT owner cannot be resolved; account id exceeds NFT u32 domain. | Use a live valid id. Some views return empty/zero instead. |
| 25 `AccountModeMismatch` | The account's position mode differs from the mode the call requires, or a liquidation receiver is not in normal mode. | Use an account in the matching mode. |
| 27 `AggregatorNotSet` | The swap aggregator or price aggregator address has not been configured. | Wait for governance to set the aggregator. |
| 29 `PositionLimitsNotSet` | No position limits are stored in controller instance storage. | Wait for governance to set position limits. |
| 30 `PoolNotInitialized` | The pool or controller address is unset, or the market's params or state record does not exist. | Wait for the market to be created. |
| 32 `OwnerNotSet` | A required owner is absent. | Report it; constructors set the owner and no contract exports `renounce_ownership`. |
| 33 `MathOverflow` | Checked arithmetic, conversion or rescale fails. | Check amount/decimal domains; report impossible state. |
| 34 `InternalError` | An internal invariant fails: an expected value is absent, liquidation math is inconsistent, a timelock target is wrong, or a migration version does not increase. | Report it; the inputs are inconsistent. |
| 36 `InvalidPositionLimits` | A supply or borrow position limit is zero or above `POSITION_LIMIT_MAX`. | Use limits inside the allowed range. |
| 38 `SpotOnlyNotProductionSafe` | Both available source paths contain an unsmoothed market leg, or the only source does. | Configure a permitted smoothed or fundamental source composition. |
| 39 `InvalidTimelockDelay` | Constructor delay is zero; a delay update is zero, below the current minimum, or above `TIMELOCK_MAX_DELAY_LEDGERS`. | Use a nonzero constructor delay and an allowed update. |
| 40 `TimelockOperationExpired` | The scheduled operation's grace period has already elapsed. | Propose the operation again. |
| 41 `InvalidRole` | The role symbol is not a known governance role, a non-owner grant would combine executor and canceller, the role is not held on revoke, or an immediate revoke names a role other than guardian or oracle. | Use a valid role assignment. |
| 42 `BlendPoolNotApproved` | The target Blend pool is not on the controller's approved list. | Ask governance to approve the pool. |
| 43 `HubNotActive` | The hub id does not exist or has been deactivated. | Use an active hub. |
| 44 `NotAuthorized` | NFT owner/delegate authorization fails, including a third-party supply to a market the account does not hold; a delegate grant names an inactive manager; a revocation targets the owner or its proposer; or a non-owner proposes an owner-only operation. | Use eligible authority and an allowed governance target. |
| 45 `RegistryCapReached` | The account already has `MAX_DELEGATES` delegates. | Remove a delegate first. |
| 46 `OperationNotCancellable` | The operation is a recovery operation, or the canceller is the account the operation would revoke. | Recovery operations cannot be cancelled; a revocation needs a different canceller. |
| 47 `BorrowRoundsToZeroShares` | A positive borrow amount mints zero scaled debt shares. | Borrow a larger amount. |
| 48 `CannotRemoveLastProposer` | Revoking the role would leave the proposer role with no holders. | Grant another proposer first. |
| 49 `WithdrawRoundsToZeroShares` | A positive withdrawal burns zero scaled supply shares. | Withdraw a larger amount. |
| 50 `NetSettleRoundsToZeroShares` | A positive net settlement burns zero supply or debt shares. | Settle a larger amount. |
| 51 `SupplyRoundsToZeroShares` | A positive supply amount mints zero scaled shares. | Supply a larger amount. |
| 52 `RepayRoundsToZeroShares` | A positive net repayment burns zero scaled debt shares. | Repay a larger amount. |
| 53 `PositionNftNotSet` | The position-NFT contract address is unset in controller storage. | Wait for governance to deploy the position NFT. |
| 54 `PositionNftAlreadyDeployed` | A position-NFT contract address is already recorded. | Use the recorded deployment. |
| 55 `DivisionByZero` | A fixed-point multiply-divide received a zero denominator. Distinct from `MathOverflow`, which the same operations raise when the result does not fit `i128`. | Report it; a zero index or denominator is an internal inconsistency. |

### Collateral and market errors (100–135)

| Code / variant | Condition | Response |
| --- | --- | --- |
| 100 `InsufficientCollateral` | A post-pool risk gate finds LTV collateral below debt or HF below 1 WAD. | Increase collateral or reduce debt; ordinary supply and repayment do not run this gate. |
| 101 `HealthFactorTooHigh` | The account has no debt, or its health factor is at or above 1 WAD. | Liquidate only an eligible unhealthy position. |
| 102 `HealthFactorTooLow` | A full risk-parameter refresh (`has_risks`) leaves the health factor below 1.05 WAD (`THRESHOLD_UPDATE_MIN_HF_RAW`). | Improve the health factor first. |
| 104 `NotCollateral` | The spoke listing does not allow the asset to be used as collateral. | Choose a collateral-enabled asset. |
| 107 `AssetNotBorrowable` | The spoke listing does not allow the asset to be borrowed. | Choose a borrow-enabled asset. |
| 109 `PositionLimitExceeded` | The new supply or borrow position would push the account past its configured position count, or place a supply leg below 3 decimals beside another supply position. | Close or consolidate positions. |
| 111 `InvalidPositionMode` | The requested position mode is not Multiply, Long, or Short. | Use Multiply, Long, or Short. |
| 112 `InsufficientLiquidity` | Market cash is below the requested draw, or the draw would break the liquidation buffer. | Reduce the amount or wait for liquidity. |
| 113 `InvalidLiqThreshold` | The liquidation fee is at or above BPS, the threshold is at or below the LTV or above BPS, or threshold times (BPS plus bonus) exceeds BPS squared. | Fix the risk bounds in the proposal. |
| 114 `CannotCleanBadDebt` | Debt does not exceed collateral, or permissionless cleanup has collateral above `BAD_DEBT_USD_THRESHOLD` (5 USD, WAD). | Liquidate further; governance force-socialization bypasses only the collateral dust cap. |
| 115 `WithdrawLessThanFee` | Liquidation protocol fee exceeds gross withdrawal. | Re-estimate; caller cannot select individual collateral legs. |
| 116 `InvalidBorrowParams` | A cap or floor is negative, a cap exceeds the asset's decimal domain, or the flash-loan fee is above the maximum. A market below 3 decimals is listed borrowable, flash-loanable, or with a liquidation fee. | Use non-negative, in-range parameters. |
| 117 `InvalidUtilRange` | `mid_utilization` is nonpositive, `optimal_utilization` is not above it, or `max_utilization` is below optimal or above RAY. | Order the utilization breakpoints correctly. |
| 118 `OptUtilTooHigh` | Optimal utilization is at or above RAY, that is 100%. | Set optimal utilization below 100%. |
| 119 `InvalidReserveFactor` | The reserve factor is at or above BPS, that is 100%. | Set the reserve factor below 100%. |
| 120 `DebtPositionNotFound` | The account has no debt position for the referenced hub asset, or bad-debt cleanup targets an account with no debt. | Reference an asset the account owes. |
| 121 `CollateralPositionNotFound` | The account has no supply position for the referenced hub asset. | Reference an asset the account supplied. |
| 122 `CannotCloseWithRemainingDebt` | `repay_debt_with_collateral` sets `close_position` while borrow positions remain open. | Repay all debt before closing. |
| 123 `PoolInsolvent` | Supplier claims exceed cash plus outstanding debt, or supply is zero while debt remains. | Recapitalize or clean bad debt first. |
| 126 `MinBorrowCollateralNotMet` | LTV-weighted collateral is below the configured USD floor, or a collateral leg below 3 decimals holds under 2 whole units, while the account still has debt. | Supply more collateral or repay in full. |
| 127 `UtilizationAboveMax` | Utilization after the operation exceeds the market's `max_utilization`. | Reduce the borrow or withdrawal size. |
| 128 `BaseRateNegative` | The base borrow rate is negative. | Use a non-negative base rate. |
| 129 `SlopeNonMonotonic` | Parameters violate base <= slope1 <= slope2 <= slope3 <= max. Slopes remain additive increments in pricing. | Fix the stored parameter ordering. |
| 130 `MaxRateBelowBase` | The max borrow rate is not strictly above the base rate. | Raise the max above the base rate. |
| 131 `MaxBorrowRateTooHigh` | The max borrow rate exceeds `MAX_BORROW_RATE_RAY`. | Lower the max borrow rate. |
| 132 `AssetDecimalsTooHigh` | Market decimals exceed 18, or a cap-rescale helper is given decimals above 27. | Use the supported market decimal domain. |
| 133 `SelfLiquidationNotAllowed` | Credit receiver id equals the liquidated account id. | Choose another receiver; owner self-liquidation is otherwise allowed. |
| 134 `InvalidLiquidationCurve` | The target HF is outside (1 WAD, `MAX_LIQUIDATION_TARGET_HF_WAD`], the max-bonus HF is outside (0, target HF), or the bonus factor is outside (0, BPS]. | Fix the curve bounds in the proposal. |

### Oracle errors (201–235)

| Code / variant | Condition | Response |
| --- | --- | --- |
| 201 `InvalidAggregator` | The proposed swap or price aggregator address is not a deployed Wasm contract. | Pass a deployed contract address. |
| 204 `InvalidOracleTokenType` | A Reflector feed references its asset by string; Reflector accepts only an address or a symbol. | Reference the asset by address or symbol. |
| 205 `UnsafePriceNotAllowed` | Source deviation or only one usable leg of a configured pair makes the result unsafe. | Wait for consistent fresh sources or correct the configuration. |
| 206 `PriceFeedStale` | Resolved source/aggregate age exceeds the configured bound, or two Market legs exceed the allowed age spread. Future observations are rejected separately and may become NoLastPrice/UnsafePriceNotAllowed. | Refresh the source; do not infer a future timestamp is stale. |
| 208 `BadLastTolerance` | A governance tolerance width is outside `MIN_TOLERANCE`..=`MAX_TOLERANCE` (150–2,500 BPS); or an oracle tolerance has an upper ratio outside 10,150–12,500 BPS, a lower ratio outside 7,500–10,000 BPS, or a lower ratio other than BPS² / upper ratio rounded half up. | Use an in-range width, or the ratios `resolve_oracle_tolerance` derives from it. |
| 210 `NoLastPrice` | No source yields a usable observation, or LP binding/decimals/reserves/share/amp reads fail. | Inspect provider availability and LP configuration; not limited to LP assets. |
| 211 `NoAccumulator` | No revenue accumulator address is configured on the controller. | Wait for governance to set the accumulator. |
| 212 `ReflectorHistoryEmpty` | Reflector TWAP helper finds absent/empty history; runtime read converts this to an unusable source. | Wait for provider history. |
| 216 `OracleNotConfigured` | No oracle is registered for the price key, or the aggregator response omits a requested asset. | Register an oracle for the asset. |
| 217 `InvalidPrice` | The resolved price is zero or negative, or LP fair-value math produces a non-representable result. | Report it; the feed returned an unusable value. |
| 218 `InvalidStalenessConfig` | Staleness configuration is outside its bounds or incompatible with source freshness requirements. | Fix the source and aggregate freshness windows. |
| 219 `TwapInsufficientObservations` | Zero configured records, a record count below the smoothing minimum, excess history, spacing below provider resolution, or a history that does not span the window those records imply. | Configure a record count the provider history can span. A history with a skipped provider round is accepted while it still spans the window. Reflector `Twap` reads compute an equal-weight mean. |
| 220 `InvalidOracleBase` | A bare Reflector feed does not quote in USD; a `Scaled` factor's Reflector base is not token X for quote key `Token(X)` (a `Ref` quote or a USD base always fails); or an LP source has an invalid binding, key pairing or pool-value floor. | Match quote/base and LP constraints. |
| 221 `InvalidOracleDecimals` | The declared decimals do not match what the provider reports, they fall outside the allowed range, or a replacement oracle changes the `asset_decimals` stored for the key. | Declare the provider's real decimals. For a replacement, keep the key's stored `asset_decimals`, even after the token issuer changes its decimals. Governance `ConfigureAssetOracle` already resolves them from the stored oracle; read them with `oracle(key)`. |
| 222 `InvalidOracleResolution` | The Reflector resolution is below the minimum, above `max_stale_seconds`, or the TWAP span it implies exceeds `max_stale_seconds`. | Match records and `max_stale_seconds` to the feed resolution. |
| 223 `SanityBoundViolated` | The resolved price falls outside the oracle's stored min/max sanity band. | Wait for the price to return to the band, or ask governance to rebase it. |
| 224 `InvalidSanityBounds` | The sanity band is not positive and increasing, exceeds `MAX_REASONABLE_PRICE_WAD`, or is narrower than `MIN_SANITY_BAND_BPS`; or a scaled factor has a nonpositive minimum, a maximum below its minimum, or a maximum above `MAX_REASONABLE_PRICE_WAD`. | Use valid bounds. Scaled-factor bounds may be equal and have no minimum width. |
| 225 `OracleCycleDetected` | The price key is already being resolved higher up the composition chain. | Remove the circular oracle reference. |
| 226 `SanityBandTooWideForSingleSource` | The band of a single-source oracle, or of a pair whose sources have the same trust set, is wider than `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`; or an LP band is wider than `MAX_LP_SANITY_BAND_BPS`. | Narrow the band. A non-LP pair is exempt only when its sources have different trust sets. |
| 227 `SanityBandMustTighten` | The immediate `set_sanity_band` call would widen the stored band; only tightening is allowed on that path. | Widen the band through the timelocked `ConfigureAssetOracle` operation. |
| 228 `TwapRecordsOutOfRange` | The requested TWAP record count is above `MAX_TWAP_RECORDS`. | Request fewer TWAP records. |
| 229 `OracleDepthExceeded` | The oracle composition nests deeper than `MAX_RESOLUTION_DEPTH`. | Flatten the oracle composition. |
| 230 `FactorOutOfBounds` | Scaled factor is outside stored bounds. | Inspect feed/config; widening bounds requires authorized reconfiguration. |
| 231 `SourceCountOutOfRange` | Source count outside 1..2, LP mixed with other sources, or tolerance edit attempted on LP oracle. | Use one LP source or one/two allowed feed sources; no LP tolerance edit. |
| 232 `IndependenceNotDeclared` | Two sources share a provider contract while the policy requires disjoint sources, or the declared shared set is empty or differs from the actual one. | Declare the shared contracts, or use independent sources. |
| 234 `UnsupportedAquariusPool` | LP attestation finds wrong pool kind, absent stable amp, nonpositive reserves or share supply. | Configure a supported, funded pool. |
| 235 `InsufficientAquariusLiquidity` | The Aquarius pool's total value is below the source's `min_pool_value_wad` floor. | Wait for deeper pool liquidity. |

### Spoke errors (300–319)

| Code / variant | Condition | Response |
| --- | --- | --- |
| 300 `SpokeNotFound` | The spoke id is zero, or no configuration is stored for it. | Use an existing spoke id. |
| 301 `SpokeDeprecated` | Ordinary entry or a new listing uses a deprecated spoke, or `remove_spoke` repeats. | Use an active spoke. A `Credit(0)` liquidation can still create its receiver account on a deprecated spoke. |
| 307 `AssetNotInSpoke` | The hub asset is not listed on that spoke. | List the asset on the spoke first. |
| 308 `AssetAlreadyInSpoke` | The hub asset is already listed on that spoke. | Use `edit_asset_in_spoke` instead. |
| 309 `SpokeAssetInUse` | The listing still has non-zero supplied or borrowed usage. | Wait until all positions unwind. |
| 310 `SpokeMismatch` | The account's spoke id differs from the spoke the call targets, or a liquidation receiver sits on a different spoke. | Use an account on the same spoke. |
| 311 `SpokeSupplyCapReached` | The supply would push the spoke's tracked supply above its configured cap. | Wait for cap headroom, or supply less. |
| 312 `SpokeBorrowCapReached` | The borrow would push the spoke's tracked borrows above its configured cap. | Borrow less, or wait for cap headroom. |
| 315 `SpokeAssetPaused` | Listing paused blocks ordinary entry/exit or liquidation debt repayment. Seizure checks no_seize instead. | Wait for authorized reopening or operate on eligible assets. |
| 316 `SpokeAssetFrozen` | Listing frozen blocks entry. | Exit remains permitted, subject to other gates. |
| 317 `SpokeAssetFlagRelaxation` | The immediate guardian call or a listing edit tries to clear `paused`, `frozen`, or `no_seize`. | Clear flags through the timelocked `relax_spoke_asset_flags`. |
| 318 `SpokeAssetSeizureHalted` | A pro-rata collateral seizure leg has no_seize set. | Wait for authorized flag clearance; liquidation has no collateral-selection argument. |
| 319 `SpokeFlagsEpochMismatch` | A `RelaxSpokeAssetFlags` proposal or `relax_spoke_asset_flags` names a flags epoch other than the listing's current one: a flag write occurred after the relaxation was prepared. | Read `get_spoke_asset_flags_epoch` and the live flags again, then propose a new relaxation. |

### Flash-loan errors (400–412)

| Code / variant | Condition | Response |
| --- | --- | --- |
| 400 `FlashLoanOngoing` | A guarded monetary/maintenance call occurs while the controller flash flag is set, including nested flash_loan. | Avoid guarded reentry; renewal/delegate/admin calls are not universally covered. |
| 401 `FlashloanNotEnabled` | The market's `is_flashloanable` parameter is false. | Choose a flash-loan-enabled market. |
| 402 `InvalidFlashloanRepay` | The receiver's allowance to the pool is below principal plus fee, or the pool balance differs from the expected amount after payout, callback or repayment. | Hold principal plus fee and approve the pool pull; do not pre-push repayment. |
| 409 `StrategyFeeExceeds` | The computed flash-loan fee is larger than the borrowed amount. | Report it; the market fee parameter is misconfigured. |
| 412 `InvalidFlashloanReceiver` | Flash receiver is not deployed Wasm; flash_position receiver is controller/pool; borrow/withdraw recipient is controller/pool. | Use a valid external receiver/recipient. |

### Strategy errors (500–505)

| Code / variant | Condition | Response |
| --- | --- | --- |
| 500 `ConvertStepsRequired` | The initial payment asset is neither the collateral nor the debt asset, and no conversion swap was supplied. | Supply conversion swap steps, or pay in the collateral or debt asset. |
| 501 `RouterOverspend` | The controller's balance of the input token rose during the swap, or the router spent more than `amount_in`. | Use a route that respects the declared input amount. |
| 502 `NoSwapOutput` | The swap did not increase the controller's output-token balance. | Check that the route actually produces output. |
| 503 `CollateralRequired` | Nonempty declared flash collateral list has no positive minimum. | Declare at least one positive collateral minimum; an empty list fails with code 16. |
| 504 `CollateralMinimumNotMet` | Measured collateral delta misses its minimum or no positive deposit remains. | Make callback deliver declared minima. |
| 505 `FlashPositionClosed` | Flash position ends without supply or live debt in its declared debt market. | Keep a funded borrow/supply position open. |

### Reserved shared codes

The following variants are declared but have no production construction in this source tree. Test or mock use does not establish production reachability.

| Domain | Code / variant |
| --- | --- |
| Generic | 1 `AssetNotSupported`, 3 `InvalidTicker`, 11 `InvalidExchangeSrc`, 12 `PairNotActive` |
| Collateral | 110 `PositionNotFound`, 135 `FullCloseRequired` (liquidation accepts partial repayment in every band) |

### Price-status failures

Price-aggregator `quotes` returns an invalid `PriceStatus` with an `error_code` for individual resolution failures. Read that code before interpreting other fields: nested errors can zero the remaining fields, including stale and deviation indicators. `prices` and `price_spread` fail on invalid outcomes.

Reflector runtime TWAP errors, including 212, 219 and 222, make the source leg unusable. Outer resolution then reports `NoLastPrice`, `UnsafePriceNotAllowed`, or `PriceFeedStale` when the remaining leg is stale. Admission checks can expose their own codes. Host, external-contract and resource failures are not universally caught.

## Contract-local namespaces

### Swap aggregator

| Code / variant | Condition | Response |
| --- | --- | --- |
| 1 `EmptyBatch` | Empty/over-cap program or over-cap split table | Use a bounded nonempty program. |
| 3 `InvalidAmount` | Nonpositive amount, overdraft or spend mismatch | Correct funding and amounts. |
| 4 `BrokenTokenChain` | Prev absent or wrong token; Aquarius `get_tokens` empty, oversized, or duplicate; hop token missing from pool metadata | Repair the instruction dependency or the pool's constituent list. |
| 5 `SlippageExceeded` | Nonpositive declared minimum or vault output below minimum before payout | Correct minimum or route. |
| 7 `ZeroOutput` | Venue yields no usable output | Use a productive route. |
| 9 `IntegerOverflow` | Checked arithmetic overflow | Reduce amount/domain. |
| 11 `ZeroSplitPpm` | Split weight zero | Use positive weight. |
| 12 `SplitPpmMismatch` | Split weight exceeds 1,000,000 | Fix PPM weights. |
| 13 `InvalidRouteXdr` | XDR/program decode or validation failure | Rebuild payload for this ABI. |
| 20 `NotAdmin` | Ownable owner missing | Restore valid initialization/configuration. |
| 21 `FeeTooHigh` | Fee exceeds cap | Reduce fee. |
| 22 `ReferralNotFound` | Referral missing | Use a registered id. |
| 25 `SameToken` | Input/output token identical | Use distinct endpoints. |
| 26 `LpTokenMismatch` | Declared LP token differs from pool share token | Fix pool binding. |
| 27 `MinSharesNotMet` | Nonpositive mint minimum or minted shares below minimum | Correct route or minimum. |
| 28 `MinAmountsNotMet` | Invalid burn-minimum span or constituent receipt below minimum | Correct route or minimum. |
| 29 `ExcessiveResidual` | Residual vault balance above allowance | Fully settle route. |
| 30 `InternalInvariant` | Internal invariant, including unsupported fee-accounting state | Report and use a supported fresh deployment. |

### XOXNO oracle

| Code / variant | Condition | Response |
| --- | --- | --- |
| 1 `NotAuthorizedSigner` | Signer not registered | Use an approved signer. |
| 2 `InvalidPrice` | Price <= 0 | Submit a positive 8-decimal price. |
| 3 `InvalidThreshold` | Threshold zero/above signer count, or constructor duplicate signer | Correct quorum. |
| 4 `SignerAlreadyRegistered` | Signer already present | Reuse registration. |
| 5 `SignerNotRegistered` | Removal target absent | Use a registered signer. |
| 6 `CannotRemoveBelowThreshold` | Removal would drop signer count below threshold | Add replacement or lower threshold first. |
| 7 `NoDataForFeed` | No aggregate/history for requested feed | Wait for quorum/history. |
| 8 `StaleData` | Aggregate write timestamp too old | Submit fresh data. |
| 9 `PriceOutOfRange` | Price above maximum | Correct price scale/value. |
| 10 `LengthMismatch` | Feed and price vector lengths differ | Align vectors. |
| 11 `FutureTimestamp` | Package timestamp beyond allowed future skew | Correct millisecond timestamp. |
| 12 `FeedAlreadyMapped` | Asset/feed already owned by mapping | Use distinct mapping. |
| 13 `FeedNotMapped` | Asset has no feed mapping | Add mapping first. |
| 14 `FeedNotKnown` | Feed id unregistered | Register feed first. |
| 15 `InvalidSubmissionAge` | Submission age below minimum/above stale limit, or stale limit below submission age | Correct bounds. |
| 16 `StaleSubmission` | Submission too old or package timestamp older than signer’s prior submission | Submit a fresh nondecreasing timestamp; equality is allowed. |
| 17 `FeedAlreadyRegistered` | Feed id already registered | Reuse it. |
| 18 `InvalidRelativeSkew` | Relative skew <= permitted future skew or above submission-age maximum | Correct skew bound. |
| 19 `InvalidClusterSpread` | Cluster spread bound is 0 or above 10000 bps | Use 1..=10000 bps. |

### DeFindex strategy

| Code / variant | Condition | Response |
| --- | --- | --- |
| 401 `NotInitialized` | Missing/malformed constructor args or stored config | Initialize correct controller/hub/spoke. |
| 460 `AmountNotPositive` | Deposit/withdraw amount <= 0 | Use positive amount. |
| 461 `InsufficientBalance` | No live vault account or withdrawal exceeds balance | Reduce withdrawal. |
| 462 `ArithmeticError` | Declared without a production construction in this source tree. | Decode propagated shared arithmetic errors in their own domain. |
| 463 `AccountLookupFailed` | Controller account-existence query failed | Resolve downstream failure; a failure must not erase vault mapping. |

## Relevant inherited errors

OpenZeppelin stellar-contracts revision `fbfde388e1b72afa93d6b1c922067879b20e81db` supplies the NFT, authorization and timelock helpers. A deployed contract can raise both its own errors and these overlapping inherited codes. An error declaration does not make an unexported extension callable.

| Namespace | Codes and handling |
| --- | --- |
| NFT ownership/approval | 200 NonExistentToken: use live id; 201 IncorrectOwner: correct from; 202 InsufficientApproval: authorize spender; 203 InvalidApprover: owner/operator required; 204 InvalidLiveUntilLedger: valid expiry. |
| NFT accounting/metadata | 205 MathOverflow, 206 TokenIDsAreDepleted: report exhaustion/state; 208 TokenNotFoundInOwnerList, 209 TokenNotFoundInGlobalList: valid index; 210 UnsetMetadata: initialize; 211 BaseUriMaxLenExceeded, 213 NameMaxLenExceeded, 214 SymbolMaxLenExceeded: shorten metadata. 207 InvalidAmount and 212 InvalidRoyaltyAmount belong to unused consecutive/royalty extensions. |
| Pausable | 1000 EnforcedPause: operation is pause-gated; 1001 ExpectedPause: unpause requires paused state. |
| Ownable | 2100 OwnerNotSet: missing owner; 2101 TransferInProgress: only `renounce_ownership` raises it, and no contract in this tree exports that call; 2102 OwnerAlreadySet: helper rejects repeated initialization. Role-transfer errors below cover pending-owner state; auth can raise host errors. |
| RoleTransfer | 2200 NoPendingTransfer: initiate transfer first; 2201 InvalidLiveUntilLedger: use current-to-max valid ledger; 2202 InvalidPendingAccount: cancellation address must match; 2203 TransferExpired: initiate a fresh window. Zero deadline cancels, and acceptance checks the explicit deadline even if storage remains alive. |
| AccessControl | 2000 Unauthorized, 2001 AdminNotSet, 2002 IndexOutOfBounds, 2003 AdminRoleNotFound, 2004 RoleCountIsNotZero, 2005 RoleNotFound, 2006 AdminAlreadySet, 2007 RoleNotHeld, 2008 RoleIsEmpty, 2009 TransferInProgress, 2010 MaxRolesExceeded. Use valid held roles/admin/membership indices; generic admin APIs are not exported by governance. |
| Timelock | 4000 OperationAlreadyScheduled: new salt or existing operation; 4001 InsufficientDelay: respect minimum; 4002 InvalidOperationState: wait/check schedule; 4003 UnexecutedPredecessor: execute predecessor; 4004 Unauthorized: eligible role; 4005 MinDelayNotSet: initialize; 4006 OperationNotScheduled: schedule matching hash. |

## Source map

- [Account authorization](../../contracts/controller/src/account.rs), [risk](../../contracts/controller/src/risk/validation.rs), [liquidation](../../contracts/controller/src/positions/liquidation/mod.rs), [pool guards](../../contracts/pool/src/guards.rs).
- [Shared validation](../../common/src/validation.rs), [rate validation](../../common/src/types/pool.rs), [price resolution](../../contracts/price-aggregator/src/engine.rs).
- [Router errors](../../contracts/swap-aggregator/src/errors.rs), [XOXNO oracle](../../contracts/xoxno-oracle/src/lib.rs), [DeFindex](../../contracts/defindex-strategy/src/lib.rs), [dependency pins](../../Cargo.toml).
