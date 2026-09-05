# Pool operations and cash review

Revision: `99613335b410f70ff42dd99d13ff530f6adaee67`.
Scope: all of `contracts/pool/src/ops/` and `contracts/pool/src/guards.rs`; essential cache, rounding, fee, auth, and measured-receipt callees. Read-only production review. No scoped production edits.

## Result

No nonprivileged fund-loss exploit confirmed in this scope under honest controller ownership/position books and exact sender-debit tokens. One locally reproduced boundary limitation: liquidation fee shares are clipped against **pre-burn** supply headroom, potentially leaving withheld fees without treasury entitlement. This preserves custody and supplier backing; deployed/controller-cap reachability was not established. Treat as informational/conditional, not a demonstrated attacker extraction.

The pool deliberately trusts its owner for position values and prior transfers. Existing owner-authorized tests that call unfunded `repay` or `recapitalize` demonstrate this boundary, not user reachability. Direct caller-supplied pool positions must not be treated as attacker-controlled without first breaking the controller boundary.

## Coverage

Every production function in the assigned files was read end to end:

| File | Functions |
|---|---|
| `ops/mod.rs` | `synced_market`, `renewed_market`, `load_leg`, `run_batch` |
| `ops/borrow.rs` | `apply`, `accounting`, `mint_debt` |
| `ops/supply.rs` | `apply` |
| `ops/repay.rs` | `apply`, `accounting` |
| `ops/withdraw.rs` | `apply`, `accounting`, `resolve_close_or_partial`, `burn_position`, `gate_and_debit`, `withhold_liquidation_fee` |
| `ops/net_settle.rs` | `apply` |
| `ops/recapitalize.rs` | `apply`, `accounting` |
| `ops/revenue.rs` | `apply`, `accounting` |
| `ops/seize.rs` | `apply` |
| `ops/strategy.rs` | `apply`, `accounting`, `compute_fee` |
| `ops/flash.rs` | `apply`, `prepare`, `terms`, `book_fee`, `finalize`, `invoke_receiver`, `collect_repayment`, `require_balance`; also test/Certora-only `prepare_with_balance` |
| `ops/market.rs` | `create`, `replace_rate_model`, `accrue` |
| `guards.rs` | `require_utilization_below_max`, `require_liquidation_buffer`, `require_backed_market`, `backing_shortfall`, `require_solvent_withdraw_state` |

Essential source inspected: pool `lib.rs`; all production cache methods in `cache/{mod,cash,shares,scale}.rs`; `interest.rs`; `common/src/rates/{scaling,index,value}.rs`; relevant `common/src/math/fp.rs`, `common/src/token.rs`, market-parameter validation and constants. Controller receipt/call sites inspected in `external/pool.rs`, `positions/{debt,supply}.rs`, `positions/liquidation/apply.rs`, `strategies/legs.rs`, and `keepers.rs`. These controller reads establish the pool's boundary; they are not a full controller audit.

MCP graph discovery ran first. Graph `trace_path` omitted some real generic/function-pointer edges (e.g. pool `net_settle::apply` reported no callers despite `lib.rs:231`); authoritative source reads completed these edges.

## Exact bookkeeping notation

After the operation's interest sync, let:

- `C` = market accounting cash in native asset units.
- `B_a` = pool token balance for asset `a`, shared by every hub using that asset.
- `S`, `D`, `T` = raw scaled total supply, debt, and treasury shares; `T` is a subset of `S`.
- `I_s`, `I_b` = raw supply/debt indexes; `R = 10^27`, `q = 10^(27-decimals)`; validated decimals are `0..=18`.
- `s`, `d` = the affected user's raw supply/debt shares.
- `U_s(x) = floor(x*I_s/(R*q))`; `U_b(x) = ceil(x*I_b/(R*q))`.
- `m_s(A) = floor(A*q*R/I_s)`, `b_s(A) = ceil(A*q*R/I_s)`.
- `m_d(A) = ceil(A*q*R/I_b)`, `b_d(A) = floor(A*q*R/I_b)`.
- `F_s(F) = min(floor(F*q*R/I_s), i128::MAX-S)` for an asset fee `F`.

