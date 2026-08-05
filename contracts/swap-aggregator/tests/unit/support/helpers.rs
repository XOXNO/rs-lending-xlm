use crate::types::{StrategyPayload, SwapHop, SwapPath, SwapVenue};
use soroban_sdk::{token, vec, xdr::ToXdr, Address, Env, Vec};

pub(crate) fn new_asset<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::StellarAssetClient<'a>) {
    let contract = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = contract.address();
    let sac_admin = token::StellarAssetClient::new(env, &addr);
    (addr, sac_admin)
}

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
                pool,
                token_in,
                token_out,
            },
        ],
    }
}

pub(crate) fn strategy_xdr(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: Vec<SwapPath>,
) -> soroban_sdk::Bytes {
    strategy_xdr_with_referral(env, token_in, token_out, total_min_out, paths, 0)
}

pub(crate) fn strategy_xdr_with_referral(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: Vec<SwapPath>,
    referral_id: u64,
) -> soroban_sdk::Bytes {
    StrategyPayload {
        burn_pool: None,
        burn_min_amounts: Vec::new(env),
        mint_pool: None,
        mint_min_shares: 0,
        paths,
        pre_balance_fee_bps: 0,
        referral_id,
        token_in,
        token_out,
        total_min_out,
    }
    .to_xdr(env)
}

/// Payload with an LP burn leg, a mint leg, or both.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lp_strategy_xdr(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: Vec<SwapPath>,
    burn_pool: Option<Address>,
    burn_min_amounts: Vec<i128>,
    mint_pool: Option<Address>,
    mint_min_shares: i128,
    pre_balance_fee_bps: u32,
) -> soroban_sdk::Bytes {
    StrategyPayload {
        burn_pool,
        burn_min_amounts,
        mint_pool,
        mint_min_shares,
        paths,
        pre_balance_fee_bps,
        referral_id: 0,
        token_in,
        token_out,
        total_min_out,
    }
    .to_xdr(env)
}
