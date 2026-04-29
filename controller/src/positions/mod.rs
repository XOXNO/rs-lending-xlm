pub mod account;
pub mod borrow;
pub mod emode;
pub mod liquidation;
pub mod repay;
pub mod supply;
pub mod update;
pub mod withdraw;

use soroban_sdk::{Address, Symbol};

/// Bundles the three event-identity params that every shared execution helper
/// (`execute_withdrawal`, `execute_repayment`) requires. Eliminates repeated
/// positional-arg clutter at call sites.
pub(crate) struct EventContext {
    /// Pool-call authority — the address whose tokens are moved.
    pub caller: Address,
    /// Originator recorded in the emitted `UpdatePositionEvent`. Differs from
    /// `caller` in strategy flows where the controller is the intermediate
    /// token recipient but the real initiator should appear in the log.
    pub event_caller: Address,
    /// Event tag seen by indexers (e.g. `"withdraw"`, `"liq_seize"`).
    pub action: Symbol,
}
