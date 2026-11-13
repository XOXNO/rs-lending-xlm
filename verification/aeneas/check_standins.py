#!/usr/bin/env python3
"""Checks the hand-written stand-ins of the extraction crates against production.

Most production code reaches Lean through `#[path]` includes, so `run.sh`
notices any change to it (the regenerated Lean differs). The items below are
the exception: production code that an extraction crate carries as a copy,
because the file it lives in cannot be included whole. A change to the
production item would leave the copy, the generated Lean and every proof
untouched. This script closes that gap. It needs only Python and runs first
in `run.sh`.

Three kinds of check, all on the source text with comments, attributes and
whitespace removed:

* VERBATIM: the stand-in item equals the production item.
* SUBSET: every field / variant of the stand-in item is one of the production
  item's (the stand-in documents which ones it leaves out).
* PINNED: the stand-in is an adaptation, so the production item's text is
  pinned by hash. When production changes, the check fails; re-read the
  stand-in against the new production code, then update the hash (the failure
  message prints it).

Usage: ./check_standins.py     (exit status 1 on any mismatch)
"""
import hashlib
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent

TYPES = [f"common/src/types/{f}.rs" for f in ("controller", "oracle", "pool", "shared")]
CTRL = "verification/aeneas/controller/src"
PROD_CTRL = "contracts/controller/src"

# (stand-in file, production file(s), item selectors)
VERBATIM = [
    (f"{CTRL}/types.rs", TYPES, [
        "struct SpokeConfig", "struct HubAssetKey", "type HubPayment", "type AggregatedPayments",
        "enum AccountPositionType", "struct PriceFeedRaw", "struct PriceFeed",
        "impl PriceFeed :: fn usd_value_wad",
        "impl From<&PriceFeedRaw> for PriceFeed", "impl From<&PriceFeed> for PriceFeedRaw",
        "struct MarketIndexRaw", "struct MarketIndex",
        "impl From<&MarketIndexRaw> for MarketIndex", "impl From<&MarketIndex> for MarketIndexRaw",
        "struct AccountPositionRaw", "struct AccountPosition",
        "impl From<&AccountPositionRaw> for AccountPosition",
        "impl From<&AccountPosition> for AccountPositionRaw",
        "struct ScaledPositionRaw",
        "impl From<&AccountPosition> for ScaledPositionRaw",
        "impl From<&DebtPosition> for ScaledPositionRaw",
        "struct PoolAction", "struct PoolWithdrawEntry", "struct PoolSeizeEntry",
        "struct AssetConfig", "struct DebtPositionRaw", "struct DebtPosition",
        "impl From<&DebtPositionRaw> for DebtPosition",
        "struct PaymentTuple", "struct SeizeEntry", "struct RepayEntry", "struct LiquidationResult",
        "impl Account :: fn get_or_create_supply_position",
    ]),
    (f"{CTRL}/context.rs", [f"{PROD_CTRL}/context.rs"], ["impl Context :: fn cached_price"]),
    (f"{CTRL}/payment_legs.rs", [f"{PROD_CTRL}/payments.rs"], [
        "enum ZeroLeg", "fn aggregate_positive_payments", "fn aggregate_payments",
        "fn aggregate_payment_amount",
    ]),
    (f"{CTRL}/positions/mod.rs", [f"{PROD_CTRL}/positions/mod.rs", f"{PROD_CTRL}/positions/supply.rs"], [
        "enum FreezePolicy", "enum WithdrawKind", "fn make_pool_action",
        "fn get_supply_position_or_panic",
    ]),
    (f"{CTRL}/validation.rs", ["common/src/validation.rs"], [
        "fn require_nonneg_amount", "fn expect_invariant", "fn require_non_empty_payments",
    ]),
    (f"{CTRL}/collections.rs", ["common/src/collections.rs"], ["fn push_unique_address"]),
    (f"{CTRL}/spoke_usage.rs", [f"{PROD_CTRL}/spoke_usage.rs"], ["enum UsageSide"]),
    ("verification/aeneas/pool/src/types.rs", ["common/src/types/oracle.rs"], ["struct OracleTolerance"]),
    ("verification/aeneas/pool/src/oracle.rs", ["common/src/oracle/observation.rs"], [
        "const MAX_TWAP_RECORDS", "const MAX_SINGLE_SOURCE_SANITY_BAND_BPS",
        "const MAX_LP_SANITY_BAND_BPS",
    ]),
]

SUBSET = [
    # `Account` without `mode` (no included function reads it).
    (f"{CTRL}/types.rs", TYPES, ["struct Account"]),
    (f"{CTRL}/errors.rs", ["common/src/errors.rs"], [
        "enum GenericError", "enum CollateralError", "enum OracleError", "enum SpokeError",
    ]),
    (f"{CTRL}/events.rs", [f"{PROD_CTRL}/events/mod.rs"], ["enum PositionAction"]),
    ("verification/aeneas/src/types.rs", ["common/src/types/pool.rs"], ["struct MarketParams"]),
    ("verification/aeneas/src/errors.rs", ["common/src/errors.rs"], ["enum GenericError"]),
    ("verification/aeneas/oracle/src/errors.rs", ["common/src/errors.rs"], [
        "enum GenericError", "enum OracleError",
    ]),
]

