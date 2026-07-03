//! Contract error codes and protocol failure meanings.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GenericError {
    /// Asset is not active or not listed for this flow.
    AssetNotSupported = 1,
    /// Asset has a configured market.
    AssetAlreadySupported = 2,
    /// Token ticker or symbol failed validation.
    InvalidTicker = 3,
    /// The central liquidity pool has been deployed.
    PoolAlreadyDeployed = 5,
    /// Token contract failed required asset checks.
    InvalidAsset = 6,
    /// Operation requires two distinct assets.
    AssetsAreTheSame = 7,
    /// Token address does not match the expected market asset.
    WrongToken = 8,
    /// Pool template hash is empty or invalid.
    InvalidPoolTemplate = 10,
    /// Swap source is not accepted by the aggregator route.
    InvalidExchangeSrc = 11,
    /// Asset has no active oracle configuration for this operation.
    PairNotActive = 12,
    /// Account is missing or caller is not authorized for it.
    AccountNotInMarket = 13,
    /// Amount must be strictly positive for this operation.
    AmountMustBePositive = 14,
    /// Payment vector is empty, duplicated incorrectly, or otherwise malformed.
    InvalidPayments = 16,
    /// Address is not a deployed WASM contract.
    NotSmartContract = 18,
    /// Account id has no stored account metadata.
    AccountNotFound = 24,
    /// Account mode does not match the requested strategy or position mode.
    AccountModeMismatch = 25,
    /// Pool template WASM hash has not been configured.
    TemplateNotSet = 26,
    /// Swap aggregator contract has not been configured.
    AggregatorNotSet = 27,
    /// Position limits have not been configured.
    PositionLimitsNotSet = 29,
    /// Pool storage record missing for the asset.
    PoolNotInitialized = 30,
    /// Ownable storage has no owner.
    OwnerNotSet = 32,
    /// Integer overflow/underflow in scaled balance or index math.
    MathOverflow = 33,
    /// Controller invariant failed after prior validation.
    InternalError = 34,
    /// Token must be approved before market creation.
    TokenNotApproved = 35,
    /// Configured account position limits are zero or above the protocol cap.
    InvalidPositionLimits = 36,
    /// Rewards cannot be added when there are zero suppliers (would be lost).
    NoSuppliersToReward = 37,
    /// Single-oracle (no TWAP/anchor) mode is rejected in production config.
    SpotOnlyNotProductionSafe = 38,
    /// Timelock minimum delay is zero, which would nullify the timelock.
    InvalidTimelockDelay = 39,
    /// Timelock operation is past its execution grace period.
    TimelockOperationExpired = 40,
    /// Requested role is not part of the protocol role allowlist.
    InvalidRole = 41,
    /// Migration source pool is not on the governance Blend-pool allowlist.
    BlendPoolNotApproved = 42,
    /// Target hub id is not a registered, active hub.
    HubNotActive = 43,
    /// Caller is neither the account owner nor an active delegated manager.
    NotAuthorized = 44,
    /// Bounded instance registry (approvals, Blend pools, managers, delegates) is full.
    RegistryCapReached = 45,
    /// Operation cannot be cancelled (role revocations are protected so a rogue
    /// role holder cannot veto their own removal).
    OperationNotCancellable = 46,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollateralError {
    /// Collateral value is insufficient for the requested borrow or withdrawal.
    InsufficientCollateral = 100,
    /// Account is not liquidatable because health factor is still at or above one.
    HealthFactorTooHigh = 101,
    /// Post-operation health factor would be below the liquidation threshold.
    HealthFactorTooLow = 102,
    /// Asset is not eligible as collateral.
    NotCollateral = 104,
    /// Asset is not borrowable in the account's spoke config.
    AssetNotBorrowable = 107,
    /// Account would exceed max supply or borrow position count.
    PositionLimitExceeded = 109,
    /// Requested position does not exist.
    PositionNotFound = 110,
    /// Position mode is not valid for this flow.
    InvalidPositionMode = 111,
    /// Pool on-chain balance or scaled reserves insufficient for the operation.
    InsufficientLiquidity = 112,
    /// LTV, liquidation threshold, or bonus bounds are invalid.
    InvalidLiqThreshold = 113,
    /// Account is not eligible for bad-debt cleanup.
    CannotCleanBadDebt = 114,
    /// Liquidation withdrawal fee exceeds gross seized collateral.
    WithdrawLessThanFee = 115,
    /// Borrow or cap config is invalid.
    InvalidBorrowParams = 116,
    /// Interest model utilization breakpoints are invalid.
    InvalidUtilRange = 117,
    /// Optimal utilization must be below 100%.
    OptUtilTooHigh = 118,
    /// Reserve factor must be less than 100%.
    InvalidReserveFactor = 119,
    /// Debt position is missing for a repayment or liquidation.
    DebtPositionNotFound = 120,
    /// Collateral position is missing for withdrawal or liquidation.
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
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    /// Aggregator address is not a deployed WASM contract.
    InvalidAggregator = 201,
    /// Oracle token type is not supported.
    InvalidOracleTokenType = 204,
    /// Price resolution policy rejected an unsafe or out-of-tolerance price.
    UnsafePriceNotAllowed = 205,
    /// Price timestamp is older than the configured staleness window.
    PriceFeedStale = 206,
    /// First tolerance is outside the allowed range.
    BadFirstTolerance = 207,
    /// Last tolerance is outside the allowed range.
    BadLastTolerance = 208,
    /// Anchor tolerance bounds are inconsistent.
    BadAnchorTolerances = 209,
    /// Oracle source has no last price.
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
    /// Final price is outside configured price bounds.
    SanityBoundViolated = 223,
    /// Price-bound min/max configuration is invalid.
    InvalidSanityBounds = 224,
    /// Oracle resolution re-entered an asset already being priced — a quote/anchor
    /// cycle (e.g. two markets quoted in each other). Trapped to avoid unbounded
    /// recursion / stack exhaustion.
    OracleCycleDetected = 225,
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
    /// Spoke asset count would exceed the cap.
    SpokeAssetsLimitReached = 309,
    /// Requested spoke does not match the account's spoke.
    SpokeMismatch = 310,
    /// Spoke supply cap would be exceeded.
    SpokeSupplyCapReached = 311,
    /// Spoke borrow cap would be exceeded.
    SpokeBorrowCapReached = 312,
    /// Spoke cap would fall below current spoke usage.
    SpokeCapBelowUsage = 314,
    /// Spoke asset is paused: no supply/borrow/withdraw/repay.
    SpokeAssetPaused = 315,
    /// Spoke asset is frozen: no new supply/borrow (repay/withdraw allowed).
    SpokeAssetFrozen = 316,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FlashLoanError {
    /// Re-entrancy guard: a flash loan or strategy is executing in this tx.
    FlashLoanOngoing = 400,
    /// Asset is not configured for flash loans.
    FlashloanNotEnabled = 401,
    /// Flash-loan callback did not restore the expected pool balance.
    InvalidFlashloanRepay = 402,
    /// Strategy fee exceeds the borrowed amount.
    StrategyFeeExceeds = 409,
    // 411 reserved for off-chain monitors.
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
