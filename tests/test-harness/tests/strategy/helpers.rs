use controller::types::{PositionMode, StrategySwap};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes, Vec};
use test_harness::mock_blend::MockBlend;
use test_harness::{f64_to_i128, hub_asset, FlashPositionRequest, HubAssetKey, LendingTest, ALICE};

pub fn usdc_raw(t: &LendingTest, amount: f64) -> i128 {
    f64_to_i128(amount, t.resolve_market("USDC").decimals)
}

pub fn data(t: &LendingTest, req: FlashPositionRequest) -> Bytes {
    req.to_xdr(&t.env)
}

pub fn collaterals(t: &LendingTest, pairs: &[(&str, f64)]) -> Vec<(HubAssetKey, i128)> {
    let mut out = Vec::new(&t.env);
    for (name, min) in pairs {
        let decimals = t.resolve_market(name).decimals;
        out.push_back((
            hub_asset(t.resolve_asset(name)),
            f64_to_i128(*min, decimals),
        ));
    }
    out
}

/// Entry points bound to the argument shape most strategy tests share, so a
/// test body shows only what it varies. A test that varies a pinned argument
/// calls the underlying harness method directly.
pub trait AliceOps {
    /// `try_flash_position` as ALICE on a fresh account, `Multiply` mode,
    /// borrowing 1.0 ETH.
    fn try_alice_eth_flash(
        &mut self,
        receiver: &Address,
        payload: &Bytes,
        mins: &Vec<(HubAssetKey, i128)>,
        refunds: &Vec<Address>,
    ) -> Result<u64, soroban_sdk::Error>;

    /// `try_multiply` as ALICE opening a `Multiply` position: USDC collateral
    /// against 1.0 ETH of debt.
    fn try_alice_multiply(&mut self, steps: &StrategySwap) -> Result<u64, soroban_sdk::Error>;
}

impl AliceOps for LendingTest {
    fn try_alice_eth_flash(
        &mut self,
        receiver: &Address,
        payload: &Bytes,
        mins: &Vec<(HubAssetKey, i128)>,
        refunds: &Vec<Address>,
    ) -> Result<u64, soroban_sdk::Error> {
        self.try_flash_position(
            ALICE,
            0,
            PositionMode::Multiply,
            "ETH",
            1.0,
            receiver,
            payload,
            mins,
            refunds,
        )
    }

    fn try_alice_multiply(&mut self, steps: &StrategySwap) -> Result<u64, soroban_sdk::Error> {
        self.try_multiply(ALICE, "USDC", 1.0, "ETH", PositionMode::Multiply, steps)
    }
}

/// Register a `MockBlend` pool and put it on the controller's approved list.
pub fn register_approved_blend(t: &LendingTest) -> Address {
    let addr = t.env.register(MockBlend, ());
    let admin = t.admin();
    t.gov_client().execute_immediate(
        &admin,
        &governance_interface::AdminOperation::ApproveBlendPool(addr.clone()),
    );
    addr
}
