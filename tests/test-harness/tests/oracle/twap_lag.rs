//! A two-source key while its push leg has not written. The Reflector leg is
//! the mean of the newest `records` rounds; the RedStone leg reads its last
//! written price until its next write. The book lists XLM and USDC with the
//! LTV, threshold, bonus and fees of mainnet spoke 1 and the mainnet oracle
//! configs of both keys. `RoundReflector` answers as the public Reflector
//! contract does.

use controller::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef,
    OracleReadMode, OracleTolerance, PriceKey, PriceSource, PriceStatus, ProviderRef,
    ReflectorFeedRef,
};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{vec, Address, Env, Error, String, Symbol};
use test_harness::mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use test_harness::mock_round_reflector::{ReflectorAsset, RoundReflector, RoundReflectorClient};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, AssetConfigPreset, LendingTest, MarketPreset,
    ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};

const WAD: i128 = 1_000_000_000_000_000_000;
/// One whole token of a 7-decimal asset, in raw units.
const UNIT: i128 = 10_000_000;
/// One raw unit of a 7-decimal dollar token, in WAD dollars.
const RAW_USD_WAD: i128 = WAD / UNIT;
const RESOLUTION: u64 = 300;
/// Timestamp of the Reflector round that prints the move.
const MOVE_ROUND: u64 = 3_000;
/// XLM price before the move: 0.125 USD.
const XLM_START_WAD: i128 = WAD / 8;
const XLM_LTV_BPS: i128 = 7_500;
const XLM_THRESHOLD_BPS: i128 = 7_800;

struct Book {
    t: LendingTest,
    reflector: Address,
    redstone: Address,
}

fn market(name: &'static str, price_wad: i128, config: AssetConfigPreset) -> MarketPreset {
    MarketPreset {
        name,
        decimals: 7,
        price_wad,
        initial_liquidity: 10_000_000.0,
        config,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn reflector_leg(env: &Env, reflector: &Address, symbol: &str) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: reflector.clone(),
            asset: OracleAssetRef::Symbol(Symbol::new(env, symbol)),
            read_mode: OracleReadMode::Twap(3),
        }),
        decimals: 14,
        max_stale_seconds: 3_600,
    })
}

fn redstone_leg(env: &Env, redstone: &Address, feed: &str, max_stale: u64) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: redstone.clone(),
            feed_id: String::from_str(env, feed),
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    })
}

impl Book {
    /// Both XLM legs read `XLM_START_WAD` before the move.
    fn new() -> Self {
        Self::with_reflector_start(XLM_START_WAD)
    }

    /// Mainnet spoke 1 listings for XLM and USDC, and the mainnet oracle
    /// configs of both keys, admitted through governance. Four Reflector
    /// rounds before `MOVE_ROUND` print XLM at `reflector_start_wad`;
    /// RedStone wrote `XLM_START_WAD` at the last of them. The ledger time is
    /// `MOVE_ROUND`.
    fn with_reflector_start(reflector_start_wad: i128) -> Self {
        let t = LendingTest::new()
            .with_market(market(
                "XLM",
                XLM_START_WAD,
                AssetConfigPreset {
                    loan_to_value: 7_500,
                    liquidation_threshold: 7_800,
                    liquidation_bonus: 900,
                    liquidation_fees: 1_200,
                    ..DEFAULT_ASSET_CONFIG
                },
            ))
            .with_market(market(
                "USDC",
                usd(1),
                AssetConfigPreset {
                    loan_to_value: 7_600,
                    liquidation_threshold: 8_000,
                    liquidation_bonus: 400,
                    liquidation_fees: 1_000,
                    ..DEFAULT_ASSET_CONFIG
                },
            ))
            .with_min_borrow_collateral_disabled()
            .build();
        let env = &t.env;
        // Only the timestamp moves: the mocks keep their temporary entries.
        env.ledger()
            .with_mut(|ledger| ledger.timestamp = MOVE_ROUND);
        let reflector = env.register(RoundReflector, (RESOLUTION as u32,));
        let redstone = env.register(MockRedStonePriceFeed, ());
        let book = Book {
            t,
            reflector,
            redstone,
        };
        for back in (1..=4).rev() {
            book.round(MOVE_ROUND - back * RESOLUTION, reflector_start_wad);
        }
        book.push_redstone(XLM_START_WAD, MOVE_ROUND - RESOLUTION);

        let env = &book.t.env;
        let xlm = book.t.resolve_asset("XLM");
        let usdc = book.t.resolve_asset("USDC");
        book.t.configure_market_oracle(
            &xlm,
            &AssetOracle {
                asset_decimals: 7,
                max_price_stale_seconds: 57_600,
                sources: vec![
                    env,
                    reflector_leg(env, &book.reflector, "XLM"),
                    redstone_leg(env, &book.redstone, "XLM", 46_800),
                ],
                tolerance: OracleTolerance {
                    upper_ratio_bps: 11_000,
                    lower_ratio_bps: 9_091,
                },
                independence: IndependencePolicy::RequireDisjoint,
                min_sanity_price_wad: WAD / 20,
                max_sanity_price_wad: WAD,
            },
        );
        book.t.configure_market_oracle(
            &usdc,
            &AssetOracle {
                asset_decimals: 7,
                max_price_stale_seconds: 57_600,
                sources: vec![
                    env,
                    reflector_leg(env, &book.reflector, "USDC"),
                    redstone_leg(env, &book.redstone, "USDC", 57_600),
                ],
                tolerance: OracleTolerance {
                    upper_ratio_bps: 10_500,
                    lower_ratio_bps: 9_524,
                },
                independence: IndependencePolicy::RequireDisjoint,
                min_sanity_price_wad: WAD * 95 / 100,
                max_sanity_price_wad: WAD * 105 / 100,
            },
        );
        book
    }

