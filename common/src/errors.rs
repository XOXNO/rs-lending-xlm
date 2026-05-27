//! Contract error codes. Variant names are the canonical machine-readable
//! conditions; docs below describe the protocol-level failure for auditors
//! and indexers.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GenericError {
    AssetNotSupported = 1,
    AssetAlreadySupported = 2,
    InvalidTicker = 3,
    NoPoolFound = 4,
    TemplateEmpty = 5,
    InvalidAsset = 6,
    AssetsAreTheSame = 7,
    WrongToken = 8,
    InvalidNumTransfers = 9,
    InvalidPoolTemplate = 10,
    InvalidExchangeSrc = 11,
    PairNotActive = 12,
    AccountNotInMarket = 13,
    AmountMustBePositive = 14,
    AddressIsZero = 15,
    InvalidPayments = 16,
    AccountAttrsMismatch = 17,
    NotSmartContract = 18,
    InvalidEndpoint = 19,
    InvalidShard = 20,
    InvalidOnedexPair = 21,
    /// All mutating operations are rejected while the controller is paused.
    ContractPaused = 22,
    InvalidPositionType = 23,
    AccountNotFound = 24,
    AccountModeMismatch = 25,
    TemplateNotSet = 26,
    AggregatorNotSet = 27,
    AccumulatorNotSet = 28,
    PositionLimitsNotSet = 29,
    /// Pool storage record missing or never initialized for the asset.
    PoolNotInitialized = 30,
    PoolsListNotFound = 31,
    OwnerNotSet = 32,
    /// Integer overflow/underflow in scaled balance or index math.
    MathOverflow = 33,
    InternalError = 34,
    TokenNotApproved = 35,
    InvalidPositionLimits = 36,
    /// Rewards cannot be added when there are zero suppliers (would be lost).
    NoSuppliersToReward = 37,
    /// Single-oracle (no TWAP/anchor) mode is rejected in production config.
    SpotOnlyNotProductionSafe = 38,
    AddOverflow = 39,
    SubUnderflow = 40,
    MulOverflow = 41,
    DivByZero = 42,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollateralError {
    InsufficientCollateral = 100,
    HealthFactorTooHigh = 101,
    /// Post-operation health factor would be below the liquidation threshold.
    HealthFactorTooLow = 102,
    TokenMismatch = 103,
    NotCollateral = 104,
    SupplyCapReached = 105,
    BorrowCapReached = 106,
    AssetNotBorrowable = 107,
    NotBorrowableSiloed = 108,
    PositionLimitExceeded = 109,
    PositionNotFound = 110,
    InvalidPositionMode = 111,
    /// Pool on-chain balance or scaled reserves insufficient for the operation.
    InsufficientLiquidity = 112,
    InvalidLiqThreshold = 113,
    CannotCleanBadDebt = 114,
    WithdrawLessThanFee = 115,
    InvalidBorrowParams = 116,
    InvalidUtilRange = 117,
    OptUtilTooHigh = 118,
    InvalidReserveFactor = 119,
    DebtPositionNotFound = 120,
    CollateralPositionNotFound = 121,
    CannotCloseWithRemainingDebt = 122,
    /// Post-mutation pool state would violate the solvency invariant.
    PoolInsolvent = 123,
    PoolPaused = 124,
    DustFloorTooLow = 125,
    DustResidueNotAllowed = 126,
    /// Operation would push utilization above the configured max.
    UtilizationAboveMax = 127,
    BaseRateNegative = 128,
    SlopeNonMonotonic = 129,
    MaxRateBelowBase = 130,
    MaxBorrowRateTooHigh = 131,
    AssetDecimalsTooHigh = 132,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    PriceAggregatorNotSet = 200,
    InvalidAggregator = 201,
    OracleTokenNotFound = 202,
    OracleTokenExisting = 203,
    InvalidOracleTokenType = 204,
    /// Price resolution policy rejected an unsafe or out-of-tolerance price.
    UnsafePriceNotAllowed = 205,
    PriceFeedStale = 206,
    BadFirstTolerance = 207,
    BadLastTolerance = 208,
    BadAnchorTolerances = 209,
    NoLastPrice = 210,
    NoAccumulator = 211,
    ReflectorHistoryEmpty = 212,
    DexOracleUnavailable = 213,
    OracleAlreadyConfigured = 214,
    ReflectorNotConfigured = 215,
    OracleNotConfigured = 216,
    InvalidPrice = 217,
    InvalidStalenessConfig = 218,
    TwapInsufficientObservations = 219,
    InvalidOracleBase = 220,
    InvalidOracleDecimals = 221,
    InvalidOracleResolution = 222,
    SanityBoundViolated = 223,
    InvalidSanityBounds = 224,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum EModeError {
    EModeCategoryNotFound = 300,
    EModeCategoryDeprecated = 301,
    /// E-mode and isolation mode cannot be combined on the same account.
    EModeWithIsolated = 302,
    MixIsolatedCollateral = 303,
    DebtCeilingReached = 304,
    NotBorrowableIsolation = 305,
    AssetInEmodeExists = 306,
    AssetNotInEmode = 307,
    AssetAlreadyInEmode = 308,
    EModeAssetsLimitReached = 309,
    EModeMismatch = 310,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FlashLoanError {
    /// Re-entrancy guard: a flash loan or strategy is already executing in this tx.
    FlashLoanOngoing = 400,
    FlashloanNotEnabled = 401,
    /// Flash-loan callback did not restore the expected pool balance.
    InvalidFlashloanRepay = 402,
    FlashloanReserve = 403,
    SwapCollateralNoIso = 404,
    BulkSupplyNoIso = 405,
    SwapDebtNotSupported = 406,
    NoDebtPayments = 407,
    MultiplyExtraSteps = 408,
    StrategyFeeExceeds = 409,
    InvalidBulkTicker = 410,
    // 411 reserved — do not reuse without coordinating off-chain monitors.
    InvalidFlashloanReceiver = 412,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyError {
    ConvertStepsRequired = 500,
}
