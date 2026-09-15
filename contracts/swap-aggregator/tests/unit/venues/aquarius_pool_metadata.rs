use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, symbol_short, vec, Address, Env, Map, Vec};

use super::pool_tokens;
use crate::errors::Error;
use crate::program::MAX_ASSETS;

#[contract]
struct MetadataPool;

#[contractimpl]
impl MetadataPool {
    pub fn __constructor(env: Env, tokens: Vec<Address>) {
        env.storage()
            .instance()
            .set(&symbol_short!("tokens"), &tokens);
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&symbol_short!("tokens"))
            .unwrap()
    }
}

#[contract]
struct MetadataReader;

#[contractimpl]
impl MetadataReader {
    pub fn read(env: Env, pool: Address) -> Vec<Address> {
        let mut cache = Map::new(&env);
        let tokens = pool_tokens(&env, &mut cache, &pool);
        assert_eq!(pool_tokens(&env, &mut cache, &pool), tokens);
        tokens
    }
}

#[test]
fn constituent_metadata_is_bounded_and_unique_before_caching() {
    let env = Env::default();
    let reader_address = env.register(MetadataReader, ());
    let reader = MetadataReaderClient::new(&env, &reader_address);

    for (count, accepted) in [
        (0, false),
        (1, true),
        (2, true),
        (MAX_ASSETS, true),
        (MAX_ASSETS + 1, false),
    ] {
        let mut tokens = Vec::new(&env);
        for _ in 0..count {
            tokens.push_back(Address::generate(&env));
        }
        let pool = env.register(MetadataPool, (tokens.clone(),));
        let result = reader.try_read(&pool);
        if accepted {
            assert_eq!(result.unwrap().unwrap(), tokens);
        } else {
            assert_eq!(result.unwrap_err().unwrap(), Error::BrokenTokenChain.into());
        }
    }

    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let duplicate_lists = [
        vec![&env, a.clone(), a.clone()],
        vec![&env, a.clone(), b.clone(), a.clone()],
        vec![&env, a.clone(), a.clone(), b.clone()],
        vec![&env, a.clone(), b.clone(), b.clone()],
        vec![&env, a.clone(), b.clone(), a.clone(), c.clone()],
    ];
    for tokens in duplicate_lists {
        let pool = env.register(MetadataPool, (tokens,));
        assert_eq!(
            reader.try_read(&pool).unwrap_err().unwrap(),
            Error::BrokenTokenChain.into()
        );
    }
}