    /// Publishes the Reflector round stamped `timestamp`: XLM at
    /// `xlm_price_wad`, USDC at one dollar. Moves the ledger time forward to
    /// the round when needed.
    fn round(&self, timestamp: u64, xlm_price_wad: i128) {
        let env = &self.t.env;
        if env.ledger().timestamp() < timestamp {
            env.ledger().with_mut(|ledger| ledger.timestamp = timestamp);
        }
        let client = RoundReflectorClient::new(env, &self.reflector);
        client.publish(
            &ReflectorAsset::Other(Symbol::new(env, "XLM")),
            &timestamp,
            &xlm_price_wad,
        );
        client.publish(
            &ReflectorAsset::Other(Symbol::new(env, "USDC")),
            &timestamp,
            &usd(1),
        );
    }

    /// Writes the RedStone XLM and USDC feeds at `timestamp` (seconds).
    fn push_redstone(&self, xlm_price_wad: i128, timestamp: u64) {
        let env = &self.t.env;
        let client = MockRedStonePriceFeedClient::new(env, &self.redstone);
        let ms = timestamp * 1_000;
        client.set_price_data(&String::from_str(env, "XLM"), &xlm_price_wad, &ms, &ms);
        client.set_price_data(&String::from_str(env, "USDC"), &usd(1), &ms, &ms);
    }

    fn xlm_quote(&self) -> PriceStatus {
        let key = PriceKey::Token(self.t.resolve_asset("XLM"));
        self.t
            .price_agg_client()
            .quotes(&vec![&self.t.env, key.clone()])
            .get_unchecked(key)
    }

    fn try_borrow_usdc_raw(&mut self, user: &str, amount: i128) -> Result<(), Error> {
        let account_id = self.t.resolve_account_id(user);
        let caller = self.t.get_or_create_user(user);
        let usdc = self.t.resolve_asset("USDC");
        let borrows = vec![&self.t.env, (hub_asset(usdc), amount)];
        match self
            .t
            .ctrl_client()
            .try_borrow(&caller, &account_id, &borrows, &None)
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(err) => Err(err.expect("expected contract error, got InvokeError")),
        }
    }
}

/// Borrows the largest USDC amount the LTV gate admits, starting at
/// `limit_raw` and stepping down one raw unit at a time. Returns the amount
/// borrowed. The pool rounds debt up, so the gate can refuse the exact limit.
fn borrow_largest_admitted(book: &mut Book, user: &str, limit_raw: i128) -> i128 {
    assert_contract_error(
        book.try_borrow_usdc_raw(user, limit_raw + 1),
        errors::INSUFFICIENT_COLLATERAL,
    );
    for below in 0..=10 {
        if book.try_borrow_usdc_raw(user, limit_raw - below).is_ok() {
            return limit_raw - below;
        }
    }
    panic!("no borrow within 10 raw units of {limit_raw} is admitted");
}

/// Dollar value in WAD of `xlm_raw` at `price_wad`.
fn xlm_value_wad(xlm_raw: i128, price_wad: i128) -> i128 {
    xlm_raw * price_wad / UNIT
}

