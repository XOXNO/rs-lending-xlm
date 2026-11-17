//! Post-settlement dust: leftover vault balances become admin fee.

use soroban_sdk::{panic_with_error, Env};

use crate::constants::residual_allowance;
use crate::errors::Error;
use crate::storage;
use crate::types::DataKey;
use crate::vault::Vault;

/// Accrues each of the vault's remaining token balances as admin fee, skipping
/// tokens with a non-positive balance.
///
/// Panics with [`Error::ExcessiveResidual`] if a token's leftover balance
/// exceeds its allowance relative to the amount already credited for that
/// token.
pub(crate) fn accrue_residual_as_revenue(env: &Env, vault: &mut Vault) {
    let tokens = vault.tokens();
    let n = tokens.len();
    for i in 0..n {
        let token = tokens.get_unchecked(i);
        let amount = vault.balance_of(&token);
        if amount <= 0 {
            continue;
        }

        if amount > residual_allowance(vault.credited_of(&token)) {
            panic_with_error!(env, Error::ExcessiveResidual);
        }
        vault.withdraw(&token, amount);
        storage::accumulate_fee(env, DataKey::AdminFee(token), amount);
    }
}
