# Liqvid listing parameters

This runbook gives the listing parameters for a Liqvid deal share as
collateral, and the math that supports them. The share has 0 decimals. The
issuer posts one NAV each day. The share has no market price. Borrowers use it
to borrow USDC or EURC in its own hub.

The values are for the testnet listing `LIQVID1039` (hub 4, config spoke 5,
NAV about $1). A deal with a different NAV uses the same ratios around its own
NAV. This runbook closes the listing items G-10, G-11, G-17 and G-18 of the
2026-09-26 upgrade audit.

## Parameters

`R` is the reference NAV: the NAV that the sanity band is set around.

| Parameter | Value | Config field |
|---|---|---|
| LTV | 5000 bps | `spokes.json` 5 `ltv` |
| Liquidation threshold (LT) | 5300 bps | `liquidation_threshold` |
| Base bonus (`b0`) | 500 bps | `liquidation_bonus` |
| Liquidation fee | 0 bps (required below 3 decimals) | `liquidation_fees` |
| Curve target HF (`H`) | 1.06 | `liquidation_curve.target_hf_wad` |
| Curve HF for full bonus (`K`) | 0.90 | `liquidation_curve.hf_for_max_bonus_wad` |
| Curve factor (`f`) | 598 bps | `liquidation_curve.liquidation_bonus_factor_bps` |
| Band floor | 0.849 × R | `markets.json` `min_sanity_price_wad` |
| Band ceiling | 1.03 × R | `max_sanity_price_wad` |
| Asset staleness | 93,600 s | `max_price_stale_seconds` |
| Feed staleness | 93,600 s | `sources[0].Feed.max_stale_seconds` |
| Adapter staleness | at least 93,600 s | adapter `set_max_stale_seconds` (not in config) |
| Supply cap | floor($1,000,000 / band ceiling) = 970,873 shares | `supply_cap` |
| Borrow cap | 0 | `borrow_cap` |

The previous testnet listing used LT 6000, the default curve (`H` 1.10, `K`
0.80, `f` 10000) and a band of ±5 %. The sections below show why it failed.

Sections 1 to 9 use these preconditions, unless a section gives other ones:

- The account borrowed to the LTV limit at the reference NAV `R`.
- No interest accrued since the borrow.
- The position carries the LT of this listing (section 10).

An account that borrowed at a higher accepted NAV (up to the ceiling
`1.03 R`), or that accrued interest, has more debt for the same collateral.
It becomes liquidatable at a higher NAV, gets the full bonus at a higher NAV,
and becomes insolvent at a higher NAV.

## 1. The liquidation trigger

An account that borrows to the LTV limit has `D = LTV × C` at `R`. Its health
factor is:

    HF = LT × C / D = LT / LTV = 5300 / 5000 = 1.06

When the NAV moves from `R` to `p`, the collateral value moves with it:

    HF(p) = 1.06 × p / R

The account becomes liquidatable when `HF(p) < 1`:

    p / R < LTV / LT = 0.94340      NAV drop > 5.66 %

The harness shows the edge. At `p = 0.944 R` the account is not
liquidatable. At `p = 0.943 R` it is.

Interest also lowers HF. With a flat NAV and a borrow rate of 5 % a year, an
account at the LTV limit reaches HF 1 after about 14 months
(`ln(1.06) / 0.05`). A liquidation caused by interest always has a price
inside the band.

The previous listing needed `p / R < 5000 / 6000 = 0.8333`, a drop of
16.67 %. That is outside the ±5 % band. The oracle refused the price that had
to start the liquidation.

A wider band does not correct LT 6000. The single-source limit (section 2)
lets the ceiling be at most `11 / 9` of the floor. An asymmetric band can
include prices below 0.8333 R, but only with a ceiling below
`0.8333 × 11 / 9 = 1.0185 R`.
The lowest floor that still prices `R` is `9 / 11 R = 0.8182 R`, in the band
`[0.8182 R, R]`. In that band the liquidation range is only 0.8182 R to
0.8333 R, HF cannot go below `1.2 × 0.8182 = 0.982`, and any NAV above `R`
fails closed.

## 2. The sanity band

A single-source band must satisfy this rule. The contract rounds the left
side up to whole bps:

    (max - min) / (max + min) <= MAX_SINGLE_SOURCE_SANITY_BAND_BPS = 1000 bps

