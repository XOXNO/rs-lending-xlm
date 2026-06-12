# Governance Contract Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move config-admin + pool-admin out of the controller into a new `governance` contract that deploys and owns the controller; controller keeps all storage, hot paths, and thin owner-gated setters; all admin tooling (Makefile, configs/script.sh) routes through governance.

**Architecture:** Governance validates inputs (pure checks + live oracle probing) and forwards to thin `#[only_owner]` controller setters. The controller's ownable owner IS the governance contract address (Soroban invoker-auth makes `require_auth` on a contract address pass when that contract is the direct invoker), so existing `#[only_owner]` macros work unchanged. Storage never moves: hot paths never cross a contract boundary. State-dependent invariant checks (e-mode existence, market status, token-approved) STAY in the controller; only input validation moves. Events stay emitted by the controller at the point of state change (indexers unaffected). Governance deploys the controller from an uploaded WASM hash via `deployer().with_current_contract(salt)` and becomes its constructor admin.

**Tech Stack:** Rust / soroban-sdk 26.0.0 (pinned), stellar-access/stellar-macros 0.7.1, GNU make + stellar CLI 26.1, bash (configs/script.sh), jq.

**Measured payoff (2026-06-12 worktree trials, baseline 127,120 B deploy artifact):** config-admin block −18,950 B; + pool-admin −26,284 B. Net after thin setters ≈ −23 kB → controller ≈ 104 kB.

**Verification bar (every task):**
```bash
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # NEVER --all-features (breaks certora linking)
make wasm-size-check              # after wasm-affecting tasks
```
Note: any `cd` in Bash trips a gvm hook — use absolute paths, `--manifest-path`, `make -C`, `git -C`.

---

## Locked design decisions (do not relitigate during execution)

1. **Auth:** controller thin setters keep `#[only_owner]`. Production owner = governance contract (invoker auth). Unit tests keep EOA owner + `mock_all_auths()` — zero macro churn.
2. **Oracle-config entrypoints move with their validation.** Controller gains two NEW thin setters: `set_market_oracle_config(asset, MarketOracleConfig)` and `set_oracle_tolerance(asset, OraclePriceFluctuation)`, both `#[only_owner]`. The old `configure_market_oracle` / `edit_oracle_tolerance` entrypoints (and their ORACLE-role gating) move to governance, which carries its own ORACLE role. `disable_token_oracle` (emergency, state-check-only) STAYS on the controller, role-gated as today.
3. **Shared oracle plumbing moves to `common::oracle`** (provider client wrappers + observation helpers) because both the controller hot path and governance probing need it. No duplication; contracts must not depend on each other's rlib (`#[contractimpl]` exports would leak).
4. **`validate_quote_is_usd_market`** is rewritten in governance to read the quote market via the controller's existing `get_market_config` view (cross-call), not storage.
5. **Governance contract entrypoint names mirror today's admin API** (`create_market` replaces `create_liquidity_pool` naming at the gov level is NOT done — keep `create_liquidity_pool` so script.sh churn is a pure `--id` swap). Disambiguated names only where collision exists: `upgrade_controller`, `migrate_controller`, `grant_controller_role`, `revoke_controller_role`, `transfer_controller_ownership`. Gov's own `upgrade`/`transfer_ownership`/`accept_ownership` manage the governance contract itself.
6. **Test strategy:** controller suites keep deploying the controller directly (EOA admin). A new governance suite owns: all input-validation panic tests (≈26 moving tests), `deploy_controller` (controller.wasm fixture, same pattern as the pool.wasm fixture), and gov→controller forwarding integration. The test-harness builder registers Governance natively and wires it via a `#[cfg(any(test, feature = "testing"))] set_controller` entrypoint.
7. **configs/networks.json** gains `"governance": ""` and `"controller_wasm_hash": ""` on both networks. `controller` field stays (recorded post-deploy).
8. **Fresh testnet redeploy** (not in-place adoption). Mainnet is not yet deployed, so no migration path is needed; `transfer_controller_ownership` exists as the escape hatch.
9. New crate is `contracts/governance`, package name `governance`, added to `CONTRACTS := pool controller governance` in the Makefile, with a `wasm_size_budget.txt` entry (start 60000, tighten after first measure).
10. Work happens on branch `feat/governance-split`. Conventional commits, one concern per commit.

