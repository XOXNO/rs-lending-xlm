# Error reference

Numeric contract errors are scoped to the contract that raises them, not globally unique. Decode the failing invocation and its code; SDK `try_` calls distinguish contract errors from host/auth/storage/budget/conversion failures. Some APIs return Result or Option rather than directly panicking. Downstream token, venue and callback errors can propagate.

## Shared lending errors

[common/src/errors.rs](../../common/src/errors.rs) declares 121 discriminants across Generic (1–55), Collateral (100–135), Oracle (201–235), Spoke (300–318), FlashLoan (400–412) and Strategy (500–505). Gaps are reserved; these are not the router/oracle-local namespaces. Tables describe admission/failure conditions, not a promise that every caller reaches each check.

| Code / variant | Condition → response |
| --- | --- |
| 2 `AssetAlreadySupported` | A market already exists for this hub asset. → Use the existing market. |
| 5 `PoolAlreadyDeployed` | The singleton pool, controller, or price aggregator address is already recorded. → Nothing; deployment already happened. |
| 6 `InvalidAsset` | The token reports no decimals or symbol, or the declared decimals do not match the token. → Pass a real token and its true decimals. |
| 7 `AssetsAreTheSame` | A prohibited same-market pair is used; Long/Short also reject the same underlying token across hubs. → Choose an allowed distinct market/token pair. |
| 8 `WrongToken` | `params.asset_id` does not equal the asset the market is being created for. → Make `params.asset_id` match the asset. |
| 10 `InvalidWasmHash` | The supplied Wasm hash is all zero bytes. → Pass the hash of an uploaded Wasm. |
| 13 `AccountNotInMarket` | Account metadata is missing or caller fails the explicit NFT-owner check. → Use a live account; owner must authorize renewal/delegate writes. |
| 14 `AmountMustBePositive` | Negative input, forbidden zero, or nonpositive measured receipt. Withdrawal zero and zero flash collateral minima are allowed exceptions. → Use valid amounts and a token that delivers funds. |
| 16 `InvalidPayments` | Required list/route empty, forbidden route nonempty, input bound exceeded, duplicate/overlapping flash declarations. Empty flash collateral list reaches this code. → Correct request shape and declared asset sets. |
| 18 `NotSmartContract` | The supplied address is not a deployed contract. → Pass a contract address, not an account. |
| 24 `AccountNotFound` | Required account or its NFT owner cannot be resolved; account id exceeds NFT u32 domain. → Use a live valid id. Some views return empty/zero instead. |
| 25 `AccountModeMismatch` | The account's position mode differs from the mode the call requires, or a liquidation receiver is not in normal mode. → Use an account in the matching mode. |
| 27 `AggregatorNotSet` | The swap aggregator or price aggregator address has not been configured. → Wait for governance to set the aggregator. |
| 29 `PositionLimitsNotSet` | No position limits are stored in controller instance storage. → Wait for governance to set position limits. |
| 30 `PoolNotInitialized` | The pool or controller address is unset, or the market's params or state record does not exist. → Wait for the market to be created. |
| 32 `OwnerNotSet` | A required owner is absent. → Inspect initialization or ownership renunciation. |
| 33 `MathOverflow` | Checked arithmetic, conversion or rescale fails. → Check amount/decimal domains; report impossible state. |
| 34 `InternalError` | An internal invariant fails: an expected value is absent, liquidation math is inconsistent, a timelock target is wrong, or a migration version does not increase. → Report it; the inputs are inconsistent. |
| 36 `InvalidPositionLimits` | A supply or borrow position limit is zero or above `POSITION_LIMIT_MAX`. → Use limits inside the allowed range. |
| 38 `SpotOnlyNotProductionSafe` | Both available source paths contain an unsmoothed market leg, or the only source does. → Configure a permitted smoothed or fundamental source composition. |
| 39 `InvalidTimelockDelay` | Constructor delay is zero; a delay update is zero, below the current minimum, or above `TIMELOCK_MAX_DELAY_LEDGERS`. → Use a nonzero constructor delay and an allowed update. |
| 40 `TimelockOperationExpired` | The scheduled operation's grace period has already elapsed. → Propose the operation again. |
| 41 `InvalidRole` | The role symbol is not a known governance role, the grant would combine executor and canceller, or the role is not held on revoke. → Use a valid role that the account holds. |
| 42 `BlendPoolNotApproved` | The target Blend pool is not on the controller's approved list. → Ask governance to approve the pool. |
| 43 `HubNotActive` | The hub id does not exist or has been deactivated. → Use an active hub. |
| 44 `NotAuthorized` | NFT owner/delegate authorization fails, inactive manager grant, protected owner role revocation self-targeted revocation proposal, or ownership-transfer proposal by a non-owner. → Use eligible authority and an allowed governance target. |
| 45 `RegistryCapReached` | The account already has `MAX_DELEGATES` delegates. → Remove a delegate first. |
| 46 `OperationNotCancellable` | The operation is a recovery operation, or the canceller is the account the operation would revoke. → Recovery operations cannot be cancelled; a revocation needs a different canceller. |
| 47 `BorrowRoundsToZeroShares` | A positive borrow amount mints zero scaled debt shares. → Borrow a larger amount. |
| 48 `CannotRemoveLastProposer` | Revoking the role would leave the proposer role with no holders. → Grant another proposer first. |
| 49 `WithdrawRoundsToZeroShares` | A positive withdrawal burns zero scaled supply shares. → Withdraw a larger amount. |
| 50 `NetSettleRoundsToZeroShares` | A positive net settlement burns zero supply or debt shares. → Settle a larger amount. |
| 51 `SupplyRoundsToZeroShares` | A positive supply amount mints zero scaled shares. → Supply a larger amount. |
| 52 `RepayRoundsToZeroShares` | A positive net repayment burns zero scaled debt shares. → Repay a larger amount. |
| 53 `PositionNftNotSet` | The position-NFT contract address is unset in controller storage. → Wait for governance to deploy the position NFT. |
| 54 `PositionNftAlreadyDeployed` | A position-NFT contract address is already recorded. → Nothing; deployment already happened. |
| 55 `DivisionByZero` | A fixed-point multiply-divide received a zero denominator. Distinct from `MathOverflow`, which the same operations raise when the result does not fit `i128`. → Report it; a zero index or denominator is an internal inconsistency. |
| 100 `InsufficientCollateral` | A post-pool risk gate finds LTV collateral below debt or HF below 1 WAD. → Increase collateral or reduce debt; ordinary supply and repayment do not run this gate. |
| 101 `HealthFactorTooHigh` | No target debt or HF >= 1. → Liquidate only an eligible unhealthy position. |
| 102 `HealthFactorTooLow` | After refreshing thresholds the health factor is below the minimum required to accept the update. → Improve the health factor first. |
| 104 `NotCollateral` | The spoke listing does not allow the asset to be used as collateral. → Choose a collateral-enabled asset. |
| 107 `AssetNotBorrowable` | The spoke listing does not allow the asset to be borrowed. → Choose a borrow-enabled asset. |
| 109 `PositionLimitExceeded` | The new supply or borrow position would push the account past its configured position count. → Close or consolidate positions. |
| 111 `InvalidPositionMode` | The requested position mode is not Multiply, Long, or Short. → Use Multiply, Long, or Short. |
| 112 `InsufficientLiquidity` | Market cash is below the requested draw, or the draw would break the liquidation buffer. → Reduce the amount or wait for liquidity. |
| 113 `InvalidLiqThreshold` | The liquidation fee is at or above BPS, the threshold is at or below the LTV or above BPS, or threshold times (BPS plus bonus) exceeds BPS squared. → Fix the risk bounds in the proposal. |
| 114 `CannotCleanBadDebt` | Debt does not exceed collateral, or permissionless cleanup has collateral above $5 WAD. → Liquidate further; governance force-socialization bypasses only the collateral dust cap. |
| 115 `WithdrawLessThanFee` | Liquidation protocol fee exceeds gross withdrawal. → Re-estimate; caller cannot select individual collateral legs. |
| 116 `InvalidBorrowParams` | A cap or floor is negative, a cap exceeds the asset's decimal domain, or the flash-loan fee is above the maximum. → Use non-negative, in-range parameters. |
| 117 `InvalidUtilRange` | `mid_utilization` is nonpositive, `optimal_utilization` is not above it, or `max_utilization` is below optimal or above RAY. → Order the utilization breakpoints correctly. |
| 118 `OptUtilTooHigh` | Optimal utilization is at or above RAY, that is 100%. → Set optimal utilization below 100%. |
| 119 `InvalidReserveFactor` | The reserve factor is at or above BPS, that is 100%. → Set the reserve factor below 100%. |
| 120 `DebtPositionNotFound` | The account has no debt position for the referenced hub asset. → Reference an asset the account owes. |
| 121 `CollateralPositionNotFound` | The account has no supply position for the referenced hub asset. → Reference an asset the account supplied. |
| 122 `CannotCloseWithRemainingDebt` | `close_position` is requested while borrow positions remain open. → Repay all debt before closing. |
| 123 `PoolInsolvent` | Supplier claims exceed cash plus outstanding debt, or supply is zero while debt remains. → Recapitalize or clean bad debt first. |
| 126 `MinBorrowCollateralNotMet` | LTV-weighted collateral is below the configured USD floor while the account still has debt. → Supply more collateral or repay in full. |
| 127 `UtilizationAboveMax` | Utilization after the operation exceeds the market's `max_utilization`. → Reduce the borrow or withdrawal size. |
| 128 `BaseRateNegative` | The base borrow rate is negative. → Use a non-negative base rate. |
| 129 `SlopeNonMonotonic` | Parameters violate base <= slope1 <= slope2 <= slope3 <= max. Slopes remain additive increments in pricing. → Fix the stored parameter ordering. |
| 130 `MaxRateBelowBase` | The max borrow rate is not strictly above the base rate. → Raise the max above the base rate. |
| 131 `MaxBorrowRateTooHigh` | The max borrow rate exceeds `MAX_BORROW_RATE_RAY`. → Lower the max borrow rate. |
| 132 `AssetDecimalsTooHigh` | Market decimals exceed 18, or a cap-rescale helper is given decimals above 27. → Use the supported market decimal domain. |
| 133 `SelfLiquidationNotAllowed` | Credit receiver id equals the liquidated account id. → Choose another receiver; owner self-liquidation is otherwise allowed. |
| 134 `InvalidLiquidationCurve` | `target_hf` is outside its allowed range, `hf_for_max_bonus` is not below the target, or the bonus factor is outside (0, BPS]. → Fix the curve bounds in the proposal. |
| 135 `FullCloseRequired` | Partial repayment below ideal amount encounters the HF-preserving bonus cap. → Cover the required full close or revise liquidation size. |
| 201 `InvalidAggregator` | The proposed swap or price aggregator address is not a deployed contract. → Pass a deployed contract address. |
| 204 `InvalidOracleTokenType` | A Reflector feed references its asset by string; Reflector accepts only an address or a symbol. → Reference the asset by address or symbol. |
| 205 `UnsafePriceNotAllowed` | Source deviation or only one usable leg of a configured pair makes the result unsafe. → Wait for consistent fresh sources or correct the configuration. |
| 206 `PriceFeedStale` | Resolved source/aggregate age exceeds the configured bound, or two Market legs exceed the allowed age spread. Future observations are rejected separately and may become NoLastPrice/UnsafePriceNotAllowed. → Refresh the source; do not infer a future timestamp is stale. |
| 208 `BadLastTolerance` | The tolerance is outside `MIN_TOLERANCE`..`MAX_TOLERANCE`, or the lower ratio is not the half-up rounded reciprocal of the upper ratio. → Use an in-range tolerance with its derived lower bound. |
| 210 `NoLastPrice` | No source yields a usable observation, or LP binding/decimals/reserves/share/amp reads fail. → Inspect provider availability and LP configuration; not limited to LP assets. |
| 211 `NoAccumulator` | No revenue accumulator address is configured on the controller. → Wait for governance to set the accumulator. |
| 212 `ReflectorHistoryEmpty` | Reflector TWAP helper finds absent/empty history; runtime read converts this to an unusable source. → Wait for provider history. |
| 216 `OracleNotConfigured` | No oracle is registered for the price key, or the aggregator response omits a requested asset. → Register an oracle for the asset. |
| 217 `InvalidPrice` | The resolved price is zero or negative, or LP fair-value math produces a non-representable result. → Report it; the feed returned an unusable value. |
| 218 `InvalidStalenessConfig` | Staleness configuration is outside its bounds or incompatible with source freshness requirements. → Fix the source and aggregate freshness windows. |
| 219 `TwapInsufficientObservations` | Zero/insufficient configured records, incomplete/excess history or spacing below provider resolution. → Supply a complete permitted history window. Reflector Twap computes an equal-weight mean. |
| 220 `InvalidOracleBase` | Bare Reflector base is not USD; Scaled Reflector factor base is not the Stellar token matching quote Token(X), including Ref/USD factor bases; or LP binding/floor invalid. → Match quote/base and LP constraints. |
| 221 `InvalidOracleDecimals` | The declared decimals do not match what the provider reports, or they fall outside the allowed range. → Declare the provider's real decimals. |
| 222 `InvalidOracleResolution` | The Reflector resolution is below the minimum, above `max_stale_seconds`, or the TWAP span it implies exceeds `max_stale_seconds`. → Match records and `max_stale_seconds` to the feed resolution. |
| 223 `SanityBoundViolated` | The resolved price falls outside the oracle's stored min/max sanity band. → Wait for the price to return to the band, or ask governance to rebase it. |
| 224 `InvalidSanityBounds` | Invalid sanity band, insufficient band width, or invalid positive ordered scaled-factor bounds. → Use valid bounds; factor equality is allowed and has no sanity-band minimum-width check. |
| 225 `OracleCycleDetected` | The price key is already being resolved higher up the composition chain. → Remove the circular oracle reference. |
| 226 `SanityBandTooWideForSingleSource` | Single-source or identical-provider-trust-set pair exceeds its band cap, or LP exceeds its separate cap. → Narrow the band; a non-LP pair needs differing provider trust sets for exemption. |
| 227 `SanityBandMustTighten` | The immediate `set_sanity_band` call would widen the stored band; only tightening is allowed on that path. → Widen the band through the timelocked `ConfigureAssetOracle` operation. |
| 228 `TwapRecordsOutOfRange` | The requested TWAP record count is above `MAX_TWAP_RECORDS`. → Request fewer TWAP records. |
| 229 `OracleDepthExceeded` | The oracle composition nests deeper than `MAX_RESOLUTION_DEPTH`. → Flatten the oracle composition. |
| 230 `FactorOutOfBounds` | Scaled factor is outside stored bounds. → Inspect feed/config; widening bounds requires authorized reconfiguration. |
| 231 `SourceCountOutOfRange` | Source count outside 1..2, LP mixed with other sources, or tolerance edit attempted on LP oracle. → Use one LP source or one/two allowed feed sources; no LP tolerance edit. |
| 232 `IndependenceNotDeclared` | Two sources share a provider contract while the policy requires disjoint sources, or the declared shared set does not match the actual one. → Declare the shared contracts, or use independent sources. |
| 234 `UnsupportedAquariusPool` | LP attestation finds wrong pool kind, absent stable amp, nonpositive reserves or share supply. → Configure a supported, funded pool. |
| 235 `InsufficientAquariusLiquidity` | The Aquarius pool's total value is below the source's `min_pool_value_wad` floor. → Wait for deeper pool liquidity. |
| 300 `SpokeNotFound` | The spoke id is zero, or no configuration is stored for it. → Use an existing spoke id. |
| 301 `SpokeDeprecated` | Ordinary entry uses a deprecated spoke, or remove_spoke is repeated. → Use an active spoke; Credit(0) liquidation receiver creation is allowed in deprecated spoke. |
| 307 `AssetNotInSpoke` | The hub asset is not listed on that spoke. → List the asset on the spoke first. |
| 308 `AssetAlreadyInSpoke` | The hub asset is already listed on that spoke. → Use `edit_asset_in_spoke` instead. |
| 309 `SpokeAssetInUse` | The listing still has non-zero supplied or borrowed usage. → Wait until all positions unwind. |
| 310 `SpokeMismatch` | The account's spoke id differs from the spoke the call targets, or a liquidation receiver sits on a different spoke. → Use an account on the same spoke. |
| 311 `SpokeSupplyCapReached` | The supply would push the spoke's tracked supply above its configured cap. → Wait for cap headroom, or supply less. |
| 312 `SpokeBorrowCapReached` | The borrow would push the spoke's tracked borrows above its configured cap. → Borrow less, or wait for cap headroom. |
| 315 `SpokeAssetPaused` | Listing paused blocks ordinary entry/exit or liquidation debt repayment. Seizure checks no_seize instead. → Wait for authorized reopening or operate on eligible assets. |
| 316 `SpokeAssetFrozen` | Listing frozen blocks entry. → Exit remains permitted, subject to other gates. |
| 317 `SpokeAssetFlagRelaxation` | The immediate guardian call tries to clear `paused`, `frozen`, or `no_seize`. → Clear flags through the timelocked `edit_asset_in_spoke`. |
| 318 `SpokeAssetSeizureHalted` | A pro-rata collateral seizure leg has no_seize set. → Wait for authorized flag clearance; liquidation has no collateral-selection argument. |
| 400 `FlashLoanOngoing` | A guarded monetary/maintenance call occurs while the controller flash flag is set, including nested flash_loan. → Avoid guarded reentry; renewal/delegate/admin calls are not universally covered. |
| 401 `FlashloanNotEnabled` | The market's `is_flashloanable` parameter is false. → Choose a flash-loan-enabled market. |
| 402 `InvalidFlashloanRepay` | The receiver's allowance to the pool is below principal plus fee, or the pool balance after the callback is not the expected amount. → Hold principal plus fee and approve the pool pull; do not pre-push repayment. |
| 409 `StrategyFeeExceeds` | The computed flash-loan fee is larger than the borrowed amount. → Report it; the market fee parameter is misconfigured. |
| 412 `InvalidFlashloanReceiver` | Flash receiver is not deployed Wasm; flash_position receiver is controller/pool; borrow/withdraw recipient is controller/pool. → Use a valid external receiver/recipient. |
| 500 `ConvertStepsRequired` | The initial payment asset is neither the collateral nor the debt asset, and no conversion swap was supplied. → Supply conversion swap steps, or pay in the collateral or debt asset. |
| 501 `RouterOverspend` | The controller's balance of the input token rose during the swap, or the router spent more than `amount_in`. → Use a route that respects the declared input amount. |
| 502 `NoSwapOutput` | The swap increased the controller's output-token balance by zero. → Check that the route actually produces output. |
| 503 `CollateralRequired` | Nonempty declared flash collateral list has no positive minimum. → Declare at least one positive collateral minimum; empty list is code16. |
| 504 `CollateralMinimumNotMet` | Measured collateral delta misses its minimum or no positive deposit remains. → Make callback deliver declared minima. |
| 505 `FlashPositionClosed` | Flash position ends without supply or live debt in its declared debt market. → Keep a funded borrow/supply position open. |
| Reserved | Generic: 1 `AssetNotSupported`, 3 `InvalidTicker`, 11 `InvalidExchangeSrc`, 12 `PairNotActive`; Collateral: 110 `PositionNotFound`. No production construction in this revision; test/mock occurrences do not establish reachability. |

