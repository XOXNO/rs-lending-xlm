//! Applies and claims static and referral fees.

use common::constants::BPS;

use soroban_sdk::{panic_with_error, token, Address, Env, Vec};

use crate::constants::FEE_CAP;
use crate::errors::Error;
use crate::execute::ensure_trustline;
use crate::math::{checked_add, checked_mul};
use crate::storage;
use crate::types::DataKey;
use crate::vault::Vault;

/// Which fee ledger to claim from.
///
/// [`FeeBucket::key`] and `storage::bucket_token` encode the same key set in opposite directions.
/// `FeeBucket::key` maps a kind to a `DataKey`, and `bucket_token` maps a `DataKey` back to a
/// token, so adding a bucket kind means extending both.
#[derive(Clone, Copy)]
pub(crate) enum FeeBucket {
    Admin,
    Referral(u64),
}

impl FeeBucket {
    /// Returns the storage key for this bucket and `token`.
    fn key(self, token: Address) -> DataKey {
        match self {
            FeeBucket::Admin => DataKey::AdminFee(token),
            FeeBucket::Referral(id) => DataKey::ReferralFee(id, token),
        }
    }
}

/// Persists the static fee in basis points. Panics if the fee exceeds [`FEE_CAP`].
pub(crate) fn set_static_fee(env: &Env, fee_bps: u32) {
    if fee_bps > FEE_CAP {
        panic_with_error!(env, Error::FeeTooHigh);
    }
    storage::set_static_fee_bps(env, fee_bps);
}

/// Resolves the protocol rate and enforces the combined fee cap.
pub(crate) fn checked_protocol_fee_bps(
    env: &Env,
    referral_bps: u32,
    protocol_bps: Option<u32>,
) -> u32 {
    let protocol_bps = protocol_bps.unwrap_or_else(|| storage::static_fee_bps(env));
    if referral_bps.saturating_add(protocol_bps) > FEE_CAP {
        panic_with_error!(env, Error::FeeTooHigh);
    }
    protocol_bps
}

/// Debits the vault and accrues the static and referral fees for an active referral.
///
/// Returns the charged `(static_fee, referral_fee)`. The function charges nothing and returns
/// `(0, 0)` when `referral_id == 0`, the referral is missing or inactive, the token balance is
/// non-positive, the combined bps is 0, or the computed fee rounds down to zero. It panics if
/// the combined bps exceeds [`FEE_CAP`].
///
/// The static fee is charged only with an active referral; a swap without one pays no static
/// fee. Off-chain quotes assume this policy.
pub(crate) fn apply_fees_on_token(
    env: &Env,
    vault: &mut Vault,
    token: &Address,
    referral_id: u64,
) -> (i128, i128) {
    let (static_fee, referral_fee) = fee_amounts(env, vault.balance_of(token), referral_id);
    let total = checked_add(env, static_fee, referral_fee);
    if total != 0 {
        vault.withdraw(token, total);
        storage::accumulate_swap_fees(env, token, referral_id, static_fee, referral_fee);
    }
    (static_fee, referral_fee)
}

/// Reserves fees on the fixed payout and retains all gross output above the
/// payout plus those fees. Returns protocol revenue and the referral fee.
pub(crate) fn accrue_fixed_output_fees(
    env: &Env,
    vault: &mut Vault,
    token: &Address,
    referral_id: u64,
    payout: i128,
    (static_fee, referral_fee): (i128, i128),
) -> (i128, i128) {
    let configured_fees = checked_add(env, static_fee, referral_fee);
    let revenue = vault.balance_of(token) - payout;
    if revenue < configured_fees {
        panic_with_error!(env, Error::SlippageExceeded);
    }
    // The admin receives the configured static fee and all remaining surplus.
    let protocol_revenue = revenue - referral_fee;
    vault.withdraw(token, revenue);
    storage::accumulate_swap_fees(env, token, referral_id, protocol_revenue, referral_fee);
    (protocol_revenue, referral_fee)
}

/// Returns the `(static_fee, referral_fee)` on `amount` for an active referral, `(0, 0)`
/// otherwise. Panics with `FeeTooHigh` if the combined rate exceeds [`FEE_CAP`].
pub(crate) fn fee_amounts(env: &Env, amount: i128, referral_id: u64) -> (i128, i128) {
    if referral_id == 0 || amount <= 0 {
        return (0, 0);
    }
    let Some(cfg) = storage::try_load_referral(env, referral_id).filter(|cfg| cfg.active) else {
        return (0, 0);
    };
    let static_fee_bps = checked_protocol_fee_bps(env, cfg.fee_bps, cfg.protocol_fee_bps);

    (
        fee_amount(env, amount, static_fee_bps),
        fee_amount(env, amount, cfg.fee_bps),
    )
}

/// Transfers each positive bucket balance for `tokens` to `recipient` and clears the bucket.
pub(crate) fn claim_fee_bucket(
    env: &Env,
    router: &Address,
    recipient: &Address,
    tokens: Vec<Address>,
    bucket: FeeBucket,
) {
    let n = tokens.len();
    for i in 0..n {
        // `i < n == tokens.len()`, so the index is in range by construction.
        let token = tokens.get_unchecked(i);
        let key = bucket.key(token.clone());
        let amount = storage::take_fee_bucket(env, &key);
        if amount > 0 {
            ensure_trustline(env, &token, recipient);
            token::Client::new(env, &token).transfer(router, recipient, &amount);
        }
    }
}

/// Transfers referral `id`'s fee buckets for `tokens` to the referral owner. Panics with
/// [`Error::ReferralNotFound`] if the referral does not exist.
pub(crate) fn claim_referral_fees(env: &Env, router: &Address, id: u64, tokens: Vec<Address>) {
    let cfg = storage::load_referral(env, id);
    claim_fee_bucket(env, router, &cfg.owner, tokens, FeeBucket::Referral(id));
}

/// Computes `balance * fee_bps / BPS` in token units, rounded down, with checked multiplication.
fn fee_amount(env: &Env, balance: i128, fee_bps: u32) -> i128 {
    checked_mul(env, balance, fee_bps as i128) / BPS
}
