use flash_loan_receiver::FlashLoanTestReceiver;
use soroban_sdk::{Address, Bytes};

use crate::context::LendingTest;
use crate::helpers::{f64_to_i128, hub_asset};
use crate::receivers::bad_receiver::BadFlashLoanReceiver;
use crate::receivers::good_receiver::GoodFlashLoanReceiver;

impl LendingTest {
    pub fn deploy_flash_loan_receiver(&self) -> Address {
        self.env.register(GoodFlashLoanReceiver, ())
    }

    pub fn deploy_bad_flash_loan_receiver(&self) -> Address {
        self.env.register(BadFlashLoanReceiver, ())
    }

    pub fn deploy_adversarial_flash_loan_receiver(&self) -> Address {
        self.env.register(FlashLoanTestReceiver, ())
    }

    pub fn flash_loan(&mut self, caller: &str, asset_name: &str, amount: f64, receiver: &Address) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let caller_addr = self.get_or_create_user(caller);
        let asset = hub_asset(self.resolve_asset(asset_name));

        let ctrl = self.ctrl_client();
        ctrl.flash_loan(
            &caller_addr,
            &asset,
            &raw_amount,
            receiver,
            &Bytes::new(&self.env),
        );
    }

    pub fn try_flash_loan(
        &mut self,
        caller: &str,
        asset_name: &str,
        amount: f64,
        receiver: &Address,
    ) -> Result<(), soroban_sdk::Error> {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let caller_addr = self.get_or_create_user(caller);
        let asset = hub_asset(self.resolve_asset(asset_name));

        let ctrl = self.ctrl_client();
        match ctrl.try_flash_loan(
            &caller_addr,
            &asset,
            &raw_amount,
            receiver,
            &Bytes::new(&self.env),
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => panic!("flash loan output conversion failed"),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    pub fn try_flash_loan_with_data(
        &mut self,
        caller: &str,
        asset_name: &str,
        amount_raw: i128,
        receiver: &Address,
        data: &Bytes,
    ) -> Result<(), soroban_sdk::Error> {
        let caller_addr = self.get_or_create_user(caller);
        let asset = hub_asset(self.resolve_asset(asset_name));

        match self
            .ctrl_client()
            .try_flash_loan(&caller_addr, &asset, &amount_raw, receiver, data)
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => panic!("flash loan output conversion failed"),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    pub fn set_flash_loan_ongoing(&self, ongoing: bool) {
        self.env.as_contract(&self.controller, || {
            controller::test_support::set_flash_loan_ongoing(&self.env, ongoing);
        });
    }
}
