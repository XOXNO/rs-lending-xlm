use soroban_sdk::{contracttype, Address, String, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleAssetRef {
    /// SEP-40 lookup by Stellar asset address.
    Stellar(Address),
    /// SEP-40 lookup by symbol.
    Symbol(Symbol),
    /// Provider-specific string identifier such as a RedStone feed id.
    String(String),
}