## File structure (created / modified)

```
contracts/governance/                  NEW crate (cdylib+rlib, features: testing, certora stub not needed)
  src/lib.rs                           contract struct, modules
  src/access.rs                        __constructor, ownable, ORACLE role, upgrade (self)
  src/deploy.rs                        deploy_controller, set_controller (testing), controller() view
  src/forward.rs                       validated forwarders (config + pool admin + pause/roles)
  src/validate/mod.rs                  asset-config / risk-bounds / position-limits / market-creation validation (moved)
  src/validate/oracle_config.rs        pure shape checks (moved from controller oracle/validation/config.rs)
  src/validate/oracle_probe.rs         live source probing (moved from controller oracle/validation/oracle.rs)
  src/validate/tolerance.rs            validate_and_calculate_tolerances (moved from controller oracle/tolerance.rs)
interfaces/controller/src/admin.rs     NEW ControllerAdmin trait + #[contractclient(name="ControllerAdminClient")]
common/src/oracle/                     NEW shared module: providers/{reflector,redstone} client wrappers + observation helpers
contracts/controller/src/governance/config.rs   thinned setters
contracts/controller/src/router.rs              thinned create_liquidity_pool / upgrade_liquidity_pool_params
contracts/controller/src/validation.rs          admin-only validators removed (moved to governance)
contracts/controller/src/oracle/...             imports repointed at common::oracle
Cargo.toml                             workspace member += contracts/governance
Makefile                               CONTRACTS var, deploy/upgrade/configure flows, preflights
configs/networks.json                  + governance, controller_wasm_hash
configs/script.sh                      admin invocations → governance id
configs/wasm_size_budget.txt           + governance.wasm entry, controller budget tightened
verification/test-harness/...          builder wires governance; suites migrated
docs/superpowers/plans/…               this plan
```

---

### Task 1: Branch + scaffold governance crate

**Files:**
- Create: `contracts/governance/Cargo.toml`, `contracts/governance/src/lib.rs`, `contracts/governance/src/access.rs`
- Modify: `Cargo.toml` (workspace members), `Makefile:75` (`CONTRACTS`), `configs/wasm_size_budget.txt`

- [ ] **Step 1: Create branch**
```bash
git -C /Users/mihaieremia/GitHub/rs-lending-xlm checkout -b feat/governance-split
```

- [ ] **Step 2: Crate manifest** — `contracts/governance/Cargo.toml`:
```toml
[package]
name = "governance"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
soroban-sdk = { workspace = true }
stellar-access = { workspace = true }
stellar-contract-utils = { workspace = true }
stellar-macros = { workspace = true }
common = { path = "../../common" }
controller-interface = { path = "../../interfaces/controller" }

[features]
testing = []

[dev-dependencies]
soroban-sdk = { workspace = true, features = ["testutils"] }
```

- [ ] **Step 3: lib.rs + access.rs** — minimal contract: `#[contract] pub struct Governance;` with `__constructor(env, admin: Address)` doing `ownable::set_owner`, `access_control::set_admin`, and granting `ORACLE` to admin (mirror controller access.rs:84-105 minus position-limits/pause/app-version); `upgrade(new_wasm_hash)` `#[only_owner]` via `stellar_contract_utils::upgradeable::upgrade`; `transfer_ownership`/`accept_ownership` mirroring controller access.rs:167-188 (reuse the same `sync_*` helper shapes).
- [ ] **Step 4: Workspace + Makefile + budget**: add `"contracts/governance"` to workspace members; `CONTRACTS := pool controller governance`; append `governance.wasm 60000` to `configs/wasm_size_budget.txt`.
- [ ] **Step 5: Verify** `cargo check --workspace` clean; `make build` produces `governance.wasm`.
- [ ] **Step 6: Commit** `feat(governance): scaffold governance contract crate`

### Task 2: ControllerAdmin client trait in interface crate

