//! Per-tick caps on how many transactions or accounts the keeper will
//! submit. Prevents a misbehaving discovery layer from blowing through the
//! signer's fee balance in one go.

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
