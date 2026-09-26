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

The previous listing needed `p / R < 5000 / 6000 = 0.8333`, a drop of
16.67 %. That is outside the ±5 % band and outside the widest band that one
source can have (10 %). The oracle refused the price that had to start the
liquidation.

## 2. The sanity band

A single-source band must satisfy this rule, rounded up:

    (max - min) / (max + min) <= MAX_SINGLE_SOURCE_SANITY_BAND_BPS = 10 %

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
account with debt also needs the price. Supply does not need the price.

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
while `p > 0.9 R`. At `p = 0.95 R` it is 5.3 % above the floor, so the
freeze starts there. Repayment, liquidation and withdrawal continue during a
freeze.

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
| below 0.90 | 1000 bps | up to 6666 bps |

The target range is 500 to 1000 bps. The bonus is 6.88 % when liquidation
starts and 10 % at the band floor.

The HF-preserving cap keeps `1 + b <= C / D`. Inside the band, an account
opened at the LTV limit has `C / D >= 0.849 / 0.5 = 1.698`. The cap is then
69.8 % and does not apply. The insolvent branch needs `C < D`, which is
`p < 0.5 R`. The band branch needs `C / D < 1 + b0`, which is `p < 0.525 R`.
Both are far below the band floor. Inside the band, only the curve branch
applies.

The target HF `H = 1.06` is equal to `LT / LTV`. A liquidation puts the
account back at the health of the LTV limit, and not higher. The repayment
is:

    x = (H × D - LT × C) / (H - LT × (1 + b))

## 5. Borrower loss

The borrower loses the bonus on the repaid amount: `loss = b × x`.

| HF | Repaid, share of `D` | Loss, share of `C` | HF after |
|---|---|---|---|
| 0.99 | 14.2 % | 0.55 % | 1.06 |
| 0.95 | 22.7 % | 1.07 % | 1.06 |
| 0.90 | 33.5 % | 1.98 % | 1.06 |

The largest loss inside one band comes from one jump from `R` to the floor.
For 1,000 shares opened at $1 with $500 of debt, the liquidator repays
$167.78 and receives $184.56 of shares. The loss is $16.78, or 1.68 % of the
collateral value at `R`. After that, HF is 1.06 at the floor. A lower price
fails closed. Only interest can then start a second liquidation in the same
band. A path of 1 % NAV steps costs less: $8.49 (0.85 %).

Inside the band the loss is at most 10 % of the repaid debt, plus the
whole-share rounding in section 8.

The previous listing lost 5.5 % of `C` at HF 0.99 and 27.5 % at HF 0.90.

## 6. Staleness

`MAX_PRICE_STALE_SECONDS` is 93,600 s (26 h). The aggregator refuses a
higher value. Set both the asset value and the feed value to 93,600 s. This
gives a daily post a margin of 2 h. A post that is more than 2 h late fails
closed with `PriceFeedStale`. A cadence slower than daily cannot be set.

The Xoxno adapter has its own limit, `max_stale_seconds` (default 86,400 s).
It measures the time of the write. The adapter owner must set it to at least
93,600 s with `set_max_stale_seconds`. If not, reads fail at 24 h. The limit
applies to every feed of that adapter.

The age is the time of the signature, not the NAV date (G-17). Monitor the
NAV date off chain. The audit recommends that the signer signs the last NAV
again each hour. Then 93,600 s is the budget for a signer outage.

## 7. Supply cap in USD

The supply cap is in whole shares. Set it from a USD limit `E` and the band
ceiling:

    supply_cap = floor(E / (1.03 × R)) = floor(1,000,000 / 1.03) = 970,873

The collateral is then at most $1,000,000 at any accepted price. The debt is
at most `LTV × E = $500,000`. The lenders lose money only when `C < D`, which
is a NAV below `0.5 R`. That needs a gap past the band floor.

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
account: 10 shares, $5 of debt, NAV 0.93, bonus 732 bps. The liquidator pays
$5 and gets 6 shares, not 5.77. Its bonus is 11.6 %. The extra loss of the
borrower is less than one share.

## 9. Accounts with 1 to 5 shares

At NAV $1, these accounts cannot borrow (limit 1 above). They never become
liquidatable.

At NAV $1,000, the whole-unit rules of `whole_unit_repayment` apply. An
account with 1 share cannot borrow (limit 2). Accounts with 2 to 5 shares at
the LTV limit have `D = k × $500`:

