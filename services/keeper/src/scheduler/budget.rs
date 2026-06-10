//! Per-tick transaction budget.

#[derive(Debug, Clone, Copy)]
pub struct TickBudget {
    pub max_txs: usize,
    pub remaining: usize,
}

impl TickBudget {
    pub fn new(max_txs: usize) -> Self {
        Self {
            max_txs,
            remaining: max_txs,
        }
    }

    pub fn try_spend(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }

    pub fn spent(&self) -> usize {
        self.max_txs - self.remaining
    }
}
