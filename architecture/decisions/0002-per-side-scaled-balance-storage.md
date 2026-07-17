# ADR 0002: Per-Side Scaled-Balance Storage

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team

## Context

Accounts hold supply and debt across multiple `HubAssetKey` markets. Interest
must accrue without rewriting every account. Soroban charges per persistent
entry and needs TTL renewal — a single large account blob would force
supply/repay/withdraw to touch unrelated positions.

## Decision

Scaled-balance accounting with account state split by side:

| Key | Content |
|-----|---------|
| `ControllerKey::AccountMeta(u64)` | Owner, spoke id, position mode |
| `ControllerKey::SupplyPositions(u64)` | `Map<HubAssetKey, AccountPositionRaw>` |
| `ControllerKey::BorrowPositions(u64)` | `Map<HubAssetKey, DebtPositionRaw>` |

Balances are RAY-scaled shares. Actual token amounts use pool supply/borrow
indexes. Indexes advance via pool market sync, not account sweeps.

Collateral (supply) positions also store risk params (LTV, liquidation
threshold/bonus/fees) used by HF/LTV math at open (or refresh). Debt positions
carry only the scaled share. Risk params refresh via
`update_account_threshold` (or on supply) when spoke config changes.

Empty side maps are pruned on write to bound storage/TTL cost.

## Alternatives considered

- **Token-native balances** — interest would need account sweeps or stale balances.  
- **One combined `Positions(id)` map** — side-specific flows would read the other side.  
- **One key per account/asset/side** — multiplies entries and TTL surface.  

## Consequences

**Positive:** O(1) interest per market row; supply-only / repay-only touch one
side; TTL bounded (meta + two maps); keys match hub isolation; debt stays lean.

**Costs:** large accounts still pay map costs on the side they touch; risk
param changes need explicit refresh for existing collateral.

## References

- `common/src/types/controller.rs`  
- `contracts/controller/src/storage/account.rs`  
- `contracts/controller/src/positions`  
- `contracts/controller/src/risk/params.rs`, `pool_ops/mod.rs`  
- `contracts/pool/src/interest.rs`  
- [INVARIANTS.md](../INVARIANTS.md) §1.3, §5.2  
