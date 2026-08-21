#!/usr/bin/env python3
"""Read-only pre-deploy gate: audit every LIVE spoke supply/borrow cap.

Spoke caps carry no "unlimited" sentinel. A cap is always an enforced ceiling,
`0` means the market accepts nothing on that side, and `i128::MAX` is rejected
at config time (`common/src/validation.rs::require_cap_within_asset_domain`).

Enforcement routes the stored cap through `Ray::from_asset(cap, decimals)`
(`contracts/controller/src/spoke/caps.rs::cap_to_scaled`), which upscales by
`10^(27 - asset_decimals)` via `checked_mul(..).expect(..)`. With
`overflow-checks = true` a stored cap above `i128::MAX / 10^(27 - decimals)`
therefore PANICS, bricking every supply and borrow for that spoke asset. (The
later division by the supply index saturates, so it is not a further bound.) A
cap
that predates the current validation -- set directly through governance, or
written by an older build that exempted `i128::MAX` from enforcement -- is only
visible in LIVE state, never in the checked-in JSON. Hence this tool.

Checks, per spoke asset, against live on-chain state:
  1. no cap equals `i128::MAX`                          (deploy blocker)
  2. every cap <= `i128::MAX / 10^(27 - asset_decimals)` (deploy blocker)
  3. every cap of `0` is intentional -- cross-checked against the asset's
     collateral/borrow flags and the checked-in JSON            (review item)
Plus: any divergence between live state and `configs/<network>/spokes.json`.

Config spoke ids are NOT on-chain spoke ids. `add_spoke` takes no id argument --
it auto-increments -- so any spoke the deploy skips (`"enabled": false`) shifts
every later spoke down by one. The deploy script records the translation in
`configs/networks.json.<network>.spoke_ids`, and this tool queries through it:
config keys select the expected config, mapped on-chain ids address the
contract. With no map recorded the two are assumed to coincide, which is what a
deployment that never skipped a spoke produces.

Everything here is a simulated read (`--send=no`). No transaction is built,
signed, or submitted, and no signing identity is touched.

Run BEFORE any controller upgrade or spoke reconfiguration:
    python3 scripts/verify_spoke_caps.py --network mainnet
    python3 scripts/verify_spoke_caps.py --network all --json out.json

Override the endpoint or address when networks.json lags the deployment:
    python3 scripts/verify_spoke_caps.py --network mainnet \
        --rpc-url https://mainnet.sorobanrpc.com --controller C...

`--config-audit` runs checks 1 and 2 against the checked-in JSON with no network
access at all. It is a lint of what a deploy WOULD write, never evidence about
what is stored on-chain, and it is labelled as such in the output.

Exit 0 = every live cap is safe; non-zero = a blocker or a failed query.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONFIGS = os.path.join(REPO_ROOT, "configs")

I128_MAX = (1 << 127) - 1
RAY_DECIMALS = 27
RAY = 10 ** RAY_DECIMALS

SUPPLY_INDEX_FLOOR_RAW = RAY // 1_000

READONLY_SOURCE = "GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV"

ERR_ASSET_NOT_IN_SPOKE = 307
ERR_SPOKE_NOT_FOUND = 300

SPOKE_PROBE_MARGIN = 3

class QueryError(RuntimeError):
    """A read query failed for a reason other than a clean contract error."""

def cap_ceiling(asset_decimals: int) -> int:
    """Largest cap `Ray::from_asset` can rescale without overflowing i128.

    Mirrors `common::validation::max_cap_for_decimals`. The subsequent division
    by the supply index in `cap_to_scaled` saturates rather than panicking, so
    only the `10^(27 - decimals)` upscale needs bounding here.
    """
    if asset_decimals > RAY_DECIMALS:
        raise ValueError(f"asset_decimals {asset_decimals} exceeds RAY_DECIMALS")
    return I128_MAX // 10 ** (RAY_DECIMALS - asset_decimals)

def invoke(contract, rpc, passphrase, fn, args, timeout=120):
    """Simulate a view call. Returns (parsed_json, contract_error_code)."""
    cmd = [
        "stellar", "contract", "invoke",
        "--id", contract,
        "--source-account", READONLY_SOURCE,
        "--rpc-url", rpc,
        "--network-passphrase", passphrase,
        "--send=no",
        "--",
        fn,
    ]
    for key, value in args.items():
        cmd += [f"--{key}", value]

    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except FileNotFoundError as exc:
        raise QueryError("`stellar` CLI not found on PATH") from exc
    except subprocess.TimeoutExpired as exc:
        raise QueryError(f"{fn} timed out after {timeout}s") from exc

    if out.returncode != 0 or not out.stdout.strip():
        stderr = out.stderr.strip()
        code = parse_contract_error(stderr)
        if code is not None:
            return None, code
        raise QueryError(f"{fn} failed: {stderr or 'no output'}")

    try:
        return json.loads(out.stdout.strip()), None
    except json.JSONDecodeError as exc:
        raise QueryError(f"{fn} returned non-JSON: {out.stdout.strip()[:200]}") from exc

def parse_contract_error(stderr: str):
    """Extract N from `HostError: Error(Contract, #N)`, else None."""
    marker = "Error(Contract, #"
    idx = stderr.find(marker)
    if idx < 0:
        return None
    tail = stderr[idx + len(marker):]
    digits = ""
    for ch in tail:
        if ch.isdigit():
            digits += ch
        else:
            break
    return int(digits) if digits else None

def load_json(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)

def load_network(network):
    networks = load_json(os.path.join(CONFIGS, "networks.json"))
    if network not in networks:
        sys.exit(f"unknown network '{network}' in configs/networks.json")
    net = networks[network]
    if not net.get("controller"):
        return None, net
    return net["controller"], net

def load_markets(network):
    """asset name -> {hub_id, address, config_decimals}."""
    raw = load_json(os.path.join(CONFIGS, network, "markets.json"))
    markets = {}
    for entry in raw.get("markets", []):
        markets[entry["name"]] = {
            "hub_id": entry["hub_id"],
            "address": entry["asset_address"],
            "config_decimals": entry.get("oracle", {}).get("asset_decimals"),
        }
    return markets

def load_spokes(network):
    """spoke id (int) -> {name, assets: {asset name -> config}}."""
    raw = load_json(os.path.join(CONFIGS, network, "spokes.json"))
    spokes = {}
    for spoke_id, spoke in raw.items():
        if spoke_id.startswith("_"):
            continue
        spokes[int(spoke_id)] = {
            "name": spoke.get("name", ""),
            "enabled": spoke.get("enabled", True) is not False,
            "assets": spoke.get("assets", {}) or {},
        }
    return spokes


def load_spoke_id_map(network):
    """config spoke id (int) -> on-chain spoke id (int), from networks.json.

    `add_spoke` takes no id: it auto-increments, so on-chain ids are assigned
    purely by deploy call order. Any spoke the deploy loop skips (`"enabled":
    false`) permanently shifts every later spoke down, and the deploy script
    records the resulting translation in `networks.json.<network>.spoke_ids`.
    Reading a config key as an on-chain id is therefore only correct when
    nothing was ever skipped.

    An empty or absent map means the deploy has not written one yet; callers
    fall back to identity, which is what a never-skipped deployment produces.
    """
    networks = load_json(os.path.join(CONFIGS, "networks.json"))
    raw = (networks.get(network) or {}).get("spoke_ids") or {}
    return {int(config_id): int(onchain_id) for config_id, onchain_id in raw.items()}

def cfg_cap(asset_cfg, key):
    """Checked-in cap as int, or None when the key is absent entirely.

    A missing key is itself a divergence: the deploy script has no cap to send,
    so whatever is live was never expressed in the repo.
    """
    if key not in asset_cfg:
        return None
    return int(asset_cfg[key])

class Auditor:
    def __init__(self, network, verbose=True, rpc_url=None, controller=None):
        self.network = network
        self.verbose = verbose
        self.controller, self.net = load_network(network)
        if rpc_url:
            self.net = dict(self.net, rpc_url=rpc_url)
        if controller:
            self.controller = controller
        self.divergences = []
        self.markets = load_markets(network)
        self.spokes = load_spokes(network)
        self.spoke_ids = load_spoke_id_map(network)
        # No recorded map: the deploy never skipped a spoke, so config ids and
        # on-chain ids coincide. Probing config ids directly is correct here.
        self.identity_ids = not self.spoke_ids
        self.config_by_onchain = {v: k for k, v in self.spoke_ids.items()}
        if len(self.config_by_onchain) != len(self.spoke_ids):
            self.divergences.append(
                f"configs/networks.json {network}.spoke_ids maps two config ids to the "
                "same on-chain id; the map is corrupt and this audit cannot be trusted"
            )
        self.pool = None
        self.decimals_cache = {}
        self.rows = []
        self.query_failures = []

    def log(self, message):
        if self.verbose:
            print(message, flush=True)

    def call(self, contract, fn, args):
        return invoke(contract, self.net["rpc_url"], self.net["network_passphrase"], fn, args)

    def hub_asset_arg(self, hub_id, address):
        return json.dumps({"asset": address, "hub_id": hub_id}, separators=(",", ":"))

    def live_decimals(self, hub_id, address):
        """asset_decimals straight off the live pool market params."""
        key = (hub_id, address)
        if key in self.decimals_cache:
            return self.decimals_cache[key]
        data, err = self.call(
            self.pool, "get_sync_data", {"hub_asset": self.hub_asset_arg(hub_id, address)}
        )
        value = None if err is not None else data["params"]["asset_decimals"]
        self.decimals_cache[key] = value
        return value

    def config_for(self, onchain_id):
        """Config spoke id that `onchain_id` belongs to, or None if unmapped."""
        if self.identity_ids:
            return onchain_id if onchain_id in self.spokes else None
        return self.config_by_onchain.get(onchain_id)

    def label(self, config_id, onchain_id):
        """`spoke N` when the ids coincide, `spoke N (on-chain M)` when they do not."""
        if config_id is None:
            return f"on-chain spoke {onchain_id}"
        if config_id == onchain_id:
            return f"spoke {config_id}"
        return f"spoke {config_id} (on-chain {onchain_id})"

    def discover_spokes(self):
        """Live spokes as (config_id, onchain_id) pairs, probed on-chain.

        Probes ON-CHAIN ids, never config keys: with a recorded `spoke_ids` map
        the two differ for every spoke after the first skipped one. The margin
        beyond the highest known id is what surfaces a spoke that exists
        on-chain but was never written back to the repo.
        """
        known_onchain = set(self.spoke_ids.values()) or set(self.spokes)
        highest = max(known_onchain) if known_onchain else 0
        candidates = sorted(known_onchain | set(range(1, highest + SPOKE_PROBE_MARGIN + 1)))

        live = []
        for onchain_id in candidates:
            try:
                data, err = self.call(self.controller, "get_spoke", {"spoke_id": str(onchain_id)})
            except QueryError as exc:
                self.query_failures.append(f"get_spoke({onchain_id}): {exc}")
                continue
            if err == ERR_SPOKE_NOT_FOUND:
                continue
            if err is not None:
                self.query_failures.append(f"get_spoke({onchain_id}): contract error #{err}")
                continue

            config_id = self.config_for(onchain_id)
            live.append((config_id, onchain_id))
            if config_id is None:
                self.divergences.append(
                    f"on-chain spoke {onchain_id} is not mapped to any spoke in "
                    f"configs/{self.network}/spokes.json"
                    + ("" if self.identity_ids else
                       f" by configs/networks.json {self.network}.spoke_ids")
                    + f" (live config: {json.dumps(data)})"
                )
            elif not self.spokes.get(config_id, {}).get("enabled", True):
                self.divergences.append(
                    f"{self.label(config_id, onchain_id)}: deployed on-chain but marked "
                    f'"enabled": false in configs/{self.network}/spokes.json'
                )
        return live

    def audit(self):
        if not self.controller:
            self.query_failures.append(
                f"configs/networks.json has no controller address for '{self.network}' -- "
                "nothing is deployed to query"
            )
            return False

        try:
            pool, err = self.call(self.controller, "get_pool_address", {})
        except QueryError as exc:
            self.query_failures.append(f"get_pool_address: {exc}")
            return False
        if err is not None:
            self.query_failures.append(f"get_pool_address: contract error #{err}")
            return False
        self.pool = pool
        self.log(f"controller {self.controller}")
        self.log(f"pool       {self.pool}")

        if self.identity_ids:
            self.log(
                f"spoke id map: none recorded in configs/networks.json for '{self.network}'; "
                "assuming config ids == on-chain ids"
            )
        else:
            self.log(
                "spoke id map: "
                + ", ".join(f"{c}->{o}" for c, o in sorted(self.spoke_ids.items()))
            )

        for config_id, onchain_id in self.discover_spokes():
            self.audit_spoke(config_id, onchain_id)

        live_ids = {row["spoke_id"] for row in self.rows}
        for spoke_id, spoke in self.spokes.items():
            if not spoke.get("enabled", True):
                # Deliberately not deployed: it has no on-chain id to diverge from.
                self.log(f"spoke {spoke_id} ({spoke['name']}): enabled=false, not deployed")
                continue
            probe_id = self.spoke_ids.get(spoke_id, spoke_id if self.identity_ids else None)
            if probe_id is None:
                self.divergences.append(
                    f"spoke {spoke_id} is enabled in configs/{self.network}/spokes.json but "
                    f"configs/networks.json {self.network}.spoke_ids has no on-chain id for "
                    "it -- it was never deployed, or the map was not written back"
                )
                continue
            if spoke_id not in live_ids and not any(
                f"get_spoke({probe_id})" in failure for failure in self.query_failures
            ):
                self.divergences.append(
                    f"spoke {spoke_id} is in configs/{self.network}/spokes.json but has no "
                    "live assets on-chain"
                )
        return True

    def audit_spoke(self, config_id, onchain_id):
        """Probe EVERY known market against this spoke, not just configured ones.

        An asset added on-chain but never written back to spokes.json is exactly
        the drift this gate exists to surface.

        `onchain_id` addresses the contract; `config_id` selects the checked-in
        config to compare against. They differ whenever a skipped spoke shifted
        the auto-incremented ids.
        """
        cfg_assets = self.spokes.get(config_id, {}).get("assets", {})
        where = self.label(config_id, onchain_id)
        seen = set()

        for name, market in self.markets.items():
            try:
                live, err = self.call(
                    self.controller,
                    "get_spoke_asset",
                    {
                        "spoke_id": str(onchain_id),
                        "hub_asset": self.hub_asset_arg(market["hub_id"], market["address"]),
                    },
                )
            except QueryError as exc:
                self.query_failures.append(f"get_spoke_asset({onchain_id}, {name}): {exc}")
                continue

            if err == ERR_ASSET_NOT_IN_SPOKE:
                if name in cfg_assets:
                    self.divergences.append(
                        f"{where} / {name}: present in configs/{self.network}/"
                        "spokes.json but NOT on-chain"
                    )
                continue
            if err is not None:
                self.query_failures.append(
                    f"get_spoke_asset({onchain_id}, {name}): contract error #{err}"
                )
                continue

            seen.add(name)
            if name not in cfg_assets:
                self.divergences.append(
                    f"{where} / {name}: live on-chain but NOT in "
                    f"configs/{self.network}/spokes.json"
                )
            self.rows.append(
                self.evaluate(config_id, onchain_id, name, market, live, cfg_assets.get(name, {}))
            )

        for name in cfg_assets:
            if name not in seen and name not in self.markets:
                self.divergences.append(
                    f"{where} / {name}: in spokes.json but has no entry in "
                    f"configs/{self.network}/markets.json -- cannot resolve an address"
                )

    def evaluate(self, config_id, onchain_id, name, market, live, cfg):
        decimals = self.live_decimals(market["hub_id"], market["address"])
        decimals_source = "live"
        if decimals is None:
            decimals = market["config_decimals"]
            decimals_source = "config (live pool query failed)"
            self.query_failures.append(
                f"get_sync_data({name}): could not read live asset_decimals; "
                "fell back to markets.json"
            )

        supply_cap = int(live["supply_cap"])
        borrow_cap = int(live["borrow_cap"])
        ceiling = cap_ceiling(decimals) if decimals is not None else None

        row = {
            "spoke_id": config_id,
            "onchain_spoke_id": onchain_id,
            "spoke_name": self.spokes.get(config_id, {}).get("name", ""),
            "asset": name,
            "hub_id": market["hub_id"],
            "address": market["address"],
            "decimals": decimals,
            "decimals_source": decimals_source,
            "cap_ceiling": ceiling,
            "supply_cap": supply_cap,
            "borrow_cap": borrow_cap,
            "is_collateralizable": live["is_collateralizable"],
            "is_borrowable": live["is_borrowable"],
            "paused": live["paused"],
            "frozen": live["frozen"],
            "blockers": [],
            "warnings": [],
        }

        for side, cap, flag_name, flag in (
            ("supply", supply_cap, "is_collateralizable", live["is_collateralizable"]),
            ("borrow", borrow_cap, "is_borrowable", live["is_borrowable"]),
        ):

            if cap == I128_MAX:
                row["blockers"].append(
                    f"{side}_cap == i128::MAX; enforcement will panic in "
                    "Ray::from_asset and brick this market"
                )

            elif ceiling is not None and cap > ceiling:
                row["blockers"].append(
                    f"{side}_cap {cap} exceeds the {decimals}-decimal ceiling {ceiling}; "
                    "Ray::from_asset will overflow and panic"
                )

            if cap == 0:
                if flag:
                    row["warnings"].append(
                        f"{side}_cap == 0 while {flag_name} is ENABLED -- this market is "
                        f"silently dead on the {side} side"
                    )
                else:
                    row["warnings"].append(
                        f"{side}_cap == 0 and {flag_name} is disabled (consistent)"
                    )
            if cap < 0:
                row["blockers"].append(f"{side}_cap is negative ({cap})")

        self.compare_config(row, cfg, config_id, onchain_id, name)
        return row

    def compare_config(self, row, cfg, config_id, onchain_id, name):
        if not cfg:
            return
        for side, key in (("supply", "supply_cap"), ("borrow", "borrow_cap")):
            expected = cfg_cap(cfg, key)
            live = row[f"{side}_cap"]
            if expected is None:
                self.divergences.append(
                    f"{self.label(config_id, onchain_id)} / {name}: configs/{self.network}/"
                    f"spokes.json omits '{key}' entirely; live value is {live}"
                )
            elif expected != live:
                self.divergences.append(
                    f"{self.label(config_id, onchain_id)} / {name}: {key} "
                    f"live={live} config={expected}"
                )
        for flag, key in (
            ("is_collateralizable", "can_be_collateral"),
            ("is_borrowable", "can_be_borrowed"),
        ):
            if key in cfg and bool(cfg[key]) != bool(row[flag]):
                self.divergences.append(
                    f"{self.label(config_id, onchain_id)} / {name}: {flag} "
                    f"live={row[flag]} config={cfg[key]}"
                )

def scan_config_events(auditor, lookback=119000):
    """Cross-check the asset set against live `config/spoke_asset` events.

    The probe loop above is driven by markets.json, so an asset configured
    on-chain whose address never made it into that file would be invisible to
    it. Events are an independent witness: `UpdateSpokeAssetEvent` carries the
    asset, spoke_id and hub_id of every add/edit.

    This only narrows the gap, it does not close it. Soroban RPC retains roughly
    120k ledgers (~7 days) of events, and there is no RPC method that enumerates
    every ledger entry a contract owns -- `getLedgerEntries` is key-based. An
    asset configured before the retention floor, with an address absent from
    markets.json, remains undetectable with this tooling.
    """
    rpc = auditor.net["rpc_url"]
    try:
        latest = subprocess.run(
            ["curl", "-s", "-m", "30", "-X", "POST", rpc,
             "-H", "Content-Type: application/json",
             "-d", '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}'],
            capture_output=True, text=True, timeout=60,
        )
        sequence = json.loads(latest.stdout)["result"]["sequence"]
    except Exception as exc:
        auditor.query_failures.append(f"event scan: could not read latest ledger ({exc})")
        return

    start = max(1, sequence - lookback)
    cmd = [
        "stellar", "events",
        "--rpc-url", rpc,
        "--network-passphrase", auditor.net["network_passphrase"],
        "--id", auditor.controller,
        "--start-ledger", str(start),
        "--count", "200",
        "--output", "json",
    ]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    except Exception as exc:
        auditor.query_failures.append(f"event scan: {exc}")
        return
    if out.returncode != 0:
        auditor.query_failures.append(f"event scan: {out.stderr.strip()[:200]}")
        return

    known = {market["address"] for market in auditor.markets.values()}
    events = []
    for line in out.stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue

    config_events = [e for e in events if "Spoke" in (e.get("event_name") or "")]
    auditor.log(
        f"event scan: ledgers {start}-{sequence}, {len(events)} events, "
        f"{len(config_events)} spoke-config events"
    )
    for event in config_events:
        params = event.get("params", {})
        asset = params.get("asset")
        if asset and asset not in known:
            auditor.divergences.append(
                f"event {event.get('event_name')} at ledger {event.get('ledger')} configures "
                f"asset {asset}, which is absent from configs/{auditor.network}/markets.json -- "
                "the probe loop could not have covered it"
            )

def config_audit(network):
    """Static lint of the checked-in JSON. NOT evidence about on-chain state.

    Catches a cap that a deploy would refuse to write (`i128::MAX`, above the
    asset's domain ceiling) or that the deploy script has no value for at all,
    before anyone submits the transaction.
    """
    markets = load_markets(network)
    spokes = load_spokes(network)

    print(f"\n=== {network}: CHECKED-IN CONFIG ONLY (no network access; NOT live state) ===")
    problems = []
    checked = 0
    for spoke_id in sorted(spokes):
        for name, cfg in sorted(spokes[spoke_id]["assets"].items()):
            market = markets.get(name)
            if market is None:
                problems.append(
                    f"spoke {spoke_id} / {name}: no entry in configs/{network}/markets.json"
                )
                continue
            decimals = market["config_decimals"]
            if decimals is None:
                problems.append(f"spoke {spoke_id} / {name}: markets.json has no asset_decimals")
                continue
            ceiling = cap_ceiling(decimals)
            for key in ("supply_cap", "borrow_cap"):
                cap = cfg_cap(cfg, key)
                if cap is None:
                    problems.append(
                        f"spoke {spoke_id} / {name}: '{key}' is absent from spokes.json"
                    )
                    continue
                checked += 1
                if cap == I128_MAX:
                    problems.append(f"spoke {spoke_id} / {name}: {key} == i128::MAX")
                elif cap > ceiling:
                    problems.append(
                        f"spoke {spoke_id} / {name}: {key} {cap} exceeds the {decimals}-decimal "
                        f"ceiling {ceiling}"
                    )
                elif cap < 0:
                    problems.append(f"spoke {spoke_id} / {name}: {key} is negative ({cap})")

    print(f"checked {checked} cap values across {len(spokes)} spokes")
    for msg in problems:
        print(f"  - {msg}")
    ok = not problems
    print(f"{network} config audit: {'PASS' if ok else 'FAIL'} (config only, not live)")
    return ok

def row_label(row):
    """`spoke N`, or `spoke N (on-chain M)` when a skipped spoke shifted the ids."""
    config_id, onchain_id = row["spoke_id"], row["onchain_spoke_id"]
    if config_id is None:
        return f"on-chain spoke {onchain_id}"
    if config_id == onchain_id:
        return f"spoke {config_id}"
    return f"spoke {config_id} (on-chain {onchain_id})"

def render(auditor):
    print(f"\n=== {auditor.network} ===")
    if not auditor.rows:
        print("no live spoke assets read")
    else:
        header = (
            f"{'spoke':>5} {'chain':>5} {'asset':<14} {'hub':>3} {'dec':>3} "
            f"{'supply_cap':>26} {'borrow_cap':>26} {'coll':>5} {'borr':>5}  verdict"
        )
        print(header)
        print("-" * len(header))
        # Sort by the on-chain id: the config id is None for an unmapped spoke.
        for row in sorted(auditor.rows, key=lambda r: (r["onchain_spoke_id"], r["asset"])):
            if row["blockers"]:
                verdict = "FAIL"
            elif any("ENABLED" in w for w in row["warnings"]):
                verdict = "PASS (dead market)"
            else:
                verdict = "PASS"
            config_id = "-" if row["spoke_id"] is None else row["spoke_id"]
            print(
                f"{config_id:>5} {row['onchain_spoke_id']:>5} {row['asset']:<14} "
                f"{row['hub_id']:>3} "
                f"{str(row['decimals']):>3} {row['supply_cap']:>26} {row['borrow_cap']:>26} "
                f"{str(row['is_collateralizable']):>5} {str(row['is_borrowable']):>5}  {verdict}"
            )

    blockers = [(r, b) for r in auditor.rows for b in r["blockers"]]
    dead = [
        (r, w) for r in auditor.rows for w in r["warnings"] if "ENABLED" in w
    ]

    if blockers:
        print("\nDEPLOY BLOCKERS:")
        for row, msg in blockers:
            print(f"  - {row_label(row)} / {row['asset']}: {msg}")

    if dead:
        print("\nZERO CAP ON AN ENABLED SIDE (legal, but the market accepts nothing):")
        for row, msg in dead:
            print(f"  - {row_label(row)} / {row['asset']}: {msg}")

    if auditor.divergences:
        print("\nLIVE vs CHECKED-IN CONFIG DIVERGENCE:")
        for msg in auditor.divergences:
            print(f"  - {msg}")

    if auditor.query_failures:
        print("\nQUERY FAILURES (live verification incomplete for these):")
        for msg in auditor.query_failures:
            print(f"  - {msg}")

    ok = not blockers and not auditor.query_failures
    print(f"\n{auditor.network}: {'PASS' if ok else 'FAIL'}")
    return ok

def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument(
        "--network", default="mainnet", choices=["mainnet", "testnet", "all"],
        help="network to audit (default: mainnet)",
    )
    parser.add_argument("--json", metavar="PATH", help="write the full result set as JSON")
    parser.add_argument("--quiet", action="store_true", help="suppress progress logging")
    parser.add_argument(
        "--rpc-url", help="override the network's rpc_url (single --network only)"
    )
    parser.add_argument(
        "--controller", help="override the controller address (single --network only)"
    )
    parser.add_argument(
        "--config-audit", action="store_true",
        help="lint the checked-in JSON offline; makes NO claim about on-chain state",
    )
    parser.add_argument(
        "--scan-events", action="store_true",
        help="also scan retained config events for assets missing from markets.json",
    )
    args = parser.parse_args()

    networks = ["mainnet", "testnet"] if args.network == "all" else [args.network]

    if (args.rpc_url or args.controller) and len(networks) > 1:
        sys.exit("--rpc-url/--controller apply to a single --network, not 'all'")

    if args.config_audit:
        return 0 if all(config_audit(network) for network in networks) else 1

    results = {}
    overall = True
    for network in networks:
        auditor = Auditor(
            network,
            verbose=not args.quiet,
            rpc_url=args.rpc_url,
            controller=args.controller,
        )
        if auditor.audit() and args.scan_events:
            scan_config_events(auditor)
        overall &= render(auditor)
        results[network] = {
            "controller": auditor.controller,
            "pool": auditor.pool,
            "rows": auditor.rows,
            "divergences": auditor.divergences,
            "query_failures": auditor.query_failures,
        }

    if args.json:
        with open(args.json, "w", encoding="utf-8") as handle:
            json.dump(results, handle, indent=2)
        print(f"\nwrote {args.json}")

    print(f"\nOVERALL: {'PASS' if overall else 'FAIL'}")
    return 0 if overall else 1

if __name__ == "__main__":
    sys.exit(main())