The left side is the half-width of the band divided by its midpoint
`(max + min) / 2`. The rule does not limit the room below `R` alone. It
limits `max / min` to `11 / 9 = 1.2222`.

The floor comes from the curve. At the floor, an account opened at `R` with
the LTV limit must reach the full-bonus HF `K`:

    floor = R × K × LTV / LT = R × 0.90 × 0.94340 = 0.84906 R  ->  0.849 R
    HF at the floor = 1.06 × 0.849 = 0.89994  (at or below K)

The ceiling limits a bad high price. A borrow at the ceiling must not open
below HF 1 at `R`. That needs a ceiling below `LT / LTV = 1.06`:

    ceiling = 1.03 R
    HF at R of a borrow made at the ceiling = 1.06 / 1.03 = 1.029

The band width is inside both on-chain limits:

    (1.03 - 0.849) / (1.03 + 0.849) = 0.181 / 1.879 = 9.633 %
    rounded up: 964 bps <= 1000 bps   (MAX_SINGLE_SOURCE_SANITY_BAND_BPS)
    rounded down: 963 bps >= 50 bps   (MIN_SANITY_BAND_BPS)

A symmetric band cannot do this. With the same room above and below, the
10 % limit puts the floor at `0.90 R`. There, HF is `0.954` and the bonus
stops at 8.31 %. A lower NAV then fails closed.

Every price below the floor or above the ceiling fails closed with
`SanityBoundViolated`. Borrow and liquidation stop. A withdrawal from an
account with debt also needs the price. Supply does not need the price,
except when it runs the refresh gate of section 10.

## 3. How the band moves with each NAV post

`ConfigureAssetOracle` moves the band. Only the owner can propose it, and it
waits for the standard timelock delay. `configs/networks.json` sets 12
ledgers today; `TIMELOCK_MIN_DELAY_LEDGERS` suggests about 2 days. The ORACLE
role can only make the band narrower (`set_sanity_band`). It cannot move the
band.

For each posted NAV `p`, do the step in the row that matches:

| Posted NAV `p` | Step |
|---|---|
| `0.99 R <= p <= 1.015 R` | No step. |
| `1.015 R < p <= 1.03 R` | Propose a band around `p`: `R = p`. |
| `0.95 R <= p < 0.99 R` | Propose a band around `p`: `R = p`. |
| `0.849 R <= p < 0.95 R` | Freeze the listing (`tightenAssetFlags`, `frozen`). Propose a band around `p`. Clear the freeze (`relaxAssetFlags`) after the new band is live. |
| `p < 0.849 R` or `p > 1.03 R` | Prices fail closed. Freeze the listing. Make sure that the NAV is correct. Then propose a band around `p`. |

When you propose a new band, also set the supply cap again for the new
ceiling (section 7).

The freeze stops new borrows at a low NAV. A borrow at the LTV limit at `p`
becomes liquidatable at `0.9434 p`. That point stays above the floor only
while `p > 0.9 R`. At `p = 0.95 R` that point is `0.8962 R`, which is
5.6 % above the floor (`0.8962 / 0.849 = 1.056`), so the freeze starts
there. Repayment, liquidation and withdrawal continue during a freeze.

A daily NAV of private credit moves slowly. At 10 % a year, the NAV needs
about 55 days to move 1.5 % up. The 3 % of room above `R` lasts about 110
days. That is much longer than a timelock delay of some days.

## 4. The bonus curve

The controller computes the bonus with `max_bonus_for_threshold` and the
spoke curve. For one collateral leg, the proportion is `LT`:

    M = floor(10000 × (10000 - 5300) / 5300) = 8867 bps
    s(HF) = min(1, (H - HF) / (H - K)) = min(1, (1.06 - HF) / 0.16)
    b(HF) = b0 + f × (M - b0) × s(HF) = 500 + 0.0598 × 8367 × s(HF)

The factor 598 makes the bonus exactly 10 % at `K`:
`0.0598 × 8367 = 500.3`, rounded to 500 bps.

| HF | Bonus (this listing) | Bonus (previous listing) |
|---|---|---|
| just below 1 | 688 bps | 2557 bps |
| 0.99 | 719 bps | 2761 bps |
| 0.95 | 844 bps | 3583 bps |
| 0.90 | 1000 bps | 4611 bps |
| 0.8895 | 1000 bps | 4825 bps (peak) |
| 0.85 | 1000 bps | 4166 bps (cap) |
| 0.80 | 1000 bps | 3333 bps (cap) |

