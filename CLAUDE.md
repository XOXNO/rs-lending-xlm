# XOXNO Lending — agent guide

Over-collateralized money market on Stellar (Soroban). Real funds. Assume every
change is adversarial until proven otherwise.

## Non-negotiables

- **Never weaken a check to make a test pass.** If a test fails, the test is
  usually right. Fix the cause, not the assertion.
- **Never commit** secrets, keys, `.env`, or local deploy state.
- **Keep PRs focused.** No drive-by formatting or unrelated refactors.
- **Call out auth/role changes explicitly** in the PR body. Same for anything
  touching pause/freeze, timelock, or upgrade paths.
- **Touching money, risk, oracle, governance, storage, or strategies** means
  tests + the matching deeper tier (below) — not just `cargo test`.
- `soroban-sdk` and `stellar-*` are **pinned exact** (`=26.1.0`, `=0.7.2`) for
  reproducible audited artifacts. Do not bump or unpin without being asked.

## Fixed-point discipline (the #1 source of bugs here)

Three scales, never mixed implicitly:

| Scale | Value | Used for |
|---|---|---|
| token-native | asset decimals | amounts at transfer boundaries |
| **WAD** | `1e18` | USD values, health factor, LTV |
| **RAY** | `1e27` | interest rates, indexes |

- Use the newtypes in `common/src/math/fp.rs` — `Ray(i128)`, `Wad(i128)`. Do not
  hand-roll `i128` scaling.
- Conversions are **explicit and directional**: `mul_floor`/`mul_ceil`,
  `div_floor`/`div_ceil`, `to_wad_floor`/`to_wad_ceil`, `to_asset_*`.
  Rounding direction is a security property — round against the user, in favor
  of the protocol. Changing a `floor` to a `ceil` (or the reverse) is never
  cosmetic.
- Release profile sets `overflow-checks = true`. Do not disable it.

## Layout

```
common/          shared math, constants, rates, oracle helpers, types, errors.
                 no_std, NO contract storage — consumers own TTL/persistence.
interfaces/      client-only ABI mirrors (#[contractclient]). No impls.
contracts/       pool, controller, governance, price-aggregator,
                 swap-aggregator, defindex-strategy, flash-loan-receiver,
                 xoxno-oracle
mock/            test doubles (mock-oracle, mock-redstone)
tests/test-harness   integration tests (single-threaded)
certora/         formal verification specs, per contract
skills/          integrator recipes (bots, indexing, flash-loan receivers)
```

Design shape: one central **pool** holds liquidity. Markets are keyed by
`HubAssetKey { hub_id, asset }`. **Spokes** carry risk params (LTV, caps,
pause/freeze) per account group. The **controller** owns policy; the pool owns
accounting — respect that boundary. Oracles are dual-source with a tolerance
band and **fail closed**. GUARDIAN can pause immediately; **unpause is
timelocked**.

> `docs/` is intentionally absent from the working tree. README and CONTRIBUTING
> still link to it — those links are dead. Don't chase them; read the code.

## Verification tiers

Always, every change:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Then the layer matching what you touched:

```bash
make test                     # full harness (serialized)
make test-pool                # pool unit tests
make test-one FILE=<name>     # single harness file
make test-match PATTERN=<p>   # by name
```

Money / risk / oracle / governance / storage / strategy changes also need:

```bash
make certora-wasm             # then the profile covering your domain
make miri-common              # pure math changes
make coverage                 # merged coverage report
```

Build artifacts: `make build` (via `stellar contract build`), `make optimize`.

## Conventions

- **Commits**: Conventional Commits, scope = crate.
  `fix(pool): restore market-state event on create_strategy`
- **Errors**: `#[contracterror]` enums in `common/src/errors.rs`, explicit
  discriminants. Never renumber an existing variant — it is ABI.
- **Rustdoc**: every public entrypoint documented, and the doc must match
  behavior. A comment that overstates a guarantee is a bug; there is commit
  history of fixing exactly that.
- `common/` stays storage-free and `no_std`.

## When auditing

Read the code as the source of truth. Prior audit reports and findings have been
deliberately removed from this tree so a review starts unbiased — do not go
looking for them in git history unless explicitly asked.
