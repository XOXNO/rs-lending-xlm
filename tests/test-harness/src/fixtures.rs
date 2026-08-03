use controller::constants::WAD;

use crate::context::{LendingTest, LendingTestBuilder};
use crate::helpers::usd_cents;
use crate::presets::{eth_preset, usdc_preset, wbtc_preset, ALICE, BOB, LIQUIDATOR};

impl LendingTestBuilder {
    pub fn standard_two_asset(self) -> Self {
        self.with_market(usdc_preset()).with_market(eth_preset())
    }

    pub fn standard_two_asset_dust_disabled(self) -> LendingTest {
        self.standard_two_asset()
            .with_dust_disabled_all_markets()
            .build()
    }

    pub fn three_asset_usdc_eth_wbtc(self) -> Self {
        self.with_market(usdc_preset())
            .with_market(eth_preset())
            .with_market(wbtc_preset())
    }

    pub fn three_asset_usdc_eth_wbtc_with_budget(self) -> Self {
        self.three_asset_usdc_eth_wbtc().with_budget_enabled()
    }

    pub fn dual_source_two_asset(self) -> LendingTest {
        let t = self.standard_two_asset_dust_disabled();
        configure_dual_source_oracle(&t);
        t
    }
}

pub fn liquidatable_usdc_eth() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_liquidatable_usdc_eth(&mut t);
    t
}

pub fn seed_liquidatable_usdc_eth(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
}

pub fn seed_fuzz_conservation_book(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 50_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.supply(BOB, "WBTC", 1.0);

    t.borrow(ALICE, "ETH", 5.0);
    t.borrow(BOB, "USDC", 5_000.0);
}

pub fn seed_standard_liquidity(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 50.0);
}

pub fn seed_liquidator_usdc(t: &mut LendingTest, amount: f64) {
    t.supply(LIQUIDATOR, "USDC", amount);
}

fn configure_dual_source_oracle(t: &LendingTest) {
    t.set_oracle_primary_anchor("USDC");
    t.set_oracle_primary_anchor("ETH");
    t.set_safe_price("USDC", WAD);
    t.set_safe_price("ETH", WAD * 2_000);
}