**Files:**
- Create: `interfaces/controller/src/admin.rs`
- Modify: `interfaces/controller/src/lib.rs` (module + re-export)

- [ ] **Step 1:** Define the trait with `#[soroban_sdk::contractclient(name = "ControllerAdminClient")]` covering exactly: `set_aggregator(addr)`, `set_accumulator(addr)`, `set_liquidity_pool_template(hash)`, `edit_asset_config(asset, cfg: AssetConfigRaw)`, `set_position_limits(limits: PositionLimits)`, `add_e_mode_category(ltv,threshold,bonus)->u32`, `edit_e_mode_category(id,ltv,threshold,bonus)`, `remove_e_mode_category(id)`, `add_asset_to_e_mode_category(asset,category_id,can_collateral,can_borrow)`, `edit_asset_in_e_mode_category(...)`, `remove_asset_from_e_mode(asset,category_id)`, `approve_token(token)`, `revoke_token(token)`, `set_market_oracle_config(asset, config: MarketOracleConfig)`, `set_oracle_tolerance(asset, tolerance: OraclePriceFluctuation)`, `create_liquidity_pool(asset, params: MarketParamsRaw, config: AssetConfigRaw)->Address`, `upgrade_liquidity_pool_params(asset, params: InterestRateModel)`, `deploy_pool()->Address`, `upgrade_pool(new_wasm_hash)`, `pause()`, `unpause()`, `grant_role(account, role: Symbol)`, `revoke_role(account, role: Symbol)`, `upgrade(new_wasm_hash)`, `migrate(new_version)`, `transfer_ownership(new_owner, live_until_ledger)`, `get_market_config(asset)->MarketConfig`. (Types: `MarketParamsRaw`/`InterestRateModel` from `common::types::pool`; the rest from `controller_interface::types`.)
- [ ] **Step 2:** `cargo check --workspace`; commit `feat(interface): ControllerAdmin client trait for governance forwarding`.

### Task 3: Move shared oracle plumbing to common::oracle

**Files:**
- Create: `common/src/oracle/mod.rs`, `common/src/oracle/observation.rs`, `common/src/oracle/providers/{mod.rs,reflector.rs,redstone.rs}`
- Modify: `contracts/controller/src/oracle/{observation.rs,providers/**}` → become re-export shims or have callers repointed (prefer repointing imports and deleting the originals; keep `pub use` shims only where Certora harness paths require stability)

- [ ] **Step 1:** Move verbatim (no logic edits): `observation.rs` constants (`MIN/MAX_ORACLE_DECIMALS`, `MIN/MAX_PRICE_STALE_SECONDS`, `MAX_TWAP_RECORDS`, `MIN_ORACLE_RESOLUTION_SECONDS`) + `validate_positive_price_timestamps`, `u256_to_i128`, `millis_to_seconds`; reflector client wrappers (`reflector_base_call`, `reflector_decimals_call`, `reflector_lastprice_call`, `reflector_prices_call`, `reflector_resolution_call`, `to_reflector_asset`, `min_twap_observations`, `ReflectorAsset`, `ReflectorPriceData`); redstone `read_price_data_uncached`, `RedStonePriceData`, `REDSTONE_DECIMALS`. Anything in those files used ONLY by the hot path (cached reads, prefetch) stays in the controller.
- [ ] **Step 2:** Repoint controller imports; run the full bar (`check`, `clippy -D warnings`, `cargo test --workspace`). Pool/controller WASM byte sizes must be ~unchanged (`make build` + compare).
- [ ] **Step 3:** Commit `refactor(common): move shared oracle client plumbing to common::oracle`.

### Task 4: Thin the controller admin surface

**Files:**
- Modify: `contracts/controller/src/governance/config.rs`, `contracts/controller/src/router.rs:166-260`, `contracts/controller/src/validation.rs:142-209`, `contracts/controller/src/oracle/tolerance.rs`, `contracts/controller/src/oracle/validation/` (delete config.rs+oracle.rs after Task 5 consumes them)

