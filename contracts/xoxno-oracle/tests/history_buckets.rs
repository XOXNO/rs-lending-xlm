#![cfg(test)]
//! The history holds one entry per resolution bucket. `prices` and `price`
//! serve each entry as a round stamped with the start of its bucket, so a
//! TWAP reader gets distinct timestamps at least one resolution apart at any
//! signer cadence. `price` uses only entries written at or before the asked
//! time. The spot reads keep the package and write timestamps.
extern crate std;

mod common;
use common::*;

use ::common::oracle::providers::redstone::RedStonePriceData;
use ::common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};
use soroban_sdk::{Address, Env, String, Vec, U256};
use xoxno_oracle::{Error, XoxnoOracleClient};

const RESOLUTION: u64 = TEST_RESOLUTION as u64;

struct Fixture {
    env: Env,
    client: XoxnoOracleClient<'static>,
    signers: std::vec::Vec<Address>,
    feed: String,
    asset: ReflectorAsset,
}

/// Three signers, threshold two, resolution 300 s, ledger time 10 000 s.
fn fixture() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    let asset = xlm_asset(&env);
    client.add_feed(&feed, &asset);
    advance_ledger_seconds(&env, 10_000);
    Fixture {
        env,
        client,
        signers,
        feed,
        asset,
    }
}

impl Fixture {
    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    /// Every signer submits `price` with the ledger time as package time.
    fn submit_all(&self, price: i128) {
        let ts_ms = self.now() * 1_000;
        for signer in self.signers.iter() {
            self.client.submit_price(signer, &self.feed, &price, &ts_ms);
        }
    }

    /// Runs `rounds` rounds `cadence` seconds apart. Round `r` submits
    /// `1_000 + r`. Returns the `(bucket start, price)` of the last round in
    /// each bucket, oldest first.
    fn run(&self, cadence: u64, rounds: u32) -> std::vec::Vec<(u64, i128)> {
        let mut closes: std::vec::Vec<(u64, i128)> = std::vec::Vec::new();
        for round in 0..rounds {
            advance_ledger_seconds(&self.env, cadence);
            let price = 1_000 + i128::from(round);
            self.submit_all(price);
            let bucket = self.now() / RESOLUTION * RESOLUTION;
            match closes.last_mut() {
                Some(last) if last.0 == bucket => last.1 = price,
                _ => closes.push((bucket, price)),
            }
        }
        closes
    }

    fn rounds(&self, records: u32) -> std::vec::Vec<(u64, i128)> {
        pairs(&self.client.prices(&self.asset, &records).expect("history"))
    }

    /// Replaces the stored history with `entries` (price, package time in
    /// seconds, write time in seconds), oldest first, as an older contract
    /// version could have left it.
    fn store_legacy_history(&self, entries: &[(i128, u64, u64)]) {
        let mut history = Vec::new(&self.env);
        for (price, package_seconds, write_seconds) in entries {
            history.push_back(RedStonePriceData {
                price: U256::from_u128(&self.env, *price as u128),
                package_timestamp: package_seconds * 1_000,
                write_timestamp: write_seconds * 1_000,
            });
        }
        self.env.as_contract(&self.client.address, || {
            self.env
                .storage()
                .persistent()
                .set(&MirrorKey::History(self.feed.clone()), &history);
        });
    }

    /// Stored history as `(price, write time in seconds)`, oldest first.
    fn stored(&self) -> std::vec::Vec<(i128, u64)> {
        let newest_first = self.client.read_price_history(&self.feed, &100);
        newest_first
            .iter()
            .rev()
            .map(|e| {
                (
                    e.price.to_u128().unwrap() as i128,
                    e.write_timestamp / 1_000,
                )
            })
            .collect()
    }
}

fn pairs(history: &Vec<ReflectorPriceData>) -> std::vec::Vec<(u64, i128)> {
    history.iter().map(|p| (p.timestamp, p.price)).collect()
}