/// Dollar value in WAD of `usdc_raw` at one dollar.
fn usdc_value_wad(usdc_raw: i128) -> i128 {
    usdc_raw * RAW_USD_WAD
}

/// XLM falls 36% between two Reflector rounds and RedStone has not written.
/// The mean of the newest three rounds is 0.11 against 0.125: outside the
/// 1.10 band. Withdrawal and borrowing on an indebted account fail closed.
#[test]
fn test_withdraw_fall_inside_one_round_with_late_push_fails_closed() {
    let mut book = Book::new();
    book.t.supply(ALICE, "XLM", 1_000_000.0);
    book.t.borrow(ALICE, "USDC", 37_500.0);

    book.round(MOVE_ROUND, WAD * 8 / 100);
    let quote = book.xlm_quote();
    assert!(!quote.valid);
    assert_eq!(quote.error_code, Some(errors::UNSAFE_PRICE));
    assert_eq!(
        quote.primary_wad, 110_000_000_000_000_000,
        "three-round mean"
    );
    assert_eq!(
        quote.secondary_wad, XLM_START_WAD,
        "RedStone has not written"
    );

    assert_contract_error(
        book.t.try_withdraw_raw(ALICE, "XLM", UNIT),
        errors::UNSAFE_PRICE,
    );
    assert_contract_error(book.try_borrow_usdc_raw(ALICE, UNIT), errors::UNSAFE_PRICE);
    assert_eq!(book.t.supply_balance_raw(ALICE, "XLM"), 1_000_000 * UNIT);
    assert_eq!(book.t.borrow_balance_raw(ALICE, "USDC"), 37_500 * UNIT);
}

/// Both legs read 0.125 before the move. The largest fall inside one round
/// that the band still admits is 27.28%. The key then serves 1.3126 times the
/// market price. A maximum borrow at LTV 75% stays covered by the collateral
/// at the market price. A fall of 27.29% fails closed.
#[test]
fn test_borrow_largest_admitted_fall_with_late_push_stays_solvent() {
    for (fall_bps, admitted) in [
        (2_700_i128, true),
        (2_728, true),
        (2_729, false),
        (2_800, false),
    ] {
        let mut book = Book::new();
        let market_wad = XLM_START_WAD * (10_000 - fall_bps) / 10_000;
        book.round(MOVE_ROUND, market_wad);
        let quote = book.xlm_quote();
        book.t.supply(ALICE, "XLM", 1_000_000.0);

        if !admitted {
            assert_eq!(quote.error_code, Some(errors::UNSAFE_PRICE), "{fall_bps}");
            assert_contract_error(book.try_borrow_usdc_raw(ALICE, UNIT), errors::UNSAFE_PRICE);
            continue;
        }
        assert!(quote.valid, "{fall_bps}");
        // Served price over market price, in parts per ten thousand.
        let served_over_market = quote.final_wad * 10_000 / market_wad;
        assert!(
            served_over_market <= 13_127,
            "{fall_bps}: {served_over_market}"
        );

        let collateral_raw = book.t.supply_balance_raw(ALICE, "XLM");
        let limit_wad = xlm_value_wad(collateral_raw, quote.final_wad) * XLM_LTV_BPS / 10_000;
        borrow_largest_admitted(&mut book, ALICE, limit_wad / RAW_USD_WAD);

        let collateral_at_market = xlm_value_wad(collateral_raw, market_wad);
        let debt = usdc_value_wad(book.t.borrow_balance_raw(ALICE, "USDC"));
        assert!(collateral_at_market > debt, "{fall_bps}");
        if fall_bps == 2_728 {
            assert_eq!(served_over_market, 13_126);
            // Collateral at the market price covers the debt 1.0157 times.
            assert_eq!(collateral_at_market * 10_000 / debt, 10_157);
        }
    }
}

