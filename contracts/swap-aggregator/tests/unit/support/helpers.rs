//! Shared builders for router unit tests.
//!
//! Keeps token registration and `StrategyPayload` XDR encoding in one place so
//! venue and surface tests stay focused on the behaviour under test.

use crate::types::{StrategyPayload, SwapHop, SwapPath, SwapVenue};
use soroban_sdk::{token, vec, xdr::ToXdr, Address, Env, Vec};

/// Register a Stellar Asset Contract (SAC) and return its address + mint client.
pub(crate) fn new_asset<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::StellarAssetClient<'a>) {
    let contract = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = contract.address();
    let sac_admin = token::StellarAssetClient::new(env, &addr);
    (addr, sac_admin)
}

/// One-hop path with `amount_out = 0` (venues that size from live state ignore
/// the quoted field; malicious-report tests set their own hops when needed).
pub(crate) fn one_hop_path(
    env: &Env,
    venue: SwapVenue,
    pool: Address,
    token_in: Address,
    token_out: Address,
    split_ppm: u32,
) -> SwapPath {
    SwapPath {
        split_ppm,
        hops: vec![
            env,
            SwapHop {
                venue,
                amount_out: 0,
                pool,
                token_in,
                token_out,
            },
        ],
    }
}

/// Encode a strategy with no referral (`referral_id = 0`).
pub(crate) fn strategy_xdr(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: Vec<SwapPath>,
) -> soroban_sdk::Bytes {
    strategy_xdr_with_referral(env, token_in, token_out, total_min_out, paths, 0)
}

/// Encode a strategy that may attach a referral id for fee attribution.
pub(crate) fn strategy_xdr_with_referral(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: Vec<SwapPath>,
    referral_id: u64,
) -> soroban_sdk::Bytes {
    StrategyPayload {
        paths,
        referral_id,
        token_in,
        token_out,
        total_min_out,
    }
    .to_xdr(env)
}
