use common::types::BatchSwap;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

// ---------------------------------------------------------------------------
// MockAggregator
//
// Minimal stand-in for `stellar-router-contract::Router::batch_execute`. The
// real router walks each path through SAC pools; this mock just pulls each
// path's allocated input from `batch.sender` and pushes
// `batch.total_min_out` of the last hop's `token_out` back to
// `batch.sender`. Tests pre-fund the mock with output tokens.
//
// Per-path allocation mirrors the router: `path_input = total_in *
// split_ppm / 1_000_000`, last path absorbs PPM rounding.
// ---------------------------------------------------------------------------

const PPM_DENOMINATOR: i128 = 1_000_000;

fn path_input(total_in: i128, split_ppm: u32, is_last: bool, consumed_so_far: i128) -> i128 {
    if is_last {
        total_in - consumed_so_far
    } else {
        total_in * (split_ppm as i128) / PPM_DENOMINATOR
    }
}

#[contract]
pub struct MockAggregator;

#[contractimpl]
impl MockAggregator {
    pub fn __constructor(_env: Env, _admin: Address) {}

    /// Returns the total `token_out` delivered to `batch.sender` —
    /// matches the real router's signature. Pulls `total_in` once,
    /// pushes `total_min_out` back. Per-path PPM allocation is a
    /// router-internal concern; this mock doesn't simulate it because
    /// the controller-side checks don't depend on per-path movements.
    pub fn batch_execute(env: Env, batch: BatchSwap) -> i128 {
        batch.sender.require_auth();
        let router = env.current_contract_address();

        let first_hop = batch.paths.get(0).unwrap().hops.get(0).unwrap();
        let last_hop_path = batch.paths.get(batch.paths.len() - 1).unwrap();
        let last_hop = last_hop_path
            .hops
            .get(last_hop_path.hops.len() - 1)
            .unwrap();

        // Pull the entire input once.
        let in_client = soroban_sdk::token::Client::new(&env, &first_hop.token_in);
        in_client.transfer(&batch.sender, &router, &batch.total_in);

        // Push `total_min_out` back as a "swap succeeded" stand-in.
        if batch.total_min_out > 0 {
            let out_client = soroban_sdk::token::Client::new(&env, &last_hop.token_out);
            out_client.transfer(&router, &batch.sender, &batch.total_min_out);
        }

        // Keep `path_input` compiled for tests that model per-path movements.
        let _ = path_input(batch.total_in, 1_000_000, true, 0);

        batch.total_min_out
    }
}

// ---------------------------------------------------------------------------
// Adversarial aggregator for controller-side router validation.
//
// Three misbehaviors are supported via a simple mode enum stored in instance
// storage so tests can flip the behavior between runs:
//
//   1. Refund: send extra `token_in` BACK to `sender` after the swap —
//      the controller must detect `balance_in_after > balance_in_before`
//      and panic with InternalError.
//   2. OverPull: pull MORE than `total_in` from `sender` — the
//      controller's `actual_in_spent > amount_in` guard must fire.
//   3. OutputShortfall: pull input but skip the output transfer — the
//      controller's `received < total_min_out` guard must fire.
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Copy)]
pub enum BadMode {
    Refund,
    OverPull,
    OutputShortfall,
}

#[contract]
pub struct BadAggregator;

#[contractimpl]
impl BadAggregator {
    pub fn __constructor(env: Env, _admin: Address, mode: BadMode) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "MODE"), &mode);
    }

    pub fn batch_execute(env: Env, batch: BatchSwap) -> i128 {
        batch.sender.require_auth();
        let router = env.current_contract_address();
        let mode: BadMode = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "MODE"))
            .expect("mode must be set by constructor");

        // Single-path simplification: the adversarial cases all model the
        // simplest router response, so test fixtures pass a 1-path batch
        // with `split_ppm = 1_000_000`.
        let path = batch.paths.get(0).unwrap();
        let first_hop = path.hops.get(0).unwrap();
        let last_hop = path.hops.get(path.hops.len() - 1).unwrap();
        let in_client = soroban_sdk::token::Client::new(&env, &first_hop.token_in);
        let out_client = soroban_sdk::token::Client::new(&env, &last_hop.token_out);

        match mode {
            BadMode::Refund => {
                // Send token_out as if the swap succeeded.
                if batch.total_min_out > 0 {
                    out_client.transfer(&router, &batch.sender, &batch.total_min_out);
                }
                // Then send extra token_in BACK to sender — the controller
                // must trip its `balance_in_after > balance_in_before`
                // guard. The mock must be pre-funded with token_in.
                in_client.transfer(&router, &batch.sender, &batch.total_in);
            }
            BadMode::OverPull => {
                // Pull `total_in * 2` — controller's
                // `actual_in_spent > amount_in` guard must fire when the
                // controller happens to hold enough; otherwise the SAC
                // rejects the over-pull with insufficient-balance.
                let overshoot = batch.total_in.saturating_mul(2);
                in_client.transfer(&batch.sender, &router, &overshoot);
                if batch.total_min_out > 0 {
                    out_client.transfer(&router, &batch.sender, &batch.total_min_out);
                }
            }
            BadMode::OutputShortfall => {
                // Pull input but deliberately skip the output transfer —
                // the controller's slippage check on `total_min_out` fires.
                in_client.transfer(&batch.sender, &router, &batch.total_in);
            }
        }

        batch.total_min_out
    }
}
