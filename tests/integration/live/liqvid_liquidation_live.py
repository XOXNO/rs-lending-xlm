#!/usr/bin/env python3
"""Live testnet run: 0-decimal Liqvid shares (the real Asterizm RWA WASM) as
collateral, hub-4 USDC debt, accounts at several health factors, and every
liquidation path, with math checks on each liquidation.

Spends testnet signers (deployer and lqv_* identities). Usage:
    python3 tests/integration/live/liqvid_liquidation_live.py <log> [pass2 <account_id> | pass3]
"""
import json
import re
import subprocess
import sys
import time
from pathlib import Path

W = Path(__file__).resolve().parents[3]
NET = "testnet"
CTRL = "CCXRWJ6SIU2WPFEGLFGJVITPL57QAYIMIO6OAM2NBGNDQSSCK2FFV3F3"
AGG = "CBMARXSYE56NPDLX42TTICM5GS5XQCYRD5VMCJME624TS6C2S53VETYU"
POOL = "CBSGF6QOQAMPFBEVSYPEQHSZRIHJ6RCGUPCRDMUX36DEKRWFAO2PZB5A"
ADAPTER = "CDHPWYORLKTMN2XAG7Q7KMSBRNVEO3ZECDDKOAZ3EIZCLS42JOLX4JKL"
LIQ = "CAH7FDDP6IDC76VTXNPN6MF5TWVZMM2N3PCU5C7TXRLTCKBSG6BGUGEA"
USDC = "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA"
USDC_ISSUER = "GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5"
HUB, SPOKE = 4, 5
USDC_UNIT = 10**7
WAD = 10**18
LT = 0.60
LOG = Path(sys.argv[1] if len(sys.argv) > 1 else "liqvid_live.log")
RESULTS = []


def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    print(line, flush=True)
    with LOG.open("a") as fh:
        fh.write(line + "\n")


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    log(f"{'PASS' if ok else 'FAIL'} {name} {detail}")


def cli(args, timeout=240):
    for attempt in range(4):
        p = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
        err = p.stderr
        if p.returncode == 0 or "Error(Contract" in err or "HostError" in err:
            return p.returncode, p.stdout.strip(), err
        if any(s in err for s in ("TxBadSeq", "timeout", "Timeout", "429", "503", "connection")):
            time.sleep(5 + 5 * attempt)
            continue
        return p.returncode, p.stdout.strip(), err
    return p.returncode, p.stdout.strip(), err


def invoke(src, cid, fn, *args, send=True):
    cmd = ["stellar", "contract", "invoke", "--network", NET, "--source-account", src, "--id", cid]
    if not send:
        cmd.append("--send=no")
    cmd += ["--", fn, *[str(a) for a in args]]
    rc, out, err = cli(cmd)
    if rc == 0:
        last = out.splitlines()[-1] if out else ""
        try:
            return True, json.loads(last)
        except json.JSONDecodeError:
            return True, last
    m = re.search(r"Error\(Contract, #(\d+)\)", err)
    return False, int(m.group(1)) if m else err[-600:]


def view(cid, fn, *args):
    ok, val = invoke("deployer", cid, fn, *args, send=False)
    if not ok:
        raise RuntimeError(f"view {fn} failed: {val}")
    return val


def must(res, what):
    ok, val = res
    if not ok:
        raise RuntimeError(f"{what} failed: {val}")
    return val


