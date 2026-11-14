# Skill evaluations

Grader fixtures only. Do not load this directory to answer a user task.

Scenarios for the XOXNO skills, in the [stellar-dev-skill evals format](https://github.com/stellar/stellar-dev-skill/blob/main/evals/README.md).

- Paths: `scenarios/<skill>/NN-<slug>.json`
- Build generated Rust with `stellar contract build` (stellar-cli 25.2 or
  newer), `soroban-sdk = "28"`, and `xoxno-contract-sdk = "0.2"`. Plain
  `cargo build` does not work for a Wasm target with `soroban-sdk` 28.
- Check generated TypeScript with `tsc --noEmit` against the SDK version in
  `xoxno-lending-sdk/SKILL.md`.
- An eval must not require restating a whole companion file; point at the
  heading instead.