The target range is 500 to 1000 bps. The bonus is 6.88 % when liquidation
starts and 10 % at the band floor.

The previous-listing column includes the HF-preserving cap (next
paragraph). For one leg with LT 6000, the cap is `HF / 0.6 - 1`. It is lower than the default
curve below HF 0.8895. Thus the effective bonus of the previous listing peaks
at about 4,825 bps at HF 0.8895 and then falls with HF. It never reaches
`M = 6666` bps.

The HF-preserving cap keeps `1 + b <= C / D`. Inside the band, an account
opened at the LTV limit has `C / D >= 0.849 / 0.5 = 1.698`. The cap is then
69.8 % and does not apply. An account that borrowed at the ceiling has
`C / D >= 0.849 / (0.5 × 1.03) = 1.649`, and the cap still does not apply.
The insolvent branch needs `C < D`, which is `p < 0.5 R`. The band branch
needs `C / D < 1 + b0`, which is `p < 0.525 R`. Both are far below the band
floor. Inside the band, only the curve branch applies.

The target HF `H = 1.06` is equal to `LT / LTV`. A liquidation puts the
account back at the health of the LTV limit, and not higher. The repayment
is:

    x = (H × D - LT × C) / (H - LT × (1 + b))

## 5. Borrower loss

The borrower loses the bonus on the repaid amount: `loss = b × x`.

| HF | Repaid, share of `D` | Loss, share of `C` at that NAV | HF after |
|---|---|---|---|
| 0.99 | 14.2 % | 0.55 % | 1.06 |
| 0.95 | 22.7 % | 1.07 % | 1.06 |
| 0.90 | 33.5 % | 1.98 % | 1.06 |

The largest loss inside one band comes from one jump to the floor. For
1,000 shares opened at $1 with $500 of debt, the liquidator repays $167.48
and receives 217 whole shares, worth $184.23 at the floor. The loss is
$16.75. After that, HF is 1.06 at the floor. A lower price fails closed. Only
interest can then start a second liquidation in the same band.

An account that borrowed at a higher accepted NAV has more debt, so the same
jump costs it more. The table gives the loss of 1,000 shares at the LTV
limit, with no interest. The loss is a share of the collateral value at `R`
and, in brackets, of the value at the floor. The model column has no whole
shares.

| Borrowed at | HF at the floor | Shares seized | Loss | Model loss |
|---|---|---|---|---|
| `R` | 0.900 | 217 | $16.75, 1.67 % (1.97 %) | 1.68 % (1.98 %) |
| `1.015 R` | 0.887 | 238 | $18.37, 1.84 % (2.16 %) | 1.84 % (2.17 %) |
| `1.03 R` (ceiling) | 0.874 | 260 | $20.07, 2.01 % (2.36 %) | 2.01 % (2.37 %) |

A path of 1 % NAV steps (0.99 R, 0.98 R, ..., 0.85 R, then the floor) costs
less. For an account opened at `R`, it liquidates only at 0.94 R and at
0.88 R. The total loss is $8.76 (0.88 % of the value at `R`), and HF is 1.022
at the floor.

The harness test `lqv_params_floor_jump_costs_more_than_one_percent_steps`
pins the shares seized, each loss in dollars and the step path. It also
checks that HF is between 1.055 and 1.065 after each jump, and between 1.020
and 1.025 at the floor after the steps.

Inside the band the loss is at most 10 % of the repaid debt, plus the
whole-share rounding in section 8.

The previous listing lost 5.5 % of `C` at HF 0.99 and 27.5 % at HF 0.90.

## 6. Staleness

`MAX_PRICE_STALE_SECONDS` is 93,600 s (26 h). The aggregator refuses a
higher value. Set both the asset value and the feed value to 93,600 s. This
gives a daily post a margin of 2 h. A post that is more than 2 h late fails
closed with `PriceFeedStale`. A cadence slower than daily cannot be set.

The Xoxno adapter (`contracts/xoxno-oracle`) has its own limit,
`max_stale_seconds`. Its default is 86,400 s (`DEFAULT_MAX_STALE_SECONDS`).
It measures the time of the write. If the limit stays at 86,400 s, reads fail
with `StaleData` at 24 h and the 93,600 s of the aggregator never applies.
The limit applies to every feed of that adapter.