Two-step floor/floor and ceil/ceil implementations reduce to these formulas in the validated decimal domain. `resolve_withdrawal` additionally uses half-up display value as its full-close switch, but **pays the floor value** on full close (`common/src/rates/scaling.rs:103-119`).

Stable success-state invariants:

1. `C >= 0`, `0 <= T <= S`, `D >= 0`, indexes positive in valid state.
2. For exact sender-debit tokens and correctly measured incoming credits: `B_a >= sum_h C_(h,a)`. Excess/donations remain outside cash; equality is not required.
3. Per-market accounting backing is `C + U_b(D) >= U_s(S)` for a solvent book. `backing_shortfall = max(U_s(S) - (C + U_b(D)), 0)`, with saturating native additions/subtractions (`guards.rs:56-60`). This is **not** a token-balance solvency check.
4. Revenue is already inside supplier claims; do not add `U_s(T)` on top of `U_s(S)`.
5. Withdraw/net settle/revenue claim additionally forbid `S'=0 && D'>0`. This guard is narrowly structural, not the full backing inequality (`guards.rs:64-67`).

## Every money flow

All deltas below exclude interest accrued immediately before the operation.

| Flow | Token movement | Book delta and result |
|---|---|---|
| Supply `A` | Controller measures payer→pool receipt `A` **before** the pool call | `C+=A`, `S+=m_s(A)`, user `s+=m_s(A)`; no pool token transfer. Preexisting backing shortfall rejects entry. Positive `A` must mint positive shares. |
| Borrow `A` | Pool→receiver `A` after commit | `D+=m_d(A)`, user `d+=m_d(A)`, `C-=A`; positive amount, `C>=A`, 2% liquidation reserve and post-mint utilization checked. |
| Withdrawal | Pool→receiver net `N` after commit | Resolve full/partial burn `b`; gross `G` is floor position value on full close, otherwise requested `A`. `S-=b`, user `s-=b`, `C-=G`. Normal withdrawal checks utilization and no-orphan-debt guard. |
| Liquidation withdrawal | Pool→liquidator `N=G-F` | First mint `f=F_s(F)` into `S` and `T`; then burn user `b` from `S`; `C-=G-F`. Require `0<=F<=G`, reserves and no-orphan debt; deliberately skip utilization. Mutation reports **gross** `G`, not net `N`. |
| Repayment `A` | Controller first delivers measured `A`; pool returns overpayment `O` directly to payer after commit | If `A>=U_b(d)`, burn full `d`, `O=A-U_b(d)`; else burn `b_d(A)`, `O=0`. `C+=A-O`, `D-=burn`, user debt decreases equally. Mutation reports `A-O`. Refund does **not** debit old `C`: `O` was never booked. |
| Net settlement | None | `G=min(requested,U_s(s),U_b(d))`. Full supply burn only if `G==U_s(s)`, else `min(b_s(G),s)`; full debt burn only if `G==U_b(d)`, else `min(b_d(G),d)`. Burn both totals and both user positions; `C` unchanged. Positive settle must burn both sides positively. |
| Recapitalization `A` | Controller first delivers measured `A`; pool returns refund `A-J` to payer | `J=min(A,backing_shortfall)`, `C+=J`; no shares minted; mutation reports `J`. Refund is from the unbooked part of the current incoming transfer. |
| Claim treasury revenue | Pool→Ownable owner, normally controller; controller forwards observed receipt to accumulator | `N=min(C,U_s(T))`. Zero/nonpositive `N` returns zero. Full claim burns `T`; partial claim burns `ceil(T*N/U_s(T))`. Decrease `T` and `S` equally, `C-=N`; utilization and no-orphan debt checks. |
| Strategy draw | Pool→controller/receiver `A-F` | Mint debt `m_d(A)` and user debt for **gross** `A`; mint fee shares `F_s(F)` into both `S,T`; `C-=A-F`. If `charge_fee=false`, `F=0`. Shared borrow checks conservatively reserve the gross draw. |
| Flash loan | Pool→receiver `A`; callback; pull receiver→pool `A+F` via allowance | No principal book mutation. After exact balance checks, `C+=F`, `S,T+=F_s(F)`. Final pool balance is original `B_a+F`. |
| Seize borrow debt | None | Loss value `L=ceil(d*I_b/R)` in ray units. Supply index is reduced pro rata, floored at `R/1000`; `D-=d`. No `C` delta. Floor-induced residual shortfall is handled by supply guard/recapitalization, not silently erased. |
| Seize deposit shares | None | Existing shares reclassified: `T+=s`, `S` unchanged, require `T<=S`. Controller removes/reassigns the corresponding user entitlement. |
| Create/update/accrue market | None | Creation starts `C,S,D,T=0`, `I_s=I_b=R`. Model update accrues under old params before validated replacement. Accrual changes indexes and treasury/supply shares; never token cash. |

