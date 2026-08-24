//! Contract error enums returned by fallible entry points, grouped by domain.

use soroban_sdk::contracterror;

/// Error codes for general contract, registry, account, timelock, and
/// role-management failures not covered by a more specific error enum below.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GenericError {
    // Reserved: retired codes kept declared so their numbers can never be
    // reused and existing error codes never shift. See docs/reference/errors.md.
    // Not dead code — do not delete.
    AssetNotSupported = 1,

    AssetAlreadySupported = 2,

    InvalidTicker = 3,

    PoolAlreadyDeployed = 5,

    InvalidAsset = 6,

    AssetsAreTheSame = 7,

    WrongToken = 8,

    InvalidWasmHash = 10,

    // Reserved: see the note on `AssetNotSupported` above.
    InvalidExchangeSrc = 11,

    // Reserved: see the note on `AssetNotSupported` above.
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

    PositionNftNotSet = 53,

    PositionNftAlreadyDeployed = 54,
}

/// Error codes for collateral, position, interest-rate-curve, and
/// liquidation validation failures.
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

/// Error codes for oracle configuration, price-feed validation, and
/// staleness failures.
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

    /// The immediate `set_sanity_band` path may only tighten a band. Widening
    /// (a lower min or higher max) must go through the timelocked
    /// `ConfigureAssetOracle` path so it has a reaction window (INV-AUTH-04).
    SanityBandMustTighten = 227,

    TwapRecordsOutOfRange = 228,

    OracleDepthExceeded = 229,

    FactorOutOfBounds = 230,

    SourceCountOutOfRange = 231,

    IndependenceNotDeclared = 232,

    UnsupportedAquariusPool = 234,

    InsufficientAquariusLiquidity = 235,
}

/// Error codes for spoke registration and per-spoke asset configuration
/// failures.
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

    /// Immediate GUARDIAN `set_spoke_asset_flags` only: cannot clear paused/frozen/no_seize.
    /// Timelocked `edit_asset_in_spoke` may clear flags intentionally.
    SpokeAssetFlagRelaxation = 317,

    /// The listing's `no_seize` flag is set: this asset cannot be taken as liquidation
    /// collateral. Unlike `SpokeAssetPaused` this gates only the seizure leg.
    SpokeAssetSeizureHalted = 318,
}

/// Error codes for flash-loan execution and repayment failures.
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

/// Error codes for strategy conversion and swap-routing failures.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyError {
    ConvertStepsRequired = 500,

    RouterOverspend = 501,

    NoSwapOutput = 502,

    /// Declared collateral list is empty or every minimum is zero.
    CollateralRequired = 503,

    /// Measured collateral push is below the caller-declared minimum.
    CollateralMinimumNotMet = 504,

    /// `flash_position` finished debt-free or without supply — a round-trip
    /// close that would be a free cash flash loan.
    FlashPositionClosed = 505,
}