- [ ] **Step 1 (failing tests first):** in the controller suite add tests for the two NEW thin setters: `set_market_oracle_config` stores config + flips `MarketStatus::PendingOracle→Active` + emits `UpdateAssetOracleEvent`; `set_oracle_tolerance` overwrites `market.oracle_config.tolerance`. Both keep the `PairNotActive` status gate from config.rs:365-372 and the `cfg!(feature="testing")` asset_decimals preservation (config.rs:380-382). Run: expect FAIL (functions missing).
- [ ] **Step 2:** Implement thin setters in `governance/config.rs`; entrypoints `configure_market_oracle` and `edit_oracle_tolerance` are REMOVED from the controller. Thin the others: `set_aggregator`/`set_accumulator` drop `require_contract_address`; `set_liquidity_pool_template` drops `require_nonzero_wasm_hash`; `edit_asset_config` drops `validation::validate_asset_config` (keeps e_mode_categories preservation + write + event); `set_position_limits` drops the bounds check (keeps write + event); `add/edit_e_mode_category` drop `validate_risk_bounds` (keep deprecation/state asserts). `approve_token`/`revoke_token` unchanged. E-mode state-dependent asserts (AssetAlreadyInEmode, AssetNotInEmode, EModeCategoryDeprecated) all STAY.
- [ ] **Step 3:** `router.rs`: `create_liquidity_pool` keeps `has_market_config`/`is_token_approved` asserts, pool call, market write, `CreateMarketEvent`, approval consumption; DELETE `validate_market_creation` + the `validate_and_fetch_token_decimals` call (moves to gov). `upgrade_liquidity_pool_params` drops `params.verify(env)` (keeps accrual + pool call + event). `deploy_pool`/`upgrade_pool` unchanged.
- [ ] **Step 4:** `validation.rs`: delete `validate_risk_bounds`, `validate_asset_config`, `validate_and_fetch_token_decimals` (their unit tests move to governance in Task 6). `oracle/tolerance.rs`: `validate_and_calculate_tolerances` extracted for the move; runtime tolerance application stays.
- [ ] **Step 5:** Run controller suite; validation-panic tests now fail — mark the exact list (expected: the ≈26 tests inventoried in validation_admin.rs + admin.rs) and MOVE them in Task 6; everything else green. Commit `refactor(controller)!: thin admin setters; validation moves to governance`.

### Task 5: Governance contract — validation + forwarders + deploy_controller

**Files:**
- Create: `contracts/governance/src/{deploy.rs,forward.rs}`, `contracts/governance/src/validate/{mod.rs,oracle_config.rs,oracle_probe.rs,tolerance.rs}`

- [ ] **Step 1:** Land the moved validation modules (from Task 4 deletions): `validate/mod.rs` gets `validate_risk_bounds`, `validate_asset_config`, `validate_and_fetch_token_decimals`, position-limits bounds (`POSITION_LIMIT_MAX = 10` moves here), market-creation pure checks (asset_id match, decimals range, `params.verify_rate_model`, production `asset_decimals == token_decimals` under `#[cfg(not(feature = "testing"))]`); `validate/oracle_config.rs` + `validate/oracle_probe.rs` land verbatim with imports repointed at `common::oracle`; `validate_quote_is_usd_market` now calls `ControllerAdminClient::get_market_config` instead of `crate::storage`.
- [ ] **Step 2:** `deploy.rs`:
```rust
const CONTROLLER_DEPLOY_SALT: [u8; 32] = [0u8; 32];

#[only_owner]
pub fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address {
    assert_with_error!(env, !storage_has_controller(&env), GenericError::PoolAlreadyDeployed);
    let controller = env.deployer()
        .with_current_contract(BytesN::from_array(&env, &CONTROLLER_DEPLOY_SALT))
        .deploy_v2(wasm_hash, (env.current_contract_address(),));
    set_controller_storage(&env, &controller);
    controller
}

pub fn controller(env: Env) -> Address;             // view, panics OwnerNotSet-style if unset
#[cfg(any(test, feature = "testing"))]
pub fn set_controller(env: Env, addr: Address);      // test wiring only
```
(Storage key: a `GovernanceKey::Controller` instance entry in the governance crate.)
- [ ] **Step 3:** `forward.rs` — every entrypoint validates then forwards via `ControllerAdminClient::new(&env, &controller(env))`. Owner-gated: `set_aggregator`/`set_accumulator` (`require_contract_address` — addr.exists + Wasm executable, moved from config.rs:149-157), `set_liquidity_pool_template` (nonzero-hash check), `edit_asset_config` (validate_asset_config), `set_position_limits` (bounds), e-mode trio + asset e-mode trio (risk-bounds where applicable), `approve_token`/`revoke_token` (pure forward), `create_liquidity_pool` (token decimals fetch + market-creation validation, then forward), `upgrade_liquidity_pool_params` (`params.verify`), `deploy_pool`, `upgrade_pool`, `pause`, `unpause`, `grant_controller_role`, `revoke_controller_role`, `upgrade_controller`, `migrate_controller`, `transfer_controller_ownership`. ORACLE-role-gated on governance: `configure_market_oracle(caller, asset, cfg: MarketOracleConfigInput)` (tolerances + sources probe → forward `set_market_oracle_config`), `edit_oracle_tolerance(caller, asset, first, last)` (→ forward `set_oracle_tolerance`).
- [ ] **Step 4:** `cargo check/clippy/test --workspace`; `make build`; record `governance.wasm` size and tighten its budget line. Commit `feat(governance): validated admin forwarders + controller deployment`.