/// What the price aggregator's `Twap` read requires of the rounds it uses:
/// newest first, at least one resolution apart, none after the ledger time.
/// The rounds also sit on the resolution grid.
fn assert_twap_readable(rounds: &[(u64, i128)], resolution: u64, now: u64) {
    assert!(!rounds.is_empty());
    for (timestamp, price) in rounds {
        assert!(*timestamp <= now, "round {timestamp} is after {now}");
        assert_eq!(timestamp % resolution, 0, "round {timestamp} off the grid");
        assert!(*price > 0);
    }
    for pair in rounds.windows(2) {
        assert!(
            pair[0].0 >= pair[1].0 + resolution,
            "rounds {} and {} are closer than {resolution}",
            pair[0].0,
            pair[1].0
        );
    }
}

/// The newest `count` closes, newest first.
fn newest(closes: &[(u64, i128)], count: usize) -> std::vec::Vec<(u64, i128)> {
    closes.iter().rev().take(count).copied().collect()
}

#[test]
fn history_grows_one_entry_per_bucket_at_a_60_second_cadence() {
    let fx = fixture();
    // One hour at a 60 s cadence, resolution 300 s: 12 resolution periods.
    let closes = fx.run(60, 60);

    let rounds = fx.rounds(12);
    assert_eq!(rounds.len(), 12);
    assert_eq!(rounds, newest(&closes, 12));
    // Rounds run at 10 060, 10 120, .., 13 600. The newest bucket starts at
    // 13 500 and closes with round 59; the one before closes with round 57.
    assert_eq!(rounds[0], (13_500, 1_059));
    assert_eq!(rounds[1], (13_200, 1_057));
    assert_eq!(rounds[11], (10_200, 1_007));
    assert_twap_readable(&rounds, RESOLUTION, fx.now());
    for pair in rounds.windows(2) {
        assert_eq!(pair[0].0 - pair[1].0, RESOLUTION, "contiguous buckets");
    }

    // The point-in-time read answers for a time 30 minutes ago (11 800). Bucket
    // 11 700 closes with round 32, written at 11 980, which is after 11 800. Bucket
    // 11 400 closes with round 27, written at 11 680.
    let half_hour_ago = fx.now() - 1_800;
    let sample = fx
        .client
        .price(&fx.asset, &half_hour_ago)
        .expect("a round written at or before 11 800");
    assert_eq!((sample.timestamp, sample.price), (11_400, 1_027));
}

#[test]
fn history_is_twap_readable_at_every_cadence_up_to_the_resolution() {
    for cadence in [30u64, 60, 150, 299, 300] {
        let fx = fixture();
        let rounds_run = (14 * RESOLUTION / cadence) as u32;
        let closes = fx.run(cadence, rounds_run);

        let rounds = fx.rounds(12);
        assert_eq!(rounds.len(), 12, "cadence {cadence}");
        assert_eq!(rounds, newest(&closes, 12), "cadence {cadence}");
        assert_twap_readable(&rounds, RESOLUTION, fx.now());
        for pair in rounds.windows(2) {
            assert_eq!(pair[0].0 - pair[1].0, RESOLUTION, "cadence {cadence}");
        }
        // The newest round is the bucket of the last write.
        assert_eq!(rounds[0].0, fx.now() / RESOLUTION * RESOLUTION);
    }
}

#[test]
fn history_keeps_the_gaps_of_a_cadence_slower_than_the_resolution() {
    for (cadence, widest_gap) in [(400u64, 600u64), (700, 900), (1_500, 1_500)] {
        let fx = fixture();
        fx.client.set_max_stale_seconds(&3_600);
        let closes = fx.run(cadence, 14);
        assert_eq!(closes.len(), 14, "every round opens a bucket");

        let rounds = fx.rounds(12);
        assert_eq!(rounds, newest(&closes, 12), "cadence {cadence}");
        assert_twap_readable(&rounds, RESOLUTION, fx.now());
        let gap = rounds.windows(2).map(|p| p[0].0 - p[1].0).max().unwrap();
        assert_eq!(gap, widest_gap, "cadence {cadence}");
    }
}

