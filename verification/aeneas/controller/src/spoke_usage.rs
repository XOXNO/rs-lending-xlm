//! `UsageSide` from `contracts/controller/src/spoke_usage.rs`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UsageSide {
    Supply,
    Borrow,
}
