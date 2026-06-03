//! Decide what each discovered entry needs this tick.

/// What the keeper should do with a single entry this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Live and inside the safety margin → `ExtendFootprintTtl`.
    Extend,
    /// Archived but its data is still present → `RestoreFootprint`.
    Restore,
    /// Healthy (comfortable headroom), absent (never written / evicted), or
    /// without a TTL — nothing to do.
    Skip,
}

/// Decide what to do with one entry. `value_present` is whether the RPC
/// returned the entry's data: the RPC omits never-written and fully-evicted
/// entries, and neither can be extended or restored, so those `Skip`.
pub fn classify(
    live_until: Option<u32>,
    value_present: bool,
    current_ledger: u32,
    safety_ledgers: u32,
) -> Decision {
    if !value_present {
        return Decision::Skip;
    }
    let Some(live_until) = live_until else {
        return Decision::Skip;
    };
    // Soroban liveness is inclusive: an entry is live through its `live_until`
    // ledger, so only `live_until < current` is archived. At equality the entry
    // is on its last live ledger — extend it (remaining 0 < safety).
    if live_until < current_ledger {
        return Decision::Restore;
    }
    if live_until - current_ledger < safety_ledgers {
        return Decision::Extend;
    }
    Decision::Skip
}

#[cfg(test)]
mod tests {
    use super::{classify, Decision};

    const SAFETY: u32 = 100;
    const NOW: u32 = 1_000;

    #[test]
    fn healthy_live_entry_skips() {
        assert_eq!(classify(Some(NOW + SAFETY + 50), true, NOW, SAFETY), Decision::Skip);
    }

    #[test]
    fn entry_exactly_at_safety_boundary_skips() {
        // remaining == safety is *not* inside the margin (strict `<`).
        assert_eq!(classify(Some(NOW + SAFETY), true, NOW, SAFETY), Decision::Skip);
    }

    #[test]
    fn live_entry_inside_margin_extends() {
        assert_eq!(classify(Some(NOW + 10), true, NOW, SAFETY), Decision::Extend);
    }

    #[test]
    fn live_until_equal_to_current_is_still_live_and_extends() {
        // Inclusive liveness: the entry is on its last live ledger, not archived.
        assert_eq!(classify(Some(NOW), true, NOW, SAFETY), Decision::Extend);
    }

    #[test]
    fn expired_present_entry_restores() {
        assert_eq!(classify(Some(NOW - 1), true, NOW, SAFETY), Decision::Restore);
        assert_eq!(classify(Some(0), true, NOW, SAFETY), Decision::Restore);
    }

    #[test]
    fn absent_entry_skips_even_when_expired_looking() {
        // RPC omits never-written / evicted entries — nothing to extend or
        // restore.
        assert_eq!(classify(Some(0), false, NOW, SAFETY), Decision::Skip);
        assert_eq!(classify(None, false, NOW, SAFETY), Decision::Skip);
    }

    #[test]
    fn present_entry_without_ttl_skips() {
        assert_eq!(classify(None, true, NOW, SAFETY), Decision::Skip);
    }
}
