//! Market, oracle, and position presets for integration tests.

use common::constants::WAD;

use crate::context::{LendingTest, LendingTestBuilder};
use crate::helpers::usd_cents;
use crate::presets::{eth_preset, usdc_preset, wbtc_preset, ALICE, BOB, LIQUIDATOR};

impl LendingTestBuilder {
    /// USDC + ETH with default reflector oracle wiring from `build()`.
    pub fn standard_two_asset(self) -> Self {
        self.with_market(usdc_preset()).with_market(eth_preset())
    }

    /// USDC + ETH, dust floors disabled (oracle tolerance / sub-$10 math tests).
    pub fn standard_two_asset_dust_disabled(self) -> LendingTest {
        self.standard_two_asset()
            .with_dust_disabled_all_markets()
            .build()
    }

    /// USDC, ETH, and WBTC markets.
    pub fn three_asset_usdc_eth_wbtc(self) -> Self {
        self.with_market(usdc_preset())
            .with_market(eth_preset())
            .with_market(wbtc_preset())
    }

    /// Three-asset market with Soroban default budget limits enabled.
    pub fn three_asset_usdc_eth_wbtc_with_budget(self) -> Self {
        self.three_asset_usdc_eth_wbtc().with_budget_enabled()
    }

    /// Built two-asset market with dual-source (primary TWAP + anchor spot) safe prices.
    pub fn dual_source_two_asset(self) -> LendingTest {
        let t = self.standard_two_asset_dust_disabled();
        configure_dual_source_oracle(&t);
        t
    }
}

/// USDC/ETH market with Alice liquidatable after USDC crash to $0.50.
pub fn liquidatable_usdc_eth() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    seed_liquidatable_usdc_eth(&mut t);
    t
}

/// Seeds Alice with liquidatable USDC/ETH debt.
pub fn seed_liquidatable_usdc_eth(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.assert_healthy(ALICE);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
}

/// Two-user seed for accounting conservation properties.
pub fn seed_fuzz_conservation_book(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 50_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.supply(BOB, "WBTC", 1.0);
}

/// Seeds a healthy two-user USDC/ETH book.
pub fn seed_standard_liquidity(t: &mut LendingTest) {
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 50.0);
}

/// Liquidator wallet with USDC for liquidation tests.
pub fn seed_liquidator_usdc(t: &mut LendingTest, amount: f64) {
    t.supply(LIQUIDATOR, "USDC", amount);
}

fn configure_dual_source_oracle(t: &LendingTest) {
    t.set_oracle_primary_anchor("USDC");
    t.set_oracle_primary_anchor("ETH");
    t.set_safe_price("USDC", WAD, true, true);
    t.set_safe_price("ETH", WAD * 2_000, true, true);
}
