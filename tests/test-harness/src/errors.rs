pub use common::errors::{
    CollateralError, FlashLoanError, GenericError, OracleError, SpokeError, StrategyError,
};

pub mod codes {
    use super::*;

    pub const INVALID_ASSET: u32 = GenericError::InvalidAsset as u32;
    pub const ASSETS_ARE_THE_SAME: u32 = GenericError::AssetsAreTheSame as u32;
    pub const ACCOUNT_NOT_IN_MARKET: u32 = GenericError::AccountNotInMarket as u32;
    pub const AMOUNT_MUST_BE_POSITIVE: u32 = GenericError::AmountMustBePositive as u32;
    pub const INVALID_PAYMENTS: u32 = GenericError::InvalidPayments as u32;
    pub const ACCOUNT_MODE_MISMATCH: u32 = GenericError::AccountModeMismatch as u32;
    pub const INVALID_POSITION_LIMITS: u32 = GenericError::InvalidPositionLimits as u32;
    pub const NOT_SMART_CONTRACT: u32 = GenericError::NotSmartContract as u32;
    pub const INVALID_ROLE: u32 = GenericError::InvalidRole as u32;
    pub const BLEND_POOL_NOT_APPROVED: u32 = GenericError::BlendPoolNotApproved as u32;
    pub const NOT_AUTHORIZED: u32 = GenericError::NotAuthorized as u32;
    pub const ACCOUNT_NOT_FOUND: u32 = GenericError::AccountNotFound as u32;
    pub const POSITION_NFT_ALREADY_DEPLOYED: u32 = GenericError::PositionNftAlreadyDeployed as u32;

    pub const INSUFFICIENT_COLLATERAL: u32 = CollateralError::InsufficientCollateral as u32;
    pub const HEALTH_FACTOR_TOO_HIGH: u32 = CollateralError::HealthFactorTooHigh as u32;
    pub const HEALTH_FACTOR_TOO_LOW: u32 = CollateralError::HealthFactorTooLow as u32;
    pub const FULL_CLOSE_REQUIRED: u32 = CollateralError::FullCloseRequired as u32;
    pub const NOT_COLLATERAL: u32 = CollateralError::NotCollateral as u32;
    pub const ASSET_NOT_BORROWABLE: u32 = CollateralError::AssetNotBorrowable as u32;

    pub const POSITION_LIMIT_EXCEEDED: u32 = CollateralError::PositionLimitExceeded as u32;
    pub const POSITION_NOT_FOUND: u32 = CollateralError::PositionNotFound as u32;
    pub const INVALID_POSITION_MODE: u32 = CollateralError::InvalidPositionMode as u32;
    pub const INSUFFICIENT_LIQUIDITY: u32 = CollateralError::InsufficientLiquidity as u32;
    pub const INVALID_LIQ_THRESHOLD: u32 = CollateralError::InvalidLiqThreshold as u32;
    pub const CANNOT_CLEAN_BAD_DEBT: u32 = CollateralError::CannotCleanBadDebt as u32;
    pub const INVALID_BORROW_PARAMS: u32 = CollateralError::InvalidBorrowParams as u32;
    pub const INVALID_UTIL_RANGE: u32 = CollateralError::InvalidUtilRange as u32;
    pub const DEBT_POSITION_NOT_FOUND: u32 = CollateralError::DebtPositionNotFound as u32;
    pub const COLLATERAL_POSITION_NOT_FOUND: u32 =
        CollateralError::CollateralPositionNotFound as u32;
    pub const CANNOT_CLOSE_WITH_REMAINING_DEBT: u32 =
        CollateralError::CannotCloseWithRemainingDebt as u32;
    pub const POOL_INSOLVENT: u32 = CollateralError::PoolInsolvent as u32;
    pub const MIN_BORROW_COLLATERAL_NOT_MET: u32 =
        CollateralError::MinBorrowCollateralNotMet as u32;
    pub const UTILIZATION_ABOVE_MAX: u32 = CollateralError::UtilizationAboveMax as u32;
    pub const MAX_BORROW_RATE_TOO_HIGH: u32 = CollateralError::MaxBorrowRateTooHigh as u32;
    pub const SELF_LIQUIDATION_NOT_ALLOWED: u32 = CollateralError::SelfLiquidationNotAllowed as u32;
    pub const INVALID_LIQUIDATION_CURVE: u32 = CollateralError::InvalidLiquidationCurve as u32;

