//! Aquarius AMM: hop swaps and LP mint/burn legs.

mod burn;
mod mint;
mod pool;
mod swap;

pub(crate) use burn::remove_liquidity;
pub(crate) use mint::{add_liquidity, MintLiquidity, PreSwap};
pub(crate) use swap::swap;

#[cfg(test)]
pub(crate) use mint::pre_balance_possible;
