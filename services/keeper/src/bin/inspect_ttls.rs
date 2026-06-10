//! Read-only TTL inspector for keeper-discovered entries.

use anyhow::Result;
use clap::Parser;
use keeper_bot::{
    config::{KeeperConfig, LEDGERS_PER_DAY},
    discovery::{snapshot, ContractIds},
    policy::{classify, Decision},
    stellar::{client::LedgerEntryQuery, RpcClient},
};
use std::path::PathBuf;
use stellar_xdr::curr::{ContractId, Hash, LedgerKey, ScAddress, ScMapEntry, ScSymbol, ScVal};

#[derive(Debug, Parser)]
#[command(
    name = "inspect_ttls",
    about = "Read-only TTL inspector for the XOXNO Lending keeper set"
)]
struct Args {
    /// YAML config path.
    #[arg(
        short,
        long,
        env = "KEEPER_CONFIG",
        default_value = "/etc/keeper/testnet.yaml"
    )]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = KeeperConfig::load(&args.config)?;
    let client = RpcClient::new(&cfg.rpc)?;
    let ids = ContractIds::resolve(&cfg.contracts)?;

    let snap = snapshot(&client, &ids, cfg.schedule.asset_chunk).await?;
    let current = snap.current_ledger;
    let safety = cfg.safety_margin_ledgers();

    println!("network            : {}", cfg.network);
    println!("controller         : {}", cfg.contracts.controller);
    println!("current ledger     : {current}");
    println!(
        "safety margin      : {} days ({safety} ledgers)",
        cfg.schedule.ttl_safety_margin_days
    );
    println!("assets in PoolsList: {}", snap.assets.len());
    println!();

    let mut acted = 0usize;
    acted += print_section("PERSISTENT", &snap.persistent_entries, current, safety);
    acted += print_section("INSTANCE", &snap.instance_entries, current, safety);
    acted += print_section("WASM CODE", &snap.wasm_code_entries, current, safety);

    let total =
        snap.persistent_entries.len() + snap.instance_entries.len() + snap.wasm_code_entries.len();
    println!();
    println!(
        "SUMMARY: {total} entries inspected, {acted} expired (restore) or in-margin (extend) \
         → would be acted on this tick"
    );
    Ok(())
}

fn print_section(title: &str, entries: &[LedgerEntryQuery], current: u32, safety: u32) -> usize {
    println!("── {title} ({} entries) ─────────────────", entries.len());
    let mut bumped = 0;
    for row in entries {
        let (status, acted) = status_of(row, current, safety);
        if acted {
            bumped += 1;
        }
        let live = row
            .live_until_ledger
            .map(|l| l.to_string())
            .unwrap_or_else(|| "—".to_string());
        let remaining = match row.live_until_ledger {
            Some(l) => {
                let r = l.saturating_sub(current);
                format!("{r} ledgers (~{:.1}d)", r as f64 / LEDGERS_PER_DAY as f64)
            }
            None => "—".to_string(),
        };
        println!(
            "  [{status:<22}] live_until={live:<10} remaining={remaining:<22} {}",
            label_ledger_key(&row.key)
        );
    }
    bumped
}

/// Maps an entry to display status and action flag.
fn status_of(row: &LedgerEntryQuery, current: u32, safety: u32) -> (&'static str, bool) {
    let decision = classify(row.live_until_ledger, row.value.is_some(), current, safety);
    let acted = !matches!(decision, Decision::Skip);
    let label = match decision {
        Decision::Restore => "EXPIRED (restore)",
        Decision::Extend => "IN-MARGIN (extend)",
        // The RPC omits never-written / evicted entries, so a missing value is
        // an absent entry; otherwise it has comfortable headroom.
        Decision::Skip if row.value.is_none() => "ABSENT / ARCHIVED",
        Decision::Skip if row.live_until_ledger.is_none() => "no-ttl",
        Decision::Skip => "OK",
    };
    (label, acted)
}

fn label_ledger_key(key: &LedgerKey) -> String {
    match key {
        LedgerKey::ContractData(cd) => {
            let contract = match &cd.contract {
                ScAddress::Contract(ContractId(Hash(b))) => {
                    format!("{}", stellar_strkey::Contract(*b))
                }
                other => format!("{other:?}"),
            };
            format!("{contract}  {}", label_scval_key(&cd.key))
        }
        LedgerKey::ContractCode(cc) => format!("wasm-code {}", hex::encode(cc.hash.0)),
        other => format!("{other:?}"),
    }
}

/// Decodes a contract-data key into a readable label.
fn label_scval_key(key: &ScVal) -> String {
    match key {
        ScVal::LedgerKeyContractInstance => "instance".to_string(),
        ScVal::Vec(Some(v)) => {
            v.0.iter()
                .map(label_scval_arg)
                .collect::<Vec<_>>()
                .join(" ")
        }
        other => format!("{other:?}"),
    }
}

fn label_scval_arg(v: &ScVal) -> String {
    match v {
        ScVal::Symbol(ScSymbol(s)) => s.to_utf8_string_lossy(),
        ScVal::U32(n) => n.to_string(),
        ScVal::Address(ScAddress::Contract(ContractId(Hash(b)))) => {
            format!("{}", stellar_strkey::Contract(*b))
        }
        ScVal::Address(ScAddress::Account(acc)) => {
            let stellar_xdr::curr::AccountId(stellar_xdr::curr::PublicKey::PublicKeyTypeEd25519(
                stellar_xdr::curr::Uint256(b),
            )) = acc;
            format!("{}", stellar_strkey::ed25519::PublicKey(*b))
        }
        ScVal::Map(Some(m)) => {
            let inner =
                m.0.iter()
                    .map(|ScMapEntry { key, val }| {
                        format!("{}={}", label_scval_arg(key), label_scval_arg(val))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
            format!("{{{inner}}}")
        }
        other => format!("{other:?}"),
    }
}