Ordinary amount/share conversions favor the market: exact liability reduction on repay is at most cash paid; exact supplier claim removed on withdrawal is at least cash paid. Net settlement combines both directions. Rounding dust does not create user extraction.

## Signed preconditions reaching cash/token helpers

`Cache::debit_cash` uses signed `i128::checked_sub`, which alone does **not** reject a result below zero. Current production call sites supply the missing checks:

| Helper/path | Nonnegativity and bound before helper |
|---|---|
| `credit_cash` / supply | `ops::load_leg:44` requires `A>=0`; positive converted shares required at `supply.rs:29-33`. |
| `credit_cash` / repay | `A>=0`; shared repayment math rejects negative debt inputs; `net=A-O` checked and `0<=O<=A` on valid debt. Positive net requires positive burn (`repay.rs:45-58`). |
| `credit_cash` / recap | Explicit `A>=0`; shortfall is clamped `>=0`; `J=min(A,shortfall)` is nonnegative (`recapitalize.rs:49-57`). |
| `credit_cash` / flash | Fee comes from nonnegative `u32` bps and positive amount, under verified fee bounds; invoked only after exact repayment (`flash.rs:111-131`). |
| `debit_cash` / borrow | Shared `mint_debt` requires `A>0`, reserves `C>=A`, and liquidation buffer before `borrow.rs:47`. |
| `debit_cash` / withdrawal | `A>=0`, `F>=0`; full/partial math yields `G>=0`; liquidation requires `F<=G`; `require_reserves(N)` before debit at `withdraw.rs:96-103`. |
| `debit_cash` / strategy | Borrow helper first checks `A>0` and `C>=A`; `0<=F<=A`, so `0<=A-F<=C` (`strategy.rs:64-79`, `94-100`). |
| `debit_cash` / revenue | Positive claim is explicitly bounded by `min(C,treasury_actual)`; zero-claim branch returns zero (`cache/shares.rs:54-74`, `revenue.rs:43-47`). |
| `transfer_out` / refunds | Repay `O` and recap `A-J` are nonnegative, bounded by measured prefunding. They are intentionally not bounded by old accounting cash. |
| `transfer_out` / ordinary payouts | Above debit bounds ensure positive amounts are backed in accounting. `transfer_out` no-ops for `amount<=0`, but no valid path uses a negative amount to credit/debit books. |
| Direct flash `transfer`/`transfer_from` | Positive principal, checked principal+fee arithmetic, sufficient book reserves; exact token balance brackets and receiver allowance gate repayment. |

`load_leg` validates amount, not `position.scaled_amount`. Supply/borrow/strategy can accept a negative raw position from the authorized owner because `Ray::from` and checked addition do not validate sign. Their **global** minted delta still derives from nonnegative amount. Such data cannot be supplied directly by an ordinary controller user: controller loads the stored position and builds `PoolAction`. Withdraw/repay reject negative position through nonnegative math; seize/net settle additionally reject it explicitly. This is an owner-boundary hardening observation, not a demonstrated attacker path.

## Ordering, auth, and rollback