The owner of the Liqvid NAV adapter must set the limit to at least 93,600 s.
`set_max_stale_seconds` is an owner-only call. It fails with
`InvalidSubmissionAge` when the value is below the adapter's
`max_submission_age_seconds`:

    stellar contract invoke --id <Liqvid NAV adapter> --network testnet \
      --source-account <adapter owner> -- set_max_stale_seconds --seconds 93600

Use the adapter that the `LIQVID1039` source names, not the adapter in
`configs/networks.json`. On testnet these are two different contracts:

| Config | Adapter | Staleness in config |
|---|---|---|
| `markets.json` `LIQVID1039` `sources[0]` `Xoxno.contract` | `CDHPWYORLKTMN2XAG7Q7KMSBRNVEO3ZECDDKOAZ3EIZCLS42JOLX4JKL` | none |
| `networks.json` `testnet.xoxno_oracle_adapter` | `CDYX4ZEO556YZDYDJLUE5XQUE2DLWVFJDTBJJGF7HYQP5HK5NICNTQ6F` | `oracle_feeds.json` `max_stale_seconds` 86,400 |

`configureOracleWindows` and `setOracleMaxStale` in `configs/script.sh` call
only the `networks.json` adapter. They cannot set the limit of the Liqvid NAV
adapter. `validateConfigs` warns that the two contracts differ. A read-only
simulation of `max_stale_seconds` on 2026-09-26 gave 86,400 s on both
contracts.

The harness uses a mock adapter that has no limit of its own. The staleness
test in section 12 does not cover the adapter limit.

The age is the time of the signature, not the NAV date (G-17). Monitor the
NAV date off chain. The audit recommends that the signer signs the last NAV
again each hour. Then 93,600 s is the budget for a signer outage.

## 7. Supply cap in USD

The supply cap is in whole shares. Set it from a USD limit `E` and the band
ceiling:

    supply_cap = floor(E / (1.03 × R)) = floor(1,000,000 / 1.03) = 970,873

The collateral is then at most $1,000,000 at any accepted price. At borrow
time, the debt is at most `LTV × E = $500,000`. Interest adds to it later.

The lenders lose money only when `C < D`. For an account that borrowed at
`R` with no interest, that is a NAV below `0.5 R`. For an account that
borrowed at the ceiling, it is a NAV below `0.515 R`. When interest has
multiplied the debt by `g`, these points move to `0.5 g R` and `0.515 g R`.
With little interest, each point is far below the band floor, so a loss needs
a gap past the floor.

## 8. Minimum position

Three limits apply:

1. `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD` ($5). The LTV-weighted collateral
   must be at least $5 to hold debt: `N × p × 0.5 >= 5`. At $1, `N >= 10`
   shares. At the band floor, `N >= 12`.
2. `MIN_WHOLE_UNIT_COLLATERAL` (2 shares). This limit applies only when the
   NAV is above $5 (`2 × p × 0.5 >= 5`).
3. `BAD_DEBT_USD_THRESHOLD` ($5). A quote that leaves less than $5 of debt
   becomes a full close. Permissionless bad-debt cleanup needs collateral of
   $5 or less (5 shares at $1).

A full close rounds the seizure up to whole shares. The liquidator can get up
to one share more than `D × (1 + b) / p`. The harness shows the smallest
account: 10 shares that borrowed $5 at $1, NAV 0.93, bonus 732 bps. The
liquidator pays $5 and gets 6 shares, not 5.77. Its bonus is 11.6 %. The
extra loss of the borrower is less than one share.

## 9. Accounts with 1 to 5 shares

At NAV $1, these accounts cannot borrow (limit 1 above). They never become
liquidatable.