| Shares | NAV $943 (HF 0.9996, bonus 689) | NAV $849 (HF 0.8999, bonus 1000) |
|---|---|---|
| 2 | sells 1 share for $882.22, HF after 4.24 | sells 1 share for $771.82, HF after 1.97 |
| 3 | sells 1 share for $882.22, HF after 1.62 | sells 1 share for $771.82, HF after 1.24 |
| 4 | sells 1 share for $882.22, HF after 1.34 | sells 1 share for $771.82, HF after 1.10 |
| 5 | sells 1 share for $882.22, HF after 1.24 | sells 1 share for $771.82, HF after 1.04 |

The curve quote seizes less than one share, so rule 2 raises the repayment to
one share at `p / (1 + b)`. At the floor, 5 shares already quote 1.09 shares
(rule 3), and the seizure keeps 1 whole share.

Rule 1 (full close for one share) needs `k × LT × (1 + b) < 1`. At these
parameters only `k = 1` meets it. A 1-share account with debt is only the rest
of a 2-share account after a sale. Inside the band its HF is at least 1.97.
It becomes liquidatable only after a further NAV fall of about 50 %, outside
the band. Rule 1 then pays the liquidator up to `1 / LT - 1 = 88.7 %`.

## 10. Verification

The harness tests are in `tests/test-harness/tests/controller/liqvid_listing_params.rs`.
They use the parameters above with one Xoxno NAV feed and the Asterizm gate.

    cargo test -p test-harness --test controller lqv_params_

| Test | What it shows |
|---|---|
| `lqv_params_listing_oracle_config_sits_inside_the_on_chain_caps` | The band and 93,600 s pass admission. A floor of 0.80 R and 93,601 s are refused. The supply cap admits 970,873 shares and no more. |
| `lqv_params_max_ltv_account_turns_liquidatable_inside_the_band` | HF 1.06 at open. Not liquidatable at 0.944 R. Liquidatable at 0.943 R, inside the band. Bonus 688 bps. HF 1.06 after. |
| `lqv_params_bonus_at_hf_099_095_090_stays_in_the_target_range` | Bonus 719, 844 and 1000 bps. The bonus that the liquidator gets matches the quote within 2 bps. |
| `lqv_params_nav_sweep_liquidates_inside_the_band_and_fails_closed_outside` | Each NAV step from 0.943 R to 0.849 R liquidates. One step past the floor or the ceiling fails closed. The edges price. |
| `lqv_params_nav_prices_until_the_26_hour_staleness_budget` | A NAV signed 93,600 s ago prices. At 93,601 s liquidation and borrow fail closed. |
| `lqv_params_one_dollar_accounts_below_ten_shares_cannot_borrow` | 1 to 9 shares cannot borrow at $1. The 10-share account closes in full with a whole-share round-up. |
| `lqv_params_two_to_five_thousand_dollar_shares_sell_one_share_inside_the_band` | At $1,000, 1 share cannot borrow. 2 to 5 shares sell exactly 1 share at `p / (1 + b)` and end above HF 1. |

With LT 6000, five of these tests fail. With factor 10000, four fail. With a
band floor of 0.95 R, six fail.

The loss figures in section 5, the stepwise path, and the "HF after" values in
section 9 come from an integer model of `estimate_liquidation_amount`. The
harness checks the bonus, the trigger, the seizure and `HF > 1` for these
cases, but not each loss figure.

## 11. Open items

- One signer posts the NAV (H-10, H-29). Inside the band, a bad signer can
  post the floor. Each account at the LTV limit then loses up to 1.68 % of
  its collateral. A bad signer can also post the ceiling. Borrowers can then
  borrow 3 % more, but no account opens below HF 1 at the true NAV.
- A NAV fall larger than 15.1 % fails closed until a new band is live. On
  mainnet this can take the full timelock delay. Accounts can become
  insolvent in that time.
- The adapter staleness limit (section 6) is not 93,600 s in the config
  files. `configs/testnet/oracle_feeds.json` sets 86,400 s for the adapter in
  `configs/networks.json`. The `LIQVID1039` source in `markets.json` names a
  different Xoxno contract, and `configs/script.sh` does not manage it (G-17).
- This runbook changes `configs/testnet` only. Nothing is applied on chain.
  Apply the curve with `configureSpokeCurves`, the listing with
  `editAssetInSpoke`, and the oracle with a timelocked
  `ConfigureAssetOracle`.
