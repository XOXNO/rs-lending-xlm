//! `PositionAction` from `contracts/controller/src/events/mod.rs`, limited to
//! the variants the included liquidation code emits (codes match production).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PositionAction {
    LiqRepay = 4,
    LiqSeize = 5,
    LiqCredit = 15,
}
