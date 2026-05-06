use flash_loan_receiver::{FlashLoanMode, FlashLoanRequest};
use soroban_sdk::{xdr::ToXdr, Env};

fn main() {
    let mode = std::env::args()
        .nth(1)
        .map(|arg| match arg.as_str() {
            "Success" => FlashLoanMode::Success,
            "NoRepay" => FlashLoanMode::NoRepay,
            "UnderRepay" => FlashLoanMode::UnderRepay,
            "ReenterPoolFlashLoan" => FlashLoanMode::ReenterPoolFlashLoan,
            "Panic" => FlashLoanMode::Panic,
            "ReenterControllerSupply" => FlashLoanMode::ReenterControllerSupply,
            _ => panic!("unknown flash loan mode: {arg}"),
        })
        .unwrap_or(FlashLoanMode::Success);

    let env = Env::default();
    let request = FlashLoanRequest { mode };
    let bytes = request.to_xdr(&env);

    for byte in bytes.iter() {
        print!("{byte:02x}");
    }
    println!();
}
