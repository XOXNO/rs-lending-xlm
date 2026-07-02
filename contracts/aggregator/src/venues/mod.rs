//! Per-venue swap dispatchers.
//!
//! Every venue's `swap` takes the same shape: it pulls `amount_in` of
//! `token_in` out of the router's SAC balance (transferring to the pool if
//! needed), performs the trade, and returns the `amount_out` of `token_out`
//! now credited to the router. The caller is responsible for vault
//! accounting.

pub(crate) mod aquarius;
pub(crate) mod comet;
pub(crate) mod phoenix;
pub(crate) mod soroswap;
pub(crate) mod sushi;

use crate::errors::Error;
use crate::types::{SwapHop, SwapVenue};
use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    panic_with_error, token, vec, Address, Env, IntoVal, Symbol, Val,
};

/// Execute a single hop against the venue indicated by `hop.venue`.
pub(crate) fn dispatch_hop(env: &Env, router: &Address, hop: &SwapHop, amount_in: i128) -> i128 {
    let ctx = HopContext::new(env, router, hop, amount_in);
    match hop.venue {
        SwapVenue::Soroswap => soroswap::swap(&ctx),
        SwapVenue::Aquarius => aquarius::swap(&ctx),
        SwapVenue::Phoenix => phoenix::swap(&ctx),
        SwapVenue::Sushi => sushi::swap(&ctx),
        SwapVenue::CometDex => comet::swap(&ctx),
    }
}

/// Immutable per-hop execution context shared by all venue dispatchers.
///
/// The constructor performs the common positive-amount check once. Venue
/// modules then focus only on protocol-specific calls and accounting.
pub(crate) struct HopContext<'a> {
    pub env: &'a Env,
    pub router: &'a Address,
    pub hop: &'a SwapHop,
    pub amount_in: i128,
}

impl<'a> HopContext<'a> {
    fn new(env: &'a Env, router: &'a Address, hop: &'a SwapHop, amount_in: i128) -> Self {
        if amount_in <= 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }
        Self {
            env,
            router,
            hop,
            amount_in,
        }
    }

    /// Authorize a pool-pull venue to move exactly this hop's input amount.
    pub fn authorize_pool_pull(&self) {
        authorize_token_transfer(
            self.env,
            &self.hop.token_in,
            self.router,
            &self.hop.pool,
            self.amount_in,
        );
    }

    /// Current router balance of this hop's output token.
    pub fn output_balance(&self) -> i128 {
        token::Client::new(self.env, &self.hop.token_out).balance(self.router)
    }

    /// Infer token0->token1 direction, rejecting hops that do not match the pool.
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

/// Authorize one SAC `transfer(from, to, amount)` on behalf of the router.
///
/// Pool-pull venues use this immediately before invoking the pool. Keeping
/// this helper narrow prevents a venue from accidentally authorizing a broader
/// token movement than the current hop requires.
pub(crate) fn authorize_token_transfer(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
) {
    authorize_as_current(
        env,
        token,
        "transfer",
        vec![
            env,
            from.into_val(env),
            to.into_val(env),
            amount.into_val(env),
        ],
    );
}

/// Authorize one SAC `approve(owner, spender, amount, expiration_ledger)`.
///
/// Comet needs this because its pool pulls via `transfer_from` rather than a
/// direct `transfer`. The approval amount is still limited to the current hop.
pub(crate) fn authorize_token_approve(
    env: &Env,
    token: &Address,
    owner: &Address,
    spender: &Address,
    amount: i128,
    expiration_ledger: u32,
) {
    authorize_as_current(
        env,
        token,
        "approve",
        vec![
            env,
            owner.into_val(env),
            spender.into_val(env),
            amount.into_val(env),
            expiration_ledger.into_val(env),
        ],
    );
}

/// Build one node in Soroban's current-contract authorization tree.
///
/// Most venues only need a leaf entry. Comet uses this to express the nested
/// `swap_exact_amount_in -> transfer_from` authorization expected by its pool.
pub(crate) fn auth_entry(
    env: &Env,
    contract: &Address,
    fn_name: &str,
    args: soroban_sdk::Vec<Val>,
    sub_invocations: soroban_sdk::Vec<InvokerContractAuthEntry>,
) -> InvokerContractAuthEntry {
    InvokerContractAuthEntry::Contract(SubContractInvocation {
        context: ContractContext {
            contract: contract.clone(),
            fn_name: Symbol::new(env, fn_name),
            args,
        },
        sub_invocations,
    })
}

/// Authorize the router as invoker for a single leaf contract call.
pub(crate) fn authorize_as_current(
    env: &Env,
    contract: &Address,
    fn_name: &str,
    args: soroban_sdk::Vec<Val>,
) {
    env.authorize_as_current_contract(vec![
        env,
        auth_entry(env, contract, fn_name, args, vec![env]),
    ]);
}
