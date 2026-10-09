use std::env::var_os;
use std::fs::read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use common::types::{HubAssetKey, SeizeMode};
use controller::constants::WAD;
use controller::ControllerClient;
use soroban_sdk::testutils::{Ledger, MockAuthInvoke};
use soroban_sdk::token::Client as TokenClient;
use soroban_sdk::xdr::{
    AccountId, AlphaNum12, AlphaNum4, AssetCode12, AssetCode4, LedgerEntry, LedgerEntryData,
    LedgerEntryExt, LedgerKey, LedgerKeyTrustLine, PublicKey, ScAddress, SorobanAuthorizationEntry,
    SorobanCredentials, TrustLineAsset, TrustLineEntry, TrustLineEntryExt, Uint256,
};
use soroban_sdk::{
    Address, Bytes, ContractExecutable, Env, IntoVal, TryIntoVal, Vec as SorobanVec,
};
use test_harness::freezable_token::FreezableToken;
use test_harness::mainnet::MainnetSpoke;
use test_harness::mock_reflector::MockReflector;
use test_harness::{ALICE, CAROL};

const NETWORK_CPU: i64 = 400_000_000;
const CPU_CEILING: i64 = 360_000_000;
const NETWORK_MEM: i64 = 41_943_040;
const NETWORK_LEDGER_ENTRIES: u32 = 400;
const NETWORK_DISK_READ_ENTRIES: u32 = 200;
const NETWORK_WRITES: u32 = 200;
const NETWORK_READ_BYTES: u32 = 200_000;
const NETWORK_WRITE_BYTES: u32 = 132_096;
const NETWORK_EVENT_BYTES: u32 = 16_384;
const LEGACY_LEDGER_ENTRIES: u32 = 100;
const LEGACY_WRITES: u32 = 50;

struct Book {
    spoke: u32,
    collateral: &'static [(&'static str, i128)],
    debt: &'static [(&'static str, i128)],
    crash: &'static [(&'static str, i128)],
    widen_bands: bool,
}

const BLUE_CHIP: Book = Book {
    spoke: 1,
    collateral: &[
        ("XLM", 5 * WAD),
        ("SolvBTC", 3 * WAD / 2),
        ("xSolvBTC", 3 * WAD / 2),
        ("EURC", WAD / 20),
        ("PYUSD", WAD / 20),
    ],
    debt: &[
        ("USDC", WAD),
        ("USDT0", WAD),
        ("XLM", WAD),
        ("EURC", 3 * WAD / 4),
        ("PYUSD", 3 * WAD / 4),
    ],
    crash: &[
        ("XLM", 53_000_000_000_000_000),
        ("SolvBTC", 20_000 * WAD),
        ("xSolvBTC", 20_000 * WAD),
    ],
    widen_bands: false,
};

const STABLES: Book = Book {
    spoke: 4,
    collateral: &[
        ("USST", 9 * WAD / 2),
        ("USDY", 9 * WAD / 2),
        ("USDC", 3 * WAD / 10),
        ("EURC", 3 * WAD / 10),
        ("PYUSD", 3 * WAD / 10),
    ],
    debt: &[
        ("USDC", 2 * WAD),
        ("EURC", 2 * WAD),
        ("PYUSD", 2 * WAD),
        ("USDT0", 3 * WAD / 2),
    ],
    crash: &[
        ("USST", 300_000_000_000_000_000),
        ("USDY", 340_000_000_000_000_000),
    ],
    widen_bands: true,
};

const AMM_LP: Book = Book {
    spoke: 5,
    collateral: &[
        ("XLMSolvBTC_LP", 3 * WAD),
        ("AQUAUSDC_LP", 3 * WAD),
        ("XLMUSDC_LP", 5 * WAD / 2),
        ("xSolvBTCSolvBTC_LP", WAD),
        ("USDYUSDC_LP", WAD),
    ],
    debt: &[
        ("USDC", 2 * WAD),
        ("EURC", 3 * WAD / 2),
        ("PYUSD", 13 * WAD / 10),
        ("XLM", WAD / 10),
    ],
    crash: &[
        ("XLMSolvBTC_LP", 25 * WAD),
        ("AQUAUSDC_LP", 13_000_000_000_000_000),
        ("XLMUSDC_LP", 460_000_000_000_000_000),
        ("xSolvBTCSolvBTC_LP", 4_000 * WAD),
        ("USDYUSDC_LP", 1_800_000_000_000_000_000),
    ],
    widen_bands: false,
};

