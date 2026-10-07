# Skill evaluations

Grader fixtures only. Do not load this directory to answer a user task.

Scenarios for the XOXNO skills, in the [stellar-dev-skill evals format](https://github.com/stellar/stellar-dev-skill/blob/main/evals/README.md).

- Paths: `scenarios/<skill>/NN-<slug>.json`
- Build generated Rust with `stellar contract build` and stellar-cli 25.2 or
  newer. Use `soroban-sdk = "28"` and `xoxno-contract-sdk = "0.2"`.
- Do not use plain `cargo build` for a Wasm target with `soroban-sdk` 28.
- Check generated TypeScript with `tsc --noEmit`. Use the SDK version named
  in `xoxno-lending-sdk/SKILL.md`.
- Do not write an eval that requires restating a whole companion file. Point at the heading instead.
