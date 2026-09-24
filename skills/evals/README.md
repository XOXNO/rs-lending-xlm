# Skill evaluations

Grader fixtures only. Do not load this directory to answer a user task.

Scenarios for the XOXNO skills, in the [stellar-dev-skill evals format](https://github.com/stellar/stellar-dev-skill/blob/main/evals/README.md).

- Paths: `scenarios/<skill>/NN-<slug>.json`
- Machine checks: generated Rust builds with `stellar contract build` (stellar-cli 25.2 or newer; `soroban-sdk` 28 rejects a plain `cargo build` for a Wasm target) against `soroban-sdk = "28"` and `xoxno-contract-sdk = "0.2"`; generated TypeScript passes `tsc --noEmit` against the `@xoxno/sdk-js` version named in `xoxno-lending-sdk/SKILL.md`
- Do not write an eval that requires restating a whole companion file. Point at the heading instead.
