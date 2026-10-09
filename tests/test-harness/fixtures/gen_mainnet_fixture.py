#!/usr/bin/env python3
"""Writes mainnet-<date>.json from configs/ plus simulated aggregator prices() reads."""

import json
import subprocess
import sys
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RPC = "https://stellar-gateway.xoxno.com"
PASSPHRASE = "Public Global Stellar Network ; September 2015"
LISTING_KEYS = [
    "hub_id",
    "can_be_collateral",
    "can_be_borrowed",
    "ltv",
    "liquidation_threshold",
    "liquidation_bonus",
    "liquidation_fees",
    "supply_cap",
    "borrow_cap",
]


def load(rel):
    return json.loads((ROOT / rel).read_text())


def live_price(aggregator, asset):
    out = subprocess.run(
        [
            "stellar", "contract", "invoke", "--send=no",
            "--rpc-url", RPC, "--network-passphrase", PASSPHRASE,
            "--source", "deployer", "--id", aggregator, "--",
            "prices", "--keys", json.dumps([{"Token": asset}]),
        ],
        capture_output=True, text=True, timeout=120, check=True,
    ).stdout.strip().splitlines()[-1]
    (feed,) = json.loads(out).values()
    return feed["price_wad"], feed["timestamp"]


def main():
    network = load("configs/networks.json")["mainnet"]
    spokes = load("configs/mainnet/spokes.json")
    markets = {m["name"]: m for m in load("configs/mainnet/markets.json")["markets"]}
    hubs = load("configs/mainnet/hubs.json")

    out_spokes = {}
    used = set()
    for cfg_id, spoke in spokes.items():
        if not spoke.get("enabled", True) or cfg_id not in network["spoke_ids"]:
            continue
        assets = {
            name: {k: listing[k] for k in LISTING_KEYS}
            for name, listing in spoke["assets"].items()
            if markets[name].get("enabled", True)
        }
        used.update(assets)
        out_spokes[cfg_id] = {
            "onchain_id": network["spoke_ids"][cfg_id],
            "name": spoke["name"],
            "liquidation_curve": spoke["liquidation_curve"],
            "assets": assets,
        }

    out_markets = {}
    for name in [n for n in markets if n in used]:
        m = markets[name]
        price, ts = live_price(network["price_aggregator"], m["asset_address"])
        out_markets[name] = {
            "hub_id": m["hub_id"],
            "asset_address": m["asset_address"],
            "decimals": m["oracle"]["asset_decimals"],
            "min_sanity_price_wad": m["oracle"]["min_sanity_price_wad"],
            "max_sanity_price_wad": m["oracle"]["max_sanity_price_wad"],
            "price_wad": price,
            "price_timestamp": ts,
            "market_params": m["market_params"],
        }
        print(f"{name}: {price}", file=sys.stderr)

    fixture = {
        "source": {
            "configs_commit": subprocess.run(
                ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True
            ).stdout.strip(),
            "prices": "price aggregator prices() simulated with --send=no",
            "date": date.today().isoformat(),
        },
        "hubs": {
            cfg_id: {"onchain_id": network["hub_ids"][cfg_id], "name": hub["name"]}
            for cfg_id, hub in hubs.items()
        },
        "spokes": out_spokes,
        "markets": out_markets,
    }
    path = Path(__file__).with_name(f"mainnet-{fixture['source']['date']}.json")
    path.write_text(json.dumps(fixture, indent=2) + "\n")
    print(path, file=sys.stderr)


if __name__ == "__main__":
    main()
