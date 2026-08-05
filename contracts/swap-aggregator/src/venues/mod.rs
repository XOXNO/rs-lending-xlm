pub(crate) mod aquarius;
pub(crate) mod comet;
pub(crate) mod phoenix;
pub(crate) mod soroswap;
pub(crate) mod sushi;

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    panic_with_error, token, vec, Address, Env, IntoVal, Map, Symbol, Val, Vec,
};

use crate::errors::Error;
use crate::types::{SwapHop, SwapVenue};

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

    pub fn authorize_pool_pull(&self) {
        authorize_token_transfer(
            self.env,
            &self.hop.token_in,
            self.router,
            &self.hop.pool,
            self.amount_in,
        );
    }

    pub fn input_balance(&self) -> i128 {
        token::Client::new(self.env, &self.hop.token_in).balance(self.router)
    }

    pub fn output_balance(&self) -> i128 {
        token::Client::new(self.env, &self.hop.token_out).balance(self.router)
    }

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
