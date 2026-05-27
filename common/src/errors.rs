//! Contract error codes. Variant names are the canonical machine-readable
//! conditions; docs below describe the protocol-level failure for auditors
//! and indexers.

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GenericError {
    /// Asset has no configured market.
    AssetNotSupported = 1,
    /// Asset already has a configured market.
    AssetAlreadySupported = 2,
    /// Token ticker or symbol failed validation.
    InvalidTicker = 3,
    /// Pool address is missing for an asset expected to be listed.
    NoPoolFound = 4,
    /// Pool template WASM hash has not been configured.
    TemplateEmpty = 5,
    /// Token contract failed required asset checks.
    InvalidAsset = 6,
    /// Operation requires two distinct assets.
    AssetsAreTheSame = 7,
    /// Token address does not match the expected market asset.
    WrongToken = 8,
    /// Transfer batch length does not match the expected shape.
    InvalidNumTransfers = 9,
    /// Pool template hash is empty or invalid.
    InvalidPoolTemplate = 10,
    /// Swap source is not accepted by the aggregator route.
    InvalidExchangeSrc = 11,
    /// Market is not active for the requested operation.
    PairNotActive = 12,
    /// Account does not belong to the requested market or caller.
    AccountNotInMarket = 13,
    /// Amount must be strictly positive for this operation.
    AmountMustBePositive = 14,
    /// Zero address is not accepted.
    AddressIsZero = 15,
    /// Payment vector is empty, duplicated incorrectly, or otherwise malformed.
    InvalidPayments = 16,
    /// Account attributes do not match the expected account mode.
    AccountAttrsMismatch = 17,
    /// Address is not a deployed WASM contract.
    NotSmartContract = 18,
    /// Oracle or router endpoint failed validation.
    InvalidEndpoint = 19,
    /// Oracle shard identifier failed validation.
    InvalidShard = 20,
    /// Onedex pair failed validation.
    InvalidOnedexPair = 21,
    /// All mutating operations are rejected while the controller is paused.
    ContractPaused = 22,
    /// Position side is not valid for the requested operation.
    InvalidPositionType = 23,
    /// Account id has no stored account metadata.
    AccountNotFound = 24,
    /// Account mode does not match the requested strategy or position mode.
    AccountModeMismatch = 25,
    /// Pool template WASM hash has not been configured.
    TemplateNotSet = 26,
    /// Swap aggregator contract has not been configured.
    AggregatorNotSet = 27,
    /// Revenue accumulator contract has not been configured.
    AccumulatorNotSet = 28,
    /// Position limits have not been configured.
    PositionLimitsNotSet = 29,
    /// Pool storage record missing or never initialized for the asset.
    PoolNotInitialized = 30,
    /// Pool list storage is missing.
    PoolsListNotFound = 31,
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
    /// Checked addition overflowed.
    AddOverflow = 39,
    /// Checked subtraction would underflow.
    SubUnderflow = 40,
    /// Checked multiplication overflowed.
    MulOverflow = 41,
    /// Division denominator is zero.
    DivByZero = 42,
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
    /// Asset in a position or transfer does not match the expected token.
    TokenMismatch = 103,
    /// Asset is not eligible as collateral.
    NotCollateral = 104,
    /// Supply cap would be exceeded.
    SupplyCapReached = 105,
    /// Borrow cap would be exceeded.
    BorrowCapReached = 106,
    /// Asset is not borrowable in the current market config.
    AssetNotBorrowable = 107,
    /// Siloed borrowing asset cannot coexist with other debt assets.
    NotBorrowableSiloed = 108,
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
    /// Borrow, cap, or isolation config is invalid.
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
    /// Pool is paused for this operation.
    PoolPaused = 124,
    /// Dust floor must be zero/zero or at least the protocol minimum.
    DustFloorTooLow = 125,
    /// Operation would leave a non-zero position below the dust floor.
    DustResidueNotAllowed = 126,
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
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    /// Price aggregator contract has not been configured.
    PriceAggregatorNotSet = 200,
    /// Aggregator address is not a deployed WASM contract.
    InvalidAggregator = 201,
    /// Oracle token mapping was not found.
    OracleTokenNotFound = 202,
    /// Oracle token mapping already exists.
    OracleTokenExisting = 203,
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
    /// DEX oracle source could not provide a usable price.
    DexOracleUnavailable = 213,
    /// Market oracle is already configured.
    OracleAlreadyConfigured = 214,
    /// Reflector source is missing required config.
    ReflectorNotConfigured = 215,
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
    /// Final price is outside sanity bounds.
    SanityBoundViolated = 223,
    /// Sanity bound min/max configuration is invalid.
    InvalidSanityBounds = 224,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum EModeError {
    /// E-mode category id does not exist.
    EModeCategoryNotFound = 300,
    /// E-mode category has been deprecated.
    EModeCategoryDeprecated = 301,
    /// E-mode and isolation mode cannot be combined on the same account.
    EModeWithIsolated = 302,
    /// Isolated collateral cannot be mixed with other collateral.
    MixIsolatedCollateral = 303,
    /// Isolated debt ceiling would be exceeded.
    DebtCeilingReached = 304,
    /// Asset cannot be borrowed while account is isolated.
    NotBorrowableIsolation = 305,
    /// Asset is already present in an e-mode category.
    AssetInEmodeExists = 306,
    /// Asset is not present in the requested e-mode category.
    AssetNotInEmode = 307,
    /// Asset is already in the requested e-mode category.
    AssetAlreadyInEmode = 308,
    /// E-mode category asset count would exceed the cap.
    EModeAssetsLimitReached = 309,
    /// Requested e-mode category does not match account category.
    EModeMismatch = 310,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FlashLoanError {
    /// Re-entrancy guard: a flash loan or strategy is already executing in this tx.
    FlashLoanOngoing = 400,
    /// Asset is not configured for flash loans.
    FlashloanNotEnabled = 401,
    /// Flash-loan callback did not restore the expected pool balance.
    InvalidFlashloanRepay = 402,
    /// Flash-loan reserves are insufficient.
    FlashloanReserve = 403,
    /// Swap-collateral strategy is not allowed for isolated accounts.
    SwapCollateralNoIso = 404,
    /// Bulk supply cannot include isolated collateral.
    BulkSupplyNoIso = 405,
    /// Swap-debt strategy is not supported for this debt set.
    SwapDebtNotSupported = 406,
    /// Strategy requires at least one debt payment.
    NoDebtPayments = 407,
    /// Multiply strategy received unsupported extra route steps.
    MultiplyExtraSteps = 408,
    /// Strategy fee exceeds the borrowed amount.
    StrategyFeeExceeds = 409,
    /// Strategy bulk ticker is invalid.
    InvalidBulkTicker = 410,
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
}
