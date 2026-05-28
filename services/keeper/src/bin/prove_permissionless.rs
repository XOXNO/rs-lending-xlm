//! One-shot proof that `ExtendFootprintTtl` is permissionless for the
//! controller's *storage* (not just its wasm-code) entries.
//!
//! Builds an extend op targeting:
//!   - the controller's instance entry (ContractData / Persistent /
//!     LedgerKeyContractInstance)
//!   - the controller's persistent `PoolsList` entry
//!
//! Signs from our throwaway signer (which does NOT hold the KEEPER role)
//! and submits via send_transaction_polling. If the tx lands, no on-chain
//! role grant is required for TTL bumping of contract storage.

use anyhow::{anyhow, Result};
use clap::Parser;
use keeper_bot::{
    keys::{contract_instance_key, ControllerPersistentKey},
    signer::signer_from_mnemonic,
    stellar::{
        client::{contract_id_from_strkey, RpcClient},
        tx::{submit_with_sim, SubmitOutcome, TxContext},
        ttl::extend_footprint_ttl,
    },
};

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "https://soroban-testnet.stellar.org")]
    rpc: String,
    #[arg(long, default_value = "Test SDF Network ; September 2015")]
    passphrase: String,
    #[arg(long, default_value = "CBSCWXCIAASFR2F2332D2I7C6VWUJZKUW4ONOZR2LZ32KOZ5UZVNJ3LA")]
    controller: String,
    #[arg(long)]
    mnemonic: String,
    #[arg(long, default_value = "m/44'/148'/0'")]
    derivation_path: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let rpc_cfg = keeper_bot::config::RpcConfig {
        url: args.rpc.clone(),
        passphrase: args.passphrase.clone(),
        timeout_seconds: 30,
    };
    let client = RpcClient::new(&rpc_cfg)?;
    let signer = signer_from_mnemonic(&args.mnemonic, &args.derivation_path)?;
    println!("signer: {}", signer.public_key_strkey());

    let controller_id = contract_id_from_strkey(&args.controller)?;

    let instance_key = contract_instance_key(&controller_id);
    let pools_list_key = ControllerPersistentKey::PoolsList.to_ledger_key(&controller_id)?;

    println!("attempting external ExtendFootprintTtl over:");
    println!("  - controller instance (ContractData / Persistent)");
    println!("  - controller PoolsList persistent entry");

    let job = extend_footprint_ttl(&[instance_key, pools_list_key], 535_679)?;

    let ctx = TxContext {
        client: &client,
        signer: &signer,
        network_passphrase: &args.passphrase,
        base_fee_stroops: 100,
        resource_fee_multiplier: 1.20,
        poll_timeout_seconds: 60,
    };

    match submit_with_sim(&ctx, job).await? {
        SubmitOutcome::Success(resp) => {
            println!("SUCCESS — tx {:?} landed at ledger {:?}", resp.tx_hash, resp.ledger);
            println!("→ ExtendFootprintTtl IS permissionless for contract storage entries");
        }
        SubmitOutcome::SkippedSimError(reason) => {
            println!("SIM REJECTED — {}", reason);
            return Err(anyhow!("simulation rejected"));
        }
        SubmitOutcome::Retriable(reason) => {
            println!("RETRIABLE — {}", reason);
            return Err(anyhow!("retriable failure"));
        }
        SubmitOutcome::Failed(reason) => {
            println!("FAILED — {}", reason);
            return Err(anyhow!("on-chain failure"));
        }
    }
    Ok(())
}
