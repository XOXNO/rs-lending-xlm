//! Contract error codes and protocol failure meanings.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GenericError {
    AssetNotSupported = 1,
    AssetAlreadySupported = 2,
    InvalidTicker = 3,
    PoolAlreadyDeployed = 5,
    InvalidAsset = 6,
    AssetsAreTheSame = 7,
    WrongToken = 8,
    InvalidPoolTemplate = 10,
    InvalidExchangeSrc = 11,
    PairNotActive = 12,
    AccountNotInMarket = 13,
    /// Amount must be strictly positive for this operation.
    AmountMustBePositive = 14,
    InvalidPayments = 16,
    NotSmartContract = 18,
    AccountNotFound = 24,
    AccountModeMismatch = 25,
    TemplateNotSet = 26,
    AggregatorNotSet = 27,
    PositionLimitsNotSet = 29,
    PoolNotInitialized = 30,
    OwnerNotSet = 32,
    /// Fixed-point or ledger arithmetic overflow/underflow.
    MathOverflow = 33,
    /// Internal invariant failed after prior validation (should be unreachable).
    InternalError = 34,
    InvalidPositionLimits = 36,
    NoSuppliersToReward = 37,
    SpotOnlyNotProductionSafe = 38,
    InvalidTimelockDelay = 39,
    TimelockOperationExpired = 40,
    InvalidRole = 41,
    BlendPoolNotApproved = 42,
    HubNotActive = 43,
    NotAuthorized = 44,
    RegistryCapReached = 45,
    OperationNotCancellable = 46,
    /// Positive raw borrow floors to zero scaled debt (would free-borrow tokens).
    BorrowRoundsToZeroShares = 47,
    /// Would remove the last PROPOSER (permanently freezes governance scheduling).
    CannotRemoveLastProposer = 48,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollateralError {
    InsufficientCollateral = 100,
    HealthFactorTooHigh = 101,
    HealthFactorTooLow = 102,
    NotCollateral = 104,
    AssetNotBorrowable = 107,
    PositionLimitExceeded = 109,
    PositionNotFound = 110,
    InvalidPositionMode = 111,
    InsufficientLiquidity = 112,
    InvalidLiqThreshold = 113,
    CannotCleanBadDebt = 114,
    WithdrawLessThanFee = 115,
    InvalidBorrowParams = 116,
    InvalidUtilRange = 117,
    /// Optimal utilization must be below 100%.
    OptUtilTooHigh = 118,
    /// Reserve factor must be less than 100%.
    InvalidReserveFactor = 119,
    DebtPositionNotFound = 120,
    CollateralPositionNotFound = 121,
    CannotCloseWithRemainingDebt = 122,
    /// Post-mutation pool state would violate the solvency invariant.
    PoolInsolvent = 123,
    MinBorrowCollateralNotMet = 126,
    UtilizationAboveMax = 127,
    BaseRateNegative = 128,
    /// Interest-rate slopes must be monotonic.
    SlopeNonMonotonic = 129,
    /// Max borrow rate must exceed base rate.
    MaxRateBelowBase = 130,
    MaxBorrowRateTooHigh = 131,
    AssetDecimalsTooHigh = 132,
    SelfLiquidationNotAllowed = 133,
    InvalidLiquidationCurve = 134,
    /// Solvent-toxic: partial liq not HF-safe; full debt close required.
    FullCloseRequired = 135,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    InvalidAggregator = 201,
    InvalidOracleTokenType = 204,
    UnsafePriceNotAllowed = 205,
    PriceFeedStale = 206,
    BadFirstTolerance = 207,
    BadLastTolerance = 208,
    BadAnchorTolerances = 209,
    NoLastPrice = 210,
    NoAccumulator = 211,
    ReflectorHistoryEmpty = 212,
    OracleNotConfigured = 216,
    InvalidPrice = 217,
    InvalidStalenessConfig = 218,
    TwapInsufficientObservations = 219,
    InvalidOracleBase = 220,
    InvalidOracleDecimals = 221,
    InvalidOracleResolution = 222,
    SanityBoundViolated = 223,
    InvalidSanityBounds = 224,
    /// Quote/anchor cycle: asset re-entered while already being priced.
    OracleCycleDetected = 225,
    SanityBandTooWideForSingleSource = 226,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SpokeError {
    SpokeNotFound = 300,
    SpokeDeprecated = 301,
    AssetNotInSpoke = 307,
    AssetAlreadyInSpoke = 308,
    SpokeAssetInUse = 309,
    SpokeMismatch = 310,
    SpokeSupplyCapReached = 311,
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
    FlashLoanOngoing = 400,
    FlashloanNotEnabled = 401,
    InvalidFlashloanRepay = 402,
    StrategyFeeExceeds = 409,
    // 411 reserved (off-chain monitor compatibility).
    InvalidFlashloanReceiver = 412,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyError {
    ConvertStepsRequired = 500,
    RouterOverspend = 501,
    NoSwapOutput = 502,
}
