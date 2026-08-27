#![no_std]
#![allow(clippy::too_many_arguments)]

pub use common::types::{
    AquariusLpSource, AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef,
    OracleTolerance, PriceKey, PriceSource, ProviderRef, ReflectorFeedRef, ScaledSource,
};
use common::types::{HubAssetKey, PositionLimits};
use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractclient, contracttype, Address, BytesN, Env, String, Symbol, Val, Vec};
pub use stellar_governance::timelock::OperationState;

pub use common::types::SpokeAssetArgs;

#[contracttype]
#[derive(Clone, Debug)]
pub struct RemoveAssetFromSpokeArgs {
    pub hub_asset: HubAssetKey,
    pub spoke_id: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CreatePoolArgs {
    pub hub_id: u32,
    pub asset: Address,
    pub params: MarketParamsRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct UpgradePoolParamsArgs {
    pub hub_asset: HubAssetKey,
    pub params: InterestRateModel,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployPositionNftArgs {
    pub wasm_hash: BytesN<32>,
    pub uri: String,
    pub name: String,
    pub symbol: String,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TransferOwnershipArgs {
    pub new_owner: Address,
    pub live_until_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ConfigureAssetOracleArgs {
    pub key: PriceKey,
    pub oracle: AssetOracle,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EditToleranceArgs {
    pub key: PriceKey,
    pub tolerance: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeLiquidationCurveArgs {
    pub spoke_id: u32,
    pub target_hf_wad: i128,
    pub hf_for_max_bonus_wad: i128,
    pub liquidation_bonus_factor_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RoleArgs {
    pub account: Address,
    pub role: Symbol,
}

#[contracttype]
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum AdminOperation {
    SetSwapAggregator(Address),

    SetPriceAggregator(Address),
    SetAccumulator(Address),
    SetPositionLimits(PositionLimits),
    SetMinBorrowCollateralUsd(i128),
    CreateHub,
    AddSpoke,
    RemoveSpoke(u32),
    AddAssetToSpoke(SpokeAssetArgs),
    EditAssetInSpoke(SpokeAssetArgs),
    RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs),
    ApproveBlendPool(Address),
    RevokeBlendPool(Address),
    CreateLiquidityPool(CreatePoolArgs),
    UpgradeLiquidityPoolParams(UpgradePoolParamsArgs),
    DeployPool(BytesN<32>),
    DeployPositionNft(DeployPositionNftArgs),
    UpgradePool(BytesN<32>),

    UpgradePositionNft(BytesN<32>),

    UpgradePriceAggregator(BytesN<32>),
    SetPositionManager(Address, bool),
    UpgradeController(BytesN<32>),
    MigrateController(u32),
    TransferCtrlOwnership(TransferOwnershipArgs),

    EditOracleTolerance(EditToleranceArgs),
    SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs),

    ForceSocializeBadDebt(u64),

    Unpause,

    UpgradeGov(BytesN<32>),
    UpdateGovDelay(u32),
    GrantGovRole(RoleArgs),
    RevokeGovRole(RoleArgs),
    TransferGovOwnership(TransferOwnershipArgs),

    ConfigureAssetOracle(ConfigureAssetOracleArgs),
}

#[contractclient(name = "GovernanceClient")]

pub trait GovernanceInterface {
    fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address;

    fn controller(env: Env) -> Address;

    fn deploy_price_aggregator(env: Env, wasm_hash: BytesN<32>) -> Address;

    fn price_aggregator(env: Env) -> Address;

    fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val;

    fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>);

    fn get_min_delay(env: Env) -> u32;

    fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState;

    fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32;

    fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance;

    fn resolve_asset_oracle(env: Env, key: PriceKey, oracle: AssetOracle) -> AssetOracle;

    fn propose(env: Env, proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32>;

    fn pause(env: Env, caller: Address);

    fn set_spoke_asset_flags(
        env: Env,
        caller: Address,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
        no_seize: bool,
    );

    fn set_sanity_band(env: Env, caller: Address, key: PriceKey, min_wad: i128, max_wad: i128);

    fn create_hub(env: Env, caller: Address) -> u32;

    fn add_spoke(env: Env, caller: Address) -> u32;

    fn revoke_role_immediate(env: Env, account: Address, role: Symbol);

    fn execute_self(env: Env, executor: Option<Address>, op: AdminOperation, salt: BytesN<32>);

    fn propose_canceller_reset(
        env: Env,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    fn execute_canceller_reset(
        env: Env,
        executor: Option<Address>,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    );

    fn accept_ownership(env: Env);

    fn has_role(env: Env, account: Address, role: Symbol) -> bool;
}