#[derive(Clone, Copy)]
enum Call {
    Liquidate(SeizeMode),
    CleanBadDebt,
}

struct Prepared {
    controller: Address,
    reflector: Address,
    native_tokens: Vec<Address>,
    pool: Address,
    source: AccountId,
    liquidator: Address,
    account_id: u64,
    payments: Vec<(HubAssetKey, i128)>,
}

fn wasm_dir() -> PathBuf {
    var_os("STRESS_BUDGET_WASM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/wasm/deploy")
        })
}

fn add_trustline(env: &Env, source: &AccountId, token: &Address) -> bool {
    let name = TokenClient::new(env, token).name().to_string();
    let Some((code, issuer)) = name.split_once(':') else {
        return false;
    };
    let ScAddress::Account(issuer) = ScAddress::from(&Address::from_str(env, issuer)) else {
        panic!("SAC issuer must be an account")
    };
    let asset = if code.len() <= 4 {
        let mut c = [0; 4];
        c[..code.len()].copy_from_slice(code.as_bytes());
        TrustLineAsset::CreditAlphanum4(AlphaNum4 {
            asset_code: AssetCode4(c),
            issuer,
        })
    } else {
        let mut c = [0; 12];
        c[..code.len()].copy_from_slice(code.as_bytes());
        TrustLineAsset::CreditAlphanum12(AlphaNum12 {
            asset_code: AssetCode12(c),
            issuer,
        })
    };
    let key = LedgerKey::Trustline(LedgerKeyTrustLine {
        account_id: source.clone(),
        asset: asset.clone(),
    });
    let entry = LedgerEntry {
        last_modified_ledger_seq: 0,
        ext: LedgerEntryExt::V0,
        data: LedgerEntryData::Trustline(TrustLineEntry {
            account_id: source.clone(),
            asset,
            balance: 0,
            limit: i64::MAX,
            flags: 1,
            ext: TrustLineEntryExt::V0,
        }),
    };
    env.host()
        .add_ledger_entry(&Rc::new(key), &Rc::new(entry), None)
        .unwrap();
    true
}

fn prepare(book: &Book) -> (MainnetSpoke, Prepared) {
    let mut m = MainnetSpoke::build(book.spoke);
    for (asset, usd) in book.collateral {
        let units = m.usd_to_raw(asset, *usd);
        m.supply(ALICE, asset, units);
    }
    for (asset, usd) in book.debt {
        if m.listing(asset).can_be_collateral {
            let depth = m.usd_to_raw(asset, 10_000 * WAD);
            m.supply(CAROL, asset, depth);
        }
        let units = m.usd_to_raw(asset, *usd);
        m.borrow(ALICE, asset, units);
    }
    let account_id = m.account_id(ALICE);
    let (supply, borrow) = m.t.ctrl_client().get_account_positions(&account_id);
    assert_eq!(
        (supply.len() as usize, borrow.len() as usize),
        (book.collateral.len(), book.debt.len())
    );

    for (asset, price) in book.crash {
        if book.widen_bands {
            m.t.seed_sanity_band(asset, price / 2, m.price(asset) * 2);
        }
        m.set_price(asset, *price);
    }
    let ctrl = m.t.ctrl_client();
    let c = ctrl.get_total_collateral_usd(&account_id);
    let d = ctrl.get_total_borrow_usd(&account_id);
    assert!(
        d > c && c <= 5 * WAD,
        "book must be dust-cleanable: C={c} D={d}"
    );

    let source = AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([83; 32])));
    let liquidator: Address = ScAddress::Account(source.clone())
        .try_into_val(&m.t.env)
        .unwrap();
    let mut native_tokens = Vec::new();
    for listing in &m.config.listings {
        let token = m.t.resolve_asset(&listing.asset);
        if !add_trustline(&m.t.env, &source, &token) {
            native_tokens.push(token);
        }
    }

    let quote = m
        .estimate(
            ALICE,
            &book
                .debt
                .iter()
                .map(|(a, _)| (*a, 2 * m.debt_raw(ALICE, a)))
                .collect::<Vec<_>>(),
            SeizeMode::Transfer,
        )
        .max_payment_wad;
    let legs = book.debt.len() as i128;
    let offers: Vec<(&str, i128)> = book
        .debt
        .iter()
        .map(|(a, _)| {
            (
                *a,
                (m.usd_to_raw(a, quote / legs) - 1).min(m.debt_raw(ALICE, a)),
            )
        })
        .collect();
    let estimate = m.estimate(ALICE, &offers, SeizeMode::Transfer);
    assert!(estimate.refunds.is_empty(), "offers stay within the quote");
    assert_eq!(
        estimate.seized_collaterals.len() as usize,
        book.collateral.len(),
        "every collateral leg is seized"
    );
    for (asset, amount) in &offers {
        m.t.resolve_market(asset)
            .token_admin
            .mint(&liquidator, amount);
    }
    let payments = offers.iter().map(|(a, x)| (m.hub_asset(a), *x)).collect();

    let pool = m.t.resolve_market(book.debt[0].0).pool.clone();
    let dir = wasm_dir();
    for (address, file) in [
        (&m.t.controller, "controller.wasm"),
        (&m.t.price_aggregator, "price_aggregator.wasm"),
        (&pool, "pool.wasm"),
        (&m.t.position_nft, "position_nft.wasm"),
    ] {
        let wasm = read(dir.join(file))
            .unwrap_or_else(|e| panic!("{file}: {e}; run `make deploy-artifacts`"));
        let hash =
            m.t.env
                .deployer()
                .upload_contract_wasm(Bytes::from_slice(&m.t.env, &wasm));
        m.t.env.as_contract(address, || {
            m.t.env
                .deployer()
                .update_current_contract(ContractExecutable::Wasm(hash))
        });
    }
    let prepared = Prepared {
        controller: m.t.controller.clone(),
        reflector: m.t.mock_reflector.clone(),
        native_tokens,
        pool,
        source,
        liquidator,
        account_id,
        payments,
    };
    (m, prepared)
}

