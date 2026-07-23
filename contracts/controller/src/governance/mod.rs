//! Owner-gated controller lifecycle: pause, upgrade, migrate, ownership.
//! Caller roles at this surface are owner / pending-owner; GUARDIAN reaches
//! `pause` via governance immediate. See
//! [ADR 0010](../../../docs/explanation/decisions/0010-governance-timelock-for-controller-admin.md)
//! and [INVARIANTS](../../../docs/reference/invariants.md) §5.1 / §5.4.

pub(crate) mod access;