#[test]
fn a_write_on_a_bucket_boundary_opens_the_next_bucket() {
    let fx = fixture();
    // The ledger moves from 10 000 to 10 199, the last second of bucket 9 900.
    advance_ledger_seconds(&fx.env, 199);
    fx.submit_all(1_000);
    assert_eq!(fx.rounds(12), [(9_900, 1_000)]);

    // One second later the bucket 10 200 starts.
    advance_ledger_seconds(&fx.env, 1);
    fx.submit_all(1_001);
    assert_eq!(fx.rounds(12), [(10_200, 1_001), (9_900, 1_000)]);

    // 299 s later the write is still in bucket 10 200.
    advance_ledger_seconds(&fx.env, 299);
    fx.submit_all(1_002);
    assert_eq!(fx.rounds(12), [(10_200, 1_002), (9_900, 1_000)]);
    assert_eq!(fx.client.read_price_history(&fx.feed, &12).len(), 2);
}

/// One signer of the quorum does not refresh. The aggregate keeps the oldest
/// clustered package timestamp, so two consecutive aggregates carry the same
/// package timestamp. The rounds still carry distinct bucket timestamps.
#[test]
fn a_signer_that_does_not_refresh_does_not_repeat_a_round_timestamp() {
    let fx = fixture();
    let t0 = fx.now();
    let t0_ms = t0 * 1_000;
    fx.client
        .submit_price(&fx.signers[0], &fx.feed, &1_000, &t0_ms);
    fx.client
        .submit_price(&fx.signers[1], &fx.feed, &1_000, &t0_ms);

    // One resolution later only signer 0 refreshes. Signer 1's entry is still
    // inside the 900 s age and skew windows, so the quorum forms again.
    advance_ledger_seconds(&fx.env, RESOLUTION);
    let t1 = fx.now();
    fx.client
        .submit_price(&fx.signers[0], &fx.feed, &1_001, &(t1 * 1_000));

    let raw = fx.client.read_price_history(&fx.feed, &12);
    assert_eq!(raw.len(), 2);
    assert_eq!(raw.get_unchecked(0).package_timestamp, t0_ms);
    assert_eq!(raw.get_unchecked(1).package_timestamp, t0_ms);

    let rounds = fx.rounds(12);
    assert_eq!(rounds, [(10_200, 1_000), (9_900, 1_000)]);
    assert_twap_readable(&rounds, RESOLUTION, fx.now());

    // The spot read still reports the oldest clustered package time.
    let spot = fx.client.lastprice(&fx.asset).expect("spot");
    assert_eq!((spot.timestamp, spot.price), (t0, 1_000));
}

#[test]
fn prices_returns_the_newest_records_rounds() {
    let fx = fixture();
    let closes = fx.run(60, 60);

    assert_eq!(fx.rounds(3), newest(&closes, 3));
    assert_eq!(fx.rounds(1), newest(&closes, 1));
    assert_eq!(fx.rounds(100), newest(&closes, 12));
    assert!(fx.client.prices(&fx.asset, &0).is_none());
}

#[test]
fn history_is_capped_at_twelve_buckets_at_a_sub_resolution_cadence() {
    let fx = fixture();
    // 100 rounds at 60 s cross 20 buckets.
    let closes = fx.run(60, 100);
    assert_eq!(closes.len(), 21);

    let raw = fx.client.read_price_history(&fx.feed, &100);
    assert_eq!(raw.len(), 12);
    assert_eq!(fx.rounds(100), newest(&closes, 12));
}

#[test]
fn price_returns_the_newest_round_written_at_or_before_the_timestamp() {
    let fx = fixture();
    fx.run(60, 20);
    // Rounds at 10 060 .. 11 200 close the buckets 9 900 (round 2, written at
    // 10 180), 10 200 (round 7, 10 480), 10 500 (round 12, 10 780), 10 800
    // (round 17, 11 080) and 11 100 (round 19, 11 200).
    let at = |timestamp: u64| {
        fx.client
            .price(&fx.asset, &timestamp)
            .map(|p| (p.timestamp, p.price))
    };

    assert_eq!(at(fx.now()), Some((11_100, 1_019)));
    // A round is not served before its newest price is written.
    assert_eq!(at(11_199), Some((10_800, 1_017)));
    assert_eq!(at(11_100), Some((10_800, 1_017)));
    assert_eq!(at(11_080), Some((10_800, 1_017)));
    assert_eq!(at(11_079), Some((10_500, 1_012)));
    assert_eq!(at(10_480), Some((10_200, 1_007)));
    assert_eq!(at(10_479), Some((9_900, 1_002)));
    assert_eq!(at(10_180), Some((9_900, 1_002)));
    assert_eq!(at(10_179), None);
    assert_eq!(at(0), None);
}