struct Usage {
    cpu: i64,
    mem: i64,
    entries: u32,
    disk_reads: u32,
    writes: u32,
    read_bytes: u32,
    write_bytes: u32,
    event_bytes: u32,
}

fn replay(p: &Prepared, env: Env, call: Call) -> Usage {
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    let rebind = |a: &Address| -> Address { ScAddress::from(a).try_into_val(&env).unwrap() };
    env.register_at(&rebind(&p.reflector), MockReflector, ());
    for token in &p.native_tokens {
        env.register_at(&rebind(token), FreezableToken, ());
    }
    let controller = rebind(&p.controller);
    let signer = rebind(&p.liquidator);
    let pool = rebind(&p.pool);
    let mut offered = SorobanVec::new(&env);
    let mut assets = Vec::new();
    for (key, amount) in &p.payments {
        let asset = rebind(&key.asset);
        offered.push_back((
            HubAssetKey {
                hub_id: key.hub_id,
                asset: asset.clone(),
            },
            *amount,
        ));
        assets.push((asset, *amount));
    }
    let transfers: Vec<MockAuthInvoke> = assets
        .iter()
        .map(|(asset, amount)| MockAuthInvoke {
            contract: asset,
            fn_name: "transfer",
            args: (signer.clone(), pool.clone(), *amount).into_val(&env),
            sub_invokes: &[],
        })
        .collect();
    let root = match call {
        Call::Liquidate(mode) => MockAuthInvoke {
            contract: &controller,
            fn_name: "liquidate",
            args: (signer.clone(), p.account_id, offered.clone(), mode).into_val(&env),
            sub_invokes: &transfers,
        },
        Call::CleanBadDebt => MockAuthInvoke {
            contract: &controller,
            fn_name: "clean_bad_debt",
            args: (signer.clone(), p.account_id).into_val(&env),
            sub_invokes: &[],
        },
    };
    env.host().set_source_account(p.source.clone()).unwrap();
    env.set_auths(&[SorobanAuthorizationEntry {
        credentials: SorobanCredentials::SourceAccount,
        root_invocation: (&root).into(),
    }]);
    let client = ControllerClient::new(&env, &controller);
    match call {
        Call::Liquidate(mode) => {
            client.liquidate(&signer, &p.account_id, &offered, &mode);
        }
        Call::CleanBadDebt => client.clean_bad_debt(&signer, &p.account_id),
    }
    let r = env.cost_estimate().resources();
    let usage = Usage {
        cpu: r.instructions,
        mem: r.mem_bytes,
        entries: r.disk_read_entries + r.memory_read_entries + r.write_entries,
        disk_reads: r.disk_read_entries,
        writes: r.write_entries,
        read_bytes: r.disk_read_bytes,
        write_bytes: r.write_bytes,
        event_bytes: r.contract_events_size_bytes,
    };
    assert!(
        client.try_get_account_positions(&p.account_id).is_err()
            || client.get_account_positions(&p.account_id).1.is_empty(),
        "the call clears the debt"
    );
    usage
}

