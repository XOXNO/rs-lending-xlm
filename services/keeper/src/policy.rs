//! Decide which entries deserve a bump this tick.

use stellar_xdr::curr::LedgerKey;

#[derive(Debug, Clone)]
pub struct BumpDecision<K> {
    pub key: K,
    pub live_until: Option<u32>,
    pub reason: BumpReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpReason {
    /// Entry exists and is within the safety margin window.
    BelowSafetyMargin,
    /// Entry is below threshold but already expired — best-effort bump still
    /// useful so it doesn't get archived in the next ledger.
    Expired,
    /// Entry not present — skip.
    Missing,
}

pub fn needs_bump(live_until: Option<u32>, current_ledger: u32, safety_ledgers: u32) -> BumpReason {
    let Some(live_until) = live_until else {
        return BumpReason::Missing;
    };
    if live_until <= current_ledger {
        BumpReason::Expired
    } else if live_until.saturating_sub(current_ledger) < safety_ledgers {
        BumpReason::BelowSafetyMargin
    } else {
        // already well above the margin — caller treats this as "no work".
        BumpReason::Missing
    }
}

/// Convenience wrapper for plain `LedgerKey` entries.
pub fn classify(
    key: LedgerKey,
    live_until: Option<u32>,
    current_ledger: u32,
    safety_ledgers: u32,
) -> BumpDecision<LedgerKey> {
    BumpDecision {
        key,
        live_until,
        reason: needs_bump(live_until, current_ledger, safety_ledgers),
    }
}