def addr(name):
    p = subprocess.run(["stellar", "keys", "address", name], capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else None


def identity(name):
    a = addr(name)
    if a:
        return a
    subprocess.run(["stellar", "keys", "generate", name, "--network", NET, "--fund"], check=True,
                   capture_output=True, text=True)
    return addr(name)


def trust_usdc(name):
    cli(["stellar", "tx", "new", "change-trust", "--source-account", name,
         "--line", f"USDC:{USDC_ISSUER}", "--network", NET])


def hub_key(asset, hub=HUB):
    return json.dumps({"hub_id": hub, "asset": asset})


def pay(asset, amount, hub=HUB):
    return json.dumps([[{"hub_id": hub, "asset": asset}, str(amount)]])


D = None


def allow(*addrs):
    must(invoke("deployer", LIQ, "add_to_allowlist", "--accounts", json.dumps(list(addrs))), "allowlist")


def mint_liq(to, n):
    must(invoke("deployer", LIQ, "mint", "--to", to, "--amount", n), "mint LIQ")


def usdc_bal(a):
    return int(view(USDC, "balance", "--id", a))


def liq_bal(a):
    return int(view(LIQ, "balance", "--id", a))


def units(acct):
    try:
        return int(view(CTRL, "get_collateral_amount", "--account_id", acct, "--hub_asset", hub_key(LIQ)))
    except RuntimeError:
        return 0


def debt(acct):
    try:
        return int(view(CTRL, "get_borrow_amount", "--account_id", acct, "--hub_asset", hub_key(USDC)))
    except RuntimeError:
        return 0


def hf(acct):
    return int(view(CTRL, "get_health_factor", "--account_id", acct)) / WAD


def liquidatable(acct):
    return bool(view(CTRL, "is_liquidatable", "--account_id", acct))


NAV = {"now": None, "band": 1.00}


def push_nav(price):
    ts = int(time.time() * 1000)
    must(invoke("deployer", ADAPTER, "submit_price", "--signer", D, "--feed_id", "LIQVID1039",
                "--price", int(round(price * 10**8)), "--package_timestamp", ts), "submit NAV")
    NAV["now"] = price


def move_band(center):
    path = W / "configs/testnet/markets.json"
    cfg = json.loads(path.read_text())
    for m in cfg["markets"]:
        if m["name"] == "LIQVID1039":
            m["oracle"]["min_sanity_price_wad"] = str(int(round(center * 0.95 * WAD)))
            m["oracle"]["max_sanity_price_wad"] = str(int(round(center * 1.05 * WAD)))
    path.write_text(json.dumps(cfg, indent=2) + "\n")
    log(f"band -> [{center*0.95:.4f}, {center*1.05:.4f}] via governance (timelocked)")
    p = subprocess.run(["make", "testnet", "configureMarketOracle", "LIQVID1039"], cwd=W,
                       capture_output=True, text=True, timeout=1200)
    if p.returncode != 0:
        raise RuntimeError("band move failed: " + (p.stdout + p.stderr)[-1500:])


def set_nav(price):
    if NAV["band"] != price:
        move_band(price)
        NAV["band"] = price
    push_nav(price)
    log(f"NAV now ${price}")


def supply(name, acct, n):
    return int(must(invoke(name, CTRL, "supply", "--caller", addr(name), "--account_id", acct,
                           "--spoke_id", SPOKE, "--assets", pay(LIQ, n)), f"{name} supply"))


def borrow(name, acct, raw):
    return invoke(name, CTRL, "borrow", "--caller", addr(name), "--account_id", acct,
                  "--borrows", pay(USDC, raw), "--to", "null")


def estimate(acct, offer, mode='"Transfer"'):
    return view(CTRL, "get_liquidation_estimate", "--account_id", acct,
                "--debt_payments", pay(USDC, offer), "--seize_mode", mode)


def liquidate(name, acct, offer, mode='"Transfer"'):
    return invoke(name, CTRL, "liquidate", "--liquidator", addr(name), "--account_id", acct,
                  "--debt_payments", pay(USDC, offer), "--seize_mode", mode)


def sweep_usdc(name):
    bal = usdc_bal(addr(name))
    if bal > 0:
        must(invoke(name, USDC, "transfer", "--from", addr(name), "--to", addr("lqv_liq"), "--amount", bal), "sweep USDC")


def usdc_price():
    out = view(AGG, "prices", "--keys", json.dumps([{"Token": USDC}]))
    entry = out[0] if isinstance(out, list) else list(out.values())[0]
    return int(entry["price_wad"]) / WAD


def checked_liquidation(tag, name, acct, offer, nav, mode='"Transfer"', expect_full=None):
    """Liquidates and checks the whole-unit math on the result."""
    pre_u, pre_d, pre_hf = units(acct), debt(acct), hf(acct)
    est = estimate(acct, offer, mode)
    bonus = int(est["bonus_rate_bps"]) / 10_000
    est_units = int(est["seized_collaterals"][0]["amount"]) if est["seized_collaterals"] else 0
    who = addr(name)
    usdc0, liq0 = usdc_bal(who), liq_bal(who)
    ok, res = liquidate(name, acct, offer, mode)
    if not ok:
        check(f"{tag}: liquidation settles", False, f"error {res}")
        return None
    paid = usdc0 - usdc_bal(who)
    got = liq_bal(who) - liq0 if mode == '"Transfer"' else units(int(res)) if res else 0
    post_u, post_d = units(acct), debt(acct)
    seized = pre_u - post_u
    log(f"{tag}: HF {pre_hf:.4f} units {pre_u}->{post_u} debt ${pre_d/USDC_UNIT:.4f}->${post_d/USDC_UNIT:.4f} "
        f"paid ${paid/USDC_UNIT:.4f} got {got} (est {est_units}) bonus {bonus*100:.2f}%")
    px = usdc_price()
    unit_raw = nav * USDC_UNIT / px
    fair = seized * unit_raw / (1 + bonus)
    full = post_d == 0
    insolvent = pre_u * nav * USDC_UNIT < pre_d * px
    check(f"{tag}: seized shares are whole and match the view", seized == est_units == got,
          f"seized {seized} view {est_units} received {got}")
    check(f"{tag}: debt went down", post_d < pre_d)
    if insolvent:
        backed = pre_u * unit_raw / (1 + bonus)
        check(f"{tag}: insolvent seizure charges the collateral-backed quote",
              backed - 2 <= paid <= backed + max(2, backed * 2e-6), f"paid {paid} backed {backed:.1f} (USDC ${px:.6f})")
        check(f"{tag}: insolvent seizure takes every share", post_u == 0, f"left {post_u}")
    elif not full:
        check(f"{tag}: liquidator pays the fair price for whole shares",
              abs(paid - fair) <= max(2, fair * 2e-4), f"paid {paid} fair {fair:.1f} (USDC ${px:.6f}; 2 bps accrual drift allowed)")
    else:
        check(f"{tag}: a solvent full close repays the whole debt", pre_d <= paid <= pre_d + 100, f"paid {paid} debt {pre_d} (accrual allowed)")
        check(f"{tag}: a solvent full close takes exactly one share", seized == 1, f"seized {seized}")
        log(f"{tag}: effective bonus {(nav*USDC_UNIT/px)/paid - 1:.4f} vs curve bonus {bonus:.4f}")
    if pre_u * nav * USDC_UNIT >= pre_d and post_d > 0:
        check(f"{tag}: collateral-to-debt ratio does not fall",
              post_u * pre_d >= pre_u * post_d, f"C/D {pre_u*nav*USDC_UNIT/pre_d:.4f} -> {post_u*nav*USDC_UNIT/post_d:.4f}")
    if expect_full is not None:
        check(f"{tag}: full close = {expect_full}", full == expect_full)
    return {"paid": paid, "seized": seized, "bonus": bonus, "full": full, "receiver": res}


def phase_r():
    global D
    D = addr("deployer")
    log("=== live Liqvid run, pass 2 ===")
    liq = addr("lqv_liq")
    must(invoke("lqv_liq", USDC, "transfer", "--from", liq, "--to", addr("lqv_liq2"), "--amount", 150 * USDC_UNIT), "fund liq2")
    b = int(sys.argv[3])
    set_nav(0.70)
    check("B liquidatable at NAV 0.70", liquidatable(b), f"HF {hf(b):.4f}")
    ok, err = liquidate("lqv_liq2", b, debt(b))
    check("Transfer liquidation by a funded address off the allowlist fails with token error 7", (not ok) and err == 7, f"{err}")
    r = checked_liquidation("B Credit @0.70 (liquidator off allowlist)", "lqv_liq2", b, debt(b), 0.70, mode='{"Credit":0}')
    if r and r["receiver"]:
        rid = int(r["receiver"])
        ok, err = invoke("lqv_liq2", CTRL, "withdraw", "--caller", addr("lqv_liq2"), "--account_id", rid,
                         "--withdrawals", pay(LIQ, 1), "--to", "null")
        check("credited shares cannot leave to an address off the allowlist", (not ok) and err == 7, f"{err}")
        ok, err = invoke("lqv_liq2", CTRL, "withdraw", "--caller", addr("lqv_liq2"), "--account_id", rid,
                         "--withdrawals", pay(LIQ, 1), "--to", liq)
        check("credited shares can leave to an allowlisted recipient", ok, f"{err}")
    set_nav(100.0)
    identity("lqv_bG")
    trust_usdc("lqv_bG")
    allow(addr("lqv_bG"))
    mint_liq(addr("lqv_bG"), 2)
    g = supply("lqv_bG", 0, 2)
    must(borrow("lqv_bG", g, 999 * USDC_UNIT // 10), "G borrow")
    sweep_usdc("lqv_bG")
    set_nav(80.0)
    checked_liquidation("G one-share sale @80", "lqv_liq", g, debt(g), 80.0, expect_full=False)
    residue = debt(g) / USDC_UNIT
    nav = round(residue * 1.5, 2)
    set_nav(nav)
    check("G residue is liquidatable and solvent", liquidatable(g) and nav > residue, f"HF {hf(g):.4f} NAV {nav} debt {residue:.4f}")
    checked_liquidation(f"G solvent one-share full close @{nav}", "lqv_liq", g, debt(g) + 10, nav, expect_full=True)
    set_nav(1.00)
    passed = sum(1 for r in RESULTS if r[1])
    log(f"=== pass 2: {passed}/{len(RESULTS)} checks passed ===")
    for r in RESULTS:
        if not r[1]:
            log(f"FAILED: {r[0]} {r[2]}")
    return 0 if passed == len(RESULTS) else 1


def phase_3():
    """New listing (LT 53%, curve 1.06/0.90/598, band [0.849, 1.03]): NAV
    steps stay inside the band, so no governance band move is needed."""
    global D
    D = addr("deployer")
    log("=== live Liqvid run, pass 3: new listing parameters ===")
    push_nav(1.00)
    plan = {"lqv_bH": (300, 1495), "lqv_bI": (300, 1440), "lqv_bJ": (30, 140)}
    acct = {}
    for n, (sh, tenths) in plan.items():
        identity(n)
        trust_usdc(n)
        allow(addr(n))
        mint_liq(addr(n), sh)
        acct[n] = supply(n, 0, sh)
        must(borrow(n, acct[n], tenths * USDC_UNIT // 10), f"{n} borrow")
        sweep_usdc(n)
        expected = 0.53 * sh / (tenths / 10)
        check(f"{n}: HF after borrow at LT 53%", abs(hf(acct[n]) - expected) < 0.01, f"{hf(acct[n]):.4f} vs {expected:.4f}")
    h, i, j = acct["lqv_bH"], acct["lqv_bI"], acct["lqv_bJ"]
    push_nav(0.97)
    check("nobody is liquidatable after a 3% markdown", not any(liquidatable(a) for a in (h, i, j)))
    push_nav(0.94)
    check("the max-LTV account is liquidatable after a 6% markdown, inside the band", liquidatable(h), f"HF {hf(h):.4f}")
    r = checked_liquidation("H partial @0.94 (in band)", "lqv_liq", h, debt(h), 0.94)
    if r:
        check("H bonus is on the new curve (below 10%)", r["bonus"] < 0.10, f"{r['bonus']*100:.2f}%")
        check("H is healthy again after the liquidation", not liquidatable(h), f"HF {hf(h):.4f}")
    push_nav(0.90)
    check("I is liquidatable at 0.90", liquidatable(i), f"HF {hf(i):.4f}")
    r = checked_liquidation("I partial @0.90 (in band)", "lqv_liq", i, debt(i), 0.90)
    if r:
        check("I bonus is on the new curve (at most 10%)", r["bonus"] <= 0.1001, f"{r['bonus']*100:.2f}%")
    push_nav(0.87)
    if liquidatable(j):
        checked_liquidation("J small account @0.87 (in band)", "lqv_liq", j, debt(j), 0.87)
    push_nav(0.84)
    candidates = [a for a in (h, i, j) if debt(a) > 0]
    ok, err = liquidate("lqv_liq", candidates[0], debt(candidates[0]))
    check("a NAV below the band floor fails closed", (not ok), f"error {err}")
    ok, err = borrow("lqv_bH", h, USDC_UNIT)
    check("borrowing fails closed below the band floor", not ok, f"error {err}")
    push_nav(1.00)
    passed = sum(1 for r in RESULTS if r[1])
    log(f"=== pass 3: {passed}/{len(RESULTS)} checks passed ===")
    for r in RESULTS:
        if not r[1]:
            log(f"FAILED: {r[0]} {r[2]}")
    return 0 if passed == len(RESULTS) else 1


def main():
    global D
    D = addr("deployer")
    log("=== live Liqvid run on testnet ===")
    names = ["lqv_bA", "lqv_bB", "lqv_bC", "lqv_bE", "lqv_bF", "lqv_liq", "lqv_liq2"]
    for n in names:
        identity(n)
        trust_usdc(n)
    allow(*[addr(n) for n in names if n != "lqv_liq2"])
    pool_liq0 = liq_bal(POOL)
    log(f"pool LIQ balance at start {pool_liq0}; deployer USDC {usdc_bal(D)/USDC_UNIT}")
    must(invoke("deployer", USDC, "transfer", "--from", D, "--to", addr("lqv_liq"), "--amount", 100 * USDC_UNIT), "fund liq")
    must(invoke("deployer", USDC, "transfer", "--from", D, "--to", addr("lqv_liq2"), "--amount", 45 * USDC_UNIT), "fund liq2")

    # Phase 1: $1 shares at the listed LTV 50% / LT 60%.
    set_nav(1.00)
    plan = {"lqv_bA": (400, 199), "lqv_bB": (200, 90), "lqv_bC": (30, 14)}
    acct = {}
    for n, (sh, usd) in plan.items():
        mint_liq(addr(n), sh)
        acct[n] = supply(n, 0, sh)
        must(borrow(n, acct[n], usd * USDC_UNIT), f"{n} borrow")
        expected = LT * sh / usd
        check(f"{n}: HF after borrow", abs(hf(acct[n]) - expected) < 0.01, f"{hf(acct[n]):.4f} vs {expected:.4f}")
    ok, err = borrow("lqv_bA", acct["lqv_bA"], 2 * USDC_UNIT)
    check("borrow past LTV is refused", not ok, f"error {err}")
    for n in plan:
        sweep_usdc(n)

    set_nav(0.80)
    check("A liquidatable at NAV 0.80", liquidatable(acct["lqv_bA"]), f"HF {hf(acct['lqv_bA']):.4f}")
    check("B healthy at NAV 0.80", not liquidatable(acct["lqv_bB"]), f"HF {hf(acct['lqv_bB']):.4f}")
    ok, err = liquidate("lqv_liq2", acct["lqv_bA"], debt(acct["lqv_bA"]))
    check("Transfer liquidation by an address off the allowlist fails with token error 7", (not ok) and err == 7, f"{err}")
    checked_liquidation("A partial @0.80", "lqv_liq", acct["lqv_bA"], debt(acct["lqv_bA"]), 0.80)

    set_nav(0.70)
    r = checked_liquidation("B Credit @0.70 (liquidator off allowlist)", "lqv_liq2", acct["lqv_bB"],
                            debt(acct["lqv_bB"]), 0.70, mode='{"Credit":0}')
    if r and r["receiver"]:
        ok, err = invoke("lqv_liq2", CTRL, "withdraw", "--caller", addr("lqv_liq2"), "--account_id", int(r["receiver"]),
                         "--withdrawals", pay(LIQ, 1), "--to", "null")
        check("credited shares cannot leave to an address off the allowlist", (not ok) and err == 7, f"{err}")
    checked_liquidation("C partial @0.70", "lqv_liq", acct["lqv_bC"], debt(acct["lqv_bC"]), 0.70)

    set_nav(0.30)
    a = acct["lqv_bA"]
    if debt(a) > 0:
        pre_u = units(a)
        check("A insolvent at NAV 0.30", pre_u * 0.30 * USDC_UNIT < debt(a))
        res = checked_liquidation("A insolvent seize_all @0.30", "lqv_liq", a, debt(a), 0.30)
        if res:
            check("insolvent liquidation seizes every share", units(a) == 0, f"left {units(a)}")

    # Phase 2: $100 shares, where one share is worth as much as a small debt.
    set_nav(100.0)
    mint_liq(addr("lqv_bE"), 2)
    e = supply("lqv_bE", 0, 2)
    must(borrow("lqv_bE", e, 999 * USDC_UNIT // 10), "E borrow")
    sweep_usdc("lqv_bE")
    mint_liq(addr("lqv_bF"), 6)
    f = supply("lqv_bF", 0, 6)
    must(borrow("lqv_bF", f, 200 * USDC_UNIT), "F borrow")
    sweep_usdc("lqv_bF")

    set_nav(80.0)
    est = estimate(e, debt(e))
    under = int(80 * USDC_UNIT / (1 + int(est["bonus_rate_bps"]) / 10_000)) - 1000
    ok, err = liquidate("lqv_liq", e, under)
    check("an offer below one share at NAV/(1+bonus) takes nothing (#16)", (not ok) and err == 16, f"offer {under} -> {err}")
    check("the view quotes one share for the two-share account",
          est["seized_collaterals"] and int(est["seized_collaterals"][0]["amount"]) == 1, json.dumps(est)[:200])
    checked_liquidation("E one-share sale @80", "lqv_liq", e, debt(e), 80.0, expect_full=False)

    set_nav(50.0)
    checked_liquidation("F multi-share partial @50", "lqv_liq", f, debt(f), 50.0)

    set_nav(35.0)
    if liquidatable(e):
        res = checked_liquidation("E one-share residue full close @35", "lqv_liq", e, debt(e) + 10, 35.0, expect_full=True)
        if res:
            check("full close takes exactly the last share", res["seized"] == 1)

    # Phase 3: issuer decimals relabel on the real token.
    name, symbol = view(LIQ, "name"), view(LIQ, "symbol")
    must(invoke("deployer", LIQ, "set_metadata", "--decimal", 2, "--name", name, "--symbol", symbol), "relabel")
    check("token now reports 2 decimals", int(view(LIQ, "decimals")) == 2)
    set_nav(35.01)
    stored = view(AGG, "oracle", "--key", json.dumps({"Token": LIQ}))
    check("after a relabel and an oracle update, the aggregator keeps 0 decimals", stored["asset_decimals"] == 0,
          f"{stored['asset_decimals']}")
    must(invoke("deployer", LIQ, "set_metadata", "--decimal", 0, "--name", name, "--symbol", symbol), "restore metadata")

    # Restore NAV $1 so testnet stays usable.
    set_nav(1.00)
    log(f"pool LIQ balance at end {liq_bal(POOL)} (start {pool_liq0})")
    passed = sum(1 for r in RESULTS if r[1])
    log(f"=== {passed}/{len(RESULTS)} checks passed ===")
    for r in RESULTS:
        if not r[1]:
            log(f"FAILED: {r[0]} {r[2]}")
    return 0 if passed == len(RESULTS) else 1


if __name__ == "__main__":
    try:
        mode = sys.argv[2] if len(sys.argv) > 2 else ""
        sys.exit(phase_r() if mode == "pass2" else phase_3() if mode == "pass3" else main())
    except Exception as exc:
        log(f"ABORT: {exc}")
        sys.exit(2)