fn headroom(used: i64, limit: i64) -> f64 {
    (limit - used) as f64 * 100.0 / limit as f64
}

fn measure(label: &str, book: &Book) {
    let (m, p) = prepare(book);
    let snapshot = m.t.env.to_ledger_snapshot();
    m.t.env
        .host()
        .ensure_module_cache_contains_host_storage_contracts()
        .unwrap();
    let module_cache = m.t.env.host().take_module_cache().unwrap();
    for (name, call, delay) in [
        (
            "liquidate Transfer",
            Call::Liquidate(SeizeMode::Transfer),
            0,
        ),
        (
            "liquidate Transfer",
            Call::Liquidate(SeizeMode::Transfer),
            5,
        ),
        (
            "liquidate Credit(0)",
            Call::Liquidate(SeizeMode::Credit(0)),
            0,
        ),
        (
            "liquidate Credit(0)",
            Call::Liquidate(SeizeMode::Credit(0)),
            5,
        ),
        ("clean_bad_debt", Call::CleanBadDebt, 0),
        ("clean_bad_debt", Call::CleanBadDebt, 5),
    ] {
        let env = Env::from_ledger_snapshot(snapshot.clone());
        env.host().set_module_cache(module_cache.clone()).unwrap();
        env.ledger().with_mut(|info| {
            info.timestamp += delay;
            info.sequence_number += 1;
        });
        let name = format!("{name} +{delay}s");
        let u = replay(&p, env, call);
        let checks: [(&str, i64, i64); 8] = [
            ("cpu", u.cpu, NETWORK_CPU),
            ("mem", u.mem, NETWORK_MEM),
            (
                "ledger_entries",
                i64::from(u.entries),
                i64::from(NETWORK_LEDGER_ENTRIES),
            ),
            (
                "disk_read_entries",
                i64::from(u.disk_reads),
                i64::from(NETWORK_DISK_READ_ENTRIES),
            ),
            (
                "write_entries",
                i64::from(u.writes),
                i64::from(NETWORK_WRITES),
            ),
            (
                "disk_read_bytes",
                i64::from(u.read_bytes),
                i64::from(NETWORK_READ_BYTES),
            ),
            (
                "write_bytes",
                i64::from(u.write_bytes),
                i64::from(NETWORK_WRITE_BYTES),
            ),
            (
                "event_bytes",
                i64::from(u.event_bytes),
                i64::from(NETWORK_EVENT_BYTES),
            ),
        ];
        for (metric, used, limit) in checks {
            println!(
                "{label} | {name} | {metric}={used} limit={limit} headroom={:.1}%",
                headroom(used, limit)
            );
            assert!(used <= limit, "{label} {name}: {metric} {used} > {limit}");
        }
        println!(
            "{label} | {name} | legacy footprint gate: ledger_entries {}/{LEGACY_LEDGER_ENTRIES} write_entries {}/{LEGACY_WRITES}",
            u.entries,
            u.writes
        );
        assert!(
            u.cpu <= CPU_CEILING,
            "{label} {name}: cpu {} keeps 10% headroom",
            u.cpu
        );
    }
}

#[test]
fn mainnet_blue_chip_5_collateral_5_debt_liquidate_and_cleanup_fit_network_limits() {
    measure("spoke 1 (5 coll incl. 8-dec, 5 debt)", &BLUE_CHIP);
}

#[test]
fn mainnet_stables_5_collateral_4_debt_liquidate_and_cleanup_fit_network_limits() {
    measure("spoke 4 (5 coll incl. 18-dec USST, 4 debt)", &STABLES);
}

#[test]
fn mainnet_amm_lp_5_collateral_4_debt_liquidate_and_cleanup_fit_network_limits() {
    measure("spoke 5 (5 LP coll, 4 debt)", &AMM_LP);
}
