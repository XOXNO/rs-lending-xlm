//! Static and referral fee apply / claim.

use common::constants::BPS;

use soroban_sdk::{panic_with_error, token, Address, Env, Vec};

use crate::constants::FEE_CAP;
use crate::errors::Error;
use crate::math::{checked_add, checked_mul};
use crate::storage;
use crate::types::DataKey;
use crate::vault::Vault;

/// Which fee ledger to claim from.
///
/// [`FeeBucket::key`] and `storage::bucket_token` encode the same key set inverted — kind to
/// `DataKey` here, `DataKey` back to token there — so adding a bucket kind means extending both.
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

/// Persists the static fee in basis points; panics if it exceeds [`FEE_CAP`].
pub(crate) fn set_static_fee(env: &Env, fee_bps: u32) {
    if fee_bps > FEE_CAP {
        panic_with_error!(env, Error::FeeTooHigh);
    }
    storage::set_static_fee_bps(env, fee_bps);
}

/// Debits the vault and accrues static + referral fees for an active referral.
///
/// No-op when `referral_id == 0`, the referral is missing/inactive, the token balance is
/// non-positive, the combined bps is 0, or the computed fee rounds down to zero. Panics if
/// the combined bps exceeds [`FEE_CAP`].
///
/// The static protocol fee is deliberately coupled to the referral flow: it rides along with a
/// referral and is charged nowhere else, so a swap with no referral (or with an unknown or
/// deactivated one) pays zero protocol fee. This is the intended policy, not a missed branch —
/// the off-chain quote model prices swaps the same way, and decoupling the two would silently
/// start charging every existing integration that quotes without a referral id. Residual dust
/// still accrues to the admin bucket after settlement, independently of this path.
pub(crate) fn apply_fees_on_token(env: &Env, vault: &mut Vault, token: &Address, referral_id: u64) {
    if referral_id == 0 {
        return;
    }

    let cfg = match storage::try_load_referral(env, referral_id) {
        Some(c) => c,
        None => return,
    };
    if !cfg.active {
        return;
    }

    let balance = vault.balance_of(token);
    if balance <= 0 {
        return;
    }
    let static_fee_bps = storage::static_fee_bps(env);

    let combined_bps = static_fee_bps
        .checked_add(cfg.fee_bps)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if combined_bps == 0 {
        return;
    }

    if combined_bps > FEE_CAP {
        panic_with_error!(env, Error::FeeTooHigh);
    }

    let static_fee = fee_amount(env, balance, static_fee_bps);
    let referral_fee = fee_amount(env, balance, cfg.fee_bps);
    let total = checked_add(env, static_fee, referral_fee);
    if total <= 0 {
        return;
    }

    vault.withdraw(token, total);

    // Both buckets in one call: they share `token`'s reserved total, and crediting them
    // separately would read-modify-write that entry twice for a single swap.
    storage::accumulate_swap_fees(env, token, referral_id, static_fee, referral_fee);
}

/// Transfer each positive bucket balance for `tokens` to `recipient`.
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
            token::Client::new(env, &token).transfer(router, recipient, &amount);
        }
    }
}

/// Claim referral `id` fee buckets to the referral owner.
pub(crate) fn claim_referral_fees(env: &Env, router: &Address, id: u64, tokens: Vec<Address>) {
    let cfg = storage::load_referral(env, id);
    claim_fee_bucket(env, router, &cfg.owner, tokens, FeeBucket::Referral(id));
}

/// Computes `balance * fee_bps / BPS` using checked multiplication.
fn fee_amount(env: &Env, balance: i128, fee_bps: u32) -> i128 {
    checked_mul(env, balance, fee_bps as i128) / BPS
}
