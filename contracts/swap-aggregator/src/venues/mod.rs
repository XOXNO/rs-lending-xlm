//! DEX hop adapters. Output is always a measured balance delta, never a report.

pub(crate) mod aquarius;
mod auth;
pub(crate) mod comet;
pub(crate) mod phoenix;
pub(crate) mod soroswap;
pub(crate) mod sushi;

pub(crate) use auth::{auth_entry, authorize_token_approve, authorize_token_transfer};

use soroban_sdk::{panic_with_error, token, Address, Env, Map, Vec};

use crate::errors::Error;
use crate::types::{SwapHop, SwapVenue};

/// Dispatches the hop to its venue-specific swap function and returns the measured increase in
/// the router's `token_out` balance. Panics with `Error::ZeroOutput` if the output balance does
/// not strictly increase, and with `Error::InvalidAmount` if the router's `token_in` balance does
/// not decrease by exactly `amount_in`.
pub(crate) fn dispatch_hop(
    env: &Env,
    router: &Address,
    hop: &SwapHop,
    amount_in: i128,
    tokens_cache: &mut Map<Address, Vec<Address>>,
) -> i128 {
    let ctx = HopContext::new(env, router, hop, amount_in);
    let before_in = ctx.input_balance();
    let before_out = ctx.output_balance();

    match hop.venue {
        SwapVenue::Soroswap => soroswap::swap(&ctx),
        SwapVenue::Aquarius => aquarius::swap(&ctx, tokens_cache),
        SwapVenue::Phoenix => phoenix::swap(&ctx),
        SwapVenue::Sushi => sushi::swap(&ctx),
        SwapVenue::CometDex => comet::swap(&ctx),
    };

    let received = ctx
        .output_balance()
        .checked_sub(before_out)
        .unwrap_or_else(|| panic_with_error!(env, Error::ZeroOutput));
    if received <= 0 {
        panic_with_error!(env, Error::ZeroOutput);
    }

    let after_in = ctx.input_balance();
    let spent = before_in
        .checked_sub(after_in)
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidAmount));
    if spent != amount_in {
        panic_with_error!(env, Error::InvalidAmount);
    }

    received
}

/// Shared hop inputs for venue adapters.
pub(crate) struct HopContext<'a> {
    pub env: &'a Env,
    pub router: &'a Address,
    pub hop: &'a SwapHop,
    pub amount_in: i128,
}

impl<'a> HopContext<'a> {
    /// Constructs a `HopContext`. Panics with `Error::InvalidAmount` if `amount_in` is not
    /// positive, and with `Error::SameToken` if the hop's input and output tokens are identical.
    fn new(env: &'a Env, router: &'a Address, hop: &'a SwapHop, amount_in: i128) -> Self {
        if amount_in <= 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }
        if hop.token_in == hop.token_out {
            panic_with_error!(env, Error::SameToken);
        }
        Self {
            env,
            router,
            hop,
            amount_in,
        }
    }

    /// Authorizes the pool to pull `amount_in` of `token_in` from the router.
    pub fn authorize_pool_pull(&self) {
        authorize_token_transfer(
            self.env,
            &self.hop.token_in,
            self.router,
            &self.hop.pool,
            self.amount_in,
        );
    }

    /// Router balance of hop input token.
    pub fn input_balance(&self) -> i128 {
        token::Client::new(self.env, &self.hop.token_in).balance(self.router)
    }

    /// Router balance of hop output token.
    pub fn output_balance(&self) -> i128 {
        token::Client::new(self.env, &self.hop.token_out).balance(self.router)
    }

    /// True if swapping token0→token1; false for the reverse. Panics on mismatch.
    pub fn direction_for_pair(&self, token0: &Address, token1: &Address) -> bool {
        if self.hop.token_in == *token0 && self.hop.token_out == *token1 {
            true
        } else if self.hop.token_in == *token1 && self.hop.token_out == *token0 {
            false
        } else {
            panic_with_error!(self.env, Error::BrokenTokenChain);
        }
    }
}
