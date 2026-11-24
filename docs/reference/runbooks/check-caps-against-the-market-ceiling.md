# Check listing caps against the market ceiling

Do this check before you list an asset in a spoke or raise a spoke's cap.

The pool refuses a supply, borrow, strategy opening or flash loan that would
take a market's total supply or total debt above `MAX_MARKET_ENTRY_RAY`. The
error is `MarketCeilingReached` (136). The ceiling is about 21.27 billion whole
tokens for every decimal count.

Listing validation bounds each cap alone (`max_cap_for_decimals`). The
controller does not add the caps of all spokes that list the same market. If
their sum is above the ceiling, the caps do not bind: entries in every spoke of
that market fail at the ceiling first. This runbook is the only check of the
sum.

## Add the caps

1. Apply the planned edit to `configs/<network>/spokes.json`.
2. Print each market's cap sums as a share of the ceiling:

       python3 - <network> <<'EOF'
       import json, sys
       net = sys.argv[1]
       ceiling = (2**127 - 1 - 10**27) // 8  # MAX_MARKET_ENTRY_RAY
       spokes = json.load(open(f"configs/{net}/spokes.json"))
       markets = json.load(open(f"configs/{net}/markets.json"))["markets"]
       decimals = {(m["hub_id"], m["name"]): m["oracle"]["asset_decimals"] for m in markets}
       sums = {}
       for spoke in spokes.values():
           for name, a in spoke["assets"].items():
               s = sums.setdefault((a["hub_id"], name), [0, 0])
               s[0] += int(a["supply_cap"])
               s[1] += int(a["borrow_cap"])
       for (hub, name), (supply, borrow) in sorted(sums.items()):
           cap = ceiling // 10 ** (27 - decimals[(hub, name)])
           print(f"hub {hub} {name}: supply {supply / cap:.4%}, borrow {borrow / cap:.4%}")
       EOF

3. Read the token's total supply from its contract, in whole tokens.

## Decide

- If a cap sum is above 100%, lower the caps until the sum is at or below
  100%.
- If the token's total supply is above the ceiling, the market can reach the
  ceiling. Set the caps so that their sum stays below it.
- Otherwise, submit the edit.
