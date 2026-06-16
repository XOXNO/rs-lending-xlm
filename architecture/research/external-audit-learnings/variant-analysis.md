# Variant Analysis — Antipattern Hunt Across rs-lending-xlm

For each concrete finding pattern from the corpus, the 7 subsystem agents grepped/Explored our code for the antipattern. Result: the dangerous variants are **absent or guarded**; a small number of benign/latent residuals are noted. `file:line` cited throughout; nothing was modified.

## Arithmetic & rounding

| Antipattern | Status | Where checked / note |
|-------------|--------|----------------------|
| divide-before-multiply in rate/index/share math | **clean** | All ratios route through `common/src/math/fp_core.rs:13-38` `mul_div_*` (widen to I256, x*y then /d, single final round). 0 hits of `.div(…).mul(`. |
| store-rate-then-re-divide (Certora/Kamino redeem>deposit) | **absent** | Scaled positions store `scaled_ray` and reconstruct `scaled*index` (single mul); no stored-rate re-division. |
| `^` used as power instead of `.pow()` | **absent** | `fp_core.rs:58` uses `.checked_pow`; 0 misuse hits. |
| phantom overflow (intermediate product overflows though result fits) | **clean** | `to_i256_operands` widens all operands before multiply; only the final quotient narrows via `to_i128` mapping overflow→`MathOverflow`. |
| division returns 0 instead of revert on zero denom (SigmaPrime) | **clean** | `I256::div` by zero traps in the Soroban host; index/util paths additionally zero-guard before dividing (`rates.rs:110-149`, `interest.rs:64-78`). Tests `test_ray_div_by_zero_panics`. |
| `*_ceil` that actually floors (Sec3/Kamino) | **clean** | `mul_div_ceil`/`rescale_ceil` add 1 on nonzero remainder, pinned by `fp_core.rs:334-346,498-505`. |
| unchecked i128/u32/u64 subtraction on timestamp/index/ledger-seq deltas | **clean** | Time deltas `saturating_sub` (`interest.rs:13`, `rates.rs:169`); `is_stale` guards `now>feed_ts` before subtracting; `Ray/Wad` `checked_sub` panic on negative. No raw ledger-seq subtraction. |
| missing `overflow-checks` in release profile (Scout crit) | **clean** | `Cargo.toml:59` `overflow-checks = true`; only off-chain `keeper`/excluded `fuzz` have own profiles. |
| rounding direction (HF floor / util ceil / debt up) | **clean** | Collateral/HF floor (`helpers/math.rs:27-31,149`), debt ceil (`:35-39,249-255`), user-credit floor / user-debit ceil in cache (`cache.rs:144,157`). |

## Storage, auth & Soroban platform

| Antipattern | Status | Where checked / note |
|-------------|--------|----------------------|
| unbounded `Vec`/`Map` in Instance storage (OtterSec/Veridise) | **clean** | All instance keys are scalars; growing registries are keyed-Persistent and capped (approvals 16 `instance.rs:14`, pools 256 `pools.rs:25`, e-mode assets 64 `emode.rs:52`). |
| `require_auth` auth-tree phishing / sub-call inherits asset auth (RV) | **guarded** | User auth top-level only; aggregator called with contract identity; sole downstream grant is a scoped `InvokerContractAuthEntry` for one exact transfer, `sub_invocations` empty (`swap.rs:108-128`). |
| two-step deploy/init front-running; salt not admin-bound | **clean** | Atomic `__constructor`; internal deploys use `deploy_v2` with deployer-bound salt + one-time guards. |
| storage TTL not extended on config write; logical-expiry==entry-TTL | **clean** | Config writes re-arm TTL same invocation; `PendingTransfer.live_until_ledger` stored as data, checked independently of TTL. |
| `unwrap`/`expect`/`assert!`/`panic!` on reachable paths | **clean** | 0 non-test `assert!`; production `unwrap/expect/panic` are either money-site fail-closed guards (`fp.rs`) or `get_owner().unwrap()` on the always-set owner. |
| `Map::get` on a missing key (panic surface) | **clean** | Wrapped by `expect_invariant`→typed `InternalError` or `get_*_or_panic`→typed `PositionNotFound`; cache keys populated by a prior "supported" read. |
| `unsafe {` / `core::mem::forget` / blanket `#[allow]` | **clean** | 0 of each; the 6 `#[allow]` are narrow+justified (view arg counts, macro-proxy dead_code, oracle enum size). |
| `Vec`/`Map` `Val` round-trip failure on retrieval (Veridise) | **clean** | Stored values are `#[contracttype]` structs decoded via typed `storage().get::<T>()`; no raw `ScVal` stored-then-trusted. |
| events emitted before mutation / SEP-41 non-compliance | **clean** | Events emitted post-mutation with post-state payloads; no aToken share token (underlying moves via SAC which emits its own transfer) — *indexer must read SEP-41 transfers from the SAC, by design*. |

## Access, governance & flash

