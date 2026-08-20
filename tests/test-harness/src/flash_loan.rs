use flash_loan_receiver::{FlashLoanTestReceiver, FlashLoanTestReceiverClient};
use soroban_sdk::{Address, Bytes};

use crate::context::LendingTest;
use crate::helpers::{f64_to_i128, hub_asset, HARNESS_HUB, HARNESS_SPOKE};
use crate::receivers::bad_receiver::BadFlashLoanReceiver;
use crate::receivers::flash_position::FlashPositionTestReceiver;
use crate::receivers::good_receiver::GoodFlashLoanReceiver;

impl LendingTest {
    pub fn deploy_flash_loan_receiver(&self) -> Address {
        self.env.register(GoodFlashLoanReceiver, ())
    }

    pub fn deploy_bad_flash_loan_receiver(&self) -> Address {
        self.env.register(BadFlashLoanReceiver, ())
    }

    pub fn deploy_adversarial_flash_loan_receiver(&self) -> Address {
        let receiver = self.env.register(FlashLoanTestReceiver, ());
        self.set_flash_loan_receiver_plan(&receiver, HARNESS_SPOKE, 0);
        receiver
    }

    pub fn set_flash_loan_receiver_plan(&self, receiver: &Address, spoke_id: u32, account_id: u64) {
        FlashLoanTestReceiverClient::new(&self.env, receiver).set_plan(
            &self.controller,
            &HARNESS_HUB,
            &spoke_id,
            &account_id,
        );
    }

    pub fn deploy_flash_position_receiver(&self) -> Address {
        self.env.register(FlashPositionTestReceiver, ())
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
