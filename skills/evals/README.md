# Skill evaluations

This directory holds grader fixtures only. Do not load it to answer a user
task.

The scenarios for the XOXNO skills use the [stellar-dev-skill evals format](https://github.com/stellar/stellar-dev-skill/blob/main/evals/README.md).
Follow these rules:

- Store each scenario at `scenarios/<skill>/NN-<slug>.json`.
- Build generated Rust with `stellar contract build` from stellar-cli 25.2 or
  newer. Use `soroban-sdk = "28"` and `xoxno-contract-sdk = "0.2"`. Plain
  `cargo build` does not work for a Wasm target with `soroban-sdk` 28.
- Check generated TypeScript with `tsc --noEmit` against the SDK version in
  `xoxno-lending-sdk/SKILL.md`.
- An eval must not make the answer restate a whole companion file. Point at
  the heading instead.
