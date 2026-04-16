use common::types::ControllerKey;
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
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    /// Set the flash loan ongoing flag directly (escape hatch for reentrancy tests).
    pub fn set_flash_loan_ongoing(&self, ongoing: bool) {
        self.env.as_contract(&self.controller, || {
            self.env
                .storage()
                .instance()
                .set(&ControllerKey::FlashLoanOngoing, &ongoing);
        });
    }
}
