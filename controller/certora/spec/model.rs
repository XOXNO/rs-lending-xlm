/// Ghost state and helpers for Certora verification.
///
/// Ghost variables track verification-only state that doesn't exist in the
/// actual contract. Skolem variables allow proving universally quantified
/// properties for an arbitrary representative.
// ---------------------------------------------------------------------------
// Ghost state — tracked by the prover, not stored on-chain
// ---------------------------------------------------------------------------

/// Whether a health factor check was performed during this operation.
static mut GHOST_HEALTH_CHECKED: bool = false;

/// Whether the flash loan reentrancy guard was set during callback.
static mut GHOST_FLASH_LOAN_GUARD_SET: bool = false;

/// Snapshot of supply_index before an operation (for monotonicity checks).
static mut GHOST_SUPPLY_INDEX_BEFORE: i128 = 0;

/// Snapshot of borrow_index before an operation.
static mut GHOST_BORROW_INDEX_BEFORE: i128 = 0;

// ---------------------------------------------------------------------------
// Skolem variables — arbitrary representatives for universal properties
// ---------------------------------------------------------------------------

/// Arbitrary account ID for proving per-account properties.
static mut SKOLEM_ACCOUNT_ID: u64 = 0;

/// Arbitrary asset index for proving per-asset properties.
static mut SKOLEM_ASSET_INDEX: u32 = 0;

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

pub fn get_health_checked() -> bool {
    unsafe { GHOST_HEALTH_CHECKED }
}

pub fn set_health_checked(val: bool) {
    unsafe {
        GHOST_HEALTH_CHECKED = val;
    }
}

pub fn get_flash_loan_guard_set() -> bool {
    unsafe { GHOST_FLASH_LOAN_GUARD_SET }
}

pub fn set_flash_loan_guard_set(val: bool) {
    unsafe {
        GHOST_FLASH_LOAN_GUARD_SET = val;
    }
}

pub fn get_supply_index_before() -> i128 {
    unsafe { GHOST_SUPPLY_INDEX_BEFORE }
}

pub fn set_supply_index_before(val: i128) {
    unsafe {
        GHOST_SUPPLY_INDEX_BEFORE = val;
    }
}

pub fn get_borrow_index_before() -> i128 {
    unsafe { GHOST_BORROW_INDEX_BEFORE }
}

pub fn set_borrow_index_before(val: i128) {
    unsafe {
        GHOST_BORROW_INDEX_BEFORE = val;
    }
}

pub fn skolem_account_id() -> u64 {
    unsafe { SKOLEM_ACCOUNT_ID }
}

pub fn skolem_asset_index() -> u32 {
    unsafe { SKOLEM_ASSET_INDEX }
}

/// Initialize all ghost/skolem state to nondeterministic values.
/// Called at the start of each rule.
pub fn init() {
    unsafe {
        GHOST_HEALTH_CHECKED = cvlr::nondet::nondet();
        GHOST_FLASH_LOAN_GUARD_SET = cvlr::nondet::nondet();
        GHOST_SUPPLY_INDEX_BEFORE = cvlr::nondet::nondet();
        GHOST_BORROW_INDEX_BEFORE = cvlr::nondet::nondet();
        SKOLEM_ACCOUNT_ID = cvlr::nondet::nondet();
        SKOLEM_ASSET_INDEX = cvlr::nondet::nondet();
    }
}
