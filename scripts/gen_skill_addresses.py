#!/usr/bin/env python3
"""Generate skills/xoxno-lending/addresses.md from configs/.

Source of truth: configs/networks.json (deployed contract ids, on-chain hub and
spoke id maps) and configs/<network>/{hubs,spokes,markets,blend}.json (names
and listings). Off-chain endpoints that are not in configs/ are
listed in OFFCHAIN below with the file they were read from.

Usage:
    python3 scripts/gen_skill_addresses.py            # rewrite the file
    python3 scripts/gen_skill_addresses.py --check    # exit 1 if the file is stale
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONFIGS = ROOT / "configs"
OUT = ROOT / "skills" / "xoxno-lending" / "addresses.md"
NETWORKS = ("mainnet", "testnet")

# Off-chain endpoints. Observed in sdk-js src/sdk/stellar/contracts.ts
# (quote server) and xoxno-ui src/lib/blockchain/network.ts (API base).
OFFCHAIN = {
    "mainnet": {
        "Quote server (swap aggregator API)": "https://stellar-swap.xoxno.com",
        "XOXNO API (xoxno-api-v2)": "https://api.xoxno.com",
    },
    "testnet": {
        "Quote server (swap aggregator API)": "https://testnet-stellar-swap.xoxno.com",
        "XOXNO API (xoxno-api-v2)": "https://testnet-api.xoxno.com",
    },
}

CONTRACT_ROWS = (
    ("governance", "Governance (timelock; owns controller + price aggregator)"),
    ("controller", "Controller (the only user-facing lending contract)"),
    ("pool", "Pool (custody + accounting; controller-only mutators)"),
    ("position_nft", "Position NFT (token id == account id)"),
    ("price_aggregator", "Price aggregator"),
    ("aggregator", "Swap aggregator router (`execute_strategy`)"),
    ("xoxno_oracle_adapter", "XOXNO oracle adapter"),
    ("redstone_adapter_contract", "RedStone adapter (external)"),
    ("reflector_cex_oracle", "Reflector CEX oracle (external)"),
    ("reflector_dex_oracle", "Reflector DEX oracle (external)"),
    ("reflector_fx_oracle", "Reflector FX oracle (external)"),
    ("accumulator", "Revenue accumulator (G-address)"),
)


def render_network(net: str, networks: dict) -> list[str]:
    n = networks[net]
    hubs = json.loads((CONFIGS / net / "hubs.json").read_text())
    spokes = json.loads((CONFIGS / net / "spokes.json").read_text())
    markets = json.loads((CONFIGS / net / "markets.json").read_text())["markets"]
    blend = json.loads((CONFIGS / net / "blend.json").read_text())
    hub_map = n.get("hub_ids", {})
    spoke_map = n.get("spoke_ids", {})

    out: list[str] = []
    out.append(f"## {net.capitalize()}")
    out.append("")
    out.append(f"- Network passphrase: `{n['network_passphrase']}`")
    out.append(f"- RPC gateway used by XOXNO clients: `{n['rpc_url']}`")
    for label, url in OFFCHAIN[net].items():
        out.append(f"- {label}: `{url}`")
    out.append(f"- Deployment recorded at ledger `{n['ledger_index']}`; "
               f"timelock min delay `{n['timelock_min_delay_ledgers']}` ledgers")
    out.append("")
    out.append("### Contracts")
    out.append("")
    out.append("| Role | Address |")
    out.append("|---|---|")
    for key, label in CONTRACT_ROWS:
        value = n.get(key)
        if not value:
            continue
        out.append(f"| {label} | `{value}` |")
    out.append("")

    out.append("### Hubs")
    out.append("")
    out.append("| On-chain `hub_id` | Name | Markets |")
    out.append("|---|---|---|")
    for cfg_id, hub in sorted(hubs.items(), key=lambda kv: int(kv[0])):
        onchain = hub_map.get(cfg_id)
        if onchain is None:
            continue
        syms = [m["name"] for m in markets if str(m["hub_id"]) == cfg_id]
        out.append(f"| {onchain} | {hub['name']} | {', '.join(syms) or '—'} |")
    out.append("")

    out.append("### Markets (`HubAssetKey { hub_id, asset }`)")
    out.append("")
    out.append("| Symbol | `hub_id` | `asset` | Decimals (oracle config) |")
    out.append("|---|---|---|---|")
    unmapped_hub_markets = []
    for m in markets:
        onchain_hub = hub_map.get(str(m["hub_id"]))
        if onchain_hub is None:
            unmapped_hub_markets.append(m["name"])
            continue
        out.append(f"| {m['name']} | {onchain_hub} | `{m['asset_address']}` | "
                   f"{m['oracle']['asset_decimals']} |")
    if unmapped_hub_markets:
        out.append("")
        out.append("Configured under a hub that is **not created on-chain** (absent from "
                   "`hub_ids` in `configs/networks.json`): " + ", ".join(unmapped_hub_markets) + ".")
    out.append("")
    out.append("The decimals column is the oracle configuration value; the pool reads the "
               "token contract's own `decimals` at market creation.")
    out.append("")

    out.append("### Spokes")
    out.append("")
    out.append("Live listing parameters: `get_spoke_asset(spoke_id, hub_asset)`.")
    out.append("")
    out.append("| On-chain `spoke_id` | Name | Config key | Listed markets |")
    out.append("|---|---|---|---|")
    not_deployed = []
    for cfg_id, spoke in sorted(spokes.items(), key=lambda kv: int(kv[0])):
        onchain = spoke_map.get(cfg_id)
        listed = ", ".join(f"{sym}@hub{hub_map[str(c['hub_id'])]}"
                           for sym, c in spoke["assets"].items()
                           if str(c["hub_id"]) in hub_map)
        if onchain is None:
            not_deployed.append((cfg_id, spoke["name"]))
            continue
        out.append(f"| {onchain} | {spoke['name']} | {cfg_id} | {listed} |")
    if not_deployed:
        out.append("")
        out.append("Configured but **not created on-chain** (absent from `spoke_ids` in "
                   "`configs/networks.json`): "
                   + ", ".join(f"config key {k} ({name})" for k, name in not_deployed) + ".")
    out.append("")

    if blend.get("pools"):
        out.append("### Blend pools approved for `migrate_from_blend`")
        out.append("")
        out.append("| Pool | Address |")
        out.append("|---|---|")
        for p in blend["pools"]:
            out.append(f"| {p['name']} | `{p['address']}` |")
        out.append("")
        out.append(f"Blend pool factory v2: `{blend['pool_factory_v2']}`. The controller is "
                   "authoritative: `is_blend_pool_approved(pool)`.")
        out.append("")
    return out


def render() -> str:
    networks = json.loads((CONFIGS / "networks.json").read_text())
    lines = [
        "# XOXNO Lending addresses and ids",
        "",
        "<!-- GENERATED FILE. Do not edit by hand. -->",
        "<!-- Regenerate: python3 scripts/gen_skill_addresses.py   (check: --check) -->",
        "",
        "Generated from `configs/networks.json` and `configs/<network>/*.json` in the protocol "
        "repository (github.com/XOXNO/rs-lending-xlm). Snapshot; resolve addresses from those "
        "files in code.",
        "",
    ]
    for net in NETWORKS:
        lines.extend(render_network(net, networks))
    return "\n".join(lines).rstrip() + "\n"


def main(argv: list[str]) -> int:
    content = render()
    if "--check" in argv:
        current = OUT.read_text() if OUT.exists() else ""
        if current != content:
            print(f"{OUT} is stale; run python3 scripts/gen_skill_addresses.py", file=sys.stderr)
            return 1
        print(f"{OUT} is up to date")
        return 0
    OUT.write_text(content)
    print(f"wrote {OUT} ({content.count(chr(10))} lines)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