# Production items whose stand-in is an adaptation (see the stand-in's doc
# comment). The hash is of the production item's normalised text.
PINNED = [
    # Stand-in folds `cached_pool_address` + `fetch_pool_bulk_indexes` into one opaque fetch.
    (f"{PROD_CTRL}/context.rs", "impl Context :: fn cached_market_index", "dcef54937de712b4"),
    # Stand-ins build the same items into a vector (no `impl Iterator` / `map`).
    (f"{PROD_CTRL}/storage/account.rs", "fn iter_typed_positions", "5e086d852c330973"),
    (f"{PROD_CTRL}/storage/account.rs", "fn iter_debt_positions", "97f5df0076f209d3"),
]


def strip(text: str) -> str:
    """Removes comments and attributes; keeps string literals intact."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            i = text.find("\n", i)
            i = n if i < 0 else i
        elif text.startswith("/*", i):
            i = text.find("*/", i) + 2
        elif c == '"':
            j = i + 1
            while text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i : j + 1])
            i = j + 1
        elif text.startswith("#[", i):
            depth, j = 0, i + 1
            while True:
                depth += text[j] == "["
                depth -= text[j] == "]"
                j += 1
                if depth == 0:
                    break
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def squash(text: str) -> str:
    return re.sub(r"\s+", "", text)


def block(text: str, start: int) -> str:
    """The item starting at `start`: up to its matching `}` or a top-level `;`."""
    depth, i = 0, start
    while True:
        c = text[i]
        if c == '"':
            i += 1
            while text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif c in "{([":
            depth += 1
        elif c in "})]":
            depth -= 1
            if depth == 0 and c == "}":
                return text[start : i + 1]
        elif c == ";" and depth == 0:
            return text[start : i + 1]
        i += 1


def find(text: str, selector: str):
    """Locates an item in comment-stripped text. `impl H :: fn f` selects a method."""
    if " :: " in selector:
        outer, inner = selector.split(" :: ")
        body = find(text, outer)
        return None if body is None else find(body[body.index("{") + 1 :], inner)
    kind, name = selector.split(" ", 1)
    if kind == "impl":
        tokens = re.findall(r"\w+|[^\w\s]", name)
        pattern = r"\bimpl\s+" + r"\s*".join(re.escape(t) for t in tokens) + r"\s*\{"
    else:
        pattern = rf"\b{kind}\s+{re.escape(name)}\b"
    hits = [m.start() for m in re.finditer(pattern, text)]
    return block(text, hits[0]) if len(hits) == 1 else None


def load(path: str) -> str:
    return strip((ROOT / path).read_text())


def production(paths, selector):
    found = [(p, item) for p in paths if (item := find(load(p), selector)) is not None]
    return found[0] if len(found) == 1 else (None, None)


def members(item: str):
    """Fields / variants of a struct or enum body, normalised, without visibility."""
    body = item[item.index("{") + 1 : item.rindex("}")]
    parts, depth, cur = [], 0, []
    for c in body:
        depth += c in "<([{"
        depth -= c in ">)]}"
        if c == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(c)
    parts.append("".join(cur))
    return {re.sub(r"^pub(\([a-z]+\))?", "", squash(p)) for p in parts if squash(p)}


def main() -> int:
    errors, checked = [], 0
    for table, subset in ((VERBATIM, False), (SUBSET, True)):
        for standin, prods, selectors in table:
            mine_text = load(standin)
            for sel in selectors:
                checked += 1
                mine = find(mine_text, sel)
                where, theirs = production(prods, sel)
                if mine is None:
                    errors.append(f"{standin}: `{sel}` not found (or not unique) in the stand-in")
                elif theirs is None:
                    errors.append(f"{standin}: `{sel}` not found (or not unique) in {', '.join(prods)}")
                elif subset:
                    extra = members(mine) - members(theirs)
                    if extra:
                        errors.append(
                            f"{standin}: `{sel}` has members production ({where}) lacks: {sorted(extra)}"
                        )
                elif squash(mine) != squash(theirs):
                    errors.append(f"{standin}: `{sel}` differs from production ({where})")
    for prod, sel, pinned in PINNED:
        checked += 1
        item = find(load(prod), sel)
        if item is None:
            errors.append(f"{prod}: `{sel}` not found (or not unique)")
            continue
        digest = hashlib.sha256(squash(item).encode()).hexdigest()[:16]
        if digest != pinned:
            errors.append(
                f"{prod}: `{sel}` changed (hash {digest}, pinned {pinned or 'none'}). Re-read its "
                f"stand-in against the new production code, then pin {digest} in PINNED."
            )
    for e in errors:
        print(f"check_standins: {e}", file=sys.stderr)
    print(f"check_standins: {checked} items checked, {len(errors)} mismatches")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
