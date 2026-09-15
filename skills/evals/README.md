# Skill evaluations

Grader fixtures only. Do not load this directory to answer a user task.

Scenarios for the XOXNO skills, in the [stellar-dev-skill evals format](https://github.com/stellar/stellar-dev-skill/blob/main/evals/README.md).

- Paths: `scenarios/<skill>/NN-<slug>.json`
- Machine checks: generated Rust compiles for `wasm32v1-none` against pinned `soroban-sdk = "=27.0.6"`; generated TypeScript passes `tsc --noEmit` against the `@xoxno/sdk-js` version named in `xoxno-lending-sdk/SKILL.md`
- An eval that requires restating a whole companion file is too greedy: point at the heading instead.
