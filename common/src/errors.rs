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

    InvalidWasmHash = 10,

    InvalidExchangeSrc = 11,

    PairNotActive = 12,

    AccountNotInMarket = 13,

    AmountMustBePositive = 14,

    InvalidPayments = 16,

    NotSmartContract = 18,

    AccountNotFound = 24,

    AccountModeMismatch = 25,

    AggregatorNotSet = 27,

    PositionLimitsNotSet = 29,

    PoolNotInitialized = 30,

    OwnerNotSet = 32,

    MathOverflow = 33,

    InternalError = 34,

    InvalidPositionLimits = 36,

    SpotOnlyNotProductionSafe = 38,

    InvalidTimelockDelay = 39,

    TimelockOperationExpired = 40,

    InvalidRole = 41,

    BlendPoolNotApproved = 42,

    HubNotActive = 43,

    NotAuthorized = 44,

    RegistryCapReached = 45,

    OperationNotCancellable = 46,

    BorrowRoundsToZeroShares = 47,

    CannotRemoveLastProposer = 48,

    WithdrawRoundsToZeroShares = 49,

    NetSettleRoundsToZeroShares = 50,

    SupplyRoundsToZeroShares = 51,

    RepayRoundsToZeroShares = 52,
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

    OptUtilTooHigh = 118,

    InvalidReserveFactor = 119,

    DebtPositionNotFound = 120,

    CollateralPositionNotFound = 121,

    CannotCloseWithRemainingDebt = 122,

    PoolInsolvent = 123,

    MinBorrowCollateralNotMet = 126,

    UtilizationAboveMax = 127,

    BaseRateNegative = 128,

    SlopeNonMonotonic = 129,

    MaxRateBelowBase = 130,

    MaxBorrowRateTooHigh = 131,

    AssetDecimalsTooHigh = 132,

    SelfLiquidationNotAllowed = 133,

    InvalidLiquidationCurve = 134,

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

    BadLastTolerance = 208,

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

    OracleCycleDetected = 225,

    SanityBandTooWideForSingleSource = 226,

    TwapRecordsOutOfRange = 228,

    OracleDepthExceeded = 229,

    FactorOutOfBounds = 230,

    SourceCountOutOfRange = 231,

    IndependenceNotDeclared = 232,

    UnsupportedAquariusPool = 234,

    InsufficientAquariusLiquidity = 235,
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

    SpokeAssetPaused = 315,

    SpokeAssetFrozen = 316,

    /// Immediate GUARDIAN `set_spoke_asset_flags` only: cannot clear paused/frozen.
    /// Timelocked `edit_asset_in_spoke` may clear flags intentionally.
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