/// A limit that the read rule does not bound. Before the move Reflector
/// prints 0.1275 and RedStone reads 0.125: a 2% gap, inside the band. The
/// two older rounds then hold the mean up, and the band admits a fall to
/// 0.086. The key serves 1.3876 times the market price, and a maximum borrow
/// at LTV 75% exceeds the collateral at the market price by 3.9% of the debt.
#[test]
fn test_borrow_fall_after_leg_gap_with_late_push_admits_bad_debt() {
    let mut book = Book::with_reflector_start(XLM_START_WAD * 102 / 100);
    assert!(book.xlm_quote().valid);

    let market_wad = WAD * 86 / 1_000;
    book.round(MOVE_ROUND, market_wad);
    let quote = book.xlm_quote();
    assert!(quote.valid);
    assert_eq!(
        quote.primary_wad, 113_666_666_666_660_000,
        "three-round mean"
    );
    assert_eq!(
        quote.secondary_wad, XLM_START_WAD,
        "RedStone has not written"
    );
    assert_eq!(quote.final_wad * 10_000 / market_wad, 13_875);

    book.t.supply(ALICE, "XLM", 1_000_000.0);
    let collateral_raw = book.t.supply_balance_raw(ALICE, "XLM");
    let limit_wad = xlm_value_wad(collateral_raw, quote.final_wad) * XLM_LTV_BPS / 10_000;
    borrow_largest_admitted(&mut book, ALICE, limit_wad / RAW_USD_WAD);

    let collateral_at_market = xlm_value_wad(collateral_raw, market_wad);
    let debt = usdc_value_wad(book.t.borrow_balance_raw(ALICE, "USDC"));
    assert_eq!(collateral_at_market, 86_000 * WAD);
    assert!(debt > collateral_at_market);
    // Shortfall over debt, in BPS.
    assert_eq!((debt - collateral_at_market) * 10_000 / debt, 391);
}

/// After the push lands at the market price the legs disagree until the mean
/// catches up: two rounds for a 36% fall.
#[test]
fn test_quotes_push_lands_after_fall_fails_closed_until_mean_catches_up() {
    let book = Book::new();
    let market_wad = WAD * 8 / 100;
    book.round(MOVE_ROUND, market_wad);
    book.push_redstone(market_wad, MOVE_ROUND);

    for (later, valid) in [(0, false), (1, false), (2, true), (3, true)] {
        if later > 0 {
            book.round(MOVE_ROUND + later * RESOLUTION, market_wad);
        }
        let quote = book.xlm_quote();
        assert_eq!(quote.valid, valid, "{later} rounds after the push");
        if !valid {
            assert_eq!(quote.error_code, Some(errors::UNSAFE_PRICE));
        }
    }
}

/// A limit that the read rule does not bound. Reflector prints a 9% fall in
/// every round of its window, so the mean equals the market price, and
/// RedStone is late. The legs stay inside the band. The key serves 1.0495
/// times the market price, so liquidation refuses an account whose health
/// factor at the market price is between 0.9529 and 1.
#[test]
fn test_liquidate_printed_fall_with_late_push_refuses_unhealthy_account() {
    let mut book = Book::new();
    book.t.supply(ALICE, "XLM", 1_000_000.0);
    book.t.borrow(ALICE, "USDC", 92_900.0);
    book.t.supply(BOB, "XLM", 1_000_000.0);
    book.t.borrow(BOB, "USDC", 93_400.0);

    let market_wad = XLM_START_WAD * 91 / 100;
    for later in 0..4 {
        book.round(MOVE_ROUND + later * RESOLUTION, market_wad);
    }
    let quote = book.xlm_quote();
    assert!(quote.valid);
    assert_eq!(quote.primary_wad, market_wad, "Reflector prints the fall");
    assert_eq!(
        quote.secondary_wad, XLM_START_WAD,
        "RedStone has not written"
    );
    assert_eq!(quote.final_wad, 119_375_000_000_000_000);
    assert_eq!(quote.final_wad * 10_000_000 / market_wad, 10_494_505);

    // Health factor at the market price, in BPS.
    let health_at_market = |book: &Book, user: &str| {
        let collateral = xlm_value_wad(book.t.supply_balance_raw(user, "XLM"), market_wad);
        let debt = usdc_value_wad(book.t.borrow_balance_raw(user, "USDC"));
        collateral * XLM_THRESHOLD_BPS / debt
    };
    assert_eq!(health_at_market(&book, ALICE), 9_550);
    assert!(book.t.health_factor_raw(ALICE) >= WAD);
    assert_contract_error(
        book.t.try_liquidate(LIQUIDATOR, ALICE, "USDC", 1_000.0),
        errors::HEALTH_FACTOR_TOO_HIGH,
    );

    assert_eq!(health_at_market(&book, BOB), 9_499);
    assert!(book.t.health_factor_raw(BOB) < WAD);
    book.t
        .try_liquidate(LIQUIDATOR, BOB, "USDC", 1_000.0)
        .unwrap();
}
