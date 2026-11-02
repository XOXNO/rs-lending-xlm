"""Checks for the lender-envelope gate: the reach rules reproduce Specula run 3's
published figures (S3-MC-1, S3-CR-1), and the declaration file is judged in
both directions (undeclared and stale)."""

import json
import math
import os
import tempfile
import unittest

import check_lender_envelope as le

REDSTONE = "CA526Y2NQWGWVVQ7RFFPGAZMU66PSYJ3UC2MTVAV4ZU7OM5BOPHDXUSG"
REFLECTOR = "CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN"
CHAINLINK = "CDQBHEUGLDHMSUHG4IH33RJBUTMNCZOT4JIAQR4WZFRDZW6DR6S32ZLZ"
WAD = 10**18


def redstone(feed_id):
    return {
        "Feed": {
            "provider": {"RedStone": {"contract": REDSTONE, "feed_id": feed_id, "nature": "Fundamental"}},
            "decimals": 8,
            "max_stale_seconds": 57600,
        }
    }


def reflector(symbol, contract=REFLECTOR):
    return {
        "Feed": {
            "provider": {
                "Reflector": {"contract": contract, "asset": {"Symbol": symbol}, "read_mode": {"Twap": 3}}
            },
            "decimals": 14,
            "max_stale_seconds": 3600,
        }
    }


def oracle(sources, low, high, upper_bps=10500):
    return {
        "sources": sources,
        "tolerance": {"upper_ratio_bps": upper_bps, "lower_ratio_bps": 0},
        "min_sanity_price_wad": str(int(low * WAD)),
        "max_sanity_price_wad": str(int(high * WAD)),
    }


def market(name, orc, enabled=True):
    out = {"name": name, "asset_address": f"ADDR_{name}", "oracle": orc}
    if not enabled:
        out["enabled"] = False
    return out


