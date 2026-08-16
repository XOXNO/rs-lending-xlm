#!/usr/bin/env python3
"""Static access-control gate: every contract entrypoint is gated or declared.

The Rust/Soroban equivalent of the Slither access-control script Trail of Bits
shipped with the Aave V4 audit (report appendix E). That script asserted three
properties about the Solidity configurator; this one asserts the analogous
properties about every `#[contractimpl]` entrypoint in `contracts/*`:

  1. every entrypoint is classified into exactly one authorization category;
  2. every entrypoint that can change state and is NOT governance-gated
     (owner / role / timelock) carries an explicit, justified line in
     `scripts/permissionless_entrypoints.txt`;
  3. every line in that file names a live entrypoint whose detected category
     still matches the declared one -- a stale or over-broad exception fails
     just as loudly as a missing one.

Categories, in precedence order (first match wins):

  constructor    `__constructor`; the host runs it once, at deploy.
  test-only      the whole `#[contractimpl]` block is behind
                 `#[cfg(test)]` / `#[cfg(feature = "testing")]`, so the symbol
                 does not exist in a deployable WASM (see ADR-0017 and the
                 `wasm-testing-abi-check` Make target, which verifies the
                 artifact rather than the source).
  owner          `#[only_owner]`, or the body reaches an ownership primitive
                 (`ownable::enforce_owner`, the two-step transfer/accept pair).
  role-timelock  the body reaches a role check (`access_control::ensure_role`,
                 the oracle's registered-signer check) or a timelock primitive
                 (`schedule_operation` / `set_execute_operation`).
  caller-auth    the body reaches `Address::require_auth`, i.e. the call
                 authorizes an address the CALLER supplies. Anyone may invoke
                 it; the auth only proves they control that address. This is
                 the permissionless surface INV-AUTH-03 governs, so every such
                 entrypoint needs a declared line.
  view           no authorization evidence and no reachable state write.
  UNGATED-MUTATOR
                 no authorization evidence and a reachable state write. Hard
                 failure unless declared.

Evidence is collected by a depth-limited call-graph walk over the workspace's
own Rust sources (`contracts/*/src`, `common/src`, `interfaces/*/src`), so a
guard that lives three helpers deep still counts. The walk resolves calls by
name, preferring a module-path match when the call site is path-qualified.

Fail-closed properties:

  * an entrypoint with no gate evidence is only allowed to pass as `view` when
    the walk proves it reaches no state write; anything unresolved on a write
    path (an `env.invoke_contract`, an unknown contract client, an unknown
    method on a known client) counts AS a write;
  * cross-contract calls into workspace contracts are resolved against those
    contracts' own classification via a fixpoint, so a controller entrypoint
    that only mutates through the pool is still a mutator;
  * a `#[contractimpl]` block, impl target, or function the parser cannot
    understand is an error, not a silent skip.

Scope: the deployable contracts under `contracts/*/src`. `mock/` doubles and the
`certora/*/spec` harnesses are deliberately excluded -- neither ships in a
protocol WASM -- and the walk skips `tests/` trees reached through `#[path]`
includes. A contract that grows a new crate under `contracts/` is picked up with
no change to this script.

Deterministic, no network, no build. Sources are read, never written.

    python3 scripts/check_access_control.py            # table + verdict
    python3 scripts/check_access_control.py --quiet     # violations only
    python3 scripts/check_access_control.py --json out.json

Exit 0 = every entrypoint is gated or declared; non-zero = a violation.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONTRACTS_DIR = os.path.join(REPO_ROOT, "contracts")
ALLOWLIST = os.path.join(REPO_ROOT, "scripts", "permissionless_entrypoints.txt")

# Extra crates whose functions entrypoints delegate into. Guards and storage
# writes routinely live here, so the call-graph walk has to see them.
SUPPORT_SRC_DIRS = ("common/src", "interfaces")

# Depth cap for the call-graph walk. The deepest real guard chain in this
# workspace is ~4 hops (entrypoint -> process_* -> require_* -> storage::*);
# the cap only bounds pathological cycles.
MAX_DEPTH = 12

CATEGORIES = (
    "constructor",
    "test-only",
    "owner",
    "role-timelock",
    "caller-auth",
    "view",
    "UNGATED-MUTATOR",
)

# The categories a line in the declaration file may claim. Everything else is
# either governance-controlled (owner, role-timelock), inert (view), or outside
# a deployable WASM (constructor, test-only).
DECLARABLE_CATEGORIES = frozenset({"caller-auth", "UNGATED-MUTATOR"})

# --- authorization primitives -------------------------------------------------
#
# Each pattern below is matched against a function body (comments stripped).
# Adding a new guard mechanism to the contracts means adding it here; an
# unrecognized guard shows up as UNGATED-MUTATOR, never as a silent pass.

OWNER_ATTRS = ("only_owner",)

OWNER_PATTERNS = (
    # `stellar_access::ownable`: `#[only_owner]` expands to `enforce_owner`,
    # and the two-step handover pair authenticates owner / pending owner.
    re.compile(r"\bownable::enforce_owner\s*\("),
    re.compile(r"\bownable::accept_ownership\s*\("),
    re.compile(r"\bownable::transfer_ownership\s*\("),
    re.compile(r"\bownable::renounce_ownership\s*\("),
    re.compile(r"\benforce_owner\s*\(\s*&?env"),
)

ROLE_PATTERNS = (
    re.compile(r"\baccess_control::ensure_role\s*\("),
    re.compile(r"\bensure_role\s*\("),
    # governance: renew + require_auth + ensure_role, in one helper.
    re.compile(r"\bbegin_immediate\s*\("),
    # xoxno-oracle: submissions are restricted to the registered signer set.
    re.compile(r"\brequire_registered_signer\s*\("),
)

TIMELOCK_PATTERNS = (
    re.compile(r"\bschedule_operation\s*\("),
    re.compile(r"\bset_execute_operation\s*\("),
)

CALLER_AUTH_PATTERNS = (
    re.compile(r"\.require_auth\s*\(\s*\)"),
    re.compile(r"\.require_auth_for_args\s*\("),
)

# Not a category of its own: an entrypoint can reach `require_auth` on an
# address the caller picks AND separately pin that address to the account
# owner or an approved delegate (INV-AUTH-02). Recorded as evidence and shown
# in the table so a reviewer can tell the two shapes apart, but not treated as
# a gate: reachability cannot prove the check runs on every path through a
# helper that branches on an account guard.
ACCOUNT_OWNER_PATTERNS = (
    re.compile(r"\brequire_owner_or_delegate\s*\("),
    re.compile(r"\brequire_account_owner\s*\("),
    re.compile(r"\bis_owner_or_delegate\s*\("),
)

# --- state-write primitives ---------------------------------------------------

# Chained form: `env.storage().persistent().set(..)`.
STORAGE_WRITE_CHAINED = re.compile(
    r"storage\s*\(\s*\)\s*\.\s*(?:instance|persistent|temporary)\s*\(\s*\)"
    r"\s*\.\s*(?:set|remove|update|try_update)\s*\("
)
# Bound form: `let persistent = env.storage().persistent();` then
# `persistent.set(..)`. `.extend_ttl` / `.bump` are deliberately NOT writes:
# read paths renew TTLs, and a TTL bump changes no accounting.
STORAGE_HANDLE_BINDING = re.compile(
    r"\blet\s+(?:mut\s+)?(\w+)\s*(?::[^=;]*)?=\s*[^;]*?storage\s*\(\s*\)"
    r"\s*\.\s*(?:instance|persistent|temporary)\s*\(\s*\)\s*;"
)

OTHER_WRITE_PATTERNS = (
    re.compile(r"\benv\s*\.\s*deployer\s*\(\s*\)"),
    re.compile(r"\bupdate_current_contract_wasm\s*\("),
    re.compile(r"\bupgradeable::upgrade\s*\("),
    re.compile(r"\bownable::set_owner\s*\("),
    re.compile(r"\bownable::transfer_ownership\s*\("),
    re.compile(r"\bownable::accept_ownership\s*\("),
    re.compile(r"\bownable::renounce_ownership\s*\("),
    re.compile(r"\baccess_control::(?:set_admin|grant_role|revoke_role)\w*\s*\("),
    re.compile(r"\b(?:grant_role_no_auth|revoke_role_no_auth)\s*\("),
    re.compile(r"\brole_transfer::(?:transfer_role|accept_transfer)\s*\("),
    re.compile(r"\bpausable::(?:pause|unpause)\s*\("),
    re.compile(r"\bcancel_operation\s*\("),
    re.compile(r"\bschedule_operation\s*\("),
    re.compile(r"\bset_execute_operation\s*\("),
    re.compile(r"\bset_min_delay\s*\("),
    # An arbitrary target and function symbol: assume the worst.
    re.compile(r"\benv\s*\.\s*invoke_contract\s*\("),
)

# Soroban contract clients whose target contract lives in this workspace. The
# method's own classification decides whether the call mutates, so a
# controller entrypoint that only writes through the pool is still a mutator.
WORKSPACE_CLIENTS = {
    "LiquidityPoolClient": "pool",
    "ControllerClient": "controller",
    "ControllerAdminClient": "controller",
    "PriceAggregatorClient": "price-aggregator",
    "GovernanceClient": "governance",
    "SwapAggregatorClient": "swap-aggregator",
    "XoxnoOracleClient": "xoxno-oracle",
}

# Clients for contracts outside this workspace: their read-only methods have to
# be enumerated by hand. Any method not listed here counts as a write, so a new
# external call cannot slip through as a view.
EXTERNAL_CLIENT_READS = {
    "TokenClient": frozenset(
        {"allowance", "balance", "spendable_balance", "decimals", "name", "symbol"}
    ),
    "StellarAssetClient": frozenset({"authorized", "admin"}),
    "ReflectorClient": frozenset(
        {"base", "decimals", "resolution", "lastprice", "prices", "assets", "price"}
    ),
    "RedStonePriceFeedClient": frozenset(
        {
            "read_price_data",
            "read_price_data_for_feed",
            "read_prices",
            "read_timestamp",
            "decimals",
            "base",
            "resolution",
            "assets",
        }
    ),
    "AquariusPoolClient": frozenset(
        {"a", "pool_type", "get_reserves", "get_tokens", "share_id", "get_total_shares"}
    ),
    "XoxnoOracleAdapterClient": frozenset(
        {
            "base",
            "decimals",
            "resolution",
            "assets",
            "lastprice",
            "prices",
            "max_submission_age_seconds",
        }
    ),
    # position-nft is a workspace contract, but its standard NFT surface
    # (owner_of, balance, transfer, approvals) is generated by OpenZeppelin's
    # `contracttrait` macro, so the checker never sees those bodies and cannot
    # resolve them like other workspace entrypoints. Enumerate the read-only
    # method the controller consumes. `mint` and `burn` are deliberately
    # absent: they write, and must resolve conservatively as mutations.
    "PositionNftClient": frozenset({"owner_of"}),
    # Blend's `submit` moves positions and funds; nothing here is a read.
    "BlendPoolClient": frozenset(),
}

# `token::Client` is imported under several spellings.
TOKEN_CLIENT_ALIASES = ("TokenClient", "StellarAssetClient")

# `soroban_sdk::token::Client` is the SAC client under another spelling; fold it
# onto `TokenClient` so one set of patterns covers both.
TOKEN_CLIENT_PATH = re.compile(r"\btoken\s*::\s*Client\b")

CLIENT_TYPE = re.compile(r"\b([A-Z]\w*Client)\b")
# `fn pay_out(asset: &token::Client, ..)` -- a client arrives as a parameter,
# so the type never appears in the body's statements.
CLIENT_PARAM = re.compile(r"\b(\w+)\s*:\s*&?\s*(?:mut\s+)?([A-Z]\w*Client)\b")
# `SomeClient::new(env, addr).method(` -- the chained form used across the
# controller's `external` module and the oracle providers.
CLIENT_CHAINED_CALL = re.compile(
    r"\b([A-Z]\w*Client)\s*::\s*new\s*\((?:[^()]|\([^()]*\))*\)\s*\.\s*(\w+)\s*\("
)
# `let c = SomeClient::new(..)` / `let c: SomeClient<'_> = ..`, then `c.method(`.
# The constructor has to be the whole right-hand side: in
# `let reserves = match Client::new(..).try_get_reserves() { .. }` the binding
# holds the RESULT, and attributing `reserves.len()` to the client would invent
# an unknown -- and therefore mutating -- cross-contract call.
CLIENT_BINDING = re.compile(
    r"\blet\s+(?:mut\s+)?(\w+)\s*(?::\s*([A-Z]\w*Client)\b[^=;]*?)?=\s*&?\s*"
    r"([A-Z]\w*Client)\s*::\s*new\s*\("
)
METHOD_CALL = re.compile(r"\.\s*(\w+)\s*\(")

FN_NAME = re.compile(r"\bfn\s+(\w+)")
# Modifiers that sit between an attribute list and the `fn` keyword.
FN_MODIFIERS = re.compile(
    r"(?:\bpub\s*(?:\([^)]*\)\s*)?|\bconst\s+|\basync\s+|\bunsafe\s+|\bextern\s+\"[^\"]*\"\s+)+$"
)
IMPL_HEADER = re.compile(r"\bimpl\s*(?:<[^>]*>\s*)?([^{]*?)\{", re.S)
# What an impl header may contain: type paths, generics, lifetimes, `for`. If a
# `{` goes missing the header regex swallows the rest of the file, so anything
# outside this alphabet means the parse went off the rails and the entrypoints
# below it would be silently dropped.
IMPL_TARGET_OK = re.compile(r"^[\w:<>,'&\s]+$")
# A call site: `name(` or `a::b::name(`.
CALL_SITE = re.compile(r"(?:(\w+(?:\s*::\s*\w+)*)\s*::\s*)?\b(\w+)\s*\(")
# A path reference with no call parens: a function item passed as a value,
# e.g. `run_batch(&env, entries, ops::supply::apply)`.
PATH_REFERENCE = re.compile(r"\b(\w+(?:\s*::\s*\w+)*)\s*::\s*(\w+)\b(?!\s*[(:])")
CFG_TEST_ONLY = re.compile(r'\btest\b|feature\s*=\s*"testing"')


class ParseError(RuntimeError):
    """A source shape the parser refuses to guess at."""


class AllowlistError(RuntimeError):
    """The declaration file is malformed."""


# --------------------------------------------------------------------------- #
# Rust source scanning
# --------------------------------------------------------------------------- #


def strip_comments(src: str) -> str:
    """Blank out comments, keeping every byte offset intact.

    Offsets are preserved so slices taken from the stripped text line up with
    the original file, which keeps reported line numbers honest. String
    literals are skipped so a `//` inside one survives; `'` is only treated as
    a char literal when it closes like one, so lifetimes are left alone.
    """
    out = list(src)
    i, n = 0, len(src)
    while i < n:
        ch = src[i]
        if ch == '"':
            i += 1
            while i < n:
                if src[i] == "\\":
                    i += 2
                    continue
                if src[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if ch == "'":
            # `'a` is a lifetime; `'x'` and `'\n'` are char literals.
            if i + 2 < n and src[i + 1] == "\\":
                end = src.find("'", i + 2)
                if end != -1 and end - i <= 5:
                    i = end + 1
                    continue
            elif i + 2 < n and src[i + 2] == "'":
                i += 3
                continue
            i += 1
            continue
        if ch == "/" and i + 1 < n and src[i + 1] == "/":
            while i < n and src[i] != "\n":
                out[i] = " "
                i += 1
            continue
        if ch == "/" and i + 1 < n and src[i + 1] == "*":
            depth, start = 1, i
            i += 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth += 1
                    i += 2
                elif src.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            for k in range(start, i):
                if out[k] != "\n":
                    out[k] = " "
            continue
        i += 1
    return "".join(out)


def match_block(text: str, open_at: int) -> int:
    """Return the index just past the `{...}` block that opens at `open_at`."""
    depth, i, n = 0, open_at, len(text)
    while i < n:
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise ParseError(f"unbalanced braces from offset {open_at}")


def match_parens(text: str, open_at: int) -> int:
    """Return the index just past the `(...)` group that opens at `open_at`."""
    depth, i, n = 0, open_at, len(text)
    while i < n:
        if text[i] == "(":
            depth += 1
        elif text[i] == ")":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise ParseError(f"unbalanced parens from offset {open_at}")


def line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def iter_fn_decls(text: str):
    """Yield `(name, params_open, body_open, body_end)` for every `fn` with a body.

    Generic parameter lists are matched with balanced angle brackets, not a
    naive `<...>`: `fn set_shared<V: IntoVal<Env, Val>>(..)` nests, and skipping
    such functions would drop the controller's whole persistent-storage write
    layer out of the call graph.
    """
    for m in FN_NAME.finditer(text):
        i = m.end()
        while i < len(text) and text[i] in " \t\n\r":
            i += 1
        if i < len(text) and text[i] == "<":
            depth = 0
            while i < len(text):
                if text[i] == "<":
                    depth += 1
                elif text[i] == ">":
                    depth -= 1
                    if depth == 0:
                        i += 1
                        break
                i += 1
            while i < len(text) and text[i] in " \t\n\r":
                i += 1
        if i >= len(text) or text[i] != "(":
            continue
        try:
            args_end = match_parens(text, i)
        except ParseError:
            continue
        brace = text.find("{", args_end)
        semi = text.find(";", args_end)
        if brace == -1 or (semi != -1 and semi < brace):
            # A trait method signature with no body: nothing to walk into.
            continue
        try:
            end = match_block(text, brace)
        except ParseError:
            continue
        yield m.group(1), m.start(), brace, end


def declaration_start(text: str, fn_kw: int) -> int:
    """Rewind from the `fn` keyword past `pub` / `const` / `async` modifiers.

    Attributes sit above the modifiers, not above `fn`, so `#[only_owner]` on a
    `pub fn` is invisible unless the rewind happens first.
    """
    prefix = text[:fn_kw]
    m = FN_MODIFIERS.search(prefix)
    return m.start() if m else fn_kw


def preceding_attributes(text: str, start: int) -> list[str]:
    """Collect the `#[...]` attributes immediately above offset `start`."""
    attrs: list[str] = []
    i = start
    while True:
        j = i - 1
        while j >= 0 and text[j] in " \t\n\r":
            j -= 1
        if j < 0 or text[j] != "]":
            break
        depth, k = 0, j
        while k >= 0:
            if text[k] == "]":
                depth += 1
            elif text[k] == "[":
                depth -= 1
                if depth == 0:
                    break
            k -= 1
        if k <= 0 or text[k - 1] != "#":
            break
        attrs.append(text[k - 1 : j + 1])
        i = k - 1
    attrs.reverse()
    return attrs


def iter_functions(body: str, base: int) -> list[dict]:
    """Parse the function items declared directly inside an impl block body."""
    fns: list[dict] = []
    for name, fn_kw, brace, end in iter_fn_decls(body):
        # Only items at the impl block's own nesting level; a nested closure or
        # inner `fn` sits deeper. `body` starts at the block's own `{`, so an
        # item declared directly inside it sits at depth exactly 1.
        if body.count("{", 0, fn_kw) - body.count("}", 0, fn_kw) != 1:
            continue
        decl = declaration_start(body, fn_kw)
        fns.append(
            {
                "name": name,
                "attrs": preceding_attributes(body, decl),
                "signature": body[decl:brace],
                "body": body[brace:end],
                "offset": base + fn_kw,
            }
        )
    return fns


def module_path(rel_path: str) -> str:
    """`contracts/pool/src/ops/supply.rs` -> `pool::ops::supply`."""
    parts = rel_path.replace(os.sep, "/").split("/")
    if parts[0] == "contracts":
        crate, rest = parts[1], parts[3:]
    elif parts[0] == "common":
        crate, rest = "common", parts[2:]
    elif parts[0] == "interfaces":
        crate, rest = parts[1], parts[3:]
    else:
        crate, rest = parts[0], parts[1:]
    rest = [p[:-3] if p.endswith(".rs") else p for p in rest]
    if rest and rest[-1] in ("mod", "lib"):
        rest.pop()
    return "::".join([crate] + rest)


def rust_sources() -> list[tuple[str, str]]:
    """Every workspace `.rs` file the walk may follow, as (rel_path, text)."""
    roots = [os.path.join("contracts", d, "src") for d in sorted(os.listdir(CONTRACTS_DIR))]
    roots += list(SUPPORT_SRC_DIRS)
    files: list[tuple[str, str]] = []
    for root in roots:
        abs_root = os.path.join(REPO_ROOT, root)
        if not os.path.isdir(abs_root):
            continue
        for dirpath, dirnames, filenames in os.walk(abs_root):
            dirnames.sort()
            # `tests/` trees are reachable through `#[path]` includes; they are
            # not part of a deployable contract.
            dirnames[:] = [d for d in dirnames if d not in ("tests", "target")]
            for name in sorted(filenames):
                if not name.endswith(".rs"):
                    continue
                abs_path = os.path.join(dirpath, name)
                rel = os.path.relpath(abs_path, REPO_ROOT)
                with open(abs_path, encoding="utf-8") as fh:
                    files.append((rel, strip_comments(fh.read())))
    return files


# --------------------------------------------------------------------------- #
# Call graph
# --------------------------------------------------------------------------- #


class CallGraph:
    """Name-resolved call graph over the workspace's Rust sources.

    Resolution is by function name, scoped to the calling crate plus the shared
    `common` crate, and narrowed further by module path when the call site is
    qualified (`account::require_owner_or_delegate(..)`). Crate scoping matters:
    without it a call to `upgrade` in one contract resolves into every other
    contract's `upgrade`, which manufactures guard evidence that is not there.

    When a name still resolves to several definitions the walk visits all of
    them. For guard detection that can only over-report a gate, which is why a
    detected gate is never the sole basis for a pass: the allowlist covers
    everything that is not owner/role/timelock gated.
    """

    def __init__(self, sources: list[tuple[str, str]]) -> None:
        self.by_name: dict[str, list[dict]] = {}
        for rel, text in sources:
            mod = module_path(rel)
            crate = mod.split("::")[0]
            for name, fn_kw, _brace, end in iter_fn_decls(text):
                # The node keeps its signature, so a `&token::Client` parameter
                # is visible when deciding whether the body moves funds.
                self.by_name.setdefault(name, []).append(
                    {"module": mod, "crate": crate, "file": rel, "body": text[fn_kw:end]}
                )
        self._cache: dict[tuple[int, str], list[dict]] = {}

    def callees(self, body: str, crate: str) -> list[dict]:
        """Definitions referenced from `body`, best-effort resolved.

        Both call sites (`foo::bar(..)`) and bare path references
        (`ops::supply::apply` passed as a value) count: the pool hands mutation
        legs to `run_batch` as function items, and missing those would make
        every batched mutator look read-only.
        """
        key = (id(body), crate)
        hit = self._cache.get(key)
        if hit is not None:
            return hit
        found: list[dict] = []
        seen: set[int] = set()
        matches = list(CALL_SITE.finditer(body)) + list(PATH_REFERENCE.finditer(body))
        for m in matches:
            qual, name = m.group(1), m.group(2)
            candidates = self.by_name.get(name)
            if not candidates:
                continue
            candidates = [c for c in candidates if c["crate"] in (crate, "common")]
            if not candidates:
                continue
            if qual:
                tail = qual.replace(" ", "").split("::")[-1]
                narrowed = [c for c in candidates if c["module"].split("::")[-1] == tail]
                if narrowed:
                    candidates = narrowed
            for c in candidates:
                if id(c) not in seen:
                    seen.add(id(c))
                    found.append(c)
        self._cache[key] = found
        return found

    def walk(self, body: str, crate: str, predicate) -> bool:
        """Whether `predicate(text)` holds for `body` or any transitive callee."""
        stack = [(body, 0)]
        seen: set[int] = set()
        while stack:
            text, depth = stack.pop()
            if predicate(text):
                return True
            if depth >= MAX_DEPTH:
                continue
            for c in self.callees(text, crate):
                if id(c) not in seen:
                    seen.add(id(c))
                    stack.append((c["body"], depth + 1))
        return False

    def reaches(self, body: str, crate: str, patterns) -> bool:
        """Whether `body` or anything it transitively calls matches `patterns`."""
        return self.walk(body, crate, lambda text: any(p.search(text) for p in patterns))


# --------------------------------------------------------------------------- #
# Entrypoint discovery
# --------------------------------------------------------------------------- #


def discover_entrypoints() -> list[dict]:
    """Every `#[contractimpl]` method in `contracts/*/src`, with its context."""
    entrypoints: list[dict] = []
    for contract in sorted(os.listdir(CONTRACTS_DIR)):
        src_root = os.path.join(CONTRACTS_DIR, contract, "src")
        if not os.path.isdir(src_root):
            continue
        for dirpath, dirnames, filenames in os.walk(src_root):
            dirnames.sort()
            dirnames[:] = [d for d in dirnames if d not in ("tests", "target")]
            for name in sorted(filenames):
                if not name.endswith(".rs"):
                    continue
                abs_path = os.path.join(dirpath, name)
                rel = os.path.relpath(abs_path, REPO_ROOT)
                with open(abs_path, encoding="utf-8") as fh:
                    text = strip_comments(fh.read())
                entrypoints += _entrypoints_in_file(contract, rel, text)
    return entrypoints


def _entrypoints_in_file(contract: str, rel: str, text: str) -> list[dict]:
    found: list[dict] = []
    for m in re.finditer(r"#\[\s*(?:soroban_sdk\s*::\s*)?contractimpl\s*\]", text):
        attrs = preceding_attributes(text, m.start())
        cfg = " ".join(a for a in attrs if a.startswith("#[cfg"))
        header = IMPL_HEADER.search(text, m.end())
        if header is None or header.start() > m.end() + 400:
            raise ParseError(f"{rel}:{line_of(text, m.start())}: #[contractimpl] with no impl")
        target = " ".join(header.group(1).split())
        if not IMPL_TARGET_OK.fullmatch(target):
            raise ParseError(
                f"{rel}:{line_of(text, m.start())}: cannot parse impl header {target[:60]!r}"
            )
        brace = header.end() - 1
        block_end = match_block(text, brace)
        body = text[brace:block_end]
        methods = iter_functions(body, brace)
        if not methods:
            raise ParseError(
                f"{rel}:{line_of(text, m.start())}: #[contractimpl] block exposes no entrypoints"
            )
        for fn in methods:
            fn_cfg = cfg + " " + " ".join(a for a in fn["attrs"] if a.startswith("#[cfg"))
            found.append(
                {
                    "contract": contract,
                    "impl": target,
                    "name": fn["name"],
                    "file": rel,
                    "line": line_of(text, fn["offset"]),
                    "attrs": fn["attrs"],
                    "signature": " ".join(fn["signature"].split()),
                    "body": fn["body"],
                    "cfg": fn_cfg.strip(),
                }
            )
    return found


# --------------------------------------------------------------------------- #
# Classification
# --------------------------------------------------------------------------- #


def has_attr(fn: dict, names) -> bool:
    return any(any(re.search(rf"#\[\s*{n}\s*\]", a) for a in fn["attrs"]) for n in names)


def is_test_only(fn: dict) -> bool:
    """Whether the entrypoint exists only under `cfg(test)` / `feature="testing"`."""
    return bool(fn["cfg"]) and bool(CFG_TEST_ONLY.search(fn["cfg"]))


def storage_writes(text: str) -> bool:
    """Whether `text` itself writes contract storage (TTL bumps excluded)."""
    if STORAGE_WRITE_CHAINED.search(text):
        return True
    for m in STORAGE_HANDLE_BINDING.finditer(text):
        handle = m.group(1)
        if re.search(rf"\b{re.escape(handle)}\s*\.\s*(?:set|remove|update|try_update)\s*\(", text):
            return True
    return any(p.search(text) for p in OTHER_WRITE_PATTERNS)


def client_calls(text: str) -> list[tuple[str, str | None]]:
    """(client type, method) pairs called in `text`; method None when unknown."""
    text = TOKEN_CLIENT_PATH.sub("TokenClient", text)
    calls: list[tuple[str, str | None]] = []
    resolved: set[str] = set()
    for m in CLIENT_PARAM.finditer(text):
        var, client = m.group(1), m.group(2)
        resolved.add(client)
        methods = re.findall(rf"\b{re.escape(var)}\s*\.\s*(\w+)\s*\(", text)
        calls += [(client, meth) for meth in methods] or [(client, None)]
    for m in CLIENT_CHAINED_CALL.finditer(text):
        calls.append((m.group(1), m.group(2)))
        resolved.add(m.group(1))
    for m in CLIENT_BINDING.finditer(text):
        var, client = m.group(1), m.group(2) or m.group(3)
        try:
            after = match_parens(text, m.end() - 1)
        except ParseError:
            continue
        # `let x = Client::new(..).method(..);` binds the RESULT, not a client;
        # the chained pattern above already recorded that call, and treating
        # `x` as a client would invent unknown methods on it.
        if text[after:].lstrip()[:1] != ";":
            continue
        resolved.add(client)
        methods = re.findall(rf"\b{re.escape(var)}\s*\.\s*(\w+)\s*\(", text)
        if methods:
            calls += [(client, meth) for meth in methods]
        else:
            calls.append((client, None))
    # A client type mentioned with no call we could tie to it: assume the worst
    # rather than assume it is inert.
    for m in CLIENT_TYPE.finditer(text):
        if m.group(1) not in resolved:
            calls.append((m.group(1), None))
            resolved.add(m.group(1))
    return calls


def make_write_predicate(mutating: dict[tuple[str, str], bool]):
    """Build `text -> bool` for "this body changes on-chain state".

    `mutating` maps (contract, entrypoint) to the current belief about whether
    that workspace entrypoint mutates; it is refined by the fixpoint in
    `classify`.
    """

    def predicate(text: str) -> bool:
        if storage_writes(text):
            return True
        for client, method in client_calls(text):
            name = method[4:] if method and method.startswith("try_") else method
            if client in WORKSPACE_CLIENTS:
                if name is None:
                    return True
                key = (WORKSPACE_CLIENTS[client], name)
                # Unknown method on a known contract: fail closed.
                if mutating.get(key, True):
                    return True
                continue
            reads = EXTERNAL_CLIENT_READS.get(client)
            if reads is None:
                # `token::Client` is spelled several ways; anything else is an
                # external contract we have no method table for.
                if client in TOKEN_CLIENT_ALIASES:
                    reads = EXTERNAL_CLIENT_READS["TokenClient"]
                else:
                    return True
            if name is None or name not in reads:
                return True
        return False

    return predicate


def classify(entrypoints: list[dict], graph: CallGraph) -> None:
    """Assign `category`, `evidence`, and `mutates` to every entrypoint."""
    for fn in entrypoints:
        body, crate = fn["body"], fn["contract"]
        evidence: list[str] = []
        if has_attr(fn, OWNER_ATTRS):
            evidence.append("#[only_owner]")
            fn["gate"] = "owner"
        elif graph.reaches(body, crate, OWNER_PATTERNS):
            evidence.append("ownable primitive")
            fn["gate"] = "owner"
        elif graph.reaches(body, crate, ROLE_PATTERNS):
            evidence.append("role check")
            fn["gate"] = "role-timelock"
        elif graph.reaches(body, crate, TIMELOCK_PATTERNS):
            evidence.append("timelock operation")
            fn["gate"] = "role-timelock"
        elif graph.reaches(body, crate, CALLER_AUTH_PATTERNS):
            evidence.append("require_auth")
            fn["gate"] = "caller-auth"
        else:
            fn["gate"] = None
        if fn["gate"] in (None, "caller-auth") and graph.reaches(
            body, crate, ACCOUNT_OWNER_PATTERNS
        ):
            evidence.append("account owner/delegate")
        if has_attr(fn, ("when_not_paused",)):
            # A liveness switch, not an authorization gate: it says WHEN a call
            # is allowed, never WHO may make it.
            evidence.append("#[when_not_paused] (not a gate)")
        fn["evidence"] = evidence

    # Cross-contract calls resolve against the workspace's own classification.
    # Start from "everything mutates" and let the set of proven-read-only
    # entrypoints grow until it stops changing: growth only ever follows from
    # resolved evidence, so the result stays conservative.
    mutating = {(fn["contract"], fn["name"]): True for fn in entrypoints}
    for _ in range(len(WORKSPACE_CLIENTS) + 2):
        predicate = make_write_predicate(mutating)
        updated = {
            (fn["contract"], fn["name"]): graph.walk(fn["body"], fn["contract"], predicate)
            for fn in entrypoints
        }
        if updated == mutating:
            break
        mutating = updated
    else:
        raise ParseError("cross-contract write analysis did not converge")

    for fn in entrypoints:
        fn["mutates"] = mutating[(fn["contract"], fn["name"])]
        if fn["name"] == "__constructor":
            fn["category"] = "constructor"
        elif is_test_only(fn):
            fn["category"] = "test-only"
        elif fn["gate"]:
            fn["category"] = fn["gate"]
        elif fn["mutates"]:
            fn["category"] = "UNGATED-MUTATOR"
        else:
            fn["category"] = "view"
        if fn["category"] not in CATEGORIES:
            raise ParseError(f"unknown category for {fn['contract']}::{fn['name']}")


# --------------------------------------------------------------------------- #
# Declaration file
# --------------------------------------------------------------------------- #


def load_allowlist(path: str) -> dict[str, dict]:
    """Parse `contract::function | category | invariants | justification`."""
    if not os.path.exists(path):
        raise AllowlistError(f"declaration file missing: {path}")
    entries: dict[str, dict] = {}
    with open(path, encoding="utf-8") as fh:
        for lineno, raw in enumerate(fh, 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            fields = [f.strip() for f in line.split("|")]
            if len(fields) != 4:
                raise AllowlistError(
                    f"{path}:{lineno}: expected 4 '|'-separated fields, got {len(fields)}"
                )
            key, category, invariants, justification = fields
            if not re.fullmatch(r"[a-z0-9-]+::\w+", key):
                raise AllowlistError(f"{path}:{lineno}: bad entrypoint key {key!r}")
            if category not in DECLARABLE_CATEGORIES:
                raise AllowlistError(
                    f"{path}:{lineno}: category {category!r} is not declarable "
                    f"(expected one of {sorted(DECLARABLE_CATEGORIES)})"
                )
            refs = [r.strip() for r in invariants.split(",") if r.strip()]
            if not refs or not all(re.fullmatch(r"INV-[A-Z]+-\d+", r) for r in refs):
                raise AllowlistError(
                    f"{path}:{lineno}: invariant field must be a comma-separated list "
                    f"of INV-XXX-NN references, got {invariants!r}"
                )
            if len(justification) < 24:
                raise AllowlistError(f"{path}:{lineno}: justification too short to be useful")
            if key in entries:
                raise AllowlistError(f"{path}:{lineno}: duplicate entry for {key}")
            entries[key] = {
                "line": lineno,
                "category": category,
                "invariants": refs,
                "justification": justification,
            }
    return entries


def check(entrypoints: list[dict], declared: dict[str, dict]) -> list[str]:
    """Return one message per violation; empty means the tree is clean."""
    violations: list[str] = []
    seen: set[str] = set()

    for fn in sorted(entrypoints, key=lambda f: (f["contract"], f["name"])):
        key = f"{fn['contract']}::{fn['name']}"
        seen.add(key)
        entry = declared.get(key)
        needs_declaration = fn["category"] in DECLARABLE_CATEGORIES

        if needs_declaration and entry is None:
            violations.append(
                f"{key} is {fn['category']} but is not declared in "
                f"{os.path.relpath(ALLOWLIST, REPO_ROOT)}\n"
                f"    {fn['file']}:{fn['line']}  {fn['signature'].strip()}\n"
                f"    evidence: {', '.join(fn['evidence']) or 'none'}\n"
                f"    fix: gate it, or add a justified line to the declaration file."
            )
        elif not needs_declaration and entry is not None:
            violations.append(
                f"{key} is declared permissionless but classifies as "
                f"{fn['category']} ({os.path.relpath(ALLOWLIST, REPO_ROOT)}:{entry['line']})\n"
                f"    fix: drop the stale declaration."
            )
        elif entry is not None and entry["category"] != fn["category"]:
            violations.append(
                f"{key} is declared as {entry['category']} but classifies as "
                f"{fn['category']} ({os.path.relpath(ALLOWLIST, REPO_ROOT)}:{entry['line']})\n"
                f"    fix: reconcile the declaration with the code."
            )

        if fn["category"] == "test-only" and fn["mutates"]:
            # Belt to `wasm-testing-abi-check`'s braces: that target proves the
            # symbol is absent from the artifact, this one proves the source
            # still confines it to a test cfg.
            if not CFG_TEST_ONLY.search(fn["cfg"]):
                violations.append(f"{key}: test-only mutator with no test cfg: {fn['cfg']!r}")

    for key, entry in sorted(declared.items()):
        if key not in seen:
            violations.append(
                f"{key} is declared in {os.path.relpath(ALLOWLIST, REPO_ROOT)}:{entry['line']} "
                f"but no such entrypoint exists\n    fix: drop the stale declaration."
            )
    return violations


# --------------------------------------------------------------------------- #
# Reporting
# --------------------------------------------------------------------------- #


def print_table(entrypoints: list[dict]) -> None:
    rows = sorted(entrypoints, key=lambda f: (f["contract"], f["category"], f["name"]))
    width_c = max(len(f["contract"]) for f in rows)
    width_n = max(len(f["name"]) for f in rows)
    width_k = max(len(f["category"]) for f in rows)
    current = None
    for fn in rows:
        if fn["contract"] != current:
            current = fn["contract"]
            print(f"\n{current}")
        state = "mut " if fn["mutates"] else "view"
        print(
            f"  {fn['contract']:<{width_c}}  {fn['name']:<{width_n}}  "
            f"{fn['category']:<{width_k}}  {state}  {', '.join(fn['evidence']) or '-'}"
        )


def summarize(entrypoints: list[dict]) -> dict[str, int]:
    counts = {c: 0 for c in CATEGORIES}
    for fn in entrypoints:
        counts[fn["category"]] += 1
    return counts


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Assert every Soroban entrypoint is access-gated or declared permissionless.",
    )
    parser.add_argument("--quiet", action="store_true", help="print violations only")
    parser.add_argument("--json", metavar="PATH", help="write the full classification as JSON")
    args = parser.parse_args()

    try:
        entrypoints = discover_entrypoints()
        if not entrypoints:
            print("FAIL: no #[contractimpl] entrypoints found -- the parser is broken")
            return 2
        graph = CallGraph(rust_sources())
        classify(entrypoints, graph)
        declared = load_allowlist(ALLOWLIST)
    except (ParseError, AllowlistError, OSError) as exc:
        print(f"FAIL: {exc}")
        return 2

    violations = check(entrypoints, declared)

    if args.json:
        payload = [
            {
                k: fn[k]
                for k in ("contract", "name", "impl", "file", "line", "category", "evidence")
            }
            | {"mutates": fn["mutates"], "signature": fn["signature"]}
            for fn in sorted(entrypoints, key=lambda f: (f["contract"], f["name"]))
        ]
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump({"entrypoints": payload, "violations": violations}, fh, indent=2)
            fh.write("\n")

    if not args.quiet:
        print_table(entrypoints)
        counts = summarize(entrypoints)
        print("\n" + "  ".join(f"{k}={v}" for k, v in counts.items() if v))
        print(f"declared permissionless: {len(declared)}")

    if violations:
        print(f"\nFAIL: {len(violations)} access-control violation(s)\n")
        for v in violations:
            print(f"  - {v}")
        return 1

    print(f"\nOK   {len(entrypoints)} entrypoints: all gated or declared permissionless")
    return 0


if __name__ == "__main__":
    sys.exit(main())
