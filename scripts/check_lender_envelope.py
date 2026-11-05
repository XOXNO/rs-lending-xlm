#!/usr/bin/env python3
"""Lender-envelope gate: one provider contract cannot push a listing underwater.

The threat model (docs/explanation/threat-model.md, "Price integrity and
availability") shows that lenders cannot lose while `LT < 1 / u`, where `u` is
how far a reported price can move, and that when the debt price can also move
the effective `u` is the product of the two ratios. This check applies that
condition to every listing in `configs/<network>`:

    for every spoke, every provider contract P, every collateral c (LT > 0)
    and every borrowable debt d on that spoke:

        LT_c * u_P(c) * u_P(d) < 1 / (1 + MARGIN)

`u_P(k)` is the largest ratio between two prices the aggregator accepts for key
`k` while P alone changes its reports and every other provider stays put. It is
derived from the oracle configuration in `markets.json` (the `oracle` block of
each market and of each `references` entry); `oracle_feeds.json` lists only the
XOXNO feed ids and carries no band, tolerance or factor bound. The rules follow
the price-aggregator read path:

  * Feed leg: unbounded when P is its provider contract, else 1. The trust set
    of a feed is its provider contract alone (properties.rs:36-45).
  * Scaled leg: `factor * quote` (engine.rs:629-650). The factor must sit in
    `[min_factor_wad, max_factor_wad]` (engine.rs:639), so P moves it by at most
    `max/min` when P provides it, else 1; times `u_P(quote)`.
  * Aquarius volatile LP: `2 * sqrt(value_a * value_b)` (common/src/oracle/lp.rs:
    48-57, 76), so the reach is `sqrt(u_P(key_a) * u_P(key_b))`. The pool
    contract is not in the trust set (properties.rs:105-111) and is not treated
    as a provider here.
  * Aquarius stable LP: `D * min(price_a, price_b)` (common/src/oracle/lp_stable.rs:
    86-95, 115), so the reach is at most `max(u_P(key_a), u_P(key_b))`.
  * One-source key: the leg reach, capped by the sanity band `max/min`
    (engine.rs:142-146 rejects a price outside the band).
  * Two-source key: the legs must agree within `tolerance.upper_ratio_bps`
    (engine.rs:428); the read compares a half-up rounded BPS ratio
    (tolerance.rs:13-26), so the legs can differ by up to `(upper + 0.5) / BPS`,
    written `t` below. The key prices at the midpoint (engine.rs:430).
      - P reaches no leg: 1.
      - P reaches one leg: the other leg is fixed at `a`, the moved leg stays in
        `[a / t, a * t]`, so the midpoint moves by at most `min(t, leg reach)`.
      - P reaches both legs: the leg with the smaller reach `r` anchors the
        pair and the other stays within `t` of it, so the midpoint moves by at
        most `min(r * t, max leg reach)` (PR #196's range-bounded rule). Two
        plain feeds from P reach only the band; such a key has one trust set
        and keeps the 10% single-source band cap. A key whose two legs have
        different trust sets (SolvBTC: Scaled(factor x Ref BTC) + Feed) is
        exempt from that cap (admin.rs:171-177), so its band may not bound
        the reach.
    Either way the sanity band caps the result.

A key with several market configs for one token address (testnet hubs share
USDC) takes the largest reach. A pair whose two keys read identical sources is
skipped: both prices come from the same inputs, so their ratio cannot move.

Accepted violations are listed in `scripts/lender_envelope_exceptions.txt`,
each with the largest product it accepts. A violation that is not listed fails
the check, and so does a listed line that no longer violates (stale) or names
nothing, a listed pair whose product rose above its declared maximum
(worse), and a listed pair whose declared maximum sits more than LOOSE_SLACK
above its current product (loose: the headroom would silently accept a
worsening). Every line's network, spoke and listings must exist in
`configs/<network>`, whether or not that network is being checked.

    python3 scripts/check_lender_envelope.py                  # mainnet
    python3 scripts/check_lender_envelope.py mainnet testnet  # both
    python3 scripts/check_lender_envelope.py --top 40 --include-disabled testnet

Exit 0 = every pair is inside the envelope or declared at its maximum;
1 = an undeclared violation, a stale declaration, or a declared pair above its
maximum or loosely below it; 2 = a config or declaration-file error.
Deterministic, stdlib only, no network, no build.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import sys
from dataclasses import dataclass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONFIG_DIR = os.path.join(ROOT, "configs")
EXCEPTIONS_FILE = os.path.join(ROOT, "scripts", "lender_envelope_exceptions.txt")

# Safety margin: a pair fails when LT * u_c * u_d >= 1 / (1 + MARGIN).
MARGIN = 0.01
LIMIT = 1 / (1 + MARGIN)
BPS = 10_000
UNBOUNDED = math.inf
DEFAULT_NETWORKS = ("mainnet",)
FINDING_ID = re.compile(r"^S\d+-[A-Z0-9]+-\d+$")
# A declared pair fails as worse when its product exceeds max_product + this.
PRODUCT_SLACK = 1e-6
# A declared pair fails as loose when max_product exceeds its product by more
# than this. Products sit near 1, so 1e-3 absolute is about 0.1% relative: wide
# enough for a hand-rounded max-product (1.0144 or 1.015 for 1.014397), and a
# tenth of MARGIN, so the headroom a declaration can hide stays well inside
# the safety margin. A typo such as 1.5 for 1.0144 fails.
LOOSE_SLACK = 1e-3


class ConfigError(Exception):
    """A config or declaration file the check cannot interpret."""


def key_id(key: dict) -> str:
    """Stable label for a price key: `Token:<address>` or `Ref:<name>`."""
    if not isinstance(key, dict) or len(key) != 1:
        raise ConfigError(f"unrecognised price key {key!r}")
    (tag, value), = key.items()
    if tag not in ("Token", "Ref"):
        raise ConfigError(f"unrecognised price key {key!r}")
    return f"{tag}:{value}"


def span(low, high) -> float:
    low, high = int(low), int(high)
    return high / low if low > 0 else UNBOUNDED


def tolerance_ratio(oracle: dict) -> float:
    """Widest leg ratio a half-up `upper_ratio_bps` comparison admits."""
    return (oracle["tolerance"]["upper_ratio_bps"] + 0.5) / BPS


def feed_contract(feed: dict) -> str:
    (provider,) = feed["provider"].values()
    return provider["contract"]


def feed_provider_label(feed: dict) -> str:
    (kind,) = feed["provider"].keys()
    return kind


class Oracles:
    """The oracle configs of one network, indexed by price key."""

    def __init__(self, markets_doc: dict, include_disabled: bool = False):
        self.by_key: dict[str, list[dict]] = {}
        self.market_key: dict[str, str] = {}
        self.labels: dict[str, str] = {}
        for ref in markets_doc.get("references", []):
            self.by_key.setdefault(key_id(ref["key"]), []).append(ref["oracle"])
        for market in markets_doc.get("markets", []):
            if not include_disabled and market.get("enabled", True) is False:
                continue
            key = key_id({"Token": market["asset_address"]})
            self.by_key.setdefault(key, []).append(market["oracle"])
            self.market_key[market["name"]] = key
        for oracles in self.by_key.values():
            for oracle in oracles:
                self._collect_labels(oracle["sources"])

    def _collect_labels(self, sources):
        for source in sources:
            (kind, body), = source.items()
            if kind == "Feed":
                self.labels[feed_contract(body)] = feed_provider_label(body)
            elif kind == "Scaled":
                self.labels[feed_contract(body["factor"])] = feed_provider_label(body["factor"])

    def oracles(self, key: str) -> list[dict]:
        found = self.by_key.get(key)
        if not found:
            raise ConfigError(f"no oracle configured for {key}")
        return found

    def contracts(self, key: str, stack: tuple = ()) -> set[str]:
        """Every provider contract the price of `key` depends on."""
        if key in stack:
            raise ConfigError(f"oracle cycle through {key}")
        out: set[str] = set()
        for oracle in self.oracles(key):
            for source in oracle["sources"]:
                out |= self._source_contracts(source, stack + (key,))
        return out

    def _source_contracts(self, source: dict, stack: tuple) -> set[str]:
        (kind, body), = source.items()
        if kind == "Feed":
            return {feed_contract(body)}
        if kind == "Scaled":
            return {feed_contract(body["factor"])} | self.contracts(key_id(body["quote"]), stack)
        if kind in ("AquariusLp", "AquariusStableLp"):
            return self.contracts(key_id(body["key_a"]), stack) | self.contracts(
                key_id(body["key_b"]), stack
            )
        raise ConfigError(f"unrecognised price source {kind}")

    def reach(self, provider: str, key: str, stack: tuple = ()) -> float:
        """u_P(key): how far `provider` alone can move the accepted price of `key`."""
        if key in stack:
            raise ConfigError(f"oracle cycle through {key}")
        return max(self.oracle_reach(provider, o, stack + (key,)) for o in self.oracles(key))

    def oracle_reach(self, provider: str, oracle: dict, stack: tuple = ()) -> float:
        sources = oracle["sources"]
        if not 1 <= len(sources) <= 2:
            raise ConfigError(f"oracle with {len(sources)} sources")
        legs = [self.leg_reach(provider, source, stack) for source in sources]
        if len(legs) == 1:
            moved = legs[0]
        else:
            tolerance = tolerance_ratio(oracle)
            reached = [leg for leg in legs if leg > 1]
            if not reached:
                moved = 1.0
            elif len(reached) == 1:
                moved = min(reached[0], tolerance)
            else:
                moved = min(max(legs), min(legs) * tolerance)
        if moved <= 1:
            return 1.0
        return min(moved, span(oracle["min_sanity_price_wad"], oracle["max_sanity_price_wad"]))

    def leg_reach(self, provider: str, source: dict, stack: tuple = ()) -> float:
        (kind, body), = source.items()
        if kind == "Feed":
            return UNBOUNDED if feed_contract(body) == provider else 1.0
        if kind == "Scaled":
            factor = 1.0
            if feed_contract(body["factor"]) == provider:
                factor = span(body["min_factor_wad"], body["max_factor_wad"])
            return factor * self.reach(provider, key_id(body["quote"]), stack)
        if kind in ("AquariusLp", "AquariusStableLp"):
            reach_a = self.reach(provider, key_id(body["key_a"]), stack)
            reach_b = self.reach(provider, key_id(body["key_b"]), stack)
            return math.sqrt(reach_a * reach_b) if kind == "AquariusLp" else max(reach_a, reach_b)
        raise ConfigError(f"unrecognised price source {kind}")


@dataclass
class Pair:
    network: str
    spoke: str
    collateral: str
    debt: str
    provider: str
    provider_label: str
    lt: float
    ltv: float
    u_c: float
    u_d: float

    @property
    def product(self) -> float:
        return self.lt * self.u_c * self.u_d

    @property
    def ltv_product(self) -> float:
        return self.ltv * self.u_c * self.u_d

    @property
    def violates(self) -> bool:
        return self.product >= LIMIT

    @property
    def ident(self) -> tuple:
        return (self.network, self.spoke, self.collateral, self.debt, self.provider)


def load_json(path: str):
    try:
        with open(path) as handle:
            return json.load(handle)
    except (OSError, json.JSONDecodeError) as err:
        raise ConfigError(f"{path}: {err}") from err


def evaluate(network: str, markets_doc: dict, spokes_doc: dict, include_disabled: bool = False):
    """Every (spoke, collateral, debt, provider) pair of one network."""
    oracles = Oracles(markets_doc, include_disabled)
    pairs: list[Pair] = []

    def enabled(entry: dict) -> bool:
        return include_disabled or entry.get("enabled", True) is not False

    for spoke_id, spoke in spokes_doc.items():
        if not enabled(spoke):
            continue
        listed = {name: asset for name, asset in spoke["assets"].items() if enabled(asset)}
        for name in listed:
            if name not in oracles.market_key:
                raise ConfigError(f"{network} spoke {spoke_id}: no enabled market named {name}")
        collaterals = [
            (name, asset)
            for name, asset in listed.items()
            if asset.get("can_be_collateral") and asset["liquidation_threshold"] > 0
        ]
        debts = [name for name, asset in listed.items() if asset.get("can_be_borrowed")]
        for c_name, c_asset in collaterals:
            c_key = oracles.market_key[c_name]
            for d_name in debts:
                d_key = oracles.market_key[d_name]
                if [o["sources"] for o in oracles.oracles(c_key)] == [
                    o["sources"] for o in oracles.oracles(d_key)
                ]:
                    continue
                for provider in sorted(oracles.contracts(c_key) | oracles.contracts(d_key)):
                    pairs.append(
                        Pair(
                            network=network,
                            spoke=spoke_id,
                            collateral=c_name,
                            debt=d_name,
                            provider=provider,
                            provider_label=oracles.labels.get(provider, "?"),
                            lt=c_asset["liquidation_threshold"] / BPS,
                            ltv=c_asset["ltv"] / BPS,
                            u_c=oracles.reach(provider, c_key),
                            u_d=oracles.reach(provider, d_key),
                        )
                    )
    return pairs


def evaluate_network(config_dir: str, network: str, include_disabled: bool = False):
    base = os.path.join(config_dir, network)
    markets_doc = load_json(os.path.join(base, "markets.json"))
    spokes_doc = load_json(os.path.join(base, "spokes.json"))
    return evaluate(network, markets_doc, spokes_doc, include_disabled)


@dataclass
class Exception_:
    network: str
    spoke: str
    collateral: str
    debt: str
    provider: str
    finding: str
    max_product: float
    reason: str
    line: int

    @property
    def ident(self) -> tuple:
        return (self.network, self.spoke, self.collateral, self.debt, self.provider)


def parse_exceptions(text: str, origin: str = "exceptions") -> list[Exception_]:
    out: list[Exception_] = []
    seen: set[tuple] = set()
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split(None, 7)
        if len(fields) < 8:
            raise ConfigError(f"{origin}:{number}: expected `network spoke collateral debt provider finding-id max-product reason`")
        *head, raw_max, reason = fields
        try:
            max_product = float(raw_max)
        except ValueError:
            max_product = math.nan
        if not math.isfinite(max_product) or max_product <= 0:
            raise ConfigError(
                f"{origin}:{number}: max-product {raw_max!r} is not a positive number"
            )
        entry = Exception_(*head, max_product=max_product, reason=reason, line=number)
        if not FINDING_ID.match(entry.finding):
            raise ConfigError(f"{origin}:{number}: finding id {entry.finding!r} is not like S3-MC-1")
        if entry.ident in seen:
            raise ConfigError(f"{origin}:{number}: duplicate declaration")
        seen.add(entry.ident)
        out.append(entry)
    return out


def check_declared_networks(
    config_dir: str, exceptions: list[Exception_], origin: str = "exceptions"
):
    """Every declaration names an existing network, spoke and both listings in
    `configs/<network>/spokes.json`, whether or not the network is being
    checked, so a misspelt name is a precise error rather than a stale line
    plus an undeclared violation. Lines for checked networks are then judged
    against the evaluated pairs."""
    spokes_by_network: dict[str, dict] = {}
    for e in exceptions:
        base = os.path.join(config_dir, e.network)
        if not os.path.isdir(base):
            raise ConfigError(f"{origin}:{e.line}: no configs/{e.network} directory for {e.network!r}")
        if e.network not in spokes_by_network:
            spokes_by_network[e.network] = load_json(os.path.join(base, "spokes.json"))
        assets = spokes_by_network[e.network].get(e.spoke, {}).get("assets")
        if assets is None:
            raise ConfigError(f"{origin}:{e.line}: {e.network} has no spoke {e.spoke}")
        for name in (e.collateral, e.debt):
            if name not in assets:
                raise ConfigError(
                    f"{origin}:{e.line}: {e.network} spoke {e.spoke} does not list {name}"
                )


def judge(pairs: list[Pair], exceptions: list[Exception_], networks):
    """Split into undeclared violations, declared violations, stale lines,
    declared pairs whose product rose above the declared maximum (worse) and
    declared pairs whose maximum sits more than LOOSE_SLACK above the current
    product (loose)."""
    declared = {e.ident: e for e in exceptions if e.network in networks}
    violating = {p.ident: p for p in pairs if p.violates}
    undeclared = [p for ident, p in violating.items() if ident not in declared]
    accepted = [p for ident, p in violating.items() if ident in declared]
    stale = [e for ident, e in declared.items() if ident not in violating]
    worse = [
        (p, declared[p.ident])
        for p in accepted
        if p.product > declared[p.ident].max_product + PRODUCT_SLACK
    ]
    loose = [
        (p, declared[p.ident])
        for p in accepted
        if declared[p.ident].max_product > p.product + LOOSE_SLACK
    ]
    return undeclared, accepted, stale, worse, loose


def fmt_ratio(value: float) -> str:
    return "inf" if value == UNBOUNDED else f"{value:.6f}"


def print_table(pairs: list[Pair], declared: set, top: int):
    rows = sorted(pairs, key=lambda p: p.product, reverse=True)[:top]
    if not rows:
        return
    header = (
        f"{'network':<8} {'spoke':>5} {'collateral':<20} {'debt':<12} "
        f"{'provider':<20} {'LT':>5} {'u_c':>9} {'u_d':>9} {'LT*u*u':>9} {'LTV*u*u':>9}  status"
    )
    print(header)
    print("-" * len(header))
    for p in rows:
        status = "ok"
        if p.violates:
            status = "excepted" if p.ident in declared else "VIOLATION"
        provider = f"{p.provider_label}:{p.provider[:6]}"
        print(
            f"{p.network:<8} {p.spoke:>5} {p.collateral:<20} {p.debt:<12} {provider:<20} "
            f"{p.lt:>5.2f} {fmt_ratio(p.u_c):>9} {fmt_ratio(p.u_d):>9} "
            f"{p.product:>9.6f} {p.ltv_product:>9.6f}  {status}"
        )


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("networks", nargs="*", default=list(DEFAULT_NETWORKS))
    parser.add_argument("--config-dir", default=CONFIG_DIR)
    parser.add_argument("--exceptions", default=EXCEPTIONS_FILE)
    parser.add_argument("--top", type=int, default=15, help="rows in the worst-pairs table")
    parser.add_argument(
        "--include-disabled",
        action="store_true",
        help="also check spokes, listings and markets with enabled=false (report only)",
    )
    args = parser.parse_args(argv)

    try:
        origin = os.path.relpath(args.exceptions, ROOT)
        with open(args.exceptions) as handle:
            exceptions = parse_exceptions(handle.read(), origin)
        check_declared_networks(args.config_dir, exceptions, origin)
        pairs = []
        for network in args.networks:
            pairs += evaluate_network(args.config_dir, network, args.include_disabled)
    except (ConfigError, OSError) as err:
        print(f"lender-envelope: error: {err}", file=sys.stderr)
        return 2

    undeclared, accepted, stale, worse, loose = judge(pairs, exceptions, set(args.networks))
    declared = {p.ident for p in accepted}
    print(
        f"lender envelope: LT * u_P(c) * u_P(d) < 1/(1+{MARGIN:g}) = {LIMIT:.6f}; "
        f"networks {', '.join(args.networks)}; {len(pairs)} (pair, provider) rows"
    )
    print_table(pairs, declared, args.top)

    if args.include_disabled:
        print("\n--include-disabled is a report-only run; declarations are not judged.")
        return 0

    for p in sorted(undeclared, key=lambda p: p.product, reverse=True):
        print(
            f"\nVIOLATION {' '.join(p.ident)}: LT {p.lt:.4f} * u_c {fmt_ratio(p.u_c)} "
            f"* u_d {fmt_ratio(p.u_d)} = {p.product:.6f} >= {LIMIT:.6f}\n"
            "    fix: tighten the listing or oracle config, or declare the pair in "
            "scripts/lender_envelope_exceptions.txt with its finding id and max-product."
        )
    for e in stale:
        print(
            f"\nSTALE {origin}:{e.line} "
            f"{' '.join(e.ident)} ({e.finding}) no longer violates the envelope\n"
            "    fix: drop the stale declaration."
        )
    worse_ids = {p.ident for p, _ in worse}
    loose_ids = {p.ident for p, _ in loose}
    if accepted:
        print("\ndeclared pairs (current product vs declared max-product):")
        for p in sorted(accepted, key=lambda p: p.product, reverse=True):
            e = next(x for x in exceptions if x.ident == p.ident)
            mark = "WORSE" if p.ident in worse_ids else "LOOSE" if p.ident in loose_ids else "ok"
            print(
                f"  {origin}:{e.line} {' '.join(p.ident[:4])} {p.provider_label}:{p.provider[:6]} "
                f"current {p.product:.6f} declared {e.max_product:.6f}  {mark}"
            )
    for p, e in sorted(worse, key=lambda pe: pe[0].product, reverse=True):
        print(
            f"\nWORSE {origin}:{e.line} {' '.join(p.ident)} ({e.finding}): "
            f"current {p.product:.6f} > declared max-product {e.max_product:.6f}\n"
            "    fix: undo the change that widened the pair, or re-declare it with the "
            "new product if the increase is accepted."
        )
    for p, e in sorted(loose, key=lambda pe: pe[0].product, reverse=True):
        print(
            f"\nLOOSE {origin}:{e.line} {' '.join(p.ident)} ({e.finding}): "
            f"declared max-product {e.max_product:.6f} > current {p.product:.6f} "
            f"+ {LOOSE_SLACK:g}\n"
            "    fix: set max-product to the current product; the headroom would "
            "accept a worsening unseen."
        )
    failed = bool(undeclared or stale or worse or loose)
    print(
        f"\n{'FAIL' if failed else 'OK'}: {len(undeclared)} undeclared violation(s), "
        f"{len(accepted)} declared, {len(stale)} stale declaration(s), "
        f"{len(worse)} declared pair(s) above max-product, "
        f"{len(loose)} declared pair(s) loosely below max-product."
    )
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