Price-aggregator `quotes` returns invalid PriceStatus with `error_code` for individual resolution failures. Read that code: nested errors may zero all other fields, including stale/deviation. `prices` and `price_spread` fail on invalid outcomes. Reflector runtime TWAP errors (including 212/219/222) are swallowed into an unusable leg; outer resolution selects NoLastPrice, UnsafePriceNotAllowed or stale-partial precedence. Admission checks can expose their own codes. Host, external-contract and resource failures are not universally caught.

## Contract-local namespaces

| Contract: code / variant | Condition → response |
| --- | --- |
| Router 1 `EmptyBatch` | Empty/over-cap program or over-cap split table → use a bounded nonempty program. |
| Router 3 `InvalidAmount` | Nonpositive amount, overdraft or spend mismatch → correct funding and amounts. |
| Router 4 `BrokenTokenChain` | Prev absent or wrong token → repair instruction dependency. |
| Router 5 `SlippageExceeded` | Nonpositive declared minimum or vault output below minimum before payout → correct minimum or route. |
| Router 7 `ZeroOutput` | Venue yields no usable output → use a productive route. |
| Router 9 `IntegerOverflow` | Checked arithmetic overflow → reduce amount/domain. |
| Router 11 `ZeroSplitPpm` | Split weight zero → use positive weight. |
| Router 12 `SplitPpmMismatch` | Split weight exceeds 1,000,000 → fix PPM weights. |
| Router 13 `InvalidRouteXdr` | XDR/program decode or validation failure → rebuild payload for this ABI. |
| Router 20 `NotAdmin` | Ownable owner missing → restore valid initialization/configuration. |
| Router 21 `FeeTooHigh` | Fee exceeds cap → reduce fee. |
| Router 22 `ReferralNotFound` | Referral missing → use a registered id. |
| Router 25 `SameToken` | Input/output token identical → use distinct endpoints. |
| Router 26 `LpTokenMismatch` | Declared LP token differs from pool share token → fix pool binding. |
| Router 27 `MinSharesNotMet` | Nonpositive mint minimum or minted shares below minimum → correct route or minimum. |
| Router 28 `MinAmountsNotMet` | Invalid burn-minimum span or constituent receipt below minimum → correct route or minimum. |
| Router 29 `ExcessiveResidual` | Residual vault balance above allowance → fully settle route. |
| Router 30 `InternalInvariant` | Internal invariant, including unsupported old fee-accounting state → report and use a supported fresh deployment. |
| XOXNO 1 `NotAuthorizedSigner` | Signer not registered → use an approved signer. |
| XOXNO 2 `InvalidPrice` | Price <= 0 → submit a positive 8-decimal price. |
| XOXNO 3 `InvalidThreshold` | Threshold zero/above signer count, or constructor duplicate signer → correct quorum. |
| XOXNO 4 `SignerAlreadyRegistered` | Signer already present → reuse registration. |
| XOXNO 5 `SignerNotRegistered` | Removal target absent → use a registered signer. |
| XOXNO 6 `CannotRemoveBelowThreshold` | Removal would drop signer count below threshold → add replacement or lower threshold first. |
| XOXNO 7 `NoDataForFeed` | No aggregate/history for requested feed → wait for quorum/history. |
| XOXNO 8 `StaleData` | Aggregate write timestamp too old → submit fresh data. |
| XOXNO 9 `PriceOutOfRange` | Price above maximum → correct price scale/value. |
| XOXNO 10 `LengthMismatch` | Feed and price vector lengths differ → align vectors. |
| XOXNO 11 `FutureTimestamp` | Package timestamp beyond allowed future skew → correct millisecond timestamp. |
| XOXNO 12 `FeedAlreadyMapped` | Asset/feed already owned by mapping → use distinct mapping. |
| XOXNO 13 `FeedNotMapped` | Asset has no feed mapping → add mapping first. |
| XOXNO 14 `FeedNotKnown` | Feed id unregistered → register feed first. |
| XOXNO 15 `InvalidSubmissionAge` | Submission age below minimum/above stale limit, or stale limit below submission age → correct bounds. |
| XOXNO 16 `StaleSubmission` | Submission too old or package timestamp older than signer’s prior submission → submit a fresh nondecreasing timestamp; equality is allowed. |
| XOXNO 17 `FeedAlreadyRegistered` | Feed id already registered → reuse it. |
| XOXNO 18 `InvalidRelativeSkew` | Relative skew <= permitted future skew or above submission-age maximum → correct skew bound. |
| DeFindex 401 `NotInitialized` | Missing/malformed constructor args or stored config → initialize correct controller/hub/spoke. |
| DeFindex 460 `AmountNotPositive` | Deposit/withdraw amount <= 0 → use positive amount. |
| DeFindex 461 `InsufficientBalance` | No live vault account or withdrawal exceeds balance → reduce withdrawal. |
| DeFindex 462 `ArithmeticError` | Declared but no production construction in this revision; shared arithmetic errors may propagate. |
| DeFindex 463 `AccountLookupFailed` | Controller account-existence query failed → resolve downstream failure; a failure must not erase vault mapping. |

