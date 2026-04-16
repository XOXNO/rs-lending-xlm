use soroban_sdk::contracterror;

// ---------------------------------------------------------------------------
// Domain-specific error enums. Each `#[contracterror]` enum occupies its own
// code range, so codes never collide across domains. Import only the
// category your module needs.
// ---------------------------------------------------------------------------

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
    ContractPaused = 22,
    InvalidPositionType = 23,
    AccountNotFound = 24,
    AccountModeMismatch = 25,
    TemplateNotSet = 26,
    AggregatorNotSet = 27,
    AccumulatorNotSet = 28,
    PositionLimitsNotSet = 29,
    PoolNotInitialized = 30,
    PoolsListNotFound = 31,
    OwnerNotSet = 32,
    MathOverflow = 33,
    InternalError = 34,
    TokenNotApproved = 35,
    InvalidPositionLimits = 36,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CollateralError {
    InsufficientCollateral = 100,
    HealthFactorTooHigh = 101,
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
    PoolPaused = 124,
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
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum EModeError {
    EModeCategoryNotFound = 300,
    EModeCategoryDeprecated = 301,
    EModeWithIsolated = 302,
    MixIsolatedCollateral = 303,
    DebtCeilingReached = 304,
    NotBorrowableIsolation = 305,
    AssetInEmodeExists = 306,
    AssetNotInEmode = 307,
    AssetAlreadyInEmode = 308,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FlashLoanError {
    FlashLoanOngoing = 400,
    FlashloanNotEnabled = 401,
    InvalidFlashloanRepay = 402,
    FlashloanReserve = 403,
    SwapCollateralNoIso = 404,
    BulkSupplyNoIso = 405,
    SwapDebtNotSupported = 406,
    NoDebtPayments = 407,
    MultiplyExtraSteps = 408,
    StrategyFeeExceeds = 409,
    InvalidBulkTicker = 410,
    NegativeFlashLoanFee = 411,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyError {
    ConvertStepsRequired = 500,
}
