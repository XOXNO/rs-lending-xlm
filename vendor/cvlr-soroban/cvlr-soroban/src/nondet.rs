use cvlr_nondet::nondet;
use soroban_sdk::{
    Address, Bytes, BytesN, Duration, Env, IntoVal, Map, String, Symbol, Timepoint, TryFromVal,
    Val, Vec, I256, U256,
};

pub fn nondet_address() -> Address {
    let v: u64 = nondet();
    let val = Val::from_payload((v << 8) | 77);
    Address::try_from_val(&Env::default(), &val).unwrap()
}

pub fn nondet_map<K, V>() -> Map<K, V>
where
    K: IntoVal<Env, Val> + TryFromVal<Env, Val>,
    V: IntoVal<Env, Val> + TryFromVal<Env, Val>,
{
    let v: u64 = nondet();
    let val = Val::from_payload((v << 8) | 76);
    Map::try_from_val(&Env::default(), &val).unwrap()
}

pub fn nondet_string() -> String {
    let nd: u8 = nondet();
    String::from_bytes(&Env::default(), &[nd])
}

// Only use when need a Tag correct Val, recommend creating proper nondet
// Vec for any given type.
pub fn nondet_vec<V>() -> Vec<V>
where
    V: IntoVal<Env, Val> + TryFromVal<Env, Val>,
{
    let v: u64 = nondet();
    let val = Val::from_payload((v << 8) | 75);
    Vec::try_from_val(&Env::default(), &val).unwrap()
}

pub fn nondet_symbol() -> Symbol {
    let v: u64 = nondet();
    let val = Val::from_payload((v << 8) | 74);
    Symbol::try_from_val(&Env::default(), &val).unwrap()
}

pub fn nondet_bytes1() -> Bytes {
    let v: u8 = nondet();
    Bytes::from_slice(&Env::default(), &[v])
}

extern "C" {
    #[allow(improper_ctypes)]
    fn CVT_nondet_bytes_n_32() -> BytesN<32>;
}

pub fn nondet_bytes_n() -> BytesN<32> {
    unsafe { CVT_nondet_bytes_n_32() }
}

pub fn nondet_duration() -> Duration {
    Duration::from_seconds(&Env::default(), nondet())
}

pub fn nondet_timepoint() -> Timepoint {
    Timepoint::from_unix(&Env::default(), nondet())
}

pub fn nondet_u256() -> U256 {
    U256::from_parts(&Env::default(), nondet(), nondet(), nondet(), nondet())
}

pub fn nondet_i256() -> I256 {
    I256::from_parts(&Env::default(), nondet(), nondet(), nondet(), nondet())
}
