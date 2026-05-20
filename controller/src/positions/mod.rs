pub mod account;
pub mod borrow;
pub mod dust;
pub mod emode;
pub mod liquidation;
pub mod liquidation_math;
pub mod repay;
pub mod supply;
pub mod update;
pub mod withdraw;

use soroban_sdk::{Address, Symbol};

pub(crate) struct EventContext {
    pub caller: Address,
    pub event_caller: Address,
    pub action: Symbol,
}
