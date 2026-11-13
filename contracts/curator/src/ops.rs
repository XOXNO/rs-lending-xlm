//! Registrar actions. Immediate calls return in the same transaction.
//! `propose` queues the four edits that wait on governance.

use common::errors::GenericError;
use common::ttl::renew_instance;
use common::types::{HubAssetKey, MarketParamsRaw, SpokeAssetArgs};
use controller_interface::ControllerClient;
use governance_interface::{AdminOperation, GovernanceClient};
use soroban_sdk::{assert_with_error, panic_with_error, Address, BytesN, ContractExecutable, Env};

use crate::events::{CancelledEvent, ProposedEvent};
use crate::storage;
use crate::{auth, CapGrant, CuratorError, ROLE_FLAGS, ROLE_MARKET, ROLE_SPOKE};

fn governance_client(env: &Env) -> GovernanceClient<'_> {
    GovernanceClient::new(env, &storage::governance(env))
}

fn me(env: &Env) -> Address {
    env.current_contract_address()
}

fn require_governance(env: &Env) {
    renew_instance(env);
    storage::governance(env).require_auth();
}

pub(crate) fn upgrade(env: &Env, wasm_hash: &BytesN<32>) {
    require_governance(env);
    assert_with_error!(
        env,
        wasm_hash.to_array() != [0; 32],
        GenericError::InvalidWasmHash
    );
    env.deployer()
        .update_current_contract(ContractExecutable::Wasm(wasm_hash.clone()));
}

pub(crate) fn set_registrar_suspended(env: &Env, registrar: u32, suspended: bool) {
    require_governance(env);
    require_known(env, registrar);
    storage::set_suspended(env, registrar, suspended);
}

pub(crate) fn force_set_admin(env: &Env, registrar: u32, admin: &Address) {
    require_governance(env);
    require_known(env, registrar);
    storage::set_registrar_admin(env, registrar, admin);
}

pub(crate) fn reassign_hub(env: &Env, hub_id: u32, registrar: Option<u32>) {
    require_governance(env);
    if let Some(registrar) = registrar {
        auth::require_registrar(env, registrar);
    }
    storage::set_hub_registrar(env, hub_id, registrar);
}

pub(crate) fn reassign_spoke(env: &Env, spoke_id: u32, registrar: Option<u32>) {
    require_governance(env);
    if let Some(registrar) = registrar {
        auth::require_registrar(env, registrar);
    }
    storage::set_spoke_registrar(env, spoke_id, registrar);
}

fn require_known(env: &Env, registrar: u32) {
    if storage::registrar_admin(env, registrar).is_none() {
        panic_with_error!(env, CuratorError::UnknownRegistrar);
    }
}

pub(crate) fn register_registrar(env: &Env, admin: &Address) -> u32 {
    require_governance(env);
    let registrar = storage::next_registrar(env);
    storage::set_registrar_admin(env, registrar, admin);
    registrar
}

pub(crate) fn set_admin(env: &Env, caller: &Address, registrar: u32, new_admin: &Address) {
    auth::require_admin(env, caller, registrar);
    storage::set_registrar_admin(env, registrar, new_admin);
}

pub(crate) fn set_roles(env: &Env, caller: &Address, registrar: u32, account: &Address, bits: u32) {
    auth::require_admin(env, caller, registrar);
    auth::require_role_bits(env, bits);
    storage::set_role_bits(env, registrar, account, bits);
}

pub(crate) fn create_hub(env: &Env, caller: &Address, registrar: u32) -> u32 {
    auth::require_role(env, caller, registrar, ROLE_SPOKE);
    let id = governance_client(env).create_hub(&me(env));
    assign_hub(env, id, registrar);
    id
}

pub(crate) fn add_spoke(env: &Env, caller: &Address, registrar: u32) -> u32 {
    auth::require_role(env, caller, registrar, ROLE_SPOKE);
    let id = governance_client(env).add_spoke(&me(env));
    assign_spoke(env, id, registrar);
    id
}

pub(crate) fn create_market(
    env: &Env,
    caller: &Address,
    registrar: u32,
    hub_id: u32,
    params: &MarketParamsRaw,
) -> Address {
    auth::require_role(env, caller, registrar, ROLE_MARKET);
    auth::require_hub(env, hub_id, registrar);
    governance_client(env).create_liquidity_pool(&me(env), &hub_id, params)
}

