# Native coverage gap audit — 2026-11-22

`make coverage-merged` passed with **14,489/14,647 distinct source lines hit (98.92%)**. The original LLVM `LF/LH` summary is **14,590/14,855 (98.22%)**. These are different denominators: 158 distinct physical source lines have zero hits, while LLVM reports 265 missed line units. The 107-unit difference has no one-to-one list of source lines; compiled instances and sub-line regions can contribute. The full reproducible report is generated at `target/coverage/merged-report.md`.

Of the 158 zero-hit physical lines, 86 are `#[contracttype]` sites that expand into Soroban conversion code and 72 are other Rust lines. This profile includes test-only code under `src/` and measures native tests, not production WASM execution. A hit proves execution, not that an assertion would catch a regression.

Run `make coverage-merged` for the exact lines and `make coverage-branches` for the experimental nightly Rust branch report. The branch target uses an isolated Cargo target directory and writes `target/coverage/branches/merged-report.md`.

The nightly profile covered **1,546/1,632 physical branch outcomes (94.73%)**, leaving 86 unexecuted outcomes. LLVM's per-file summary reports **1,531/1,632 (93.81%)** because compiled instances are counted differently. The generated branch report lists every source condition with a missing outcome. Branch coverage is distinct from line coverage: executing an `if` line does not prove both outcomes were tested.

| Package | Zero-hit lines | Soroban macro sites | Other Rust lines |
|---|---:|---:|---:|
| common | 70 | 64 | 6 |
| controller | 39 | 9 | 30 |
| curator | 5 | 2 | 3 |
| defindex-strategy | 2 | 2 | 0 |
| governance | 7 | 1 | 6 |
| pool | 6 | 2 | 4 |
| position-nft | 1 | 1 | 0 |
| price-aggregator | 8 | 1 | 7 |
| swap-aggregator | 16 | 2 | 14 |
| xoxno-oracle | 4 | 2 | 2 |
| **Total** | **158** | **86** | **72** |

## Remaining zero-hit lines by file

The reasons below identify the control-flow condition or measurement artifact. They do not claim that every guard is reachable from a valid public call. Lines labeled as defensive or redundant need a reachability proof or source refactor before treating 100% as a gate.

