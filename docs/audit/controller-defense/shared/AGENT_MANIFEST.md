# Agent manifest (A001–A110)

Each row is one agent scope. Agents may disagree; synthesis reconciles.

## Wave 1 — Entry surface & auth (A001–A020)

| ID | Scope |
|---|---|
| A001 | Inventory all `ControllerInterface` mutators and their macros (`when_not_paused`, `only_owner`) |
| A002 | Map permissionless_entrypoints.txt claims to actual body auth for supply/repay/liquidate |
| A003 | Map INV-AUTH-02 paths: borrow/withdraw/strategies owner-or-delegate |
| A004 | Account creation / NFT mint defense on supply account_id=0 |
| A005 | Delegate add/remove + position-manager gating |
| A006 | Pause / unpause / guardian flag ratchet surface |
| A007 | Flash-loan ongoing guard vs reentrancy into position flows |
| A008 | View-only entrypoints: input bounds and absence of writes |
| A009 | Admin upgrade / set_* surface vs timelock assumptions |
| A010 | Access-control checker gaps vs undeclared UNGATED paths |
| A011 | `require_auth` placement relative to state reads (TOCTOU) |
| A012 | Third-party supply slot rules (INV-AUTH-03) |
| A013 | Liquidation self-liquidation and Credit seize recipient rules |
| A014 | clean_bad_debt vs force_socialize_bad_debt authority split |
| A015 | Keeper update_indexes / claim_revenue / update_account_threshold bounds |
| A016 | recapitalize measured-receipt and shortfall clamp |
| A017 | renew_account TTL-only mutation defense |
| A018 | Position mode gates on multiply/flash_position |
| A019 | Wasm receiver requirement for flash callbacks |
| A020 | Cross-check STRIDE.md controller rows vs live code |

## Wave 2 — Storage mutations (A021–A040)

| ID | Scope |
|---|---|
| A021 | Account storage layout: meta / supply / debt / delegates keys |
| A022 | Supply path storage writes: shares, meta, usage, events |
| A023 | Borrow path storage writes |
| A024 | Withdraw path storage writes |
| A025 | Repay path storage writes |
| A026 | Liquidation apply storage writes (debt burn, seize, fees) |
| A027 | Bad-debt socialization storage writes |
| A028 | Spoke config / spoke asset / spoke usage key families |
| A029 | Hub / protocol config storage (pool, oracle, NFT, limits) |
| A030 | Flash guard storage flag lifecycle |
| A031 | Position NFT mint/burn coupling to account create/destroy |
| A032 | Strategy finalize storage write batching |
| A033 | Event buffer drain vs durable storage order |
| A034 | TTL renewals: instance vs account vs when skipped in views |
| A035 | Certora harness storage overrides risk of false confidence |
| A036 | Account deletion / empty-position cleanup races |
| A037 | Delegate map mutation integrity |
| A038 | Market index persistence (controller cache vs pool source of truth) |
| A039 | Accumulator / revenue claim storage side effects |
| A040 | Allowed-token constraint: only listed hub assets mutate positions |

## Wave 3 — Money movement (A041–A060)

| ID | Scope |
|---|---|
| A041 | Pool deposit measured-receipt pattern |
| A042 | Pool withdraw measured-transfer pattern |
| A043 | Pool borrow / repay measured amounts |
| A044 | Flash loan principal+fee pullback |
| A045 | Flash position: debt mint, collateral measure, refunds |
| A046 | Multiply legs: borrow → swap → deposit |
| A047 | Swap debt legs money flow |
| A048 | Swap collateral legs money flow |
| A049 | Repay-with-collateral same-asset netting vs swap |
| A050 | Migrate-from-blend money flow and leftover repay |
| A051 | Liquidation Transfer seize token outflow |
| A052 | Liquidation Credit seize share credit (no token move) |
| A053 | Protocol fee skim on liquidation |
| A054 | Refund paths after overpay / excess recap |
| A055 | Token contract lying about transfer amount defenses |
| A056 | Swap aggregator min-out / slippage defenses from controller |
| A057 | Destination `to` option hijack risks |
| A058 | Controller balance delta measurement correctness |
| A059 | Rounding direction favor protocol on money paths |
| A060 | Cross-asset dust / dust-threshold bad-debt interaction |

## Wave 4 — Input validation (A061–A075)

| ID | Scope |
|---|---|
| A061 | Amount sign / zero / overflow validation (common validators) |
| A062 | Vec length bounds and duplicate hub-asset rejection |
| A063 | Spoke id / hub id existence and active checks |
| A064 | Asset listed-in-spoke and flag checks (supply/borrow/seize) |
| A065 | Oracle price freshness / sanity band usage on risk paths |
| A066 | Position limits (max supply/debt slots) |
| A067 | Min borrow collateral floor |
| A068 | Mode / SeizeMode enum exhaustive handling |
| A069 | Callback `data` / swap Bytes size and trust |
| A070 | refund_assets uniqueness and allowlist in flash_position |
| A071 | Blend pool approval check on migrate |
| A072 | Health-factor / post-pool risk gate validation |
| A073 | Interest model / market params read-side trust |
| A074 | Panic vs assert_with_error consistency |
| A075 | Fuzz/proptest coverage vs validation surface gaps |

## Wave 5 — Spoke usage tracking (A076–A085)

| ID | Scope |
|---|---|
| A076 | SpokeUsageContext apply_entry/exit semantics |
| A077 | Cap enforcement using output indexes after pool calls |
| A078 | When usage persists relative to pool mutation success |
| A079 | Multi-asset batch usage aggregation correctness |
| A080 | Exit no-op on missing row — under-accounting risk? |
| A081 | Index selection: supply vs borrow index for scaled caps |
| A082 | Usage reuse of pool return amounts (not caller inputs) |
| A083 | Cross-spoke isolation of usage maps |
| A084 | Liquidation / strategy paths that skip or double-count usage |
| A085 | Tests and Certora rules covering spoke usage |

## Wave 6 — Storage/cache optimizations (A086–A100)

| ID | Scope |
|---|---|
| A086 | Cache field inventory and invalidation rules |
| A087 | Prefetch prices / market indexes batching |
| A088 | pool_address / pool_sync_data memoization |
| A089 | spoke_config / spoke_assets memoization |
| A090 | verified_hubs memoization correctness |
| A091 | spoke_usage embedded in Cache lifecycle |
| A092 | Event update buffers (supply/debt) coalesce behavior |
| A093 | new vs new_view TTL side effects |
| A094 | Avoided re-reads after pool sync — staleness risks |
| A095 | Cross-contract read savings vs correctness tradeoffs |
| A096 | Account load shapes (borrow-only / supply-only / full) |
| A097 | Write batching on finalize_position_flow |
| A098 | Market index cache vs live accrual races within one tx |
| A099 | Optimization that skips a security check (hunt) |
| A100 | Dead cache paths / unused memo maps |

## Wave 7 — Gap hunt & impact (A101–A110)

| ID | Scope |
|---|---|
| A101 | Synthesize undefended money-movement gaps from A041–A060 |
| A102 | Synthesize validation gaps from A061–A075 |
| A103 | Synthesize spoke-usage gaps from A076–A085 |
| A104 | Synthesize cache/optimization hazards from A086–A100 |
| A105 | Compare known gaps in threat-model.md to live code |
| A106 | Quantify max loss scenarios (single account / market / protocol) |
| A107 | Residual STRIDE likelihood vs agent findings |
| A108 | Missing tests/rules for highest-severity gaps |
| A109 | Cross-agent disagreements log |
| A110 | Final prioritized remediation backlog |
