//! Path token endpoints and per-token split grouping.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::errors::Error;
use crate::types::SwapPath;

/// First hop `token_in` of `paths[index]`.
pub(crate) fn path_token_in(env: &Env, paths: &Vec<SwapPath>, index: u32) -> Address {
    paths
        .get(index)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
        .hops
        .get(0)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
        .token_in
}

/// Last hop `token_out` of `path`.
pub(crate) fn last_token_out(env: &Env, path: &SwapPath) -> Address {
    let n = path.hops.len();
    if n == 0 {
        panic_with_error!(env, Error::EmptyPath);
    }
    path.hops
        .get(n - 1)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
        .token_out
}

/// Sum of `split_ppm` for all paths that start with `token`.
pub(crate) fn group_split_ppm(env: &Env, paths: &Vec<SwapPath>, token: &Address) -> u32 {
    let n = paths.len();
    let mut sum_ppm: u32 = 0;
    for i in 0..n {
        if path_token_in(env, paths, i) != *token {
            continue;
        }
        let path = paths
            .get(i)
            .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
        sum_ppm = sum_ppm
            .checked_add(path.split_ppm)
            .unwrap_or_else(|| panic_with_error!(env, Error::SplitPpmMismatch));
    }
    sum_ppm
}

/// Lowest index of a path whose first hop is `token`.
pub(crate) fn first_index_for_token(env: &Env, paths: &Vec<SwapPath>, token: &Address) -> u32 {
    let n = paths.len();
    for i in 0..n {
        if path_token_in(env, paths, i) == *token {
            return i;
        }
    }
    panic_with_error!(env, Error::EmptyPath)
}
