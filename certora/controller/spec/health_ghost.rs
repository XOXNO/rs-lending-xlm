//! Flag set by `require_post_pool_risk_gates` after the solvency check (certora build only).

static mut GHOST_HF_CHECKED: bool = false;

pub fn reset() {
    unsafe { GHOST_HF_CHECKED = false }
}

pub fn set_checked() {
    unsafe { GHOST_HF_CHECKED = true }
}

pub fn get_checked() -> bool {
    unsafe { GHOST_HF_CHECKED }
}