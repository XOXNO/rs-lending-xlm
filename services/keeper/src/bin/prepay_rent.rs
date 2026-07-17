//! One-shot rent prepayment for the full keeper-discovered protocol key set.
//!
//! After `make <net> setup`: extend every live protocol entry by the standard
//! ~31-day bump (operator-funded). The daemon then rolls the shared set forward
//! each tick (14-day safety margin), so users only pay rent on their own account
//! entries (inline 5-day threshold does not fire for protocol keys).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use keeper_bot::{
    config::KeeperConfig,
    discovery::{snapshot, ContractIds},
    scheduler::tasks::{plan_extends_with_chunk, plan_restores},
    signer::Ed25519Signer,
    stellar::{
        tx::{submit_with_sim, SubmitOutcome, TxContext},
        RpcClient,
    },
};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "prepay_rent",
    about = "Extend every protocol storage entry by the keeper bump, once"
)]
struct Args {
    /// YAML config path (same shape as the keeper daemon config).
    #[arg(short, long, env = "KEEPER_CONFIG")]
    config: PathBuf,

    /// Env var with funding secret (S...). Does not use the daemon KeyVault
    /// signer — deploy tooling funds with the local deployer identity.
    #[arg(long, default_value = "PREPAY_SECRET")]
    secret_env: String,

    /// Plan only; do not submit.
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = KeeperConfig::load(&args.config)
        .with_context(|| format!("load config at {}", args.config.display()))?;

    let secret = std::env::var(&args.secret_env)
        .with_context(|| format!("missing funding secret in ${}", args.secret_env))?;
    let seed = stellar_strkey::ed25519::PrivateKey::from_string(secret.trim())
        .map_err(|e| anyhow!("invalid S... secret in ${}: {e:?}", args.secret_env))?;
    let signer = Ed25519Signer::from_seed_bytes(seed.0);

    let client = RpcClient::new(&cfg.rpc)?;
    let ids = ContractIds::resolve(&cfg.contracts)?;
    let snap = snapshot(&client, &ids, &cfg.contracts, &cfg.schedule).await?;

    println!("network        : {}", cfg.network);
    println!("controller     : {}", cfg.contracts.controller);
    println!("current ledger : {}", snap.current_ledger);
    println!("funding signer : {}", signer.public_key_strkey());

    // safety=u32::MAX ⇒ every live entry extends; archived-present restore first.
    // One key per tx keeps month-scale rent (incl. large wasm) under the u32 fee
    // cap and keeps one bad entry non-fatal.
    let restores = plan_restores(&snap, u32::MAX)?;
    let extends = plan_extends_with_chunk(&snap, u32::MAX, 1)?;
    println!(
        "planned        : {} restore tx(s), {} extend tx(s)",
        restores.len(),
        extends.len()
    );

    if args.dry_run {
        println!("dry-run — nothing submitted");
        return Ok(());
    }

    let ctx = TxContext {
        client: &client,
        signer: &signer,
        network_passphrase: &cfg.rpc.passphrase,
        base_fee_stroops: cfg.fees.base_fee_stroops,
        resource_fee_multiplier: cfg.fees.resource_fee_multiplier,
        poll_timeout_seconds: cfg.rpc.timeout_seconds as u32,
    };

    let mut succeeded = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for job in restores.into_iter().chain(extends) {
        match submit_with_sim(&ctx, job).await? {
            SubmitOutcome::Success(resp) => {
                succeeded += 1;
                println!("submitted (ledger {:?})", resp.ledger);
            }
            SubmitOutcome::SkippedSimError(reason) => {
                skipped += 1;
                println!("skipped (sim): {reason}");
            }
            SubmitOutcome::Retriable(reason) | SubmitOutcome::Failed(reason) => {
                failed += 1;
                println!("FAILED: {reason}");
                // Per-key txs: continue after a single failure.
            }
        }
    }
    println!("done: {succeeded} succeeded, {skipped} skipped, {failed} failed");
    if failed > 0 {
        anyhow::bail!("{failed} extend/restore tx(s) failed — rerun prepay_rent");
    }
    Ok(())
}