/// A lost quorum on an owner path removes the aggregate, and the history
/// reads answer `None`. A new quorum opens a new bucket. The rounds stay
/// distinct.
#[test]
fn losing_quorum_hides_the_history_until_a_new_quorum_forms() {
    let fx = fixture();
    fx.client.set_max_stale_seconds(&3_600);
    let closes = fx.run(60, 20);
    assert_eq!(fx.rounds(12).len(), 5);

    // Two of three signers leave the cluster. Only signer 0 stays fresh.
    advance_ledger_seconds(&fx.env, 1_000);
    let ts_ms = fx.now() * 1_000;
    fx.client
        .submit_price(&fx.signers[0], &fx.feed, &2_000, &ts_ms);
    // The submit path keeps the aggregate. The owner path removes it.
    assert_eq!(fx.rounds(12), newest(&closes, 12));
    fx.client
        .recompute_feeds(&soroban_sdk::vec![&fx.env, fx.feed.clone()]);
    assert!(fx.client.prices(&fx.asset, &12).is_none());
    assert!(fx.client.price(&fx.asset, &fx.now()).is_none());
    assert!(fx.client.lastprice(&fx.asset).is_none());

    // Signer 1 returns, and the quorum forms in bucket 12 000.
    fx.client
        .submit_price(&fx.signers[1], &fx.feed, &2_000, &ts_ms);
    let rounds = fx.rounds(12);
    assert_eq!(rounds[0], (12_000, 2_000));
    assert_eq!(rounds[1..], newest(&closes, 5)[..]);
    assert_twap_readable(&rounds, RESOLUTION, fx.now());
}

#[test]
fn purge_feed_removes_the_history_buckets() {
    let fx = fixture();
    fx.run(60, 20);
    fx.client.purge_feed(&fx.feed);
    fx.client.add_feed(&fx.feed, &fx.asset);
    assert!(fx.client.prices(&fx.asset, &12).is_none());

    advance_ledger_seconds(&fx.env, 60);
    fx.submit_all(3_000);
    assert_eq!(fx.rounds(12), [(11_100, 3_000)]);
}

#[test]
fn empty_stored_history_fails_closed_while_the_spot_price_remains_available() {
    let fx = fixture();
    fx.submit_all(1_000);
    assert_eq!(
        fx.client.read_price_data_for_feed(&fx.feed).price.to_u128(),
        Some(1_000)
    );

    // A legacy or inconsistent storage entry can contain an empty history
    // even while the current aggregate is still valid.
    fx.store_legacy_history(&[]);
    assert_eq!(
        fx.client.try_read_price_history(&fx.feed, &1),
        Err(Ok(Error::NoDataForFeed))
    );
    assert_eq!(
        fx.client.read_price_data_for_feed(&fx.feed).price.to_u128(),
        Some(1_000)
    );
}

/// The spot reads return the stored aggregate: the lower median, the oldest
/// clustered package timestamp and the ledger time of the write. The bucket
/// rule changes none of the three.
#[test]
fn spot_reads_keep_the_package_and_write_timestamps_at_a_60_second_cadence() {
    let fx = fixture();
    for round in 0..20u64 {
        advance_ledger_seconds(&fx.env, 60);
        let now = fx.now();
        // Signer 0 lags 40 s, signer 1 lags 10 s, signer 2 reports the ledger
        // time. Prices differ so the lower median is signer 1's.
        let base = 1_000 + round as i128 * 10;
        let lags = [(40u64, base + 2), (10, base + 1), (0, base)];
        for (signer, (lag, price)) in fx.signers.iter().zip(lags) {
            fx.client
                .submit_price(signer, &fx.feed, &price, &((now - lag) * 1_000));
        }

        let single = fx.client.read_price_data_for_feed(&fx.feed);
        assert_eq!(single.price.to_u128(), Some((base + 1) as u128));
        assert_eq!(single.package_timestamp, (now - 40) * 1_000);
        assert_eq!(single.write_timestamp, now * 1_000);

        let bulk = fx
            .client
            .read_price_data(&soroban_sdk::vec![&fx.env, fx.feed.clone()]);
        assert_eq!(bulk.len(), 1);
        assert_eq!(bulk.get_unchecked(0), single);

        let spot = fx.client.lastprice(&fx.asset).expect("spot");
        assert_eq!((spot.timestamp, spot.price), (now - 40, base + 1));

        // The newest stored history entry is the stored aggregate.
        let raw = fx.client.read_price_history(&fx.feed, &1);
        assert_eq!(raw.get_unchecked(0), single);
    }
}

