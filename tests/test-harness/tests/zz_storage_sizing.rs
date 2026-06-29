// Throwaway measurement harness: prints exact XDR-serialized on-chain sizes for
// account position storage entries. Run: cargo test -p test-harness --test zz_storage_sizing -- --nocapture
use controller::types::{
    AccountMeta, AccountPositionRaw, ControllerKey, DebtPositionRaw, PositionMode,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::{
    ContractDataDurability, ContractDataEntry, ExtensionPoint, LedgerEntry, LedgerEntryData,
    LedgerEntryExt, LedgerKey, LedgerKeyContractData, Limits, ScAddress, ScVal, WriteXdr,
};
use soroban_sdk::{Address, Env, IntoVal, Map, TryFromVal, Val};

fn scval_of<T: IntoVal<Env, Val>>(env: &Env, v: T) -> ScVal {
    let val: Val = v.into_val(env);
    ScVal::try_from_val(env, &val).unwrap()
}

fn dummy_contract() -> ScAddress {
    // ScVal/LedgerEntry size is value-independent for the contract address;
    // a dummy contract id is fine for sizing.
    use soroban_sdk::xdr::{ContractId, Hash};
    ScAddress::Contract(ContractId(Hash([7u8; 32])))
}

fn sizes(label: &str, key_scval: ScVal, value_scval: ScVal) {
    let contract = dummy_contract();
    let durability = ContractDataDurability::Persistent;

    let value_bytes = value_scval.to_xdr(Limits::none()).unwrap().len();

    let lk = LedgerKey::ContractData(LedgerKeyContractData {
        contract: contract.clone(),
        key: key_scval.clone(),
        durability,
    });
    let key_bytes = lk.to_xdr(Limits::none()).unwrap().len();

    let entry = LedgerEntry {
        last_modified_ledger_seq: 0,
        data: LedgerEntryData::ContractData(ContractDataEntry {
            ext: ExtensionPoint::V0,
            contract,
            key: key_scval,
            durability,
            val: value_scval,
        }),
        ext: LedgerEntryExt::V0,
    };
    let entry_bytes = entry.to_xdr(Limits::none()).unwrap().len();

    println!(
        "{:<34} value={:>5}  key={:>4}  full_LedgerEntry={:>5}",
        label, value_bytes, key_bytes, entry_bytes
    );
}

fn pos() -> AccountPositionRaw {
    AccountPositionRaw {
        scaled_amount: i128::MAX / 2,
        liquidation_threshold: 9000,
        liquidation_bonus: 10500,
        loan_to_value: 8000,
    }
}

fn debt() -> DebtPositionRaw {
    DebtPositionRaw {
        scaled_amount: i128::MAX / 2,
    }
}

#[test]
fn print_storage_sizes() {
    let env = Env::default();
    let id: u64 = 1;

    println!("\n================ ACCOUNT STORAGE ENTRY SIZES (XDR bytes) ================");

    // --- AccountMeta ---
    let meta = AccountMeta {
        owner: Address::generate(&env),
        spoke_id: 0,
        mode: PositionMode::Normal,
    };
    let meta_emode = AccountMeta {
        owner: Address::generate(&env),
        spoke_id: 3,
        mode: PositionMode::Long,
    };
    sizes(
        "AccountMeta (normal)",
        scval_of(&env, ControllerKey::AccountMeta(id)),
        scval_of(&env, meta),
    );
    sizes(
        "AccountMeta (e-mode)",
        scval_of(&env, ControllerKey::AccountMeta(id)),
        scval_of(&env, meta_emode),
    );

    println!("---- SupplyPositions: Map<Address, AccountPositionRaw> ----");
    for n in 0..=4u32 {
        let mut m: Map<Address, AccountPositionRaw> = Map::new(&env);
        for _ in 0..n {
            m.set(Address::generate(&env), pos());
        }
        sizes(
            &format!("SupplyPositions ({} assets)", n),
            scval_of(&env, ControllerKey::SupplyPositions(id)),
            scval_of(&env, m),
        );
    }

    println!("---- BorrowPositions: Map<Address, DebtPositionRaw> ----");
    for n in 0..=4u32 {
        let mut m: Map<Address, DebtPositionRaw> = Map::new(&env);
        for _ in 0..n {
            m.set(Address::generate(&env), debt());
        }
        sizes(
            &format!("BorrowPositions ({} assets)", n),
            scval_of(&env, ControllerKey::BorrowPositions(id)),
            scval_of(&env, m),
        );
    }

    println!("========================================================================\n");
}
