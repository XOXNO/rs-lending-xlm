//! Read-only TTL inspector for keeper-discovered entries.

use anyhow::Result;
use clap::Parser;
use keeper_bot::{
    config::{KeeperConfig, LEDGERS_PER_DAY},
    discovery::{snapshot, ContractIds},
    policy::{classify, Decision},
    stellar::{client::LedgerEntryQuery, RpcClient},
};
use std::collections::BTreeMap;
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

    let snap = snapshot(&client, &ids, &cfg.contracts, &cfg.schedule).await?;
    let current = snap.current_ledger;
    let safety = cfg.safety_margin_ledgers();
    let controller_id = ids.controller;

    println!("network            : {}", cfg.network);
    println!("controller         : {}", cfg.contracts.controller);
    println!(
        "governance         : {}",
        cfg.contracts.governance.as_deref().unwrap_or("(unset)")
    );
    println!("current ledger     : {current}");
    println!(
        "safety margin      : {} days ({safety} ledgers)",
        cfg.schedule.ttl_safety_margin_days
    );
    println!("configured market assets: {}", snap.assets.len());
    println!("account nonce      : {}", snap.account_nonce);
    println!("scan users         : {}", cfg.schedule.scan_users);
    println!();

    // Partition the flat persistent set into auditable coverage classes so the
    // full key surface is visible at a glance.
    let mut classes: BTreeMap<KeyClass, Vec<&LedgerEntryQuery>> = BTreeMap::new();
    for row in &snap.persistent_entries {
        classes
            .entry(classify_persistent(
                row,
                &controller_id,
                ids.governance.as_ref(),
            ))
            .or_default()
            .push(row);
    }

    let mut acted = 0usize;
    for (class, rows) in &classes {
        let owned: Vec<LedgerEntryQuery> = rows.iter().map(|r| (*r).clone()).collect();
        acted += print_section(class.title(), &owned, current, safety);
    }
    acted += print_section(
        "INSTANCE (incl. governance)",
        &snap.instance_entries,
        current,
        safety,
    );
    acted += print_section("WASM CODE", &snap.wasm_code_entries, current, safety);

    let total =
        snap.persistent_entries.len() + snap.instance_entries.len() + snap.wasm_code_entries.len();
    println!();
    println!("── COVERAGE BY CLASS ─────────────────");
    for (class, rows) in &classes {
        let present = rows.iter().filter(|r| r.value.is_some()).count();
        println!(
            "  {:<22} {:>4} keys ({} live)",
            class.title(),
            rows.len(),
            present
        );
    }
    println!(
        "  {:<22} {:>4} keys",
        "INSTANCE",
        snap.instance_entries.len()
    );
    println!(
        "  {:<22} {:>4} keys",
        "WASM CODE",
        snap.wasm_code_entries.len()
    );
    println!();
    println!(
        "SUMMARY: {total} entries inspected, {acted} expired (restore) or in-margin (extend) \
         → would be acted on this tick"
    );
    Ok(())
}

/// Coverage class for a persistent entry, ordered for stable display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KeyClass {
    PerAsset,
    Spoke,
    PerUser,
    Roles,
    Governance,
    Other,
}

impl KeyClass {
    fn title(self) -> &'static str {
        match self {
            Self::PerAsset => "PER-ASSET",
            Self::Spoke => "SPOKE",
            Self::PerUser => "PER-USER",
            Self::Roles => "ROLES",
            Self::Governance => "GOVERNANCE",
            Self::Other => "OTHER",
        }
    }
}

/// Bucket a persistent entry into a coverage class by its variant name and the
/// contract it targets.
fn classify_persistent(
    row: &LedgerEntryQuery,
    controller_id: &[u8; 32],
    governance_id: Option<&[u8; 32]>,
) -> KeyClass {
    let LedgerKey::ContractData(cd) = &row.key else {
        return KeyClass::Other;
    };
    let on_governance = matches!(
        &cd.contract,
        ScAddress::Contract(ContractId(Hash(b))) if Some(b) == governance_id
    );
    let on_controller = matches!(
        &cd.contract,
        ScAddress::Contract(ContractId(Hash(b))) if b == controller_id
    );
    let variant = match &cd.key {
        ScVal::Vec(Some(v)) => v.0.first().and_then(|s| match s {
            ScVal::Symbol(ScSymbol(s)) => Some(s.to_utf8_string_lossy()),
            _ => None,
        }),
        _ => None,
    };
    let role_variants = [
        "ExistingRoles",
        "RoleAccountsCount",
        "RoleAccounts",
        "HasRole",
        "RoleAdmin",
    ];
    match variant.as_deref() {
        Some(v) if role_variants.contains(&v) => {
            if on_governance {
                KeyClass::Governance
            } else {
                KeyClass::Roles
            }
        }
        Some("AccountMeta" | "SupplyPositions" | "BorrowPositions") if on_controller => {
            KeyClass::PerUser
        }
        Some("Market" | "Params" | "State") => KeyClass::PerAsset,
        Some("Spoke") => KeyClass::Spoke,
        _ if on_governance => KeyClass::Governance,
        _ => KeyClass::Other,
    }
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