/// `set_resolution` does not rewrite stored entries. The reads group the
/// stored entries by the live resolution and keep the newest entry of each
/// bucket, so the rounds stay distinct and one live resolution apart.
#[test]
fn rounds_follow_the_live_resolution_after_a_change() {
    for (resolution, expected) in [
        // With 900 s buckets, the closes at 10 180, 10 480 and 10 780 land in
        // 9 900, and 11 080 and 11 200 land in 10 800.
        (900u32, std::vec![(10_800u64, 1_019i128), (9_900, 1_012)]),
        // With 250 s buckets, stored closes at 10 180, 10 480, 10 780, 11 080 and
        // 11 200 land in 10 000, 10 250, 10 750, 11 000 and again 11 000.
        (
            250,
            std::vec![
                (11_000, 1_019),
                (10_750, 1_012),
                (10_250, 1_007),
                (10_000, 1_002)
            ],
        ),
        (
            60,
            std::vec![
                (11_160, 1_019),
                (11_040, 1_017),
                (10_740, 1_012),
                (10_440, 1_007),
                (10_140, 1_002)
            ],
        ),
    ] {
        let fx = fixture();
        fx.client.set_max_stale_seconds(&3_600);
        // Writes at 10 060 .. 11 200. Stored closes (write time, price):
        // (10 180, 1 002), (10 480, 1 007), (10 780, 1 012), (11 080, 1 017),
        // (11 200, 1 019).
        fx.run(60, 20);
        assert_eq!(fx.client.read_price_history(&fx.feed, &12).len(), 5);

        fx.client.set_resolution(&resolution);
        let rounds = fx.rounds(12);
        assert_eq!(rounds, expected, "resolution {resolution}");
        assert_twap_readable(&rounds, u64::from(resolution), fx.now());
        // `records` counts rounds, not stored entries.
        assert_eq!(fx.rounds(2), expected[..2], "resolution {resolution}");
        // The stored entries are unchanged.
        assert_eq!(fx.client.read_price_history(&fx.feed, &12).len(), 5);
    }
}

/// A zero resolution makes every ledger second its own bucket. No read or
/// write divides by the resolution.
#[test]
fn a_zero_resolution_uses_one_bucket_per_second() {
    let fx = fixture();
    fx.client.set_resolution(&0);

    fx.submit_all(1_000);
    fx.submit_all(1_001);
    assert_eq!(fx.rounds(12), [(10_000, 1_001)]);

    advance_ledger_seconds(&fx.env, 1);
    fx.submit_all(1_002);
    assert_eq!(fx.rounds(12), [(10_001, 1_002), (10_000, 1_001)]);
    assert_eq!(
        fx.client
            .price(&fx.asset, &10_000)
            .map(|p| (p.timestamp, p.price)),
        Some((10_000, 1_001))
    );
}