- All public mutators have `#[only_owner]` in `contracts/pool/src/lib.rs:100-243`; constructor sets owner. Existing tests verify missing owner auth for market creation and flash loan. This review verified source gates rather than claiming exhaustive no-auth dynamic coverage for every verb.
- Normal outgoing operations update/commit the complete market before transferring. An uncaught token failure propagates through the SDK client and rolls back the containing call/transaction; the pool does not catch and preserve an earlier book update.
- Batch legs reload current storage, so a repeated market sees the previous successful leg's updated state. A later failed leg rolls back the batch and earlier token movement; snapshots are emitted after all batch legs finish. Controller normally aggregates by hub/asset before building user batches. Owner-supplied duplicate positions remain an owner-correctness requirement.
- Flash holds an in-memory accrued cache through the external callback. It checks pool balance after payout, after callback, and after pull (`flash.rs:60-69`, `169-192`). Soroban forbids indirect reentry into the already-active contract; controller flash guards add a second boundary. No mutable-cache overwrite path was identified.
- Exact flash equality rejects a callback-time donation or direct push repayment; payment is an allowance-authorized pull of the fixed total. Failed fee booking also reverts the payout/repayment transaction.
- Controller post-pool risk gates intentionally occur after mutation within the same transaction. They must not be excluded when analyzing pool call reachability.

Receipt evidence: `controller/positions/supply.rs:137-147`; `controller/positions/debt.rs:172-180`; `controller/positions/liquidation/apply.rs:52-72`; `controller/strategies/legs.rs:60-81`; `controller/keepers.rs:49-65`. Share-credit liquidation correctly uses deposit seizure/reclassification, not minting a fresh fee claim (`positions/liquidation/apply.rs:123-135`, `207-219`).

## Token assumptions: conditional custody drift

`transfer_out` never compares pool balances before/after a normal payout (`cache/cash.rs:42-47`). Distinguish these cases:

1. Receiver-side tax with sender debit exactly requested: pool loses `N`, receiver gets `<N`. Book cash still falls `N`, so custody/book invariant holds. Inbound deposits credit measured receipt. This does **not** establish pool insolvency.
2. Sender surcharge: pool loses `N+tax` while cash falls only `N`, reducing `B_a-sum C` by `tax`. Rebases, clawbacks, or mutable/lying balances can break custody independently of ordinary deltas.
3. Flash checks reject sender surcharges, callback balance changes and inexact repayment; strategy borrowing into controller also asserts exact measured output (`controller/positions/debt.rs:275-284`).

Market `verify` only validates decimals/rate/fee parameters (`common/src/types/pool.rs:63-70`), not arbitrary token economics. The live threat model explicitly distrusts requested token delivery (`docs/explanation/threat-model.md:148-152`), forbids enabling flash for inexact assets (`451-453`), and leaves token implementations outside protocol control (`490-493`). Older audit prose uses a SAC-only listing assumption; current executable code does not prove that assumption. Therefore surcharge/rebase concerns are conditional listing risks and must not be promoted from an ordinary receiver-tax fixture. No hostile-token listing/deployment proof was attempted.

## Locally reproduced candidate: pre-burn fee headroom

Status: **validated conditional accounting limitation; no user-extraction finding**.

Paths: `ops/withdraw.rs:55-63` books fee before burning supply; `interest.rs:66-67` calls `protocol_fee_shares`; `common/src/rates/index.rs:94-98` caps new shares to `i128::MAX-S`.

Let `M=i128::MAX`, `H=M-S`, `f_desired=floor(F*q*R/I_s)`, and liquidation burn `b`. Current code mints `min(f_desired,H)`. Burn-first accounting could fit `min(f_desired,H+b)`. Difference is lost treasury share entitlement, while the same entire cash fee is withheld.

The scratch probe uses real SAC (7 decimals), matching market metadata, RAY indexes, valid params, actual prefunding, and only public pool methods. It does not fabricate market storage:

```text
deposit_raw = 1701411834604692317
deposit whole tokens = 170141183460.4692317
gross withdrawal = 100 tokens
withheld fee = 10 tokens
recipient paid = 90 tokens
desired treasury shares = 10000000000000000000000000000
actual treasury shares = 31687303715884105727
treasury claim in native units = 0
cash == pool token balance == 1701411833704692317
```

