extern crate std;

use super::withhold_liquidation_fee;
use crate::cache::Cache;
use crate::storage;
use crate::test_support::{hub, init_ledger};
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::errors::CollateralError;
use common::math::fp::Ray;
use common::types::{
    MarketParamsRaw, PoolAction, PoolBorrowEntry, PoolStateRaw, PoolSupplyEntry, PoolWithdrawEntry,
    ScaledPositionRaw,
};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, vec, xdr, Address, Env, Error, TryIntoVal};

struct TestSetup {
    env: Env,
    contract: Address,
    params: MarketParamsRaw,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        init_ledger(&env);

        let admin = Address::generate(&env);
        let asset = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let params = MarketParamsRaw {
            max_borrow_rate: 2 * RAY,
            base_borrow_rate: RAY / 100,
            slope1: RAY / 10,
            slope2: RAY / 5,
            slope3: RAY / 2,
            mid_utilization: RAY / 2,
            optimal_utilization: RAY * 8 / 10,
            max_utilization: RAY * 95 / 100,
            reserve_factor: 1_000,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: asset.clone(),
            asset_decimals: 7,
        };
        let contract = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &contract).create_market(&0u32, &params);

        Self {
            env,
            contract,
            params,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }

    fn cache(&self, supplied: i128, cash: i128) -> Cache {
        Cache::from_parts(
            &self.env,
            hub(&self.params.asset_id),
            &self.params,
            &PoolStateRaw {
                supplied,
                borrowed: 0,
                revenue: 0,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 0,
                cash,
            },
            1_000_000,
        )
    }
}

#[test]
fn test_withhold_liquidation_fee_noop_when_not_liquidation_or_zero_fee() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let out = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, false, 1_000_000);
        assert_eq!(out, 10_000_000);
        let out2 = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, true, 0);
        assert_eq!(out2, 10_000_000);
    });
}

#[test]
fn test_withhold_liquidation_fee_accrues_to_revenue_and_reduces_net() {
    let env = Env::default();
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let fee_raw = 2_000_000i128;

        let expected_revenue = Ray::from_asset(&env, fee_raw, t.params.asset_decimals);
        let net = withhold_liquidation_fee(&t.env, &mut cache, 10_000_000, true, fee_raw);
        assert_eq!(net, 8_000_000);
        assert_eq!(cache.revenue(), expected_revenue);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #115)")]
fn test_withhold_liquidation_fee_rejects_fee_greater_than_gross() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.cache(100 * RAY, 50_000_000);
        let _ = withhold_liquidation_fee(&t.env, &mut cache, 1_000_000, true, 2_000_000);
    });
}

#[test]
fn test_liquidation_withdraw_uses_post_burn_fee_headroom_and_final_debt_guard() {
    const UNIT: i128 = 10_000_000;
    let largest_deposit = i128::MAX / (RAY / UNIT);
    for (deposit, borrowed, full_close) in [
        (largest_deposit, 0, false),
        (largest_deposit, 0, true),
        (100 * UNIT, 10 * UNIT, true),
    ] {
        let t = TestSetup::new();
        let client = LiquidityPoolClient::new(&t.env, &t.contract);
        let payer = Address::generate(&t.env);
        let receiver = Address::generate(&t.env);
        let asset = &t.params.asset_id;
        let key = hub(asset);
        let tok = token::Client::new(&t.env, asset);
        token::StellarAssetClient::new(&t.env, asset).mint(&payer, &deposit);
        tok.transfer(&payer, &t.contract, &deposit);
        let supplied = client
            .supply(&vec![
                &t.env,
                PoolSupplyEntry {
                    action: PoolAction {
                        hub_asset: key.clone(),
                        position: ScaledPositionRaw { scaled_amount: 0 },
                        amount: deposit,
                    },
                },
            ])
            .get(0)
            .unwrap();
        if borrowed > 0 {
            client.borrow(
                &payer,
                &vec![
                    &t.env,
                    PoolBorrowEntry {
                        action: PoolAction {
                            hub_asset: key.clone(),
                            position: ScaledPositionRaw { scaled_amount: 0 },
                            amount: borrowed,
                        },
                    },
                ],
            );
        }

        let gross = if full_close { deposit } else { 100 * UNIT };
        let fee = 10 * UNIT;
        let result = client
            .withdraw(
                &receiver,
                &true,
                &vec![
                    &t.env,
                    PoolWithdrawEntry {
                        action: PoolAction {
                            hub_asset: key.clone(),
                            position: supplied.position,
                            amount: if full_close { i128::MAX } else { gross },
                        },
                        protocol_fee: fee,
                    },
                ],
            )
            .get(0)
            .unwrap();

        let state = client.get_sync_data(&key).state;
        let remaining_shares = (deposit - gross) * (RAY / UNIT);
        assert_eq!(result.actual_amount, gross);
        assert_eq!(result.position.scaled_amount, remaining_shares);
        assert_eq!(state.supplied, remaining_shares + 10 * RAY);
        assert_eq!(state.revenue, 10 * RAY);
        assert_eq!(client.get_revenue(&key), fee);
        assert_eq!(state.borrowed, borrowed * (RAY / UNIT));
        assert_eq!(state.cash, deposit - borrowed - (gross - fee));
        assert_eq!(tok.balance(&t.contract), state.cash);
        assert_eq!(tok.balance(&receiver), gross - fee);
    }
}