    pub const UNSAFE_PRICE: u32 = OracleError::UnsafePriceNotAllowed as u32;
    pub const PRICE_FEED_STALE: u32 = OracleError::PriceFeedStale as u32;
    pub const NO_LAST_PRICE: u32 = OracleError::NoLastPrice as u32;
    pub const BAD_LAST_TOLERANCE: u32 = OracleError::BadLastTolerance as u32;
    pub const ORACLE_NOT_CONFIGURED: u32 = OracleError::OracleNotConfigured as u32;
    pub const SANITY_BOUND_VIOLATED: u32 = OracleError::SanityBoundViolated as u32;
    pub const INVALID_SANITY_BOUNDS: u32 = OracleError::InvalidSanityBounds as u32;

    pub const SPOKE_NOT_FOUND: u32 = SpokeError::SpokeNotFound as u32;
    pub const SPOKE_DEPRECATED: u32 = SpokeError::SpokeDeprecated as u32;
    pub const ASSET_NOT_IN_SPOKE: u32 = SpokeError::AssetNotInSpoke as u32;
    pub const SPOKE_MISMATCH: u32 = SpokeError::SpokeMismatch as u32;
    pub const SPOKE_SUPPLY_CAP_REACHED: u32 = SpokeError::SpokeSupplyCapReached as u32;
    pub const SPOKE_BORROW_CAP_REACHED: u32 = SpokeError::SpokeBorrowCapReached as u32;
    pub const SPOKE_ASSET_PAUSED: u32 = SpokeError::SpokeAssetPaused as u32;
    pub const SPOKE_ASSET_FROZEN: u32 = SpokeError::SpokeAssetFrozen as u32;
    pub const SPOKE_ASSET_FLAG_RELAXATION: u32 = SpokeError::SpokeAssetFlagRelaxation as u32;
    pub const SPOKE_ASSET_SEIZURE_HALTED: u32 = SpokeError::SpokeAssetSeizureHalted as u32;
    pub const SPOKE_ASSET_IN_USE: u32 = SpokeError::SpokeAssetInUse as u32;

    pub const FLASH_LOAN_ONGOING: u32 = FlashLoanError::FlashLoanOngoing as u32;
    pub const FLASHLOAN_NOT_ENABLED: u32 = FlashLoanError::FlashloanNotEnabled as u32;
    pub const INVALID_FLASHLOAN_REPAY: u32 = FlashLoanError::InvalidFlashloanRepay as u32;
    pub const INVALID_FLASHLOAN_RECEIVER: u32 = FlashLoanError::InvalidFlashloanReceiver as u32;

    pub const CONVERT_STEPS_REQUIRED: u32 = StrategyError::ConvertStepsRequired as u32;
    pub const ROUTER_OVERSPEND: u32 = StrategyError::RouterOverspend as u32;
    pub const NO_SWAP_OUTPUT: u32 = StrategyError::NoSwapOutput as u32;

    // Literals below come from OpenZeppelin stellar libraries, which expose no
    // error enums to derive from: stellar-contract-utils pausable (1000),
    // stellar-access ownable (2000, 2200), stellar-governance timelock (4002).
    pub const CONTRACT_PAUSED: u32 = 1000;

    pub const UNAUTHORIZED: u32 = 2000;

    pub const NO_PENDING_TRANSFER: u32 = 2200;

    pub const TIMELOCK_UNEXPECTED_STATE: u32 = 4002;
}

pub use codes::*;
