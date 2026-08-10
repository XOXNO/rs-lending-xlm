#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {

    Extend,

    Restore,

    Skip,
}

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
        assert_eq!(
            classify(Some(NOW + SAFETY + 50), true, NOW, SAFETY),
            Decision::Skip
        );
    }

    #[test]
    fn entry_exactly_at_safety_boundary_skips() {

        assert_eq!(
            classify(Some(NOW + SAFETY), true, NOW, SAFETY),
            Decision::Skip
        );
    }

    #[test]
    fn live_entry_inside_margin_extends() {
        assert_eq!(
            classify(Some(NOW + 10), true, NOW, SAFETY),
            Decision::Extend
        );
    }

    #[test]
    fn live_until_equal_to_current_is_still_live_and_extends() {

        assert_eq!(classify(Some(NOW), true, NOW, SAFETY), Decision::Extend);
    }

    #[test]
    fn expired_present_entry_restores() {
        assert_eq!(
            classify(Some(NOW - 1), true, NOW, SAFETY),
            Decision::Restore
        );
        assert_eq!(classify(Some(0), true, NOW, SAFETY), Decision::Restore);
    }

    #[test]
    fn absent_entry_skips_even_when_expired_looking() {

        assert_eq!(classify(Some(0), false, NOW, SAFETY), Decision::Skip);
        assert_eq!(classify(None, false, NOW, SAFETY), Decision::Skip);
    }

    #[test]
    fn present_entry_without_ttl_skips() {
        assert_eq!(classify(None, true, NOW, SAFETY), Decision::Skip);
    }
}
