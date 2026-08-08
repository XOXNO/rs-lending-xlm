# 0018. The swap payload is a packed instruction stream over address and amount registries

Status: Accepted

## Context

`execute_strategy(sender, total_in, swap_xdr)` takes the route as opaque XDR.
The first format mirrored how the off-chain router thinks: a `StrategyPayload`
struct with eleven fields, holding a `Vec<SwapPath>`, each with a `Vec<SwapHop>`
of `{pool, token_in, token_out, venue}`.

That shape is expensive on two axes. Soroban serializes a `#[contracttype]`
struct as an `ScMap` with a `Symbol` key per field, so `"burn_min_amounts"`
costs 24 bytes of key on every occurrence, and a hop — four small values —
costs 224 bytes: three 40-byte `Address` values plus keys plus a tag-only enum
encoded as a one-element vec. Nothing deduplicates, so a three-way split
repeats `token_in` and `token_out` three times each. **Observed:** a 3-path,
2-hop route with 11 distinct addresses serialized to **1972 bytes**, of which
the addresses actually carried only 440 bytes of information.

The second cost is CPU. Reading a `Vec<Struct>` costs a host call per field
access, and the path-grouping logic scanned paths quadratically
(`first_index_for_token` and `group_split_ppm` each walked every path, and both
were called per path) with two `Vec::get` host calls per probe.

On Stellar a token, a pool, and an LP share token are all `Address`, so one
registry can serve every role — a structural advantage the MultiVersX
aggregator this design borrows from does not have, where token identifiers and
contract addresses are different types needing separate registries.

## Decision

The payload is three fields — `amounts: Vec<i128>`, `assets: Vec<Address>`,
`ops: Bytes` — and every instruction references the registries by `u8` index.
`contracts/swap-aggregator/src/program.rs` defines the layout: a 10-byte header
(version, `token_in`, `token_out`, `min_out`, `u32` referral id, instruction
count, weight count), then 5-byte instruction records
`{opcode, mode, pool, token_in|lp_token, token_out|amount_index}`, then 3-byte
`u24` big-endian split weights.

`Program::decode` copies the whole stream into a stack buffer with a single
`Bytes::copy_into_slice` host call and parses it in Wasm, so no host object is
allocated per instruction. Every index is range-checked against the registry
lengths before execution begins, and `Mode::Prev` is verified structurally —
the predecessor must exist, have a single output, and that output must be this
instruction's input — so a broken chain can never reach a venue.

The `mode` byte replaces path grouping. `All` consumes the vault balance of the
input token, `Prev` consumes the previous instruction's output, `2..=127`
selects an exact amount from `amounts`, and `128..=255` selects a
parts-per-million weight. Because a `Ppm` instruction measures against the
balance *at that moment*, off-chain encoders rewrite a group's absolute weights
as successive shares of the shrinking remainder and end a fully-routed group on
`All`, which absorbs ppm rounding exactly. Splits, chained hops, and the
LP-mint pre-balance swap are all ordinary instructions; `venues/aquarius/mint.rs`
no longer carries a dedicated pre-swap step.

**Verified:** the same 3-path, 2-hop route now serializes to **600 bytes**
(−70%), and the contract Wasm shrank from 27,893 to 27,151 bytes despite
gaining a decoder, because the path-grouping, payload-validation, and pre-swap
modules went away.

## Consequences

What this makes easy: routes that no longer fit the parallel-paths shape — a
DAG, a mid-route join, a fixed-amount leg — are expressible without a wire
change, because they are just instruction sequences. Adding a venue is one
opcode. Payload size now scales with *distinct* addresses rather than hops, so
deep routes get disproportionately cheaper.

What this makes hard, and the real trade-off: the contract can no longer
re-derive route structure, so two structural checks are gone. It cannot verify
that split weights for a token sum to exactly 1e6, and it cannot verify that
every path ends at `token_out`. Both were off-chain-builder bugs that used to
surface as `SplitPpmMismatch` / `BrokenTokenChain`. What still holds them is
accounting, not structure: unrouted or misrouted funds stay in the vault, and
`execute::residual::accrue_residual_as_revenue` rejects any leftover above
`max(credited / 1e6, 1000)`, so a material mistake reverts. Below that floor a
misroute is donated to the protocol as dust rather than reverting — the same
policy that already governed rounding residue. The encoders enforce the
stricter structural rules ahead of the wire, in all three implementations.

The format is now a hard cross-language ABI. Opcodes, mode ranges, and the byte
layout are pinned by
`contracts/swap-aggregator/tests/unit/payload_wire_format.rs`,
`arb-algo/stellar-indexer/src/transaction/abi.rs`, and
`sdk-js/src/sdk/stellar/__tests__/strategy-program.test.ts`, which assert the
same fixture byte-for-byte. The header carries a version byte so a mismatched
encoder fails loudly with `InvalidRouteXdr` rather than misreading a route.

What must stay true: decoding validates every index and mode before any
external call; `Prev` stays structurally verified; both LP legs stay `All`-only,
since they consume everything the vault holds and a sized mode would be
silently ignored; and the residual guard stays the backstop that makes moving
structure off-chain safe.