/// Simulate a dust close, then execute at the next ledger with precisely the
/// recorded footprint. The recipient is a classic account, not a mock token.
#[test]
fn dust_close_keeps_native_recipient_writable_when_interest_crosses_one_unit() {
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::{xdr, TryIntoVal};
    use std::rc::Rc;

    let setup = TestSetup::new();
    let native: Address = setup
        .env
        .host()
        .invoke_function(xdr::HostFunction::CreateContract(xdr::CreateContractArgs {
            contract_id_preimage: xdr::ContractIdPreimage::Asset(xdr::Asset::Native),
            executable: xdr::ContractExecutable::StellarAsset,
        }))
        .unwrap()
        .try_into_val(&setup.env)
        .unwrap();
    let account_id = xdr::AccountId(xdr::PublicKey::PublicKeyTypeEd25519(xdr::Uint256([7; 32])));
    let recipient: Address = xdr::ScAddress::Account(account_id.clone())
        .try_into_val(&setup.env)
        .unwrap();
    let key = Rc::new(xdr::LedgerKey::Account(xdr::LedgerKeyAccount {
        account_id: account_id.clone(),
    }));
    setup
        .env
        .host()
        .add_ledger_entry(
            &key,
            &Rc::new(xdr::LedgerEntry {
                data: xdr::LedgerEntryData::Account(xdr::AccountEntry {
                    account_id,
                    balance: 20_000_000_000,
                    seq_num: xdr::SequenceNumber(0),
                    num_sub_entries: 0,
                    inflation_dest: None,
                    flags: 0,
                    home_domain: Default::default(),
                    thresholds: xdr::Thresholds([1; 4]),
                    signers: Default::default(),
                    ext: xdr::AccountEntryExt::V0,
                }),
                last_modified_ledger_seq: 0,
                ext: xdr::LedgerEntryExt::V0,
            }),
            None,
        )
        .unwrap();
    token::Client::new(&setup.env, &native).transfer(&recipient, &setup.contract, &10_000_000_000);
    let mut params = setup.params.clone();
    params.asset_id = native.clone();
    LiquidityPoolClient::new(&setup.env, &setup.contract).create_market(&0, &params);
    let state = PoolStateRaw {
        supplied: 2_000 * RAY,
        borrowed: 1_000 * RAY,
        revenue: 0,
        supply_index: RAY,
        borrow_index: RAY,
        cash: 10_000_000_000,
        last_timestamp: 1_000_000,
    };
    setup.env.as_contract(&setup.contract, || {
        crate::storage::write_state(&setup.env, &hub(&native), &state)
    });

    // Start recording only after funding and market setup, so those operations
    // cannot accidentally add the recipient's account to the tested footprint.
    let snapshot = setup.env.to_ledger_snapshot();
    for (omit_zero_transfer, empty_at_simulation) in [(true, false), (false, false), (false, true)]
    {
        let env = Env::from_ledger_snapshot(snapshot.clone());
        env.mock_all_auths();
        let pool: Address = xdr::ScAddress::from(&setup.contract)
            .try_into_val(&env)
            .unwrap();
        let native: Address = xdr::ScAddress::from(&native).try_into_val(&env).unwrap();
        let recipient: Address = xdr::ScAddress::from(&recipient).try_into_val(&env).unwrap();
        let mut entry = PoolWithdrawEntry {
            action: PoolAction {
                hub_asset: hub(&native),
                amount: i128::MAX,
                position: ScaledPositionRaw {
                    scaled_amount: if empty_at_simulation {
                        0
                    } else {
                        100_000_000_000_000_000_000 - 1
                    },
                },
            },
            protocol_fee: 0,
        };
        // One test frame prevents the SDK's per-invocation meter from resetting
        // the recorded footprint before the enforcing replay.
        let replay = env.try_as_contract::<_, soroban_sdk::Error>(&pool, || {
            // Earlier RDWC routing already touches the native SAC and pool balance.
            token::Client::new(&env, &native).transfer(&pool, &pool, &0);
            let simulated = if omit_zero_transfer {
                // Negative control: the exact pre-fix payout path.
                let outcome = super::accounting(&env, false, &entry);
                outcome.cache.transfer_out(&recipient, outcome.net_transfer);
                outcome.mutation
            } else {
                super::apply(&env, &recipient, false, &entry).0
            };
            assert_eq!(simulated.actual_amount, 0);
            assert_eq!(simulated.position.scaled_amount, 0);

            // Roll back the simulated market write; transfer(0) changed no balance.
            // Do not read the recipient before enforcing the simulation footprint.
            crate::storage::write_state(&env, &hub(&native), &state);
            env.ledger().with_mut(|ledger| {
                ledger.timestamp += 5;
                ledger.sequence_number += 1;
            });
            env.host().switch_to_enforcing_storage().unwrap();
            entry.action.position.scaled_amount = 100_000_000_000_000_000_000 - 1;
            let committed = super::apply(&env, &recipient, false, &entry).0;
            assert_eq!(committed.actual_amount, 1);
            assert_eq!(committed.position.scaled_amount, 0);
            assert_eq!(
                token::Client::new(&env, &native).balance(&recipient),
                10_000_000_001
            );
        });
        if omit_zero_transfer {
            assert_eq!(
                replay,
                Err(Ok(soroban_sdk::Error::from_type_and_code(
                    xdr::ScErrorType::Storage,
                    xdr::ScErrorCode::ExceededLimit,
                )))
            );
        } else {
            assert_eq!(replay, Ok(()));
        }
    }
}

