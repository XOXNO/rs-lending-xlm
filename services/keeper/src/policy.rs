//! Decide which entries deserve a bump this tick.

/// True when a present TTL entry is at or below the safety margin (including
/// already-expired entries, where a best-effort bump still avoids archival).
/// Absent entries and those comfortably above the margin return `false`.
pub fn needs_bump(live_until: Option<u32>, current_ledger: u32, safety_ledgers: u32) -> bool {
    let Some(live_until) = live_until else {
        return false;
    };
    live_until <= current_ledger || live_until.saturating_sub(current_ledger) < safety_ledgers
}
