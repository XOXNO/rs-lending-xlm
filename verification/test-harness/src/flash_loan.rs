use flash_loan_receiver::FlashLoanTestReceiver;
use flash_loan_receiver::{FlashLoanMode, FlashLoanRequest};
use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Bytes};

use crate::context::LendingTest;
use crate::helpers::f64_to_i128;
use crate::receivers::bad_receiver::BadFlashLoanReceiver;
use crate::receivers::good_receiver::GoodFlashLoanReceiver;

impl LendingTest {
    /// Deploy a flash loan receiver that correctly repays.
    pub fn deploy_flash_loan_receiver(&self) -> Address {
        self.env.register(GoodFlashLoanReceiver, ())
    }

    /// Deploy a flash loan receiver that does NOT repay.
    pub fn deploy_bad_flash_loan_receiver(&self) -> Address {
        self.env.register(BadFlashLoanReceiver, ())
    }

    /// Deploy the standalone adversarial receiver contract used for strict
    /// repayment and callback-mode tests.
    pub fn deploy_adversarial_flash_loan_receiver(&self) -> Address {
        self.env.register(FlashLoanTestReceiver, ())
    }

    /// Execute a flash loan.
    pub fn flash_loan(&mut self, caller: &str, asset_name: &str, amount: f64, receiver: &Address) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw_amount = f64_to_i128(amount, decimals);
        let caller_addr = self.get_or_create_user(caller);
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        ctrl.flash_loan(
            &caller_addr,
            &asset_addr,
            &raw_amount,
            receiver,
            &Bytes::new(&self.env),
        );
    }

    /// Try flash loan -- returns Result.
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
        let asset_addr = self.resolve_asset(asset_name);

        let ctrl = self.ctrl_client();
        match ctrl.try_flash_loan(
            &caller_addr,
            &asset_addr,
            &raw_amount,
            receiver,
            &Bytes::new(&self.env),
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => panic!("flash loan output conversion failed"),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Try flash loan with receiver callback data -- returns contract errors directly.
    pub fn try_flash_loan_with_data(
        &mut self,
        caller: &str,
        asset_name: &str,
        amount_raw: i128,
        receiver: &Address,
        data: &Bytes,
    ) -> Result<(), soroban_sdk::Error> {
        let caller_addr = self.get_or_create_user(caller);
        let asset_addr = self.resolve_asset(asset_name);

        match self.ctrl_client().try_flash_loan(
            &caller_addr,
            &asset_addr,
            &amount_raw,
            receiver,
            data,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => panic!("flash loan output conversion failed"),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// XDR-encode adversarial receiver mode for strict repayment tests.
    pub fn flash_loan_receiver_data(&self, mode: FlashLoanMode) -> Bytes {
        FlashLoanRequest { mode }.to_xdr(&self.env)
    }

    /// Set the flash loan ongoing flag directly (escape hatch for reentrancy tests).
    pub fn set_flash_loan_ongoing(&self, ongoing: bool) {
        self.env.as_contract(&self.controller, || {
            controller::test_support::set_flash_loan_ongoing(&self.env, ongoing);
        });
    }
}