#[test]
fn empty_withdrawal_and_zero_refund_do_not_touch_recipient() {
    use soroban_sdk::{xdr, TryIntoVal};
    let t = TestSetup::new();
    // No account or credit-asset trustline: any SAC transfer would fail.
    let recipient: Address = xdr::ScAddress::Account(xdr::AccountId(
        xdr::PublicKey::PublicKeyTypeEd25519(xdr::Uint256([9; 32])),
    ))
    .try_into_val(&t.env)
    .unwrap();
    t.as_contract(|| t.cache(0, 0).transfer_out(&recipient, 0));
    let result = LiquidityPoolClient::new(&t.env, &t.contract).withdraw(
        &recipient,
        &false,
        &vec![
            &t.env,
            PoolWithdrawEntry {
                action: PoolAction {
                    hub_asset: hub(&t.params.asset_id),
                    amount: 0,
                    position: ScaledPositionRaw { scaled_amount: 0 },
                },
                protocol_fee: 0,
            },
        ],
    );
    assert_eq!(result.get(0).unwrap().actual_amount, 0);
}

#[test]
fn zero_value_full_close_succeeds_without_recipient_trustline() {
    let dust = RAY / 10_000_000 - 1;
    for scaled in [0, dust] {
        let t = TestSetup::new();
        let recipient: Address = xdr::ScAddress::Account(xdr::AccountId(
            xdr::PublicKey::PublicKeyTypeEd25519(xdr::Uint256([9; 32])),
        ))
        .try_into_val(&t.env)
        .unwrap();
        t.as_contract(|| {
            storage::write_state(
                &t.env,
                &hub(&t.params.asset_id),
                &PoolStateRaw {
                    supplied: scaled,
                    borrowed: 0,
                    revenue: 0,
                    borrow_index: RAY,
                    supply_index: RAY,
                    last_timestamp: 1_000_000,
                    cash: 0,
                },
            )
        });
        let result = LiquidityPoolClient::new(&t.env, &t.contract).withdraw(
            &recipient,
            &false,
            &vec![
                &t.env,
                PoolWithdrawEntry {
                    action: PoolAction {
                        hub_asset: hub(&t.params.asset_id),
                        amount: i128::MAX,
                        position: ScaledPositionRaw {
                            scaled_amount: scaled,
                        },
                    },
                    protocol_fee: 0,
                },
            ],
        );
        let mutation = result.get(0).unwrap();
        assert_eq!(mutation.actual_amount, 0);
        assert_eq!(mutation.position.scaled_amount, 0);
        let state = t.as_contract(|| storage::read_state(&t.env, &hub(&t.params.asset_id)));
        assert_eq!(state.supplied, 0);
        assert_eq!(state.cash, 0);
    }
}