Actual treasury entitlement is about `0.000000031687303715884105727` tokens instead of ten; the rest remains unowned surplus. This neither overpays the liquidator nor worsens backing. The cap in `protocol_fee_shares` is explicitly documented as saturation. It usually matters only near the numeric ceiling, and no deployed cap/controller-liquidation trace was proven for this particular scale. At a low supply index the same share ceiling corresponds to less asset value, but reaching that state remains to be established. No production fix is proposed by this independent review.

Reproduction files:

- `/private/tmp/astra-pool-cash-probe/Cargo.toml`
- `/private/tmp/astra-pool-cash-probe/src/lib.rs`
- `/private/tmp/astra-pool-cash-probe/Cargo.lock` (seeded from repository lockfile)
- `/private/tmp/astra-pool-cash-probe/test.log`

Command:

```sh
RUSTC_WRAPPER= CARGO_TARGET_DIR=/Users/mihaieremia/GitHub/rs-lending-xlm/target cargo test --offline --manifest-path /private/tmp/astra-pool-cash-probe/Cargo.toml -- --nocapture
```

Result: `1 passed; 0 failed`, then `0` doctests. One unused optional Certora patch warning; no test failure. An initial scratch run resolved newer cached dependency versions and was interrupted before test execution; the successful run used a copy of the repository Cargo.lock and the pinned SDK.

## False-positive checks closed

| Candidate | Disposition |
|---|---|
| Refund steals cash because no reserve debit/check | Refund is the unbooked part of the controller's measured incoming transfer. Unfunded owner tests do not establish user reachability. |
| Signed `debit_cash` permits negative reserves | True helper weakness in isolation; every current production debit call proves nonnegative amount and sufficient reserves. |
| Negative supplied user position creates negative global totals | Owner-only DTO; global mint depends on nonnegative amount, and ordinary controller constructs DTO from stored positions. No attacker path established. |
| Revenue double-counted as supplier liability | Treasury shares are included in `S`; their mint/burn updates both books. Deposit seizure reclassifies rather than mints. |
| Net settlement gives a free full close from half-up display | Shared settlement uses floor supply and ceil debt; positive zero-share burn is rejected. No cash moves. |
| Last supplier closes while debt remains | Withdraw/net/revenue reject `S'=0,D'>0`. Borrow's zero-supply utilization exception is explicit, and positive cash can be residual book dust; not a blanket invariant across every verb. |
| Receiver tax alone desynchronizes cash/custody | Sender loses requested amount, matching cash debit. Receiver's reduced proceeds are separate from pool backing. |
| Liquidation fee bypasses 2% reserve/utilization | Liquidation intentionally skips utilization; fee reduces net payout and adds a backed treasury claim. Ordinary borrower and strategy use the same borrow reserve gate. |
| Fee clipping creates unbacked claims | Opposite direction: fewer treasury claims, same retained assets; conditional treasury entitlement loss only. |

## Verification record and limits

`RUSTC_WRAPPER= cargo test -p pool --offline --lib` completed successfully: **168 passed, 0 failed, 0 ignored**, test execution 9.85 seconds. Included cache roundtrip/directed rounding checks; revenue partial burns; dust rejects; underbacked supply reject; negative seize/net positions; collateral/debt underflow; cash limits; max-utilization gates; flash success/failure/owner-auth; market isolation; duplicate-market seizure; intentionally unfunded owner refund boundary tests.

Production scoped `git diff --stat -- contracts/pool/src common/src contracts/controller/src` remained empty at review end. Existing audit output and `tests/test-harness/tests/astra_audit.rs` were not edited. No deployment, network, fuzzing, or formal-verifier run was performed. Suite success is not a claim that every hypothetical token implementation or controller state was tested.

Memory was used only to locate prior pool/common audit control points and to retain the distinction between hypotheses and validated findings: `MEMORY.md:276-279`, rollout `019f787d-27a5-79a0-9c4d-2b50cc1cb704`. Every reported code fact above was rechecked at the pinned revision.
