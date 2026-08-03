use soroban_sdk::{token, Address, Env};

pub(crate) fn sac_transfer_call(
    env: &Env,
    token: &Address,
    from: &Address,
    to: &Address,
    amount: &i128,
) {
    if *amount == 0 {
        return;
    }
    token::Client::new(env, token).transfer(from, to, amount)
}
