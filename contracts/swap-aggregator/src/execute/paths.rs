//! Multi-path split allocation and sequential hop execution.

use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::constants::PPM_DENOMINATOR;
use crate::errors::Error;
use crate::execute::path_util::{first_index_for_token, group_split_ppm, path_token_in};
use crate::math::{checked_add, checked_mul};
use crate::types::SwapPath;
use crate::vault::Vault;
use crate::venues;

/// Run all paths: group by input token, allocate vault balance by `split_ppm`.
///
/// When a token group sums to 1e6 ppm, the last path absorbs rounding dust.
pub(crate) fn execute_paths(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    paths: &Vec<SwapPath>,
    tokens_cache: &mut Map<Address, Vec<Address>>,
) {
    let n = paths.len();
    for i in 0..n {
        let token = path_token_in(env, paths, i);
        if i != first_index_for_token(env, paths, &token) {
            continue;
        }

        let available = vault.balance_of(&token);
        if available <= 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }

        let mut consumed: i128 = 0;
        let mut last = i;
        for j in i..n {
            if path_token_in(env, paths, j) == token {
                last = j;
            }
        }

        let routes_everything = group_split_ppm(env, paths, &token) == PPM_DENOMINATOR as u32;
        for j in i..n {
            if path_token_in(env, paths, j) != token {
                continue;
            }
            let path = paths
                .get(j)
                .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
            let path_input = if j == last && routes_everything {
                available - consumed
            } else {
                let allocated =
                    checked_mul(env, available, path.split_ppm as i128) / PPM_DENOMINATOR;
                consumed = checked_add(env, consumed, allocated);
                allocated
            };
            if path_input <= 0 {
                panic_with_error!(env, Error::InvalidAmount);
            }
            execute_path(env, router, vault, &path, path_input, tokens_cache);
        }
    }
}

/// Walk hops in order; vault tracks spends/credits from measured venue deltas.
fn execute_path(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    path: &SwapPath,
    path_input: i128,
    tokens_cache: &mut Map<Address, Vec<Address>>,
) {
    if path.hops.is_empty() {
        panic_with_error!(env, Error::EmptyPath);
    }

    let n = path.hops.len();
    let mut current = path_input;
    for idx in 0..n {
        let hop = path
            .hops
            .get(idx)
            .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
        if idx + 1 < n {
            let next_hop = path
                .hops
                .get(idx + 1)
                .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
            if hop.token_out != next_hop.token_in {
                panic_with_error!(env, Error::BrokenTokenChain);
            }
        }
        vault.withdraw(&hop.token_in, current);
        let out = venues::dispatch_hop(env, router, &hop, current, tokens_cache);
        if out <= 0 {
            panic_with_error!(env, Error::ZeroOutput);
        }
        vault.deposit(&hop.token_out, out);
        current = out;
    }
}