| File | Zero-hit lines | Why |
|---|---|---|
| [`common/src/oracle/lp.rs`](../../common/src/oracle/lp.rs) | 80, 159, 160 | Nonpositive LP share supply; two lines are a test-only correction loop in src. |
| [`common/src/oracle/lp_stable.rs`](../../common/src/oracle/lp_stable.rs) | 76, 111 | Solver nonconvergence or nonpositive LP share supply. |
| [`common/src/oracle/observation.rs`](../../common/src/oracle/observation.rs) | 66 | Function-closing line after the future-timestamp assertion has no distinct hit; valid boundary is exercised through the provider. |
| [`common/src/oracle/providers/chainlink.rs`](../../common/src/oracle/providers/chainlink.rs) | 11 | Generated Soroban conversion or serialization path. |
| [`common/src/oracle/providers/curve.rs`](../../common/src/oracle/providers/curve.rs) | 7, 18 | Generated Soroban conversion or serialization path. |
| [`common/src/oracle/providers/redstone.rs`](../../common/src/oracle/providers/redstone.rs) | 8 | Generated Soroban conversion or serialization path. |
| [`common/src/oracle/providers/reflector.rs`](../../common/src/oracle/providers/reflector.rs) | 10, 18 | Generated Soroban conversion or serialization path. |
| [`common/src/types/composable_oracle.rs`](../../common/src/types/composable_oracle.rs) | 20, 30, 39, 47, 57, 66, 114, 125, 140, 159, 173, 199, 211 | Generated Soroban conversion or serialization path. |
| [`common/src/types/controller.rs`](../../common/src/types/controller.rs) | 46, 56, 68, 76, 83, 91, 108, 126, 154, 164, 189, 197, 212, 230, 262, 282, 296, 310, 536 | Generated Soroban conversion or serialization path. |
| [`common/src/types/oracle.rs`](../../common/src/types/oracle.rs) | 10, 26, 37, 47, 66 | Generated Soroban conversion or serialization path. |
| [`common/src/types/pool.rs`](../../common/src/types/pool.rs) | 17, 144, 283, 334, 357, 394, 429, 452, 467, 483, 506, 513, 521, 530, 539, 549, 556, 564, 575, 585, 597 | Generated Soroban conversion or serialization path. |
| [`contracts/controller/src/account.rs`](../../contracts/controller/src/account.rs) | 254, 256 | Delegate grant targets an inactive manager. |
| [`contracts/controller/src/context.rs`](../../contracts/controller/src/context.rs) | 166, 168 | A reused usage context receives a different spoke ID. |
| [`contracts/controller/src/events/config.rs`](../../contracts/controller/src/events/config.rs) | 8 | Generated Soroban conversion or serialization path. |
| [`contracts/controller/src/events/mod.rs`](../../contracts/controller/src/events/mod.rs) | 8, 30, 42, 75, 117 | Generated Soroban conversion or serialization path. |
| [`contracts/controller/src/external/blend.rs`](../../contracts/controller/src/external/blend.rs) | 12, 21, 105 | Lines 12 and 21 are Soroban type macros; line 105 handles empty Blend debt-cap authorization. |
| [`contracts/controller/src/positions/liquidation/apply.rs`](../../contracts/controller/src/positions/liquidation/apply.rs) | 165, 167, 170, 172, 224, 248 | Wrong-spoke credit receiver, inconsistent fee split, or zero-share seizure. |
| [`contracts/controller/src/positions/liquidation/math.rs`](../../contracts/controller/src/positions/liquidation/math.rs) | 259, 436, 469, 562, 567, 694, 745 | Includes impossible guards after single-entry length and checked subtraction, plus untested rounding and excess-payment edge cases. |
| [`contracts/controller/src/positions/mod.rs`](../../contracts/controller/src/positions/mod.rs) | 72, 74 | External batch response length mismatches request length. |
| [`contracts/controller/src/risk/params.rs`](../../contracts/controller/src/risk/params.rs) | 79, 220 | The key comes from the supply map yet its value lookup fails (inconsistent map state); the account-restamp loop also has an untested empty-supply exit. |
| [`contracts/controller/src/risk/validation.rs`](../../contracts/controller/src/risk/validation.rs) | 50, 52 | Post-action insolvency rejection. |
| [`contracts/controller/src/storage/account.rs`](../../contracts/controller/src/storage/account.rs) | 278 | Generated Soroban conversion or serialization path. |
| [`contracts/controller/src/strategies/flash_position.rs`](../../contracts/controller/src/strategies/flash_position.rs) | 227, 229, 347, 349 | Too many refund assets or no callback collateral deposits. |
| [`contracts/controller/src/strategies/multiply.rs`](../../contracts/controller/src/strategies/multiply.rs) | 143, 145 | Long or short strategy uses the same token for debt and collateral. |
| [`contracts/curator/src/lib.rs`](../../contracts/curator/src/lib.rs) | 56, 73 | Generated Soroban conversion or serialization path. |
| [`contracts/curator/src/ops.rs`](../../contracts/curator/src/ops.rs) | 35, 36, 37 | Successful native execution of the current-contract WASM upgrade. |
| [`contracts/defindex-strategy/src/lib.rs`](../../contracts/defindex-strategy/src/lib.rs) | 55, 68 | Generated Soroban conversion or serialization path. |
| [`contracts/governance/src/access.rs`](../../contracts/governance/src/access.rs) | 191, 193 | Revoking the last proposer. |
| [`contracts/governance/src/storage.rs`](../../contracts/governance/src/storage.rs) | 12 | Generated Soroban conversion or serialization path. |
| [`contracts/governance/src/timelock/lifecycle.rs`](../../contracts/governance/src/timelock/lifecycle.rs) | 101, 103, 124, 126 | Self-execution target mismatch. |
| [`contracts/pool/src/cache/shares.rs`](../../contracts/pool/src/cache/shares.rs) | 68, 70 | Revenue withdrawal computes zero shares to burn. |
| [`contracts/pool/src/events.rs`](../../contracts/pool/src/events.rs) | 14, 52 | Generated Soroban conversion or serialization path. |
| [`contracts/pool/src/ops/withdraw.rs`](../../contracts/pool/src/ops/withdraw.rs) | 96, 98 | Nonzero withdrawal rounds to zero burned shares. |
| [`contracts/position-nft/src/contract.rs`](../../contracts/position-nft/src/contract.rs) | 17 | Generated Soroban conversion or serialization path. |
| [`contracts/price-aggregator/src/engine.rs`](../../contracts/price-aggregator/src/engine.rs) | 214, 379, 548 | The missing-oracle guard at 214 is redundant after `Outcome::failure`; the other lines propagate a cached-dependency or second-source failure. |
| [`contracts/price-aggregator/src/providers/aquarius.rs`](../../contracts/price-aggregator/src/providers/aquarius.rs) | 51, 84 | Both non-token-key checks follow `bound_tokens`, which already rejects non-token keys; these guards are unreachable through the current call chain. |
| [`contracts/price-aggregator/src/providers/reflector.rs`](../../contracts/price-aggregator/src/providers/reflector.rs) | 57 | Closing brace of the successful TWAP validation arm has no distinct hit; successful TWAP reads are exercised. |
| [`contracts/price-aggregator/src/registry.rs`](../../contracts/price-aggregator/src/registry.rs) | 11 | Generated Soroban conversion or serialization path. |
| [`contracts/price-aggregator/src/validation.rs`](../../contracts/price-aggregator/src/validation.rs) | 141 | LP token decimals outside the allowed range. |
| [`contracts/swap-aggregator/src/execute/mod.rs`](../../contracts/swap-aggregator/src/execute/mod.rs) | 253, 256 | Previous-hop mode has no prior output or the wrong prior token. |
| [`contracts/swap-aggregator/src/fees.rs`](../../contracts/swap-aggregator/src/fees.rs) | 70 | Fee calculation sees no positive token balance. |
| [`contracts/swap-aggregator/src/lib.rs`](../../contracts/swap-aggregator/src/lib.rs) | 130 | Referral counter overflow. |
| [`contracts/swap-aggregator/src/storage.rs`](../../contracts/swap-aggregator/src/storage.rs) | 174, 184, 206 | Wrong fee-bucket key, negative reserve, or excess release. |
| [`contracts/swap-aggregator/src/types.rs`](../../contracts/swap-aggregator/src/types.rs) | 37, 57 | Generated Soroban conversion or serialization path. |
| [`contracts/swap-aggregator/src/venues/aquarius/burn.rs`](../../contracts/swap-aggregator/src/venues/aquarius/burn.rs) | 45, 97 | No shares to burn or no tokens returned. |
| [`contracts/swap-aggregator/src/venues/aquarius/mint.rs`](../../contracts/swap-aggregator/src/venues/aquarius/mint.rs) | 57, 116 | Zero deposit or minted shares below minimum. |
| [`contracts/swap-aggregator/src/venues/mod.rs`](../../contracts/swap-aggregator/src/venues/mod.rs) | 112, 115 | Nonpositive hop input or identical input and output token. |
| [`contracts/swap-aggregator/src/venues/soroswap.rs`](../../contracts/swap-aggregator/src/venues/soroswap.rs) | 26 | Nonpositive amount to fee helper. |
| [`contracts/xoxno-oracle/src/aggregation.rs`](../../contracts/xoxno-oracle/src/aggregation.rs) | 198 | Arithmetic overflow in the spread comparison has no constructed extreme-value case. |
| [`contracts/xoxno-oracle/src/reads.rs`](../../contracts/xoxno-oracle/src/reads.rs) | 67 | Stored feed history is empty. |
| [`contracts/xoxno-oracle/src/storage/mod.rs`](../../contracts/xoxno-oracle/src/storage/mod.rs) | 58, 90 | Generated Soroban conversion or serialization path. |

## Interpreting the gaps

- **Reachable error and boundary paths:** add tests that construct the state through protocol helpers, execute the relevant action, and assert both the error or result and state rollback where relevant.
- **Defensive invariants:** prove that earlier validation makes the branch unreachable, then document or simplify the guard. Artificially calling a private helper with an impossible state would inflate coverage without strengthening the protocol test.
- **Generated code and native/WASM differences:** evaluate Soroban conversion round trips separately from the `#[contracttype]` source annotation count; native coverage cannot certify WASM-only upgrade paths.
- **LLVM summary-only misses:** inspect JSON regions or compiled instances. They cannot be converted into an exact source-line list from `LF/LH` alone.