## Relevant inherited errors

OpenZeppelin stellar-contracts revision `fbfde388e1b72afa93d6b1c922067879b20e81db` supplies the generated NFT and auth/timelock helpers. These numbers overlap custom namespaces; an inherited error can be raised by the same deployed contract using that helper. Unexported extensions do not become callable merely because their errors exist.

| Namespace | Codes and handling |
| --- | --- |
| NFT ownership/approval | 200 NonExistentToken: use live id; 201 IncorrectOwner: correct from; 202 InsufficientApproval: authorize spender; 203 InvalidApprover: owner/operator required; 204 InvalidLiveUntilLedger: valid expiry. |
| NFT accounting/metadata | 205 MathOverflow, 206 TokenIDsAreDepleted: report exhaustion/state; 208 TokenNotFoundInOwnerList, 209 TokenNotFoundInGlobalList: valid index; 210 UnsetMetadata: initialize; 211 BaseUriMaxLenExceeded, 213 NameMaxLenExceeded, 214 SymbolMaxLenExceeded: shorten metadata. 207 InvalidAmount and 212 InvalidRoyaltyAmount belong to unused consecutive/royalty extensions. |
| Pausable | 1000 EnforcedPause: operation is pause-gated; 1001 ExpectedPause: unpause requires paused state. |
| Ownable | 2100 OwnerNotSet: missing owner; 2101 TransferInProgress: resolve pending transfer before renouncing; 2102 OwnerAlreadySet: helper rejects repeated initialization. Role-transfer errors below cover pending-owner state; auth can raise host errors. |
| RoleTransfer | 2200 NoPendingTransfer: initiate transfer first; 2201 InvalidLiveUntilLedger: use current-to-max valid ledger; 2202 InvalidPendingAccount: cancellation address must match; 2203 TransferExpired: initiate a fresh window. Zero deadline cancels, and acceptance checks the explicit deadline even if storage remains alive. |
| AccessControl | 2000 Unauthorized, 2001 AdminNotSet, 2002 IndexOutOfBounds, 2003 AdminRoleNotFound, 2004 RoleCountIsNotZero, 2005 RoleNotFound, 2006 AdminAlreadySet, 2007 RoleNotHeld, 2008 RoleIsEmpty, 2009 TransferInProgress, 2010 MaxRolesExceeded. Use valid held roles/admin/membership indices; generic admin APIs are not exported by governance. |
| Timelock | 4000 OperationAlreadyScheduled: new salt or existing operation; 4001 InsufficientDelay: respect minimum; 4002 InvalidOperationState: wait/check schedule; 4003 UnexecutedPredecessor: execute predecessor; 4004 Unauthorized: eligible role; 4005 MinDelayNotSet: initialize; 4006 OperationNotScheduled: schedule matching hash. |

Source checks: [account authorization](../../contracts/controller/src/account.rs), [risk](../../contracts/controller/src/risk/validation.rs), [liquidation](../../contracts/controller/src/positions/liquidation/mod.rs), [pool guards](../../contracts/pool/src/guards.rs), [common validation](../../common/src/validation.rs), [rate validation](../../common/src/types/pool.rs), [price resolution](../../contracts/price-aggregator/src/engine.rs), [router errors](../../contracts/swap-aggregator/src/errors.rs), [XOXNO oracle](../../contracts/xoxno-oracle/src/lib.rs), [DeFindex](../../contracts/defindex-strategy/src/lib.rs), [dependency pins](../../Cargo.toml).
