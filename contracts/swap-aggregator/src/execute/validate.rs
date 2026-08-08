//! Payload shape, endpoint, and split-weight checks before execution.

use soroban_sdk::{panic_with_error, Env};

use crate::constants::PPM_DENOMINATOR;
use crate::errors::Error;
use crate::execute::path_util::{
    first_index_for_token, group_split_ppm, last_token_out, path_token_in,
};
use crate::types::StrategyPayload;

/// Panic if the payload is empty, endpoints break, or split weights are invalid.
///
/// Pure burn or pure mint may omit paths. Without a mint leg, each token group
/// must allocate exactly 1e6 ppm; with mint, less is allowed (leftover deposits).
pub(crate) fn validate_payload(env: &Env, payload: &StrategyPayload) {
    let paths = &payload.paths;
    let n = paths.len();
    if n == 0 && payload.burn_pool.is_none() && payload.mint_pool.is_none() {
        panic_with_error!(env, Error::EmptyBatch);
    }

    for i in 0..n {
        let path = paths
            .get(i)
            .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
        if path.hops.is_empty() {
            panic_with_error!(env, Error::EmptyPath);
        }
        if path.split_ppm == 0 {
            panic_with_error!(env, Error::ZeroSplitPpm);
        }

        let path_in = path_token_in(env, paths, i);
        if payload.burn_pool.is_none() && path_in != payload.token_in {
            panic_with_error!(env, Error::BrokenTokenChain);
        }
        if payload.mint_pool.is_none() && last_token_out(env, &path) != payload.token_out {
            panic_with_error!(env, Error::BrokenTokenChain);
        }

        if i != first_index_for_token(env, paths, &path_in) {
            continue;
        }
        let sum_ppm = group_split_ppm(env, paths, &path_in);
        if sum_ppm > PPM_DENOMINATOR as u32 {
            panic_with_error!(env, Error::SplitPpmMismatch);
        }

        if payload.mint_pool.is_none() && sum_ppm != PPM_DENOMINATOR as u32 {
            panic_with_error!(env, Error::SplitPpmMismatch);
        }
    }

    if payload.token_in == payload.token_out {
        panic_with_error!(env, Error::SameToken);
    }
}
