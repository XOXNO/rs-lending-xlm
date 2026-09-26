# R-23: liquidator payoff at C = D

Status: proposed. An owner decision is necessary. Date: 2026-09-26.

Source: upgrade audit finding R-23 (Addendum A.1.7), severity Info. The
audit found that the liquidator profit jumps where collateral `C` falls below
debt `D`. This memo gives the measured jump and the options. It does not
change contract code.

## Result

- The jump is real. At a 5 % base bonus, the profit goes from 0.50 % of `D`
  at `C / D = 1.005` to 4.74 % of `D` at `C / D = 0.995`.
- The jump is the same for a 7-decimal leg and for a 0-decimal leg.
- No small contract change removes the jump safely. Each candidate either
  makes new bad debt, or lets a liquidator earn more by splitting one
  liquidation into many.
- Recommendation: keep the code (option A). If the owner wants a smaller
  jump on Liqvid collateral, lower its base bonus (option B).

## Payoff shape

Let `x = C / D` and let `b` be the base bonus. The quote comes from
`estimate_liquidation_amount`. The HF-preserving cap from
`max_hf_preserving_bonus_bps` is `x - 1`. See
[bonus and target repayment](../reference/formulas.md#bonus-and-target-repayment).

| Region | Quote | Liquidator profit | Lender loss |
|---|---|---|---|
| `x >= 1 + b` | target formula at `min(curve, cap)` | repayment times bonus | 0 |
| `1 <= x < 1 + b` (band) | `D` at the cap `x - 1` | `C - D` | 0 |
| `x < 1` (insolvent) | `floor(C / (1 + b))` at `b` | `C * b / (1 + b)` | `D - C / (1 + b)` |

The band profit goes to 0 as `x` goes down to 1. The insolvent profit stays
near `D * b / (1 + b)`. Thus the payoff jumps at `x = 1`. The band pays less
than the insolvent arm while `x < 1 + b / (1 + b)`. At `b = 5 %`, this window
is `1 <= x < 1.0476`.

The liquidation threshold does not change the band or the insolvent profit,
because both arms skip the target formula. The testnet Liqvid listing (LT
6000, bonus 500) thus has the same jump as the harness listings below.

## Measured jump

Commands:

    cargo test -p test-harness --test controller payoff_jumps -- --nocapture

Pinning tests:

- `the_liquidator_payoff_jumps_where_collateral_falls_below_debt` in
  `tests/test-harness/tests/controller/liquidation_band_full_close.rs`. One
  7-decimal ETH leg, bonus 500, fee 0, LT 8000, USDC debt of $1,400.
- `zdc_liquidator_payoff_jumps_where_collateral_falls_below_debt` in
  `tests/test-harness/tests/controller/zero_decimal_collateral.rs`. One
  0-decimal leg with 2 to 5 shares, bonus 500, fee 0, LT 7000, USDC debt of
  59.9 % of a $1,000 NAV per share.

Each call offers twice the debt. Profit is the USD value that the liquidator
gets, less the USD value that it pays.

| Leg | `D` | Profit at 1.05 | Profit at 1.005 | Profit at 0.995 | Jump |
|---|---:|---:|---:|---:|---:|
| 7 decimals | $1,400 | $70.0000 | $7.0000 | $66.3333 | $59.3333 |
| 0 decimals, 2 shares | $1,198 | not run | $5.9900 | $56.7624 | $50.7724 |
| 0 decimals, 3 shares | $1,797 | not run | $8.9850 | $85.1436 | $76.1586 |
| 0 decimals, 4 shares | $2,396 | not run | $11.9800 | $113.5248 | $101.5448 |
| 0 decimals, 5 shares | $2,995 | not run | $14.9750 | $141.9060 | $126.9310 |

In each row the jump is 4.238 % of `D`: 0.500 % above and 4.738 % below.
Whole-share rounding does not change the result at these points. The band
close repays all debt, so the leg rounds up and the liquidator gets every
share. The insolvent close reaches the collateral-backed quote, so
`seize_all` takes every share.

At `C / D = 0.995` the lenders lose `D - floor(C / 1.05)`. That is 5.238 % of
`D`, or $73.33 at `D = $1,400`. The hub socializes this loss in the same call.
At `C / D = 1.005` the lenders lose nothing.

## Why no small change is safe

### A monotone payoff needs a zero insolvent bonus

A liquidation that repays `R` at bonus `β` changes the equity by
`C' - D' = C - D - R * β`. Thus a liquidation that leaves no bad debt pays at
most `C - D`. In the band this limit goes to 0 as `x` goes to 1. A payoff that
never rises as `x` falls must then pay 0 for all `x < 1`. That is the v1.0.0
behavior: no liquidator closes an insolvent account, and more bad debt stays
open. The band cannot pay more without new bad debt.

### A continuous payoff lets a liquidator split for profit

A continuous payoff needs an insolvent bonus that goes to 0 as `C` goes up to
`D`. An example is `β(x) = min(b, (1 - x) / (2x - 1))`. It pays
`min(C * b / (1 + b), D - C)`, and each lender loss is not more than today.

This bonus depends on `x`. An insolvent liquidation seizes `R * (1 + β)` for
`R`, so it lowers `x`. The next part then gets a higher bonus. A real-valued
model (not a harness run) gives these profits:

| Start `x` | One call | 1000 equal parts | Today |
|---:|---:|---:|---:|
| 0.999 | 0.10 % of `D` | 1.28 % of `D` | 4.76 % of `D` |
| 0.995 | 0.50 % of `D` | 2.58 % of `D` | 4.74 % of `D` |
| 0.99 | 1.00 % of `D` | 3.34 % of `D` | 4.71 % of `D` |

Today the insolvent bonus is a constant of the collateral mix. The Certora
rule `split_liq_chain_bound_holds_when_health_never_recovers` uses this fact.
The rule would not hold for `β(x)`. To stop the split, the insolvent quote must
be all or nothing. That change touches `curve.rs`, `math.rs`, the Certora
specifications and the liquidator tools. It is not small.

The continuous payoff also removes the incentive near `C = D`. It gives a lower
lender loss only when a liquidator acts at a profit below
`D * b / (2 * (1 + b))`, which is 2.38 % of `D` at `b = 5 %`. A liquidator of an
illiquid, allowlisted share can need more margin than that.

### The audit option (b) makes new bad debt

Option (b) of the audit quotes `floor(C / (1 + b))` while
`x < 1 + b / (1 + b)`. In the band that leaves `D - C / (1 + b)` of debt with no
collateral. At `x = 1.005` this is 4.29 % of `D`, or $60 at `D = $1,400`. At
`x = 1` it is 4.76 % of `D`. Today the band closes these accounts with no bad
debt. This option is not acceptable.

## Facts for the decision

- Only a price move or interest accrual moves `x`. A liquidation of a solvent
  account does not lower `C / D`, because the HF-preserving cap keeps
  `1 + bonus <= C / D`. A liquidator cannot push an account across `C = D`.
- A liquidator that waits for `C < D` must first let larger payoffs go. At the
  band top `x = 1 + b` the profit is `b * D`: $70 at `D = $1,400`, above the
  $66.33 below `C = D`.
- The highest profit comes before the band. The harness gives 16.30 % of `D`
  at `x = 1.163` for LT 8000 (a probe, not a pinned test). The real-valued
  model agrees with it. The model gives 29.45 % at `x = 1.2945` for LT 7000,
  and 48.25 % at `x = 1.4826` for LT 6000 with the default curve.
- The gain from a wait is at most `D * b / (1 + b) - (x - 1) * D` for each
  account. It stays in one account, and the hub socializes it.
- A NAV step that goes over the band makes the window not important. The
  account goes directly below `C = D`, and no liquidator had a choice.
- An insolvent account with collateral at or below $5 qualifies for
  permissionless cleanup. On testnet, a NAV of $1 and fewer than 6 shares is
  in this range.

## Options

### A. Keep the code (recommended)

No contract change. The pinning tests above hold the payoff shape. The
residual risk is a wait gain of at most `D * b / (1 + b)` for each account:
4.76 % of `D` at `b = 5 %`.

### B. Lower the base bonus of Liqvid collateral

This is a listing change only. The jump bound `D * b / (1 + b)` falls with `b`:

| Base bonus | Jump bound | Window top |
|---:|---:|---:|
| 500 BPS | 4.76 % of `D` | 1.0476 |
| 200 BPS | 1.96 % of `D` | 1.0196 |
| 100 BPS | 0.99 % of `D` | 1.0099 |

The curve bonus below `HF = 1` changes little, because the maximum bonus from
LT controls the ramp. At LT 6000 the maximum is 6666 BPS. At `HF = 0.99` the
curve gives about 2760 BPS with a 500 BPS base, and about 2570 BPS with a
200 BPS base. A lower bonus also lowers the incentive to close an insolvent
account. `update_account_threshold` with `has_risks` applies the lower bonus
to an open position. That call needs a final health factor of at least 1.05,
so apply the change before an account is near liquidation.

### C. Continuous insolvent arm with an all-or-nothing quote

Use `β(x)` from above, and refuse a partial insolvent repayment. This makes
the payoff continuous, but not monotone. It needs new Certora rules, new
liquidator sizing, and a new estimate shape. A liquidator must fund the whole
insolvent quote in one call. Do this only with a full design review.

The audit option (a), a protocol-owned closer with no margin, is an operations
choice. This memo does not include it.

## When the decision changes the payoff

The two pinning tests fail for any change to the payoff at `C / D = 1.005` or
`0.995`. Update them, this memo and
[formulas.md](../reference/formulas.md#bonus-and-target-repayment) together.