| Antipattern | Status | Where checked / note |
|-------------|--------|----------------------|
| admin/owner read from a function arg (Scout) | **clean** | Only `__constructor` takes admin arg (documented exception); runtime reads use stored `get_owner`/`get_admin`. |
| `set_admin` ignores `new_admin` / no new-admin auth (Aquarius ME-01) | **absent** | `accept_transfer` writes the stored pending address and requires *its* auth before promotion. |
| upgrade path not timelocked (Aquarius H-01) | **clean** | Controller `upgrade` is `#[only_owner]` (owner=governance), reachable only via timelocked `propose_upgrade_controller`; governance self-upgrade timelocked; both auto-pause. |
| global mutable timelock re-arms pending proposals (Morpho L-06) | **absent** | Per-op absolute `ready_ledger` snapshotted at schedule; delay updates monotonic non-decreasing, non-zero. |
| storage-mutating entrypoint missing `require_auth`/role | **clean** | 27 `#[only_owner]` config/upgrade; keeper/revenue `#[only_role]`; user verbs `require_auth` + `require_account_owner_match`; fuzz `privileged_auth_rejects.rs` proves rejection-without-auth. |
| flash/strategy path skips a normal-path gate (Blend M-01/M-01-CERT) | **clean** | `borrow_for_strategy` reuses `validate_borrow`; HF/LTV/`MAX_POSITIONS` at `strategy_finalize`; cap passed to pool; flash gates on pause + `is_flashloanable`. Controller `flash_loan` creates no positions (no `MAX_POSITIONS` bypass). |
| state cached across an external call then written back stale (Aave PVE-011) | **absent** | Pool index state taken from each call's return value and recorded after; only the contract's own token balance is snapshotted across the router call (for delta checks), never persisted. |
| counterparty address used without identity validation (Orbit/BL-003) | **clean** | Pool deterministically self-deployed + stored; flash-loan pool is the canonical singleton; aggregator is the governance-set, contract-validated address. |

## Liquidation & caps

| Antipattern | Status | Where checked / note |
|-------------|--------|----------------------|
| plain subtraction in bad-debt/liquidation totals | **guarded** | `interest.rs:85` `total - capped` guarded by `capped=min(bad_debt,total)`; all `liquidation_math.rs` subtractions inside `if a>b` guards; pool seize uses `checked_sub_assign`. |
| repaid value reused for both pay and seize (Morpho over-seize) | **correct-by-design** | Seize denominated in settled `repay_usd`; `NormalizedRepaymentPlan::validate` asserts `sum_repaid_usd == repay_usd`; partial seize floors, full seize half-up. |
| missing `liquidator != owner` check | **present** | `liquidation.rs:125-129` asserts `account.owner != liquidator`; harness `test_self_liquidation_rejects`. |
| position flag not cleared on full close (Aave R1) | **clean** | Position existence *is* the flag (Map entry); `update_or_remove_debt_position` removes the key when `scaled==0`. |
| socialization gated on `collateral==0` (leave-1-wei) | **fixed** | Uses `≤$5` band, not `==0`. *(Introduces the inverse residual: the ($5,debt) gap band — backlog #2.)* |
| util/cap check on borrow but absent on withdraw | **clean** | `require_utilization_below_max` on borrow + withdraw + strategy; withdraw skips only when `is_liquidation` (exits never blocked). |
| cap comparison `<` vs `<=` / per-call vs cumulative / round-down increment | **clean** | Balance caps inclusive `<=`; `PoolsList` `<` before push; caps compare aggregate `(total+delta)` not per-call. |
| reserve-list slot loop missing `break` / `>=` vs `>` (Aave/Kamino) | **absent** | No fixed-slot array; append-only Vec with dedup early-return + `<MAX` bound; removal `position()+remove(idx)` single-shot. |
| ledger-sequence / block-delta subtraction in liquidation (Blend BLRC-018) | **absent** | Synchronous liquidation, no auction; only timestamp delta uses `saturating_sub`. |

## Residuals surfaced by the hunt (not code bugs)

1. **Position limit 10/10 vs liquidation-budget-proven 5/5** (backlog #1) — the cap holds on every path, but a max-width account is not *proven* liquidatable within the Soroban budget.
2. **($5, total_debt) bad-debt gap band** (backlog #2) — the `≤$5` socialization fix creates an inverse band with no single guaranteed terminal call.
3. **`Cache` no drop-guard** (backlog #5) — no current path skips `save()`, but nothing prevents a future one from doing so.
4. **`ReflectorSourceConfig.resolution_seconds`** — a genuine instance of the Sec3 "stored-but-unread" shape, but benign: it's a listing-time cadence sanity check, not a runtime freshness gate (staleness is enforced separately and *is* read). **Resolved:** clarifying doc comment added (`interfaces/controller/src/types/oracle.rs`) noting it's config-time-only and does not gate reads.
5. **AAVE-D-050 Certora rule** points at the live `reserves()` view rather than accounted `cash` — code is correct (`require_reserves`→`cash`), proof targets the inflatable value.