# The mainnet oracle shapes Specula exercised, reduced to what the reach reads.
BTC_REF = {"name": "BTC", "key": {"Ref": "BTC"}, "oracle": oracle([reflector("BTC"), redstone("BTC")], 20000, 300000, 11000)}
SOLVBTC = oracle(
    [
        {
            "Scaled": {
                "factor": redstone("SolvBTC_FUNDAMENTAL")["Feed"],
                "quote": {"Ref": "BTC"},
                "min_factor_wad": str(WAD),
                "max_factor_wad": str(105 * WAD // 100),
            }
        },
        redstone("SolvBTC_FUNDAMENTAL/USD"),
    ],
    20000,
    300000,
)
MARKETS = {
    "references": [BTC_REF],
    "markets": [
        market("XLM", oracle([reflector("XLM"), redstone("XLM")], 0.05, 1, 11000)),
        market("USDC", oracle([reflector("USDC"), redstone("USDC")], 0.95, 1.05)),
        market("EURC", oracle([reflector("EURC"), redstone("EUROC")], 1.06, 1.22)),
        market("PYUSD", oracle([redstone("PYUSD")], 0.95, 1.05)),
        market("USDY", oracle([redstone("USDY_FUNDAMENTAL/USD")], 1.08, 1.32)),
        market("SolvBTC", SOLVBTC),
        market("xSolvBTC", SOLVBTC),
    ],
}


def key(name):
    return f"Token:ADDR_{name}"


def listing(ltv, lt, collateral=True, borrowed=True, enabled=True):
    out = {
        "can_be_collateral": collateral,
        "can_be_borrowed": borrowed,
        "ltv": ltv,
        "liquidation_threshold": lt,
    }
    if not enabled:
        out["enabled"] = False
    return out


class ReachMathTest(unittest.TestCase):
    def setUp(self):
        self.oracles = le.Oracles(MARKETS)

    def test_single_source_band_ratio_usdy(self):
        # S3-MC-1: USDY single RedStone source, band [1.08, 1.32] -> u = 11/9.
        u = self.oracles.reach(REDSTONE, key("USDY"))
        self.assertAlmostEqual(u, 11 / 9, places=12)
        self.assertEqual(round(0.85 * u, 3), 1.039)

    def test_dual_source_midpoint_lever(self):
        # S3-CR-1: one leg of XLM (tolerance 11000) -> 1.10005; USDC/EURC
        # (10500) -> 1.05005, for whichever provider holds one leg.
        self.assertAlmostEqual(self.oracles.reach(REDSTONE, key("XLM")), 1.10005, places=12)
        self.assertAlmostEqual(self.oracles.reach(REFLECTOR, key("XLM")), 1.10005, places=12)
        for name in ("USDC", "EURC"):
            self.assertAlmostEqual(self.oracles.reach(REFLECTOR, key(name)), 1.05005, places=12)

    def test_scaled_factor_times_base_lever_times_anchor(self):
        # S3-CR-1: SolvBTC under RedStone = factor range 1.05 x Ref BTC lever
        # 1.10005 x anchor tolerance 1.05005 = 1.21286.
        u = self.oracles.reach(REDSTONE, key("SolvBTC"))
        self.assertAlmostEqual(u, 1.05 * 1.10005 * 1.05005, places=12)
        self.assertEqual(round(u, 5), 1.21286)
        # Reflector reaches only the scaled leg (through BTC): the tolerance caps it.
        self.assertAlmostEqual(self.oracles.reach(REFLECTOR, key("SolvBTC")), 1.05005, places=12)

    def test_spoke_1_xlm_against_solvbtc_debt(self):
        u_c = self.oracles.reach(REDSTONE, key("XLM"))
        u_d = self.oracles.reach(REDSTONE, key("SolvBTC"))
        self.assertEqual(round(u_c, 5), 1.10005)
        self.assertEqual(round(u_d, 5), 1.21286)
        self.assertEqual(round(0.75 * u_c * u_d, 6), 1.000657)
        self.assertEqual(round(0.78 * u_c * u_d, 4), 1.0407)

    def test_tighter_btc_tolerance_and_factor_cap_close_spoke_1(self):
        # The S3-CR-1 fix: Ref BTC tolerance +-4% (10400) and a 1.03 factor cap
        # bring SolvBTC under RedStone to 1.03 x 1.04005 x 1.05005 = 1.124868,
        # so the binding spoke-1 pair, PYUSD (LT 0.79) -> SolvBTC, is 0.982187.
        btc = {**BTC_REF, "oracle": oracle([reflector("BTC"), redstone("BTC")], 20000, 300000, 10400)}
        scaled = dict(SOLVBTC["sources"][0]["Scaled"], max_factor_wad=str(103 * WAD // 100))
        solvbtc = dict(SOLVBTC, sources=[{"Scaled": scaled}, SOLVBTC["sources"][1]])
        markets = {
            "references": [btc],
            "markets": [m for m in MARKETS["markets"] if m["name"] != "SolvBTC"]
            + [market("SolvBTC", solvbtc)],
        }
        oracles = le.Oracles(markets)
        u_d = oracles.reach(REDSTONE, key("SolvBTC"))
        self.assertAlmostEqual(u_d, 1.03 * 1.04005 * 1.05005, places=12)
        u_c = oracles.reach(REDSTONE, key("PYUSD"))
        self.assertEqual(round(0.79 * u_c * u_d, 6), 0.982187)
        self.assertLess(0.79 * u_c * u_d, le.LIMIT)

    def test_control_usdc_debt(self):
        u_c = self.oracles.reach(REDSTONE, key("XLM"))
        u_d = self.oracles.reach(REDSTONE, key("USDC"))
        self.assertEqual(round(0.75 * u_c * u_d, 6), 0.866331)

    def test_spoke_5_reflector_usdc_eurc(self):
        u_c = self.oracles.reach(REFLECTOR, key("USDC"))
        u_d = self.oracles.reach(REFLECTOR, key("EURC"))
        self.assertEqual(round(u_c, 5), 1.05005)
        self.assertEqual(round(u_d, 5), 1.05005)
        self.assertEqual(round(0.92 * u_c * u_d, 4), 1.0144)

    def test_independent_key_does_not_move(self):
        self.assertEqual(self.oracles.reach(REFLECTOR, key("USDY")), 1.0)
        self.assertEqual(self.oracles.reach(CHAINLINK, key("SolvBTC")), 1.0)

    def test_two_plain_feeds_from_one_provider_take_the_band(self):
        orc = oracle([redstone("A"), redstone("B")], 1.0, 1.2)
        self.assertAlmostEqual(self.oracles.oracle_reach(REDSTONE, orc), 1.2, places=12)

    def test_band_caps_every_lever(self):
        orc = oracle([reflector("X"), redstone("X")], 1.0, 1.02, 11000)
        self.assertAlmostEqual(self.oracles.oracle_reach(REDSTONE, orc), 1.02, places=12)

    def test_lp_rules(self):
        lp = {"key_a": {"Token": "ADDR_USDY"}, "key_b": {"Token": "ADDR_USDC"}}
        volatile = oracle([{"AquariusLp": lp}], 1, 100, 0)
        stable = oracle([{"AquariusStableLp": lp}], 1, 100, 0)
        self.assertAlmostEqual(
            self.oracles.oracle_reach(REDSTONE, volatile), math.sqrt(11 / 9 * 1.05005), places=12
        )
        self.assertAlmostEqual(self.oracles.oracle_reach(REDSTONE, stable), 11 / 9, places=12)

    def test_missing_quote_is_a_config_error(self):
        orc = oracle(
            [{"Scaled": {"factor": redstone("F")["Feed"], "quote": {"Ref": "NOPE"},
                         "min_factor_wad": str(WAD), "max_factor_wad": str(WAD)}}],
            1,
            2,
        )
        with self.assertRaises(le.ConfigError):
            self.oracles.oracle_reach(REDSTONE, orc)


SPOKES = {
    "1": {
        "assets": {
            "XLM": listing(7500, 7800),
            "USDC": listing(7500, 8000),
            "SolvBTC": listing(6000, 7000),
            "xSolvBTC": listing(6000, 7000),
        }
    },
    "5": {
        "assets": {
            "USDC": listing(8800, 9200),
            "EURC": listing(8400, 8800),
            "USDY": listing(8000, 8500, borrowed=False),
            "OFF": listing(8000, 9900, borrowed=False, enabled=False),
        }
    },
    "9": {"enabled": False, "assets": {"USDY": listing(8000, 9900), "USDC": listing(7600, 8000)}},
}


def violations(pairs):
    return {(p.spoke, p.collateral, p.debt, p.provider) for p in pairs if p.violates}


class EvaluateTest(unittest.TestCase):
    def setUp(self):
        markets = dict(MARKETS)
        markets["markets"] = MARKETS["markets"] + [
            market("OFF", oracle([redstone("OFF")], 1, 2), enabled=False)
        ]
        self.pairs = le.evaluate("mainnet", markets, SPOKES)

    def test_reported_violations(self):
        self.assertEqual(
            violations(self.pairs),
            {
                ("1", "XLM", "SolvBTC", REDSTONE),
                ("1", "XLM", "xSolvBTC", REDSTONE),
                ("1", "USDC", "SolvBTC", REDSTONE),
                ("1", "USDC", "xSolvBTC", REDSTONE),
                ("5", "USDC", "EURC", REDSTONE),
                ("5", "USDC", "EURC", REFLECTOR),
                ("5", "USDY", "USDC", REDSTONE),
                ("5", "USDY", "EURC", REDSTONE),
            },
        )

    def test_identical_sources_and_same_key_are_skipped(self):
        pairs = {(p.collateral, p.debt) for p in self.pairs}
        self.assertNotIn(("SolvBTC", "xSolvBTC"), pairs)
        self.assertNotIn(("USDC", "USDC"), pairs)

    def test_disabled_spokes_and_listings_are_skipped(self):
        self.assertNotIn("9", {p.spoke for p in self.pairs})
        self.assertNotIn("OFF", {p.collateral for p in self.pairs})

    def test_margin_is_one_percent(self):
        self.assertAlmostEqual(le.LIMIT, 1 / 1.01, places=15)


class DeclarationTest(unittest.TestCase):
    def setUp(self):
        self.pairs = le.evaluate("mainnet", MARKETS, {"5": SPOKES["5"]})

    def text(self, *lines):
        return "# comment\n\n" + "\n".join(lines) + "\n"

    def test_undeclared_and_stale_both_fail(self):
        declared = le.parse_exceptions(
            self.text(
                f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 1.2 pending config fix",
                f"mainnet 5 USDY USDC {REFLECTOR} S3-MC-1 1.2 never violated",
            )
        )
        undeclared, accepted, stale, worse = le.judge(self.pairs, declared, {"mainnet"})
        self.assertEqual([p.ident[1:4] for p in accepted], [("5", "USDY", "USDC")])
        self.assertIn(("5", "USDY", "EURC", REDSTONE), {p.ident[1:] for p in undeclared})
        self.assertEqual([e.provider for e in stale], [REFLECTOR])
        self.assertEqual(worse, [])

    def test_declared_pair_above_its_max_product_fails(self):
        # USDY -> USDC under RedStone: 0.85 x 11/9 x 1.05005 = 1.090885.
        product = 0.85 * 11 / 9 * 1.05005
        for max_product, is_worse in (
            (round(product, 6), False),
            (product - 0.5e-6, False),
            (product - 2e-6, True),
            (1.0144, True),
        ):
            declared = le.parse_exceptions(
                self.text(f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 {max_product!r} r")
            )
            _, accepted, stale, worse = le.judge(self.pairs, declared, {"mainnet"})
            self.assertEqual(len(accepted), 1)
            self.assertEqual(stale, [])
            self.assertEqual(bool(worse), is_worse, msg=max_product)
            if is_worse:
                ((pair, entry),) = worse
                self.assertAlmostEqual(pair.product, product, places=12)
                self.assertEqual(entry.max_product, max_product)

    def test_malformed_lines_are_errors(self):
        for line in (
            f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 1.1",
            f"mainnet 5 USDY USDC {REDSTONE} MC1 1.1 reason",
            f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 pending config fix",
            f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 0 reason",
            f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 nan reason",
            f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 inf reason",
        ):
            with self.assertRaises(le.ConfigError, msg=line):
                le.parse_exceptions(self.text(line))
        dup = f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 1.1 reason"
        with self.assertRaises(le.ConfigError):
            le.parse_exceptions(self.text(dup, dup))

    def test_reason_keeps_spaces(self):
        (entry,) = le.parse_exceptions(self.text(f"mainnet 5 USDY USDC {REDSTONE} S3-MC-1 1.1 a b  c"))
        self.assertEqual(entry.reason, "a b  c")
        self.assertEqual(entry.max_product, 1.1)


class DeclaredNetworkTest(unittest.TestCase):
    """A line names an existing configs/<network>; a line for a network the run
    does not check must also name an existing spoke and listings there."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        for network in ("mainnet", "testnet"):
            os.makedirs(os.path.join(self.tmp.name, network))
            with open(os.path.join(self.tmp.name, network, "spokes.json"), "w") as handle:
                json.dump({"5": SPOKES["5"]}, handle)

    def check(self, line, networks=("mainnet",)):
        declared = le.parse_exceptions(line + "\n")
        le.check_declared_networks(self.tmp.name, declared, set(networks))

    def test_unknown_network_is_an_error(self):
        with self.assertRaisesRegex(le.ConfigError, "no configs/mainet"):
            self.check(f"mainet 5 USDY USDC {REDSTONE} S3-MC-1 1.1 typo")

    def test_unchecked_network_line_must_name_existing_spoke_and_listings(self):
        self.check(f"testnet 5 USDY USDC {REDSTONE} S3-MC-1 1.1 ok")
        with self.assertRaisesRegex(le.ConfigError, "has no spoke 7"):
            self.check(f"testnet 7 USDY USDC {REDSTONE} S3-MC-1 1.1 x")
        with self.assertRaisesRegex(le.ConfigError, "does not list NOPE"):
            self.check(f"testnet 5 USDY NOPE {REDSTONE} S3-MC-1 1.1 x")

    def test_checked_network_lines_are_left_to_judge(self):
        # A missing pair on a checked network is reported as stale by judge().
        self.check(f"mainnet 7 USDY NOPE {REDSTONE} S3-MC-1 1.1 x")
        pairs = le.evaluate("mainnet", MARKETS, {"5": SPOKES["5"]})
        declared = le.parse_exceptions(f"mainnet 7 USDY NOPE {REDSTONE} S3-MC-1 1.1 x\n")
        _, _, stale, _ = le.judge(pairs, declared, {"mainnet"})
        self.assertEqual(len(stale), 1)

    def test_unchecked_network_lines_are_not_judged(self):
        pairs = le.evaluate("mainnet", MARKETS, {"5": SPOKES["5"]})
        declared = le.parse_exceptions(f"testnet 5 USDY USDC {REDSTONE} S3-MC-1 1.1 x\n")
        self.assertEqual(le.judge(pairs, declared, {"mainnet"})[2], [])


if __name__ == "__main__":
    unittest.main()