### Task 6: Test migration + harness wiring

**Files:**
- Modify: `verification/test-harness/src/setup/builder.rs` (register Governance, `set_controller`, route builder admin setup through gov client)
- Create: governance test suite files (mirror the harness layout); Move: the validation-panic tests inventoried in `validation_admin.rs` (≈23) + `admin.rs` (≈3) + the `validate_asset_config`/`validate_risk_bounds` unit tests from `contracts/controller/src/validation.rs:241-274` + oracle shape tests from `oracle/validation/config.rs:101-268`
- Modify: controller suites that called `configure_market_oracle` (≈38 sites — builder routes through gov, so most collapse into the builder change)

- [ ] **Step 1:** Builder: `let gov = env.register(Governance, (admin.clone(),)); GovernanceClient::new(&env, &gov).set_controller(&controller_id);` then swap builder admin calls (market creation, oracle config, e-mode setup) to the gov client. `mock_all_auths()` keeps everything passing.
- [ ] **Step 2:** Move the panic tests; assert the same error codes (enums unchanged in `common::errors`).
- [ ] **Step 3:** New gov-only tests: `deploy_controller` happy path (upload `target/wasm32v1-none/release/controller.wasm` fixture exactly like the pool fixture at builder.rs:163-178 — note the staleness trap: rebuild via `stellar contract build` after ABI changes), `deploy_controller` twice panics, forwarding smoke test (gov.edit_asset_config → controller view reflects change), non-owner caller rejected (drop `mock_all_auths` for that one, use `mock_auths` for owner-only).
- [ ] **Step 4:** Full bar green: `cargo test --workspace` (expect ≈ today's 989 ± moved tests), plus `-p controller --lib` for the 4 cfg-gated oracle tests. Commit `test: migrate admin validation suites to governance`.

### Task 7: WASM size verification

- [ ] **Step 1:** `make deploy-artifacts`; expect controller ≈ 104 kB (≈ −23 kB vs 127,120), governance ≈ 25–35 kB, pool unchanged. `make wasm-size-check` passes; tighten `controller.wasm` budget (165000 → 120000) and set governance's real budget.
- [ ] **Step 2:** Commit `chore: tighten wasm budgets post governance split`.

### Task 8: Deploy tooling — Makefile, networks.json, script.sh

**Files:**
- Modify: `Makefile` (deploy flow `_deploy`/`_preflight-*`/`configure-controller`→`configure-governance`/`upgrade-controller`/`upgrade-pool-template`/`upgrade-pools`/`_unpause-after-setup`/`_post-setup-status`/`create-market`), `configs/networks.json`, `configs/script.sh`

- [ ] **Step 1:** networks.json: add `"governance": ""` and `"controller_wasm_hash": ""` to testnet + mainnet objects.
- [ ] **Step 2:** Makefile deploy flow becomes: upload pool.wasm → POOL_HASH; upload controller.wasm → CONTROLLER_HASH (recorded into networks.json `controller_wasm_hash`); `stellar contract deploy` governance (constructor `--admin $(SIGNER_ADDRESS)`) → alias `governance`; `invoke governance deploy_controller --wasm_hash $CONTROLLER_HASH` → parse returned address → alias `controller` + networks.json; `invoke governance set_liquidity_pool_template --hash $POOL_HASH`; `invoke governance deploy_pool` → networks.json `pool`. Upgrades: `upgrade-controller` = upload new controller.wasm → `governance upgrade_controller --new_wasm_hash`; `upgrade-pool-template` = upload pool.wasm → `governance set_liquidity_pool_template` + `governance upgrade_pool`. Pause/unpause targets → governance. All `$$CTRL` admin lookups swap to a `$$GOV` lookup (`stellar contract alias show governance` falling back to jq `.governance`). View-only targets (`_post-setup-status`, `view` helpers) keep reading the controller.
- [ ] **Step 3:** script.sh: add `get_governance()` mirroring `get_controller()` (script.sh:104); every ADMIN invocation (`create_market`, `edit_asset_config`, `add/edit_emode_*`, `ensure_*`, aggregator/accumulator setters, pause/unpause, grant/revoke role → `grant_controller_role`/`revoke_controller_role`) swaps `--id "$ctrl"` → `--id "$gov"`. Views (`get_market_config`, `list_markets`, `fetch_emode_category_json`, decimals) stay on the controller. The ORACLE-role invocations (`configure_market_oracle`, `edit_oracle_tolerance`) target governance (role granted on governance at deploy).
- [ ] **Step 4:** Dry-run lint: `bash -n configs/script.sh`; `make -n deploy-testnet` renders sane commands. Commit `feat(deploy): route admin tooling through governance`.

### Task 9: Testnet end-to-end redeploy

- [ ] **Step 1:** `make deploy-testnet` (gov → controller → pool → markets → e-modes → oracle config) — full run green; record addresses in networks.json (governance, controller, pool, hashes).
- [ ] **Step 2:** `make _post-setup-status NETWORK=testnet` shows 5 markets + e-mode 1, unpaused; spot-check one gov admin call (`edit_oracle_tolerance`) and one hot-path op (supply via existing flows) on-chain.
- [ ] **Step 3:** Commit `chore(deploy): record governance-split testnet deployment`.

### Task 10: Docs + memory + follow-ups

- [ ] **Step 1:** Update `SCF_BUILD_ARCHITECTURE.md` / relevant ADR with the governance topology (gov = deployer/owner; invoker-auth; validation-at-boundary inventory). Note the timelock follow-up: governance is its natural home.
- [ ] **Step 2:** Memory: update `gov-split-size-measurement.md` with realized sizes; note SDK/types follow-up (admin tx builders must target governance — `lending-types-sdk-release-chain` chain).
- [ ] **Step 3:** Final full bar + `superpowers:finishing-a-development-branch`.

## Self-review notes
- Spec coverage: gov contract ✔ (T1,5), self-deploys controller ✔ (T5 deploy.rs), storage stays ✔ (thin setters T4), networks.json new fields ✔ (T8), all configs/tooling re-routed ✔ (T8), agents/plan workflow ✔ (subagent-driven execution).
- Types referenced exist: `MarketOracleConfig`/`MarketOracleConfigInput` (interfaces/controller/src/types/oracle.rs:198,243), `OraclePriceFluctuation` (imported in oracle/validation/oracle.rs:9), `MarketParamsRaw::verify_rate_model` (common/src/types/pool.rs:41), `InterestRateModel::verify` (common/src/types/pool.rs:124).
- Known traps: pool.wasm/controller.wasm fixture staleness (rebuild before harness runs); `cargo fmt` touches cfg-gated certora files (revert unrelated churn); `--all-features` forbidden; Certora controller harness references moved paths — keep `pub use` shims if `check_orphans.py` flags orphans.