/// The previous contract version kept one entry at a sub-resolution cadence.
/// That entry was the newest aggregate, with its own write time. After the
/// upgrade the first reads serve that entry as one round at its bucket start.
/// Later writes replace it inside its bucket and append from the next bucket on.
#[test]
fn a_single_entry_history_from_before_the_change_reads_and_grows() {
    let fx = fixture();
    // The ledger moves from 10 000 to 10 140. The aggregate is written now. The
    // legacy history holds that aggregate only, as the previous version stores it.
    advance_ledger_seconds(&fx.env, 140);
    fx.submit_all(1_000);
    let aggregate = fx.client.read_price_data_for_feed(&fx.feed);
    fx.store_legacy_history(&[(1_000, 10_140, 10_140)]);
    assert_eq!(
        fx.client.read_price_history(&fx.feed, &12).get_unchecked(0),
        aggregate
    );

    assert_eq!(fx.rounds(12), [(9_900, 1_000)]);
    assert_eq!(
        fx.client
            .price(&fx.asset, &fx.now())
            .map(|p| (p.timestamp, p.price)),
        Some((9_900, 1_000))
    );
    let spot = fx.client.lastprice(&fx.asset).expect("spot");
    assert_eq!((spot.timestamp, spot.price), (10_140, 1_000));

    // At 10 170 the ledger is still in bucket 9 900, so the entry is replaced.
    advance_ledger_seconds(&fx.env, 30);
    fx.submit_all(1_001);
    assert_eq!(fx.stored(), [(1_001, 10_170)]);

    // 60 s cadence from 10 230 to 11 070 opens buckets 10 200, 10 500, 10 800.
    let closes = fx.run(60, 15);
    assert_eq!(closes.len(), 3);
    let rounds = fx.rounds(12);
    assert_eq!(rounds[..3], newest(&closes, 3)[..]);
    assert_eq!(rounds[3], (9_900, 1_001));
    assert_eq!(rounds.len(), 4);
    assert_twap_readable(&rounds, RESOLUTION, fx.now());
}

/// The previous contract version, at a cadence of one resolution or slower,
/// stored entries whose write times are at least one resolution apart and
/// whose package times can repeat (a signer that did not refresh). Their
/// buckets are distinct, so the reads serve every entry as its own round.
#[test]
fn an_irregular_history_from_before_the_change_serves_distinct_rounds() {
    let fx = fixture();
    advance_ledger_seconds(&fx.env, 1_500);
    fx.submit_all(1_004);
    // 11 500. Write times 10 000, 10 300, 10 750, 11 500; package times repeat.
    fx.store_legacy_history(&[
        (1_001, 10_000, 10_000),
        (1_002, 10_000, 10_300),
        (1_003, 10_700, 10_750),
        (1_004, 11_500, 11_500),
    ]);

    let rounds = fx.rounds(12);
    assert_eq!(
        rounds,
        [
            (11_400, 1_004),
            (10_500, 1_003),
            (10_200, 1_002),
            (9_900, 1_001)
        ]
    );
    assert_twap_readable(&rounds, RESOLUTION, fx.now());
    assert_eq!(
        fx.client
            .price(&fx.asset, &10_499)
            .map(|p| (p.timestamp, p.price)),
        Some((10_200, 1_002))
    );

    // The next write in bucket 11 400 replaces the newest legacy entry only.
    advance_ledger_seconds(&fx.env, 60);
    fx.submit_all(1_005);
    assert_eq!(
        fx.stored(),
        [
            (1_001, 10_000),
            (1_002, 10_300),
            (1_003, 10_750),
            (1_005, 11_560)
        ]
    );
}

/// Entries that share a bucket (left by a zero resolution, or by a larger
/// resolution set later) collapse to the newest entry of the bucket on read.
/// Nothing is rewritten, and the reads stay strictly newest first even if the
/// stored write times are out of order.
#[test]
fn legacy_entries_that_share_a_bucket_collapse_to_one_round() {
    let fx = fixture();
    advance_ledger_seconds(&fx.env, 700);
    fx.submit_all(1_005);
    // 10 700. Buckets: 9 900, 9 900, 9 900, 10 500, 10 200 (out of order), 10 500.
    // Newest first, the entry at 10 600 is not older than the round before it
    // (10 200) and is skipped.
    fx.store_legacy_history(&[
        (1_000, 10_000, 10_000),
        (1_001, 10_001, 10_001),
        (1_002, 10_002, 10_002),
        (1_003, 10_600, 10_600),
        (1_004, 10_400, 10_400),
        (1_005, 10_700, 10_700),
    ]);

    let rounds = fx.rounds(12);
    assert_eq!(rounds, [(10_500, 1_005), (10_200, 1_004), (9_900, 1_002)]);
    assert_twap_readable(&rounds, RESOLUTION, fx.now());
    assert_eq!(fx.rounds(1), [(10_500, 1_005)]);
    assert_eq!(fx.client.read_price_history(&fx.feed, &12).len(), 6);
    // Inside a shared bucket, `price` serves the newest entry written by then.
    assert_eq!(
        fx.client
            .price(&fx.asset, &10_001)
            .map(|p| (p.timestamp, p.price)),
        Some((9_900, 1_001))
    );
}
