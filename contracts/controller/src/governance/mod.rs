//! Owner-gated controller lifecycle: pause, upgrade, migrate, ownership.
//! Caller roles at this surface are owner / pending-owner; GUARDIAN reaches
//! `pause` via governance immediate.

pub(crate) mod access;
