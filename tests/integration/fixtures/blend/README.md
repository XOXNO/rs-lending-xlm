# Blend pool fixture

This directory holds a Blend v2 pool executable fetched from Stellar testnet. Its
contract meta names `github:blend-capital/blend-contracts-v2`, Rust 1.81.0, and
soroban-sdk 22.0.7. The `blend` and `sdk` live lanes deploy a fresh pool from the
executable on every run.

| File | Source contract | SHA-256 |
|---|---|---|
| `blend_pool.wasm` | CAPBMXIQTICKWFPWFDJWMAKBXBPJZUKLNONQH3MLPLLBKQ643CYN5PRW | `a41fc53d6753b6c04eb15b021c55052366a4c8e0e21bc72700f461264ec1350e` |

To refetch the file, run
`stellar contract fetch --id <source contract> --network testnet -o blend_pool.wasm`.

`scripts/build_e2e_wasm.sh fixtures` checks this hash and copies the file into
`artifacts/wasm/fixtures`, so the file gets the same `SHA256SUMS` check as every
other live fixture. A changed file fails the build. Change the file and the
pinned hash only together and only on purpose.

## Backstop

A new pool can borrow only after `set_status(0)`, and that call reads
`pool_data` from the pool's backstop. The lanes use
[`mock-blend-backstop`](../../../../mock/mock-blend-backstop/src/lib.rs), which
reports a funded backstop and rejects every other call. The migration under
test never reaches the backstop.
