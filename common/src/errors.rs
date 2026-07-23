//! Contract error codes and protocol failure meanings.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GenericError {
    /// Asset is not active or not listed for this flow.
    AssetNotSupported = 1,
    /// Asset already has a configured market.
    AssetAlreadySupported = 2,
    /// Token ticker or symbol failed validation.
    InvalidTicker = 3,
    /// The central liquidity pool has already been deployed.
    PoolAlreadyDeployed = 5,
    /// Token contract failed required asset checks.
    InvalidAsset = 6,
    /// Operation requires two distinct assets.
    AssetsAreTheSame = 7,
    /// Token address does not match the expected market asset.
    WrongToken = 8,
    /// Pool template hash is empty or invalid.
    InvalidPoolTemplate = 10,
    /// Oracle primary/anchor source configuration is invalid.
    InvalidExchangeSrc = 11,
    /// Asset has no active oracle configuration for this operation.
    PairNotActive = 12,
    /// Account meta is missing, or the caller is not the account owner.
    AccountNotInMarket = 13,
    /// Amount must be strictly positive for this operation.
    AmountMustBePositive = 14,
    /// Payment/swap/bulk vector is empty when required, non-empty when forbidden, or over-length.
    InvalidPayments = 16,
    /// Address is not a deployed WASM contract.
    NotSmartContract = 18,
    /// Account id has no stored account.
    AccountNotFound = 24,
    /// Account mode does not match the requested strategy or position mode.
    AccountModeMismatch = 25,
    /// Pool template WASM hash has not been configured.
    TemplateNotSet = 26,
    /// Swap or price aggregator contract has not been configured.
    AggregatorNotSet = 27,
    /// Position limits have not been configured.
    PositionLimitsNotSet = 29,
    /// Pool storage record missing for the market.
    PoolNotInitialized = 30,
    /// Ownable storage has no owner.
    OwnerNotSet = 32,
    /// Fixed-point or ledger arithmetic overflow/underflow.
    MathOverflow = 33,
    /// Internal invariant failed after prior validation (should be unreachable).
    InternalError = 34,
    /// Configured account position limits are zero or above the protocol cap.
    InvalidPositionLimits = 36,
    /// Rewards cannot be added when there are zero suppliers.
    NoSuppliersToReward = 37,
    /// Spot-only primary is rejected for production oracle config.
    SpotOnlyNotProductionSafe = 38,
    /// Timelock delay is zero, decreases the current delay, or exceeds the max cap.
    InvalidTimelockDelay = 39,
    /// Timelock operation is past its execution grace period.
    TimelockOperationExpired = 40,
    /// Requested role is not part of the protocol role allowlist.
    InvalidRole = 41,
    /// Migration source pool is not on the governance Blend-pool allowlist.
    BlendPoolNotApproved = 42,
    /// Target hub id is not a registered, active hub.
    HubNotActive = 43,
    /// Caller lacks the required role or account authorization.
    NotAuthorized = 44,
    /// Bounded instance registry (managers, delegates, Blend pools) is full.
    RegistryCapReached = 45,
    /// Operation cannot be cancelled (role-revocation self-veto protection).
    OperationNotCancellable = 46,
    /// Defensive invariant: a positive ceil-scaled borrow produced zero debt.
    BorrowRoundsToZeroShares = 47,
    /// Would remove the last PROPOSER (permanently freezes governance scheduling).
    CannotRemoveLastProposer = 48,
    /// Defensive invariant: a positive ceil-scaled withdrawal produced zero burn.
    WithdrawRoundsToZeroShares = 49,
    /// A positive zero-cash settlement rounds its debt credit to zero, which
    /// would make the two accounting legs diverge.
    NetSettleRoundsToZeroShares = 50,
    /// Positive supply rounds to zero scaled supply (would accept tokens without
    /// crediting the account or aggregate position).
    SupplyRoundsToZeroShares = 51,
    /// Positive repay rounds to zero scaled debt (would accept tokens without
    /// reducing the account or aggregate debt position).
    RepayRoundsToZeroShares = 52,
    /// A reward would lift `supply_index` past `SUPPLY_INDEX_REWARD_CEILING_RAY`.
    SupplyIndexRewardCeiling = 53,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollateralError {
    /// Collateral value or health factor is insufficient for the requested mutation.
    InsufficientCollateral = 100,
    /// Account is not liquidatable because health factor is still at or above one.
    HealthFactorTooHigh = 101,
    /// Post-operation health factor would fall below the required minimum.
    HealthFactorTooLow = 102,
    /// Asset is not eligible as collateral on the account's spoke.
    NotCollateral = 104,
    /// Asset is not borrowable on the account's spoke.
    AssetNotBorrowable = 107,
    /// Account would exceed max supply or borrow position count.
    PositionLimitExceeded = 109,
    /// Requested position does not exist.
    PositionNotFound = 110,
    /// Position mode is not valid for this flow.
    InvalidPositionMode = 111,
    /// Tracked pool cash is insufficient for the requested outflow.
    InsufficientLiquidity = 112,
    /// LTV, liquidation threshold, bonus, or fee bounds are invalid.
    InvalidLiqThreshold = 113,
    /// Account is not eligible for bad-debt cleanup.
    CannotCleanBadDebt = 114,
    /// Liquidation withdrawal fee exceeds gross seized collateral.
    WithdrawLessThanFee = 115,
    /// Borrow, flash-loan fee, or cap config is invalid.
    InvalidBorrowParams = 116,
    /// Interest model utilization breakpoints are invalid.
    InvalidUtilRange = 117,
    /// Optimal utilization must be below 100%.
    OptUtilTooHigh = 118,
    /// Reserve factor must be less than 100%.
    InvalidReserveFactor = 119,
    /// Debt position is missing for the requested repayment, liquidation, or debt strategy.
    DebtPositionNotFound = 120,
    /// Collateral position is missing for the requested withdrawal, liquidation, or collateral strategy.
    CollateralPositionNotFound = 121,
    /// Strategy close requested while debt remains.
    CannotCloseWithRemainingDebt = 122,
    /// Post-mutation pool state would violate the solvency invariant.
    PoolInsolvent = 123,
    /// LTV-weighted collateral is below the instance minimum while debt remains.
    MinBorrowCollateralNotMet = 126,
    /// Operation would push utilization above the configured max.
    UtilizationAboveMax = 127,
    /// Base borrow rate cannot be negative.
    BaseRateNegative = 128,
    /// Interest-rate slopes must be monotonic.
    SlopeNonMonotonic = 129,
    /// Max borrow rate must exceed base rate.
    MaxRateBelowBase = 130,
    /// Max borrow rate exceeds the Taylor-series safety envelope.
    MaxBorrowRateTooHigh = 131,
    /// Asset decimals exceed the supported RAY conversion domain.
    AssetDecimalsTooHigh = 132,
    /// An account owner cannot liquidate their own account.
    SelfLiquidationNotAllowed = 133,
    /// Spoke liquidation curve bounds are invalid.
    InvalidLiquidationCurve = 134,
    /// Solvent-toxic: partial liq not HF-safe; full debt close required.
    FullCloseRequired = 135,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    /// Aggregator address is not a deployed WASM contract.
    InvalidAggregator = 201,
    /// Oracle asset ref type is not supported by the provider.
    InvalidOracleTokenType = 204,
    /// Price resolution policy rejected an unsafe or out-of-tolerance price.
    UnsafePriceNotAllowed = 205,
    /// Price timestamp is older than the configured staleness window.
    PriceFeedStale = 206,
    /// First tolerance is outside the allowed range.
    BadFirstTolerance = 207,
    /// Last tolerance band is inverted or outside the allowed envelope.
    BadLastTolerance = 208,
    /// Anchor tolerance bounds are inconsistent.
    BadAnchorTolerances = 209,
    /// Oracle source has no last price (or required anchor leg is missing).
    NoLastPrice = 210,
    /// Revenue accumulator is not configured.
    NoAccumulator = 211,
    /// Reflector TWAP history is empty.
    ReflectorHistoryEmpty = 212,
    /// Market oracle is not configured.
    OracleNotConfigured = 216,
    /// Oracle returned a non-positive or malformed price.
    InvalidPrice = 217,
    /// Staleness configuration is invalid.
    InvalidStalenessConfig = 218,
    /// TWAP mode does not have enough observations.
    TwapInsufficientObservations = 219,
    /// Oracle base asset or quote is invalid.
    InvalidOracleBase = 220,
    /// Oracle decimals are invalid or unsupported.
    InvalidOracleDecimals = 221,
    /// Oracle resolution seconds are invalid.
    InvalidOracleResolution = 222,
    /// Final price is outside configured sanity bounds.
    SanityBoundViolated = 223,
    /// Price-bound min/max configuration is invalid.
    InvalidSanityBounds = 224,
    /// Quote/anchor cycle: asset re-entered while already being priced.
    OracleCycleDetected = 225,
    /// Single-source sanity band exceeds `MAX_SINGLE_SOURCE_SANITY_BAND_BPS`.
    SanityBandTooWideForSingleSource = 226,
    /// Strategy/anchor incoherence: `PrimaryWithAnchor` without an anchor, or
    /// `Single` with one.
    AnchorConfigMismatch = 227,
    /// TWAP record count above `MAX_TWAP_RECORDS`.
    TwapRecordsOutOfRange = 228,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SpokeError {
    /// Spoke id does not exist.
    SpokeNotFound = 300,
    /// Spoke has been deprecated.
    SpokeDeprecated = 301,
    /// Asset is not listed on the requested spoke.
    AssetNotInSpoke = 307,
    /// Asset is already listed on the requested spoke.
    AssetAlreadyInSpoke = 308,
    /// Spoke asset still carries nonzero usage (live positions).
    SpokeAssetInUse = 309,
    /// Requested spoke does not match the account's spoke.
    SpokeMismatch = 310,
    /// Spoke supply cap would be exceeded.
    SpokeSupplyCapReached = 311,
    /// Spoke borrow cap would be exceeded.
    SpokeBorrowCapReached = 312,
    /// Spoke asset is paused: no supply/borrow/withdraw/repay.
    SpokeAssetPaused = 315,
    /// Spoke asset is frozen: no new supply/borrow (repay/withdraw allowed).
    SpokeAssetFrozen = 316,
    /// Immediate guardian flag path may only tighten; relaxing rides the timelock.
    SpokeAssetFlagRelaxation = 317,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FlashLoanError {
    /// Re-entrancy guard: a flash loan or strategy is already executing in this tx.
    FlashLoanOngoing = 400,
    /// Asset is not configured for flash loans.
    FlashloanNotEnabled = 401,
    /// Flash-loan callback did not restore the expected pool cash.
    InvalidFlashloanRepay = 402,
    /// Strategy fee exceeds the borrowed amount.
    StrategyFeeExceeds = 409,
    // 411 reserved (off-chain monitor compatibility).
    /// Flash-loan receiver is not a deployed WASM contract.
    InvalidFlashloanReceiver = 412,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyError {
    /// Strategy requires conversion steps that were not supplied.
    ConvertStepsRequired = 500,
    /// Aggregator moved more input than authorized or moved it the wrong way.
    RouterOverspend = 501,
    /// Aggregator swap produced zero output.
    NoSwapOutput = 502,
}
