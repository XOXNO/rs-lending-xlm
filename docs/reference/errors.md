# Error reference

Identify an error by contract and numeric code together: shared lending, contract-local and inherited codes overlap.

SDK `try_` calls separate contract errors from host, authorization, storage, budget and conversion failures. Some APIs return `Result` or `Option` instead of panicking. Downstream token, venue and callback errors can propagate. [Endpoints](endpoints.md) lists each call's authorization and inputs.

Unless a row adds a response, correct the named input or wait for governance to configure the missing item. "Report" marks an internal inconsistency.

## Shared lending errors

[Shared error definitions](../../common/src/errors.rs) group failures by domain; gaps in the ranges are reserved. The checks a call reaches depend on its execution path.

### Generic errors (1–55)

| Code / variant | Condition and response |
| --- | --- |
| 2 `AssetAlreadySupported` | A market already exists for this hub asset. |
| 5 `PoolAlreadyDeployed` | The singleton pool, controller, or price aggregator is already recorded. |
| 6 `InvalidAsset` | Token reports no decimals or symbol, or decimals outside 0–18 (`MIN_ASSET_DECIMALS`..=`MAX_ASSET_DECIMALS`) or unequal to the listed decimals (the stored oracle's `asset_decimals` if one exists, else live decimals). |
| 7 `AssetsAreTheSame` | Prohibited same-market pair (Long/Short also reject one token across hubs), or a Blend migration repeats a debt asset. |
| 8 `WrongToken` | `params.asset_id` differs from the listed asset. |
| 10 `InvalidWasmHash` | Wasm hash is all zeros. |
| 13 `AccountNotInMarket` | Account metadata missing, or caller fails the NFT-owner check (renewal and delegate writes need the owner). |
| 14 `AmountMustBePositive` | Negative input, forbidden zero, or nonpositive measured receipt. Zero withdrawal and zero flash collateral minima are allowed. |
| 16 `InvalidPayments` | Required list/route empty, forbidden route nonempty, input bound exceeded, duplicate/overlapping flash declarations, empty flash collateral list, a withdrawal mixing zero (withdraw-all) and a positive amount for one market, or a pool `supply`/`borrow`/`withdraw`/`repay` batch repeating a market. Also a liquidation whose trimmed plan keeps no repayment leg, or whose debt token under-delivers while a seized leg is below 3 decimals. |
| 18 `NotSmartContract` | Address is not a deployed Wasm contract. |
| 24 `AccountNotFound` | Account or NFT owner unresolvable, or id beyond NFT u32. Some views return empty/zero instead. |
| 25 `AccountModeMismatch` | Account mode differs from the call's, or a liquidation receiver is not Normal. |
| 27 `AggregatorNotSet` | Swap or price aggregator unset. |
| 29 `PositionLimitsNotSet` | No position limits stored. |
| 30 `PoolNotInitialized` | Pool or controller unset, or market params/state missing. |
| 32 `OwnerNotSet` | Owner absent. Report: constructors set it and no contract exports `renounce_ownership`. |
| 33 `MathOverflow` | Checked arithmetic, conversion or rescale failed. Check amount/decimal domains; report impossible state. |
| 34 `InternalError` | Expected value absent, inconsistent liquidation math, wrong timelock target, or non-increasing migration version. Report. |
| 36 `InvalidPositionLimits` | Position limit zero or above `POSITION_LIMIT_MAX`. |
| 38 `SpotOnlyNotProductionSafe` | Every source path has an unsmoothed market leg. Use a smoothed or fundamental composition. |
| 39 `InvalidTimelockDelay` | Zero constructor delay; or an update that is zero, above `TIMELOCK_MAX_DELAY_LEDGERS`, or below both the current minimum and `TIMELOCK_MIN_DELAY_LEDGERS`. |
| 40 `TimelockOperationExpired` | Grace period elapsed. Propose again. |
| 41 `InvalidRole` | Unknown role, revoke of an unheld role, or immediate revoke of a role other than guardian, oracle or curator. |
| 42 `BlendPoolNotApproved` | Blend pool not approved. |
| 43 `HubNotActive` | Hub missing or deactivated. |
| 44 `NotAuthorized` | NFT owner/delegate auth fails (incl. third-party supply to a market the account lacks); delegate grant to an inactive manager; revocation of the owner or its proposer; or a non-owner proposing an owner-only operation. |
| 45 `RegistryCapReached` | Account already has `MAX_DELEGATES` delegates. |
| 46 `OperationNotCancellable` | Recovery operation, or the canceller is the account it would revoke. |
| 47 `BorrowRoundsToZeroShares` | Positive borrow mints zero debt shares. |
| 48 `CannotRemoveLastProposer` | Revoke would leave no proposer. Grant another first. |
| 49 `WithdrawRoundsToZeroShares` | Positive withdrawal burns zero supply shares. |
| 50 `NetSettleRoundsToZeroShares` | Positive net settlement burns zero supply or debt shares. |
| 51 `SupplyRoundsToZeroShares` | Positive supply mints zero shares. |
| 52 `RepayRoundsToZeroShares` | Positive net repayment burns zero debt shares. |
| 53 `PositionNftNotSet` | Position-NFT address unset. |
| 54 `PositionNftAlreadyDeployed` | Position-NFT address already recorded. |
| 55 `DivisionByZero` | Fixed-point multiply-divide got a zero denominator (`MathOverflow` is the non-fitting-result case). Report. |

### Collateral and market errors (100–136)

| Code / variant | Condition and response |
| --- | --- |
| 100 `InsufficientCollateral` | Post-pool risk gate: LTV collateral below debt or HF below 1 WAD. Supply and repayment skip it. |
| 101 `HealthFactorTooHigh` | No debt, or HF >= 1 WAD. |
| 102 `HealthFactorTooLow` | A full risk refresh (`has_risks`) lowers HF below both 1.05 WAD (`THRESHOLD_UPDATE_MIN_HF_RAW`) and its prior value. |
| 104 `NotCollateral` | Listing disallows collateral use. |
| 107 `AssetNotBorrowable` | Listing disallows borrowing. |
| 109 `PositionLimitExceeded` | A new position would exceed the account's position limit, or put a supply leg below 3 decimals beside another supply position. |
| 111 `InvalidPositionMode` | Mode is not Multiply, Long, or Short. |
| 112 `InsufficientLiquidity` | Cash is below the draw, or the draw breaks the liquidation buffer. |
| 113 `InvalidLiqThreshold` | Liquidation fee >= BPS, threshold <= LTV or > BPS, or threshold × (BPS + bonus) > BPS². |
| 114 `CannotCleanBadDebt` | Debt does not exceed collateral, or permissionless cleanup has collateral above `BAD_DEBT_USD_THRESHOLD` (5 USD, WAD). Force-socialization bypasses only the dust cap. |
| 115 `WithdrawLessThanFee` | Protocol fee exceeds gross withdrawal. Re-estimate; callers cannot pick collateral legs. |
| 116 `InvalidBorrowParams` | A cap or floor is negative, a cap exceeds the asset's decimal domain, or the flash-loan fee exceeds the maximum; or a market below 3 decimals is listed borrowable, flash-loanable, or with a liquidation fee. |
| 117 `InvalidUtilRange` | `mid_utilization` <= 0, `optimal_utilization` <= mid, or `max_utilization` below optimal or above RAY. |
| 118 `OptUtilTooHigh` | Optimal utilization >= RAY (100%). |
| 119 `InvalidReserveFactor` | Reserve factor >= BPS (100%). |
| 120 `DebtPositionNotFound` | No debt position for the hub asset, or bad-debt cleanup targets an account with no debt. |
| 121 `CollateralPositionNotFound` | No supply position for the hub asset. |
| 122 `CannotCloseWithRemainingDebt` | `repay_debt_with_collateral` sets `close_position` with debt open. |
| 123 `PoolInsolvent` | Supplier claims exceed cash plus debt, or zero supply with debt. Recapitalize or clean bad debt first. |
| 126 `MinBorrowCollateralNotMet` | With debt, LTV-weighted collateral below the USD floor, or a sub-3-decimal leg under 2 whole units. |
| 127 `UtilizationAboveMax` | Post-operation utilization exceeds `max_utilization`. At a 100% ceiling, a borrow or strategy opening also fails when debt value would exceed supply value. |
| 128 `BaseRateNegative` | Base rate negative. |
| 129 `SlopeNonMonotonic` | A slope is negative or above max, base + slope1 + slope2 + slope3 exceeds max, or steepness falls at a kink: slope1 / mid <= slope2 / (optimal - mid) <= slope3 / (RAY - optimal) fails. Slopes are additive increments, so a segment's steepness is its slope divided by its width. |
| 130 `MaxRateBelowBase` | Max rate not strictly above base. |
| 131 `MaxBorrowRateTooHigh` | Max rate above `MAX_BORROW_RATE_RAY`. |
| 132 `AssetDecimalsTooHigh` | Market decimals above 18, or cap-rescale decimals above 27. |
| 133 `SelfLiquidationNotAllowed` | Credit receiver is the liquidated account. Owner self-liquidation is otherwise allowed. |
| 134 `InvalidLiquidationCurve` | Target HF outside (1 WAD, `MAX_LIQUIDATION_TARGET_HF_WAD`], max-bonus HF outside (0, target HF), or bonus factor outside (0, BPS]. |
| 136 `MarketCeilingReached` | Supply, borrow, strategy opening or flash loan would leave total supply or debt above `MAX_MARKET_ENTRY_RAY` (about 21.27 billion whole tokens), as shares or value. Exits never fail it. |

### Oracle errors (201–237)

| Code / variant | Condition and response |
| --- | --- |
| 201 `InvalidAggregator` | Aggregator address is not deployed Wasm. |
| 204 `InvalidOracleTokenType` | Reflector asset given as string; only address or symbol allowed. |
| 205 `UnsafePriceNotAllowed` | Source deviation, or one usable leg of a pair. |
| 206 `PriceFeedStale` | Source or aggregate too old, or two Market legs too far apart. Future observations are rejected separately, surfacing as `NoLastPrice` or `UnsafePriceNotAllowed`. |
| 208 `BadLastTolerance` | A governance tolerance width is outside `MIN_TOLERANCE`..=`MAX_TOLERANCE` (150–2,500 BPS); or an oracle tolerance has an upper ratio outside 10,150–12,500 BPS, a lower ratio outside 7,500–10,000 BPS, or a lower ratio other than BPS² / upper ratio rounded half up. Use the ratios `resolve_oracle_tolerance` derives. |
| 210 `NoLastPrice` | No usable observation, or an LP binding/decimals/reserves/share/amp read failed. Not LP-only. |
| 211 `NoAccumulator` | No accumulator configured. |
| 212 `ReflectorHistoryEmpty` | Reflector TWAP history absent or empty; runtime reads treat the source as unusable. |
| 216 `OracleNotConfigured` | No oracle for the key, or the response omits a requested asset. |
| 217 `InvalidPrice` | Price not positive, or LP fair-value math unrepresentable. Report. |
| 218 `InvalidStalenessConfig` | Staleness config out of bounds or incompatible with source freshness. |
| 219 `TwapInsufficientObservations` | Zero records, count below the smoothing minimum, spacing below resolution, or history not spanning the window (a skipped round passes if it still spans). `Twap` is an equal-weight mean of the newest `records` samples. |
| 220 `InvalidOracleBase` | A bare Reflector feed does not quote in USD; a `Scaled` factor's Reflector base is not token X for quote key `Token(X)` (a `Ref` quote or a USD base always fails); an LP source has an invalid binding, key pairing or pool-value floor; or a Curve source has fewer than 2 or more than `MAX_CURVE_COINS` coins. |
| 221 `InvalidOracleDecimals` | Declared decimals differ from the provider's or are out of range, or a replacement changes the stored `asset_decimals`. Replacements keep it even if the issuer changes decimals; `ConfigureAssetOracle` resolves it from the stored oracle (`oracle(key)`). |
| 222 `InvalidOracleResolution` | Resolution below minimum or above `max_stale_seconds`, or TWAP oldest-sample age `records * resolution` above `max_stale_seconds`. |
| 223 `SanityBoundViolated` | Price outside the stored sanity band. Wait, or have governance rebase the band. |
| 224 `InvalidSanityBounds` | Band not positive and increasing, above `MAX_REASONABLE_PRICE_WAD`, or narrower than `MIN_SANITY_BAND_BPS`; or scaled-factor minimum <= 0, maximum below minimum or above `MAX_REASONABLE_PRICE_WAD` (factor bounds may be equal; no minimum width). |
| 225 `OracleCycleDetected` | Key already being resolved up the composition chain. |
| 226 `SanityBandTooWideForSingleSource` | Single-source or same-trust-set pair band above `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`, or LP band above `MAX_LP_SANITY_BAND_BPS`. Non-LP pairs with differing trust sets are exempt, except that a scaled leg's factor range is held to the same cap when its factor provider serves both legs. |
| 227 `SanityBandMustTighten` | Immediate `set_sanity_band` would widen; use timelocked `ConfigureAssetOracle`. |
| 228 `TwapRecordsOutOfRange` | Records above `MAX_TWAP_RECORDS`. |
| 229 `OracleDepthExceeded` | Nesting deeper than `MAX_RESOLUTION_DEPTH`. |
| 230 `FactorOutOfBounds` | Scaled factor outside stored bounds; widening needs authorized reconfiguration. |
| 231 `SourceCountOutOfRange` | Source count outside 1..2, LP mixed with others, or tolerance edit on an LP oracle. |
| 232 `IndependenceNotDeclared` | Sources share a provider under a disjoint policy, or the declared shared set is empty or differs from actual (on an upstream key change or band edit it need only cover actual). |
| 234 `UnsupportedAquariusPool` | Wrong pool kind, no stable amp, or nonpositive reserves or share supply. |
| 235 `InsufficientAquariusLiquidity` | Pool value below `min_pool_value_wad`. |
| 236 `UnsupportedCurvePool` | Executable hash, LP binding, coin order/count, decimals, `precision` (must be WAD) or normalization rates differ from config, or a `pool_info`/`decimals` read failed. |
| 237 `InsufficientCurveLiquidity` | Conservative pool value below `min_pool_value_wad`. |

### Spoke errors (300–319)

| Code / variant | Condition and response |
| --- | --- |
| 300 `SpokeNotFound` | Spoke id zero or unconfigured. |
| 301 `SpokeDeprecated` | Entry or new listing on a deprecated spoke, or repeated `remove_spoke`. `Credit(0)` can still create its receiver there. |
| 307 `AssetNotInSpoke` | Asset not listed on the spoke. |
| 308 `AssetAlreadyInSpoke` | Already listed; use `edit_asset_in_spoke`. |
| 309 `SpokeAssetInUse` | Listing still has usage. |
| 310 `SpokeMismatch` | Account or liquidation receiver on another spoke. |
| 311 `SpokeSupplyCapReached` | Supply would exceed the spoke's supply cap. |
| 312 `SpokeBorrowCapReached` | Borrow would exceed the spoke's borrow cap. |
| 315 `SpokeAssetPaused` | `paused` blocks entry/exit or liquidation debt repayment; seizure checks `no_seize`. |
| 316 `SpokeAssetFrozen` | `frozen` blocks entry; exit stays open subject to other gates. |
| 317 `SpokeAssetFlagRelaxation` | Guardian call or listing edit tries to clear a flag. Use timelocked `relax_spoke_asset_flags`. |
| 318 `SpokeAssetSeizureHalted` | A pro-rata seizure leg has `no_seize`; liquidation cannot pick collateral. |
| 319 `SpokeFlagsEpochMismatch` | `RelaxSpokeAssetFlags` or `relax_spoke_asset_flags` names a stale epoch (a flag write landed since). Reread `get_spoke_asset_flags_epoch` and flags, then propose anew. |
| 320 `SpokeAssetFlagsUnchanged` | `set_spoke_asset_flags` names only held flags; epoch unchanged. The listing is already that tight. |

### Flash-loan errors (400–412)

| Code / variant | Condition and response |
| --- | --- |
| 400 `FlashLoanOngoing` | Guarded monetary or maintenance call (incl. nested `flash_loan`) while the flash flag is set. Renewal, delegate and admin calls are not universally covered. |
| 401 `FlashloanNotEnabled` | `is_flashloanable` false. |
| 402 `InvalidFlashloanRepay` | Receiver allowance below principal plus fee, or pool balance off after payout, callback or repayment. Approve the pull; do not pre-push repayment. |
| 409 `StrategyFeeExceeds` | Fee exceeds the borrowed amount. Report: fee misconfigured. |
| 412 `InvalidFlashloanReceiver` | Flash receiver is not deployed Wasm; flash_position receiver is controller/pool; borrow/withdraw recipient is controller/pool. |

### Strategy errors (500–505)

| Code / variant | Condition and response |
| --- | --- |
| 500 `ConvertStepsRequired` | Initial payment is neither collateral nor debt, and no conversion swap given. |
| 501 `RouterOverspend` | Controller input balance rose during the swap, or router spent over `amount_in`. |
| 502 `NoSwapOutput` | Swap did not raise the controller's output balance. |
| 503 `CollateralRequired` | Nonempty collateral list with no positive minimum (empty fails with 16). |
| 504 `CollateralMinimumNotMet` | Measured collateral delta below minimum, or no positive deposit left. |
| 505 `FlashPositionClosed` | Position ends without supply, or without live debt in its declared market. |

### Reserved shared codes

These variants have no production construction in this tree; test or mock use does not make them reachable.

| Domain | Code / variant |
| --- | --- |
| Generic | 1 `AssetNotSupported`, 3 `InvalidTicker`, 11 `InvalidExchangeSrc`, 12 `PairNotActive` |
| Collateral | 110 `PositionNotFound`, 135 `FullCloseRequired` (liquidation accepts partial repayment in every band) |

### Price-status failures

`quotes` returns an invalid `PriceStatus` with an `error_code` per failed resolution. Read that code first: nested errors can zero the other fields, including the stale and deviation flags. `prices` and `price_spread` fail instead.

Runtime Reflector TWAP errors (212, 219, 222) make the leg unusable; outer resolution then reports `NoLastPrice`, `UnsafePriceNotAllowed`, or `PriceFeedStale` when the remaining leg is stale. Admission checks expose their own codes. Host, external-contract and resource failures are not universally caught.

## Contract-local namespaces

### Swap aggregator

| Code / variant | Condition and response |
| --- | --- |
| 1 `EmptyBatch` | Empty/over-cap program or over-cap split table |
| 3 `InvalidAmount` | Nonpositive amount, overdraft or spend mismatch |
| 4 `BrokenTokenChain` | Prev absent or wrong token; Aquarius `get_tokens` or Curve `pool_info().coins` has fewer than 2 or too many tokens, a duplicate, or the pool itself; Curve `coins` missing or not an address list; hop token missing from pool metadata |
| 5 `SlippageExceeded` | Nonpositive declared minimum, or vault output below minimum before payout |
| 7 `ZeroOutput` | Venue yields no usable output |
| 9 `IntegerOverflow` | Checked arithmetic overflow |
| 11 `ZeroSplitPpm` | Split weight zero |
| 12 `SplitPpmMismatch` | Split weight exceeds 1,000,000 |
| 13 `InvalidRouteXdr` | XDR/program decode or validation failure. Rebuild the payload for this ABI |
| 20 `NotAdmin` | Ownable owner missing |
| 21 `FeeTooHigh` | Fee exceeds cap |
| 22 `ReferralNotFound` | Referral missing |
| 25 `SameToken` | Input and output token identical |
| 26 `LpTokenMismatch` | Declared LP token differs from pool share token |
| 27 `MinSharesNotMet` | Nonpositive mint minimum, or minted shares below minimum |
| 28 `MinAmountsNotMet` | Invalid burn-minimum span, or constituent receipt below minimum |
| 29 `ExcessiveResidual` | Residual vault balance above allowance |
| 30 `InternalInvariant` | Internal invariant, including unsupported fee-accounting state. Report and use a supported fresh deployment |

### XOXNO oracle

| Code / variant | Condition and response |
| --- | --- |
| 1 `NotAuthorizedSigner` | Signer not registered |
| 2 `InvalidPrice` | Price <= 0. Submit a positive 8-decimal price |
| 3 `InvalidThreshold` | Threshold zero or above signer count, or constructor duplicate signer |
| 4 `SignerAlreadyRegistered` | Signer already present |
| 5 `SignerNotRegistered` | Removal target absent |
| 6 `CannotRemoveBelowThreshold` | Removal would drop signer count below threshold. Add a replacement or lower the threshold first |
| 7 `NoDataForFeed` | No aggregate or history for the feed |
| 8 `StaleData` | Aggregate write timestamp too old |
| 9 `PriceOutOfRange` | Price above maximum |
| 10 `LengthMismatch` | Feed and price vector lengths differ |
| 11 `FutureTimestamp` | Package timestamp beyond allowed future skew (milliseconds) |
| 12 `FeedAlreadyMapped` | Asset/feed already owned by a mapping |
| 13 `FeedNotMapped` | Asset has no feed mapping |
| 14 `FeedNotKnown` | Feed id unregistered |
| 15 `InvalidSubmissionAge` | Submission age below minimum or above stale limit, or stale limit below submission age |
| 16 `StaleSubmission` | Submission too old, or package timestamp older than the signer's prior submission. Equality is allowed |
| 17 `FeedAlreadyRegistered` | Feed id already registered |
| 18 `InvalidRelativeSkew` | Relative skew <= permitted future skew or above submission-age maximum |
| 19 `InvalidClusterSpread` | Cluster spread bound is 0 or above 10000 bps |

### DeFindex strategy

| Code / variant | Condition and response |
| --- | --- |
| 401 `NotInitialized` | Missing/malformed constructor args or stored config |
| 460 `AmountNotPositive` | Deposit/withdraw amount <= 0 |
| 461 `InsufficientBalance` | No live vault account, or withdrawal exceeds balance |
| 462 `ArithmeticError` | No production construction in this source tree. Decode propagated shared arithmetic errors in their own domain |
| 463 `AccountLookupFailed` | Controller account-existence query failed. A failure must not erase the vault mapping |

### Curator

| Code / variant | Condition and response |
| --- | --- |
| 801 `UnknownRegistrar` | Registrar has no admin. |
| 802 `RoleMissing` | Caller is not the admin and holds no accepted role bit. |
| 803 `RoleBits` | `set_roles` names bits outside `ROLE_MARKET`, `ROLE_SPOKE`, and `ROLE_FLAGS`. |
| 804 `HubOwned` | New hub id already bound. Report to governance. |
| 805 `HubUnowned` | Hub unbound; governance binds it with `ReassignCuratorHub`. |
| 806 `SpokeOwned` | New spoke id already bound. Report to governance. |
| 807 `SpokeUnowned` | Spoke unbound; governance binds it with `ReassignCuratorSpoke`. |
| 808 `NotHubRegistrar` | Hub owned by another registrar. |
| 809 `NotSpokeRegistrar` | Spoke owned by another registrar, or a hub owner's foreign-spoke `tighten_flags` sets anything but `frozen` alone. |
| 812 `GrantMissing` | Foreign hub listing, edit or relaxation lacks a live grant from the hub owner. |
| 813 `GrantExceeded` | Listing exceeds the grant, or `set_cap_grant` passes a negative cap. |
| 814 `Unsupported` | `propose` names an operation other than `AddAssetToSpoke`, `EditAssetInSpoke`, `UpgradeLiquidityPoolParams`, or `RelaxSpokeAssetFlags`. Ask governance to propose it. |
| 815 `RegistrarSuspended` | Caller's registrar, or the grant's grantor, is suspended. |
| 816 `ProposalUnknown` | `cancel_proposal` names an operation this contract did not queue; use a governance `CANCELLER`. |
| 817 `ProposalForeign` | Caller's registrar owns neither the operation's hub nor spoke. |
| 818 `ForeignPause` | Spoke owner pausing a listing whose hub another registrar owns. Freeze or halt seizure instead. |
| 819 `NotCapLowering` | Hub owner's foreign-spoke edit changes anything but lowering supply or borrow cap. |

## Relevant inherited errors

OpenZeppelin stellar-contracts 0.8.0 supplies the NFT, authorization and timelock helpers. A contract can raise these overlapping codes besides its own. A declared error does not make an unexported extension callable.

| Namespace | Codes and notes |
| --- | --- |
| NFT ownership/approval | 200 NonExistentToken, 201 IncorrectOwner, 202 InsufficientApproval, 203 InvalidApprover (owner/operator required), 204 InvalidLiveUntilLedger (nonzero expiry must be from the current ledger through its maximum live-until bound; 0 revokes). |
| NFT accounting/metadata | 205 MathOverflow, 206 TokenIDsAreDepleted (report), 208 TokenNotFoundInOwnerList, 209 TokenNotFoundInGlobalList, 210 UnsetMetadata, 211 BaseUriMaxLenExceeded, 213 NameMaxLenExceeded, 214 SymbolMaxLenExceeded. 207 InvalidAmount and 212 InvalidRoyaltyAmount belong to unused consecutive/royalty extensions. |
| Pausable | 1000 EnforcedPause (operation is pause-gated), 1001 ExpectedPause (unpause requires paused state). |
| Ownable | 2100 OwnerNotSet; 2101 TransferInProgress, raised only by `renounce_ownership`, which no contract in this tree exports; 2102 OwnerAlreadySet (repeated initialization). Pending-owner state uses the RoleTransfer errors; auth can raise host errors. |
| RoleTransfer | 2200 NoPendingTransfer, 2201 InvalidLiveUntilLedger (current-to-max valid ledger), 2202 InvalidPendingAccount (cancellation address must match), 2203 TransferExpired. Zero deadline cancels; acceptance checks the explicit deadline even if storage remains alive. |
| AccessControl | 2000 Unauthorized, 2001 AdminNotSet, 2002 IndexOutOfBounds, 2003 AdminRoleNotFound, 2004 RoleCountIsNotZero, 2005 RoleNotFound, 2006 AdminAlreadySet, 2007 RoleNotHeld, 2008 RoleIsEmpty, 2009 TransferInProgress, 2010 MaxRolesExceeded. Governance does not export the generic admin APIs. |
| Timelock | 4000 OperationAlreadyScheduled (use a new salt), 4001 InsufficientDelay, 4002 InvalidOperationState, 4003 UnexecutedPredecessor, 4004 Unauthorized, 4005 MinDelayNotSet, 4006 OperationNotScheduled (schedule the matching hash). |

## Source map

- [Account authorization](../../contracts/controller/src/account.rs), [risk](../../contracts/controller/src/risk/validation.rs), [liquidation](../../contracts/controller/src/positions/liquidation/mod.rs), [pool guards](../../contracts/pool/src/guards.rs).
- [Shared validation](../../common/src/validation.rs), [rate validation](../../common/src/types/pool.rs), [price resolution](../../contracts/price-aggregator/src/engine.rs).
- [Router errors](../../contracts/swap-aggregator/src/errors.rs), [XOXNO oracle](../../contracts/xoxno-oracle/src/lib.rs), [DeFindex](../../contracts/defindex-strategy/src/lib.rs), [curator](../../contracts/curator/src/lib.rs), [dependency pins](../../Cargo.toml).
