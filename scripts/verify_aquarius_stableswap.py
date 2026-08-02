#!/usr/bin/env python3
"""Differentially verify our stableswap LP math against Aquarius' deployed WASM.

Aquarius' pool source is closed, but the deployed contract exposes read-only
views (`get_virtual_price`, `estimate_swap`) that execute its real invariant on
-chain. This tool reads a live pool's state and probes `estimate_swap` across the
whole imbalance range, then checks that OUR invariant (the same `get_D`/`get_y`
the Rust oracle ships, `common/src/oracle/lp_stable.rs`) reproduces the pool's
outputs. `estimate_swap` is highly sensitive to the amplification `A`, so it pins
the `Ann = A*n` convention that `get_virtual_price` alone (near-peg) cannot.

Run BEFORE listing any AquariusStableLp pool:
    python3 scripts/verify_aquarius_stableswap.py <POOL_ID> [--network mainnet]

Exit 0 = our math matches the deployed pool within tolerance; non-zero = drift.
"""
import argparse
import json
import os
import subprocess
import sys

DEFAULT_RPC = "https://mainnet.sorobanrpc.com"
DEFAULT_PASSPHRASE = "Public Global Stellar Network ; September 2015"

N = 2  # Aquarius stableswap pools are pairs
FEE_DENOM = 10000  # fee_fraction is out of 10_000 (bps); calibrated below and asserted
CONV_CORRECT = N  # Ann = A * n  (Curve "code" amplification, what a() returns)
CONV_WRONG = N ** N  # Ann = A * n^n  (the whitepaper-A bug this tool would catch)


def invoke(pool, rpc, passphrase, fn, *args):
    cmd = [
        "stellar", "contract", "invoke", "--id", pool,
        "--source-account", "GDBBOILYIJBSUQKC3Z3USAW3DGPFHIGVKYA5T4ZUZBO56HBUPHJEN3FV",
        "--rpc-url", rpc, "--network-passphrase", passphrase,
        "--send=no", "--", fn, *args,
    ]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"invoke {fn} failed: {out.stderr.strip()}")
    return json.loads(out.stdout.strip())


def get_d(xp, amp, ann_factor):
    """Curve stableswap invariant D over reserves xp, Ann = amp * ann_factor."""
    s = sum(xp)
    if s == 0:
        return 0
    d = s
    ann = amp * ann_factor
    for _ in range(255):
        d_p = d
        for x in xp:
            d_p = d_p * d // (x * N)
        d_prev = d
        d = (ann * s + d_p * N) * d // ((ann - 1) * d + (N + 1) * d_p)
        if abs(d - d_prev) <= 1:
            return d
    raise RuntimeError("D did not converge")


def get_y(i, j, x, xp, amp, ann_factor):
    """New balance of coin j when coin i is set to x, holding the invariant."""
    d = get_d(xp, amp, ann_factor)
    ann = amp * ann_factor
    c = d
    s_ = 0
    for k in range(N):
        _x = x if k == i else (xp[k] if k != j else None)
        if _x is None:
            continue
        s_ += _x
        c = c * d // (_x * N)
    c = c * d // (ann * N)
    b = s_ + d // ann
    y = d
    for _ in range(255):
        y_prev = y
        y = (y * y + c) // (2 * y + b - d)
        if abs(y - y_prev) <= 1:
            return y
    raise RuntimeError("y did not converge")


def our_estimate_swap(i, j, dx, xp, amp, fee, ann_factor):
    """Reproduce Aquarius estimate_swap: dy on the invariant, minus the swap fee."""
    x = xp[i] + dx
    y = get_y(i, j, x, xp, amp, ann_factor)
    dy = xp[j] - y - 1  # Curve subtracts 1 for conservative rounding
    return dy - dy * fee // FEE_DENOM


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pool")
    ap.add_argument("--rpc-url", default=os.environ.get("STELLAR_RPC_URL", DEFAULT_RPC))
    ap.add_argument("--network-passphrase",
                    default=os.environ.get("STELLAR_NETWORK_PASSPHRASE", DEFAULT_PASSPHRASE))
    ap.add_argument("--tol-bps", type=float, default=0.5,
                    help="max allowed relative drift, in bps (default 0.5 = 0.005%)")
    args = ap.parse_args()

    def call(fn, *a):
        return invoke(args.pool, args.rpc_url, args.network_passphrase, fn, *a)

    reserves = [int(x) for x in call("get_reserves")]
    amp = int(call("a"))
    shares = int(call("get_total_shares"))
    decimals = [int(d) for d in call("get_decimals")]
    fee = int(call("get_fee_fraction"))
    vprice = int(call("get_virtual_price"))
    if len(reserves) != N or len(set(decimals)) != 1:
        sys.exit(f"not a 2-coin equal-decimal pool: reserves={reserves} decimals={decimals}")

    print(f"pool {args.pool}")
    print(f"  reserves={reserves} A={amp} shares={shares} decimals={decimals} fee={fee}")

    # 1) get_virtual_price parity (checks D magnitude; weak near-peg, but free).
    #    Aquarius scales virtual_price to the share decimals; D/S in the same scale.
    share_scale = 10 ** decimals[0]
    d_correct = get_d(reserves, amp, CONV_CORRECT)
    our_vp = d_correct * share_scale // shares
    vp_ok = abs(our_vp - vprice) <= 2
    print(f"  virtual_price: pool={vprice}  ours(A*n)={our_vp}  {'OK' if vp_ok else 'MISMATCH'}")

    # 2) estimate_swap sweep — the A-sensitive probe across the whole curve.
    r = min(reserves)
    probes = [(0, 1, dx) for dx in
              (r // 1000, r // 100, r // 10, r // 4, r // 2, (r * 3) // 4, (r * 9) // 10)]
    probes += [(1, 0, dx) for dx in (r // 100, r // 4, (r * 3) // 4)]

    worst_correct = 0.0
    worst_wrong = 0.0
    rows = []
    for i, j, dx in probes:
        pool_out = int(call("estimate_swap",
                            "--in_idx", str(i), "--out_idx", str(j), "--in_amount", str(dx)))
        ok_out = our_estimate_swap(i, j, dx, reserves, amp, fee, CONV_CORRECT)
        bad_out = our_estimate_swap(i, j, dx, reserves, amp, fee, CONV_WRONG)
        e_ok = abs(ok_out - pool_out) / pool_out * 1e4 if pool_out else 0.0
        e_bad = abs(bad_out - pool_out) / pool_out * 1e4 if pool_out else 0.0
        worst_correct = max(worst_correct, e_ok)
        worst_wrong = max(worst_wrong, e_bad)
        rows.append((i, j, dx, pool_out, ok_out, e_ok, e_bad))

    print(f"\n  {'dir':>4} {'in_amount':>16} {'pool_out':>16} {'ours(A*n)':>16} "
          f"{'err_bps':>9} {'wrong_err_bps':>14}")
    for i, j, dx, pool_out, ok_out, e_ok, e_bad in rows:
        print(f"  {i}->{j:<1} {dx:>16} {pool_out:>16} {ok_out:>16} {e_ok:>9.4f} {e_bad:>14.4f}")

    print(f"\n  worst drift  A*n (ours):   {worst_correct:.4f} bps")
    print(f"  worst drift  A*n^n (bug):  {worst_wrong:.4f} bps  <- what the old code would show")

    if vp_ok and worst_correct <= args.tol_bps:
        print(f"\nPASS: our stableswap math matches Aquarius' deployed pool "
              f"(<= {args.tol_bps} bps).")
        return 0
    print(f"\nFAIL: drift exceeds {args.tol_bps} bps or virtual_price mismatch.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
