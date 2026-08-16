#![no_std]

use soroban_sdk::{contractclient, Address, BytesN, Env};

/// The subset of the position-nft ABI the controller consumes. Token ids are
/// `u32` (OZ sequential ids); the controller widens/narrows exclusively in
/// `external/position_nft.rs`.
#[contractclient(name = "PositionNftClient")]
pub trait PositionNftInterface {
    fn mint(env: Env, to: Address) -> u32;
    fn burn(env: Env, token_id: u32);
    fn owner_of(env: Env, token_id: u32) -> Address;
    fn renew(env: Env, token_id: u32);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
}