At NAV $1,000, the whole-unit rules of `whole_unit_repayment` apply
([formulas](../formulas.md#bonus-and-target-repayment)). `U` is the value of
one share, `b` the bonus and `D` the debt. The controller applies the first
rule that matches:

1. **Full close.** One share at `U / (1 + b)` covers all of `D`, with one
   base unit of each debt token to spare. The liquidator repays all debt and
   gets one share.
2. **One-share sale.** The curve quote seizes less than one share. The
   repayment rises to one share at `U / (1 + b)`, plus a margin of one
   millionth.
3. **Curve quote.** If no other rule applies, the curve quote stays. The seizure takes whole
   shares and refunds the fraction.

An account with 1 share cannot borrow (limit 2). Accounts with `k` = 2 to 5
shares that borrowed to the LTV limit at $1,000, with no interest, have
`D = k × $500`. The table and the HF bounds below use these preconditions.
More debt gives a lower "HF after".

| Shares | NAV $943 (HF 0.9996, bonus 689) | NAV $849 (HF 0.8999, bonus 1000) |
|---|---|---|
| 2 | sells 1 share for $882.22, HF after 4.24 | sells 1 share for $771.82, HF after 1.97 |
| 3 | sells 1 share for $882.22, HF after 1.62 | sells 1 share for $771.82, HF after 1.24 |
| 4 | sells 1 share for $882.22, HF after 1.34 | sells 1 share for $771.82, HF after 1.10 |
| 5 | sells 1 share for $882.22, HF after 1.24 | sells 1 share for $771.82, HF after 1.04 |

The curve quote seizes less than one share, so rule 2 raises the repayment to
one share at `p / (1 + b)`. At the floor, 5 shares already quote 1.09 shares
(rule 3), and the seizure takes 1 whole share.

Rule 1 (full close for one share) needs `k × LT × (1 + b) < 1`. At these
parameters only `k = 1` meets it. A 1-share account with debt is only the rest
of a 2-share account after a sale. Inside the band its HF is at least 1.97.
It becomes liquidatable only after a further NAV fall of about 50 %, outside
the band. Rule 1 then pays the liquidator up to `1 / LT - 1 = 88.7 %`.

## 10. Existing positions keep their stamped terms

Each supply position stores its own LTV, LT, base bonus and liquidation fee.
It copies them from the spoke listing when it is created
(`get_or_create_supply_position`). After that, the controller uses the
stored values, not the live listing:

- HF uses the stored LT (`calculate_account_risk_totals`).
- The borrow limit uses the lower of the stored LTV and the stored LT.
- The base bonus `b0` is the USD-weighted stored bonus. The maximum bonus `M`
  comes from the stored LT. The fee is the stored fee
  (`get_account_bonus_params`).
- The curve (`H`, `K`, `f`) is not stored. Each liquidation reads it from the
  spoke. `configureSpokeCurves` changes it for all accounts of the spoke at
  the same time.

Thus an `editAssetInSpoke` that lowers LT changes only the positions created
after it and the positions that the controller refreshes. Every borrow and
every withdrawal refreshes the stored LTV of each listed supply position
without a condition. The stored LT, bonus and fee refresh together, and only
in these paths:

| Path | Code |
|---|---|
| A supply of the asset into the account | `merge_supply_leg` calls `refresh_supply_risk_params` |
| A withdrawal of the asset that is not a liquidation and leaves a balance | `merge_withdraw_leg` |
| `update_account_threshold(caller, has_risks = true, account_ids)` | `sync_account_thresholds` |

A liquidation never refreshes them. `update_account_threshold` with
`has_risks = false` refreshes the LTV only.

All three paths use the same gate (`apply_gated_liquidation_params`). A
change favours the liquidator when it lowers LT, raises the bonus or lowers
the fee. For an account with debt, such a change applies only when the
account HF, calculated with the new LT, is at least 1.05
(`THRESHOLD_UPDATE_MIN_HF_RAW`). If the HF is lower, the position keeps its
old LT, bonus and fee, and the call does not fail. A debt-free account always
takes the new values. In the supply path, the gate calculates the HF before
the new supply adds to the collateral. The gate reads the prices of all
assets of the account. When the gate runs, a stale price or a price outside
the band makes the call fail, also a supply.

`update_account_threshold` with `has_risks = true` also checks the account HF
after the refresh. If that HF is below 1.05, the call reverts with
`HealthFactorTooLow` (102), and one such account reverts the whole batch.
Anyone can call it with the auth of `caller`. The controller refuses it while
it is paused or while a flash loan is open.

For this listing, LT moves from 6000 to 5300, and the bonus and the fee do
not change. For an account with one collateral leg, the gate needs
`HF(LT 6000) × 5300 / 6000 >= 1.05`, that is `HF(LT 6000) >= 1.1887`.

The figures below are for an account with one collateral leg that borrowed
to the LTV limit at the NAV in the first column, with no interest. Interest
moves each NAV up in proportion to the debt.

| Borrowed at | Takes LT 5300 at a NAV of | Call reverts below | Liquidatable with LT 6000 below | HF with LT 6000 at the floor |
|---|---|---|---|---|
| `R` | `0.9906 R` or more | `0.875 R` | `0.8333 R` | 1.019 |
| `1.015 R` | `1.0054 R` or more | `0.8881 R` | `0.8458 R` | 1.004 |
| `1.03 R` (ceiling) | `1.0203 R` or more | `0.9013 R` | `0.8583 R` | 0.989 |

An account that borrowed at `q` has HF `1.06 × p / q` with LT 5300 and
`1.2 × p / q` with LT 6000. It takes LT 5300 when `1.06 × p / q >= 1.05`.
Between that NAV and the revert NAV, the call keeps LT 6000 and does not
fail. Below the revert NAV, `1.2 × p / q < 1.05`, and the call reverts with
`HealthFactorTooLow`. The harness test
`lqv_params_restamp_applies_lt_5300_only_above_the_gate` checks the `R` row
at 0.991 R, 0.990 R, 0.876 R and 0.874 R.

A position that borrowed at `R` and keeps LT 6000 becomes liquidatable only
below `0.8333 R`, which is below the band floor. Inside the band, only
interest can make it liquidatable. At the floor, its HF is
`1.2 × 0.849 = 1.0188`, so the debt must grow by more than 1.9 %. A position
that borrowed at `1.015 R` needs more than 0.37 % of debt growth. A position that
borrowed at the ceiling `1.03 R` is liquidatable below `0.8583 R`, inside the
band, with no interest.

## 11. Apply order

Nothing in this runbook is applied on chain. Steps 2 to 4 are timelocked
governance operations: propose, wait for the delay, then execute. You can
propose them at the same time, but execute them in the order below.

1. **Adapter staleness.** The owner of the Liqvid NAV adapter calls
   `set_max_stale_seconds` with 93,600 (section 6). On testnet the owner is
   the `deployer` key (read with `get_owner` on 2026-09-26), so this call is
   immediate. If the owner is a governance contract, this step is a
   timelocked governance operation too. It changes nothing while the
   aggregator limit is 86,400 s.
2. **Oracle.** `make testnet configureMarketOracle LIQVID1039` proposes the
   band `[0.849 R, 1.03 R]` and 93,600 s for the asset and the feed. Set `R` to
   the last posted NAV first (section 3).
3. **Curve.** `make testnet configureSpokeCurves` proposes the spoke 5 curve:
   `H` 1.06, `K` 0.90, `f` 598. It applies at once to all accounts of the
   spoke, also to positions that still carry LT 6000.
4. **Listing.** `make testnet editAssetInSpoke 5 LIQVID1039` proposes LT 5300
   and the supply cap 970,873. Only new positions get LT 5300 at once.
5. **Refresh.** While the NAV is near `R`, call
   `update_account_threshold(caller, true, [id])` for each account that holds
   `LIQVID1039`. An account that borrowed to the LTV limit at `R` passes the
   gate only at a NAV of `0.9906 R` or more. An account that borrowed at a
   higher NAV needs a higher NAV (section 10). Send one account for each
   call, or simulate the batch first.
   Then read each position with `get_account_positions` and list the positions
   that still carry LT 6000. Ask these borrowers to repay debt or to add
   collateral, then call again.

The order has these reasons:

- The band must be live before LT 5300. With LT 5300 and the old floor
  `0.95 R`, a new account at the LTV limit becomes liquidatable at `0.9434 R`.
  The oracle refuses that price.
- The curve must be live before LT 5300. With the default curve, a position
  with LT 5300 at HF 0.99 gets a bonus of about 36 %
  (`500 + 8367 × 0.11 / 0.30 = 3568` bps).
- The adapter must allow 93,600 s before step 2 goes live. If not, reads fail
  at 24 h and the new aggregator limit has no effect.
- The refresh copies the live listing, so it must come after step 4. The gate
  uses the current HF, so it must run while the NAV is near `R`.

## 12. Verification

The harness tests are in `tests/test-harness/tests/controller/liqvid_listing_params.rs`.
They use the parameters above with one Xoxno NAV feed and the Asterizm gate.
Each test lists the share with LT 5300, so all positions carry that LT. The
restamp test is the exception: it lists LT 6000 first and then edits the
listing to LT 5300.

    cargo test -p test-harness --test controller lqv_params_

| Test | What it shows |
|---|---|
| `lqv_params_listing_oracle_config_sits_inside_the_on_chain_caps` | The band and 93,600 s pass admission. A floor of 0.80 R and 93,601 s are refused. The supply cap admits 970,873 shares and no more. |
| `lqv_params_max_ltv_account_turns_liquidatable_inside_the_band` | HF 1.06 at open. Not liquidatable at 0.944 R. Liquidatable at 0.943 R, inside the band. Bonus 688 bps. HF 1.06 after. |
| `lqv_params_bonus_at_hf_099_095_090_stays_in_the_target_range` | Bonus 719, 844 and 1000 bps. The bonus that the liquidator gets matches the quote within 2 bps. |
| `lqv_params_nav_sweep_liquidates_inside_the_band_and_fails_closed_outside` | Each NAV step from 0.943 R to 0.849 R liquidates. One step past the floor or the ceiling fails closed. The edges price. |
| `lqv_params_nav_prices_until_the_26_hour_staleness_budget` | A NAV signed 93,600 s ago prices. At 93,601 s liquidation and borrow fail closed. The mock adapter has no limit of its own (section 6). |
| `lqv_params_one_dollar_accounts_below_ten_shares_cannot_borrow` | 1 to 9 shares cannot borrow at $1. The 10-share account closes in full with a whole-share round-up. |
| `lqv_params_two_to_five_thousand_dollar_shares_sell_one_share_inside_the_band` | At $1,000, 1 share cannot borrow. 2 to 5 shares sell exactly 1 share at `p / (1 + b)` and end above HF 1. |
| `lqv_params_floor_jump_costs_more_than_one_percent_steps` | One jump to the floor seizes 217, 238 and 260 of 1,000 shares, for a loss of $16.75, $18.37 and $20.07, from accounts that borrowed at `R`, `1.015 R` and `1.03 R`. HF is 1.055 to 1.065 after each jump. 1 % NAV steps liquidate at 0.94 R and 0.88 R only, for a loss of $8.76, and HF is 1.020 to 1.025 at the floor. |
| `lqv_params_restamp_applies_lt_5300_only_above_the_gate` | After the edit from LT 6000 to LT 5300, `update_account_threshold(caller, true, [id])` applies LT 5300 at `R` and 0.991 R. It keeps LT 6000 without a revert at 0.990 R and 0.876 R. It reverts with `HealthFactorTooLow` at 0.874 R and keeps LT 6000. |

If you change one constant in that file, these tests fail:

| Change | Failed tests (of 9) |
|---|---|
| `LT` 5300 to 6000 | 7 |
| `BONUS_FACTOR` 598 to 10000 | 6 |
| `BAND_FLOOR_PER_MILLE` 849 to 950 | 9 |

The restamp test does not fail when `LT` is 6000, because `LT` is then equal
to `PREVIOUS_LT` and the edit changes nothing.

The table in section 5 and the "HF after" values in section 9 come from an
integer model of `estimate_liquidation_amount`, without whole shares. For
those cases the harness checks the bonus, the trigger, the seizure and
`HF > 1`, but not each loss figure.

## 13. Open items

- One signer posts the NAV (H-10, H-29). Inside the band, a bad signer can
  post the floor. An account that borrowed to the LTV limit at `R` then
  loses about 1.68 % of its collateral value at `R` (section 5). An account
  that borrowed at `1.015 R` loses 1.84 %, and one that borrowed at the
  ceiling `1.03 R` loses 2.01 %, on the same basis. A bad signer can also
  post the ceiling. Borrowers can then borrow 3 % more, but no account opens
  below HF 1 at the true NAV. If the signer then posts the floor, these
  accounts lose 2.01 %.
- A NAV fall larger than 15.1 % fails closed until a new band is live. On
  mainnet this can take the full timelock delay. Accounts can become
  insolvent in that time.
- The Liqvid NAV adapter reads 86,400 s today (section 6). No file in
  `configs/` holds its limit, and `configs/script.sh` cannot set it. The
  `networks.json` adapter and `oracle_feeds.json` are for a different
  contract (G-17).
- Positions opened before the listing edit keep LT 6000 until a refresh
  passes the gate (section 10). A position that keeps LT 6000 and borrowed at
  the ceiling becomes liquidatable inside the band, below `0.8583 R`.
- This runbook changes `configs/testnet` only. Section 11 gives the steps to
  apply it.
