# 0017. Test and verification surfaces live behind cargo features, guarded by WASM-export build gates

Status: Accepted

## Context

Integration tests and formal verification both need privileged surfaces the
production ABI must never expose: seeding an oracle without provider
attestation, executing an admin operation without the timelock, rewiring the
governance→controller pointer. Two failure modes pull in opposite directions.
If those surfaces live in separate mock contracts, tests exercise bytecode
that drifts from what ships, and a bug in the real WASM can hide behind a
passing mock. If they live in the production crates behind a cargo feature,
one mis-set feature flag in a build pipeline ships a timelock bypass or an
unauthenticated oracle-seeding entrypoint to mainnet. The design must get
real-bytecode testing without trusting feature discipline alone.

## Decision

Privileged test surfaces are compiled into the production crates behind
`#[cfg(any(test, feature = "testing"))]`, as additional `#[contractimpl]`
blocks on the real contract types:
`contracts/governance/src/timelock/testing.rs::execute_immediate` (apply a
typed `AdminOperation` without scheduling, still role/owner-checked),
`contracts/governance/src/deploy.rs::set_controller` /
`::set_price_aggregator` (rewire stored addresses), and
`contracts/price-aggregator/src/lib.rs::seed_oracle` / `::remove_oracle`
(store a config bypassing admission). Integration tests therefore run against
the same source, storage layout, and entrypoints that production builds
compile — only the extra impl blocks differ.

The deployable artifacts are then checked, not trusted:
`Makefile::wasm-testing-abi-check` runs `strings` over the built
`governance.wasm` and fails if it contains `set_controller`, and over
`price_aggregator.wasm` failing on `seed_oracle(_config)?`. The gate is wired
as a dependency of `Makefile::wasm-size-check`, which both the tests and
release workflows execute (`.github/workflows/tests.yml`,
`.github/workflows/release.yml`), so no release artifact is produced without
passing it. A live-network assertion closes the loop:
`tests/integration/flows/governance.sh::flow_governance` expects the
`set_controller` invocation to fail against the deployed governance contract.

The formal-verification surface uses the same pattern: pool and controller
mount their Certora rule modules only under `feature = "certora"`, via
`#[cfg(feature = "certora")] #[path = "../../../certora/pool/spec/mod.rs"]`
in `contracts/pool/src/lib.rs` (and the controller equivalent in
`contracts/controller/src/lib.rs`), so proofs compile against near-production
code paths without the spec code existing in deployable builds.

## Alternatives

- **Separate mock/harness contracts for tests.** A dedicated
  `MockPriceAggregator` with an open `seed` method keeps the production crate
  free of any gated code, but every behavioral divergence between mock and
  real contract is a blind spot, and mocks must chase the real storage layout
  and error surface forever. In-crate gating means the tested WASM is the
  shipped WASM plus nothing but the gated blocks.
- **Feature discipline alone, no artifact check.** Trusting that no build
  pipeline ever enables `testing` for a deployable artifact is trusting a
  human-maintained flag matrix. The `strings`-level gate is crude but checks
  the artifact itself — the thing that ships — rather than the process that
  produced it, and it costs one grep per release.
- **Certora specs as an out-of-tree fork.** Keeping verification code in a
  patched copy of the contracts avoids feature plumbing, but the fork decays
  with every contract change and the proofs quietly stop describing the code
  that ships. The `#[path]` mounts keep rule bodies in `certora/` while
  compiling them against the live crate source.

## Consequences

Easy: integration tests and PoCs drive the real contract WASM end to end;
Certora rules track contract changes at compile time instead of by manual
porting; a feature leak is caught mechanically at build time and again by the
live-network flow, not by audit.

Hard: the gate is symbol-name-based, so every new cfg-gated entrypoint must
be added to `wasm-testing-abi-check` by hand — the Makefile grep list is a
second registry that can lag the code. Test-only code sharing the production
crate also means gated blocks are compiled (under `cfg(test)`) in the same
namespace and must not relax real invariants by accident.

Must stay true: `wasm-testing-abi-check` stays on the dependency path of
every artifact-producing target; each new testing entrypoint gains a gate
entry in the same change that introduces it; and the gated surfaces keep
their own auth checks anyway (as `execute_immediate` does), as depth against
a missed gate. A leaked surface is a direct AUTH- and ORACLE-domain breach —
unauthenticated oracle seeding and timelock bypass (see
../../reference/invariants.md §INV-AUTH, §INV-ORACLE, and
../threat-model.md).