fn write_market_state(t: &TestSetup, supplied: i128, borrowed: i128, cash: i128) {
    t.as_contract(|| {
        storage::write_state(
            &t.env,
            &hub(&t.params.asset_id),
            &PoolStateRaw {
                supplied,
                borrowed,
                revenue: 0,
                borrow_index: RAY,
                supply_index: RAY,
                last_timestamp: 1_000,
                cash,
            },
        )
    });
}

fn withdraw_entry(t: &TestSetup, scaled_amount: i128, amount: i128) -> PoolWithdrawEntry {
    PoolWithdrawEntry {
        action: PoolAction {
            hub_asset: hub(&t.params.asset_id),
            amount,
            position: ScaledPositionRaw { scaled_amount },
        },
        protocol_fee: 0,
    }
}

fn transfers_to(env: &Env, recipient: &Address) -> usize {
    let to = xdr::ScVal::Address(xdr::ScAddress::from(recipient));
    env.events()
        .all()
        .events()
        .iter()
        .filter(|event| {
            let xdr::ContractEventBody::V0(body) = &event.body;
            let is_transfer = matches!(
                body.topics.first(),
                Some(xdr::ScVal::Symbol(name)) if name.0.to_utf8_string().as_deref() == Ok("transfer")
            );
            is_transfer && body.topics.get(2) == Some(&to)
        })
        .count()
}

#[test]
fn zero_value_close_touches_recipient_only_for_a_live_leg_or_an_explicit_full_close() {
    let dust = RAY / 10_000_000 - 1;
    for (scaled, amount, touches) in [
        (dust, 1, 1),
        (dust, i128::MAX, 1),
        (0, i128::MAX, 1),
        (0, 0, 0),
        (0, 5, 0),
    ] {
        let t = TestSetup::new();
        write_market_state(&t, scaled, 0, 0);
        let recipient = Address::generate(&t.env);
        let mutation = LiquidityPoolClient::new(&t.env, &t.contract)
            .withdraw(
                &recipient,
                &false,
                &vec![&t.env, withdraw_entry(&t, scaled, amount)],
            )
            .get(0)
            .unwrap();
        assert_eq!(mutation.actual_amount, 0);
        assert_eq!(mutation.position.scaled_amount, 0);
        assert_eq!(
            transfers_to(&t.env, &recipient),
            touches,
            "scaled {scaled}, amount {amount}"
        );
    }
}

#[test]
fn full_close_keeps_the_utilization_gate_and_an_empty_close_skips_it() {
    for (scaled, expected) in [
        (
            10 * RAY,
            Some(Error::from_contract_error(
                CollateralError::UtilizationAboveMax as u32,
            )),
        ),
        (0, None),
    ] {
        let t = TestSetup::new();
        write_market_state(&t, 1_000 * RAY, 960 * RAY, 400_000_000);
        let result = LiquidityPoolClient::new(&t.env, &t.contract).try_withdraw(
            &Address::generate(&t.env),
            &false,
            &vec![&t.env, withdraw_entry(&t, scaled, i128::MAX)],
        );
        assert_eq!(
            result.err().map(|err| err.expect("contract error")),
            expected,
            "scaled {scaled}"
        );
    }
}