/// The spoke owner tightens any flag on a listing of its own hub's asset or of
/// a hub no registrar owns. On a listing of another registrar's hub asset it
/// may freeze or halt seizure but not pause: a pause blocks repayment and
/// liquidation of that hub's debt, and the hub owner could not lift it. The
/// owner of the listed hub may only freeze a listing of its asset in another
/// registrar's spoke.
pub(crate) fn tighten_flags(
    env: &Env,
    caller: &Address,
    registrar: u32,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    auth::require_role(env, caller, registrar, ROLE_FLAGS);
    let (paused, no_seize) = if storage::spoke_registrar(env, spoke_id) == Some(registrar) {
        if paused && owned_by_another(env, hub_asset.hub_id, registrar) {
            let current = ControllerClient::new(env, &governance_client(env).controller())
                .get_spoke_asset(&spoke_id, hub_asset);
            assert_with_error!(env, current.paused, CuratorError::ForeignPause);
        }
        (paused, no_seize)
    } else {
        auth::require_hub(env, hub_asset.hub_id, registrar);
        assert_with_error!(
            env,
            frozen && !paused && !no_seize,
            CuratorError::NotSpokeRegistrar
        );
        let current = ControllerClient::new(env, &governance_client(env).controller())
            .get_spoke_asset(&spoke_id, hub_asset);
        (current.paused, current.no_seize)
    };
    governance_client(env).set_spoke_asset_flags(
        &me(env),
        &spoke_id,
        hub_asset,
        &paused,
        &frozen,
        &no_seize,
    );
}

pub(crate) fn set_cap_grant(
    env: &Env,
    caller: &Address,
    registrar: u32,
    grantee: u32,
    hub_id: u32,
    spoke_id: u32,
    asset: &Address,
    grant: &CapGrant,
) {
    auth::require_role(env, caller, registrar, ROLE_MARKET);
    auth::require_hub(env, hub_id, registrar);
    auth::require_registrar(env, grantee);
    auth::require_spoke(env, spoke_id, grantee);
    assert_with_error!(
        env,
        grant.supply_cap >= 0 && grant.borrow_cap >= 0,
        CuratorError::GrantExceeded
    );
    storage::set_cap_grant(env, registrar, grantee, hub_id, spoke_id, asset, grant);
}

pub(crate) fn revoke_cap_grant(
    env: &Env,
    caller: &Address,
    registrar: u32,
    grantee: u32,
    hub_id: u32,
    spoke_id: u32,
    asset: &Address,
) {
    auth::require_role(env, caller, registrar, ROLE_MARKET);
    auth::require_hub(env, hub_id, registrar);
    storage::remove_cap_grant(env, registrar, grantee, hub_id, spoke_id, asset);
}

pub(crate) fn propose(
    env: &Env,
    caller: &Address,
    registrar: u32,
    op: AdminOperation,
    salt: BytesN<32>,
) -> BytesN<32> {
    let (hub, spoke) = match &op {
        AdminOperation::AddAssetToSpoke(args) | AdminOperation::EditAssetInSpoke(args) => {
            auth::require_role(env, caller, registrar, ROLE_MARKET);
            let listed = matches!(op, AdminOperation::EditAssetInSpoke(_));
            if listed && storage::spoke_registrar(env, args.spoke_id) != Some(registrar) {
                // The owner of the listed hub may lower the caps of its asset
                // in a spoke it does not own.
                auth::require_hub(env, args.hub_id, registrar);
                require_cap_lowering(env, args);
            } else {
                auth::require_spoke(env, args.spoke_id, registrar);
                require_grant(env, registrar, args, listed);
            }
            (Some(args.hub_id), Some(args.spoke_id))
        }
        AdminOperation::UpgradeLiquidityPoolParams(args) => {
            auth::require_role(env, caller, registrar, ROLE_MARKET);
            auth::require_hub(env, args.hub_asset.hub_id, registrar);
            (Some(args.hub_asset.hub_id), None)
        }
        AdminOperation::RelaxSpokeAssetFlags(args) => {
            auth::require_role(env, caller, registrar, ROLE_FLAGS);
            auth::require_spoke(env, args.spoke_id, registrar);
            if storage::hub_registrar(env, args.hub_asset.hub_id) != Some(registrar) {
                live_grant(
                    env,
                    registrar,
                    args.hub_asset.hub_id,
                    args.spoke_id,
                    &args.hub_asset.asset,
                );
            }
            (Some(args.hub_asset.hub_id), Some(args.spoke_id))
        }
        _ => panic_with_error!(env, CuratorError::Unsupported),
    };
    let op_id = governance_client(env).propose(&me(env), &op, &salt);
    storage::set_proposal(env, &op_id, hub, spoke);
    ProposedEvent {
        registrar,
        op_id: op_id.clone(),
    }
    .publish(env);
    op_id
}

