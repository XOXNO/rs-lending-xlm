use super::*;

/// The read-side coverage minimum is `(records - 1) x resolution`. Admission
/// needs `(records + 2) x resolution`: a complete history holds `records + 1`
/// rounds whose oldest sits `records` rounds behind the last one, and
/// Reflector serves that last round for two more. Mainnet's `Twap(3)` at
/// 300 s needs a 1_500 s budget; `Twap(12)` needs 4_200 s, past the 3_600 s
/// leg-age spread bound; a spot read needs two rounds.
#[test]
fn test_admission_age_covers_the_upstream_history_and_the_serving_window() {
    assert_eq!(twap_required_span(3, 300), 600);
    assert_eq!(admission_read_age(3, 300), 1_500);
    assert_eq!(admission_read_age(12, 300), 4_200);
    assert_eq!(admission_read_age(10, 300), 3_600);
    assert_eq!(admission_read_age(1, 60), 180);
    assert_eq!(admission_read_age(0, 300), 600);
    // (2^32 + 1) x (2^32 - 1) = 2^64 - 1: the widest inputs still fit.
    assert_eq!(admission_read_age(u32::MAX, u32::MAX), u64::MAX);
}