/// Cancels a queued operation on behalf of the registrar that now owns the hub
/// or the spoke it names. Checks made when the operation was queued do not run
/// again at execution, so this is how a later revocation, reassignment or
/// admin change reaches an operation that is still pending.
pub(crate) fn cancel_proposal(env: &Env, caller: &Address, registrar: u32, op_id: &BytesN<32>) {
    auth::require_role(env, caller, registrar, ROLE_MARKET | ROLE_FLAGS);
    let (hub, spoke) = storage::proposal(env, op_id)
        .unwrap_or_else(|| panic_with_error!(env, CuratorError::ProposalUnknown));
    let owns_hub = hub.is_some_and(|id| storage::hub_registrar(env, id) == Some(registrar));
    let owns_spoke = spoke.is_some_and(|id| storage::spoke_registrar(env, id) == Some(registrar));
    assert_with_error!(env, owns_hub || owns_spoke, CuratorError::ProposalForeign);
    governance_client(env).cancel(&me(env), op_id);
    storage::remove_proposal(env, op_id);
    CancelledEvent {
        registrar,
        op_id: op_id.clone(),
    }
    .publish(env);
}

/// Requires `args` to keep the live listing as it is except for a supply cap
/// or a borrow cap that is no higher than the stored one. Flags, risk
/// parameters, and collateral and borrow permissions must match the listing.
fn require_cap_lowering(env: &Env, args: &SpokeAssetArgs) {
    let current = ControllerClient::new(env, &governance_client(env).controller()).get_spoke_asset(
        &args.spoke_id,
        &HubAssetKey {
            hub_id: args.hub_id,
            asset: args.asset.clone(),
        },
    );
    let unchanged = args.can_collateral == current.is_collateralizable
        && args.can_borrow == current.is_borrowable
        && args.paused == current.paused
        && args.frozen == current.frozen
        && args.no_seize == current.no_seize
        && args.ltv == current.loan_to_value
        && args.threshold == current.liquidation_threshold
        && args.bonus == current.liquidation_bonus
        && args.liquidation_fees == current.liquidation_fees;
    assert_with_error!(
        env,
        unchanged && args.supply_cap <= current.supply_cap && args.borrow_cap <= current.borrow_cap,
        CuratorError::NotCapLowering
    );
}

/// Whether a registrar other than `registrar` owns `hub_id`.
fn owned_by_another(env: &Env, hub_id: u32, registrar: u32) -> bool {
    storage::hub_registrar(env, hub_id).is_some_and(|owner| owner != registrar)
}

fn assign_hub(env: &Env, hub_id: u32, registrar: u32) {
    assert_with_error!(
        env,
        storage::hub_registrar(env, hub_id).is_none(),
        CuratorError::HubOwned
    );
    storage::set_hub_registrar(env, hub_id, Some(registrar));
}

fn assign_spoke(env: &Env, spoke_id: u32, registrar: u32) {
    assert_with_error!(
        env,
        storage::spoke_registrar(env, spoke_id).is_none(),
        CuratorError::SpokeOwned
    );
    storage::set_spoke_registrar(env, spoke_id, Some(registrar));
}

/// Own hub passes. A foreign hub must fit the cap its registrar stored for this
/// spoke, and a listing of another registrar's hub asset may not gain a pause:
/// a new listing (`listed` false) must be unpaused, and an edit may carry
/// `paused` only if it is already set.
fn require_grant(env: &Env, registrar: u32, args: &SpokeAssetArgs, listed: bool) {
    if storage::hub_registrar(env, args.hub_id) == Some(registrar) {
        return;
    }
    if args.paused && owned_by_another(env, args.hub_id, registrar) {
        let already = listed
            && ControllerClient::new(env, &governance_client(env).controller())
                .get_spoke_asset(
                    &args.spoke_id,
                    &HubAssetKey {
                        hub_id: args.hub_id,
                        asset: args.asset.clone(),
                    },
                )
                .paused;
        assert_with_error!(env, already, CuratorError::ForeignPause);
    }
    let grant = live_grant(env, registrar, args.hub_id, args.spoke_id, &args.asset);
    let within = args.supply_cap <= grant.supply_cap
        && args.borrow_cap <= grant.borrow_cap
        && (!args.can_collateral || grant.can_collateral)
        && (!args.can_borrow || grant.can_borrow)
        && args.ltv <= grant.ltv
        && args.threshold <= grant.threshold
        && args.bonus <= grant.bonus
        && args.liquidation_fees <= grant.liquidation_fees;
    assert_with_error!(env, within, CuratorError::GrantExceeded);
}

/// The grant `grantee` holds for a foreign hub asset in `spoke_id`, while its
/// grantor still owns that hub and is not suspended.
fn live_grant(env: &Env, grantee: u32, hub_id: u32, spoke_id: u32, asset: &Address) -> CapGrant {
    let hub_owner = storage::hub_registrar(env, hub_id);
    assert_with_error!(
        env,
        !hub_owner.is_some_and(|owner| storage::is_suspended(env, owner)),
        CuratorError::RegistrarSuspended
    );
    match storage::cap_grant(env, grantee, hub_id, spoke_id, asset) {
        Some((grantor, grant)) if Some(grantor) == hub_owner => grant,
        _ => panic_with_error!(env, CuratorError::GrantMissing),
    }
}
