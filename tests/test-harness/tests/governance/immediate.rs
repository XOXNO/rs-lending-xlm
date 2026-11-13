use governance::op::{AdminOperation, RelaxSpokeAssetFlagsArgs, RoleArgs, SpokeAssetArgs};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{map, vec, Address, BytesN, IntoVal, Map, Symbol, TryFromVal, Val, Vec};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd, usdc_preset, LendingTest, ALICE,
    DEFAULT_MARKET_PARAMS, HARNESS_HUB, HARNESS_SPOKE,
};

fn flatten<T, C>(
    result: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match result {
        Ok(_) => Ok(()),
        Err(Ok(err)) => Err(err),
        Err(Err(_)) => panic!("expected contract error, got InvokeError"),
    }
}

#[test]
fn guardian_sets_spoke_asset_flags_immediately() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    let before = t
        .ctrl_client()
        .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(usdc.clone()));

    t.gov_iface_client().set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &true,
        &false,
        &false,
    );

    let after = t
        .ctrl_client()
        .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(usdc.clone()));
    assert!(after.paused, "paused flag must flip");
    assert!(!after.frozen);
    assert_eq!(after.loan_to_value, before.loan_to_value);
    assert_eq!(after.supply_cap, before.supply_cap);
    assert_eq!(after.liquidation_threshold, before.liquidation_threshold);

    assert_contract_error(
        t.try_supply(ALICE, "USDC", 10.0),
        errors::SPOKE_ASSET_PAUSED,
    );

    t.gov_iface_client().set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &true,
        &true,
        &false,
    );

    let relax = t.gov_iface_client().try_set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &false,
        &false,
        &false,
    );
    assert_contract_error(flatten(relax), errors::SPOKE_ASSET_FLAG_RELAXATION);

    let relax_frozen = t.gov_iface_client().try_set_spoke_asset_flags(
        &admin,
        &HARNESS_SPOKE,
        &hub_asset(usdc.clone()),
        &true,
        &false,
        &false,
    );
    assert_contract_error(flatten(relax_frozen), errors::SPOKE_ASSET_FLAG_RELAXATION);
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 10.0),
        errors::SPOKE_ASSET_PAUSED,
    );

    let expected_epoch = t
        .ctrl_client()
        .get_spoke_asset_flags_epoch(&HARNESS_SPOKE, &hub_asset(usdc.clone()));
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::RelaxSpokeAssetFlags(RelaxSpokeAssetFlagsArgs {
            spoke_id: HARNESS_SPOKE,
            hub_asset: hub_asset(usdc.clone()),
            expected_epoch,
            paused: false,
            frozen: false,
            no_seize: false,
        }),
    );
    assert!(
        t.try_supply(ALICE, "USDC", 10.0).is_ok(),
        "timelocked relaxation must re-open supply"
    );
}

#[test]
fn non_guardian_flags_rejected() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let stranger = Address::generate(&t.env);
    let usdc = t.resolve_asset("USDC");

    let result = gov.try_set_spoke_asset_flags(
        &stranger,
        &HARNESS_SPOKE,
        &hub_asset(usdc),
        &true,
        &false,
        &false,
    );
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);
}

#[test]
fn oracle_role_tightens_sanity_band_containing_price() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    let before = t.market_oracle_config(&usdc);

    // The market starts on `tight_single_source_band` = [0.99, 1.01]. The
    // immediate `ORACLE_ROLE` path may only tighten, so the new band sits
    // inside the old one. Widening needs the timelock.
    let min = usd(1) * 995 / 1000;
    let max = usd(1) * 1005 / 1000;
    gov.set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &min,
        &max,
    );

    let after = t.market_oracle_config(&usdc);
    assert_eq!(after.min_sanity_price_wad, min);
    assert_eq!(after.max_sanity_price_wad, max);
    assert_eq!(
        after.max_price_stale_seconds,
        before.max_price_stale_seconds
    );
    assert_eq!(after.tolerance, before.tolerance);
}

#[test]
fn sanity_band_not_containing_price_fails_closed_at_read() {
    // Both bands exclude the live price of usd(1). Each tightens the seeded
    // [0.95, 1.05] band, so the write succeeds and the read fails.
    for (min_wad, max_wad) in [
        (usd(1) * 1005 / 1000, usd(1) * 105 / 100),
        (usd(1) * 95 / 100, usd(1) * 995 / 1000),
    ] {
        let t = LendingTest::new().with_market(usdc_preset()).build();
        let gov = t.gov_iface_client();
        let admin = t.admin();
        let usdc = t.resolve_asset("USDC");

        // The market defaults to a +/-1% band. Widen it outside governance
        // first, because the governed call may only narrow it.
        t.seed_sanity_band("USDC", usd(1) * 95 / 100, usd(1) * 105 / 100);

        flatten(gov.try_set_sanity_band(
            &admin,
            &controller::types::PriceKey::Token(usdc.clone()),
            &min_wad,
            &max_wad,
        ))
        .expect("an out-of-band live price must not block the band write");

        let read = t
            .price_agg_client()
            .try_prices(&soroban_sdk::vec![
                &t.env,
                controller::types::PriceKey::Token(usdc.clone())
            ])
            .map(|inner| inner.map(|_| ()).map_err(|e| e.into()))
            .unwrap_or_else(|e| Err(e.expect("expected contract error")));
        assert_contract_error(read, errors::SANITY_BOUND_VIOLATED);
    }
}

#[test]
fn sanity_band_may_only_tighten_on_the_immediate_path() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");

    // Starting band is [0.99, 1.01] (100 bps). Tighten to 75 bps, leaving room
    // for a second tighten to `MIN_SANITY_BAND_BPS` (50).
    let narrow_min = usd(1) * 9925 / 10000;
    let narrow_max = usd(1) * 10075 / 10000;
    gov.set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &narrow_min,
        &narrow_max,
    );

    let result = gov.try_set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &(usd(1) * 110 / 100),
        &(usd(1) * 115 / 100),
    );
    assert_contract_error(flatten(result), errors::SANITY_BAND_MUST_TIGHTEN);

    // Widening is refused even when the new band contains the live price and
    // overlaps the old one. Only the timelock may widen.
    let wide_min = usd(1) * 94 / 100;
    let wide_max = usd(1) * 106 / 100;
    let widened = gov.try_set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &wide_min,
        &wide_max,
    );
    assert_contract_error(flatten(widened), errors::SANITY_BAND_MUST_TIGHTEN);

    // The band the failed calls tried to replace is untouched, and a further
    // tightening still goes through.
    let held = t.market_oracle_config(&usdc);
    assert_eq!(held.min_sanity_price_wad, narrow_min);
    assert_eq!(held.max_sanity_price_wad, narrow_max);

    let tighter_min = usd(1) * 995 / 1000;
    let tighter_max = usd(1) * 1005 / 1000;
    gov.set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &tighter_min,
        &tighter_max,
    );
    let after = t.market_oracle_config(&usdc);
    assert_eq!(after.min_sanity_price_wad, tighter_min);
    assert_eq!(after.max_sanity_price_wad, tighter_max);
}

#[test]
fn sanity_band_input_and_role_gates() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let usdc = t.resolve_asset("USDC");

    let result = gov.try_set_sanity_band(
        &admin,
        &controller::types::PriceKey::Token(usdc.clone()),
        &usd(2),
        &usd(1),
    );
    assert_contract_error(flatten(result), errors::INVALID_SANITY_BOUNDS);

    let result = gov.try_set_sanity_band(
        &stranger,
        &controller::types::PriceKey::Token(usdc.clone()),
        &(usd(1) * 95 / 100),
        &(usd(1) * 105 / 100),
    );
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);
}

#[test]
fn curator_creates_hub_and_spoke_immediately() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let curator_role = Symbol::new(&t.env, "CURATOR");

    assert_contract_error(flatten(gov.try_create_hub(&admin)), errors::UNAUTHORIZED);
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: admin.clone(),
            role: curator_role,
        }),
    );

    let hub_id = gov.create_hub(&admin);
    assert!(hub_id >= 1);
    let spoke_id = gov.add_spoke(&admin);
    assert!(spoke_id >= 1);

    assert!(!t.ctrl_client().get_spoke(&spoke_id).is_deprecated);

    assert_contract_error(flatten(gov.try_create_hub(&stranger)), errors::UNAUTHORIZED);
    assert_contract_error(flatten(gov.try_add_spoke(&stranger)), errors::UNAUTHORIZED);
}

#[test]
fn owner_revokes_role_immediately() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let usdc = t.resolve_asset("USDC");
    let guardian_role = Symbol::new(&t.env, "GUARDIAN");
    let canceller_role = Symbol::new(&t.env, "CANCELLER");
    let holder = Address::generate(&t.env);

    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: holder.clone(),
            role: guardian_role.clone(),
        }),
    );
    assert!(gov.has_role(&holder, &guardian_role));
    gov.revoke_role_immediate(&holder, &guardian_role);
    assert!(!gov.has_role(&holder, &guardian_role));

    let result = gov.try_set_spoke_asset_flags(
        &holder,
        &HARNESS_SPOKE,
        &hub_asset(usdc),
        &true,
        &false,
        &false,
    );
    assert_contract_error(flatten(result), errors::UNAUTHORIZED);

    let result = gov.try_revoke_role_immediate(&admin, &guardian_role);
    assert_contract_error(flatten(result), errors::NOT_AUTHORIZED);

    let result = gov.try_revoke_role_immediate(&holder, &guardian_role);
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    let result = gov.try_revoke_role_immediate(&holder, &Symbol::new(&t.env, "NOPE"));
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: holder.clone(),
            role: canceller_role.clone(),
        }),
    );
    let result = gov.try_revoke_role_immediate(&holder, &canceller_role);
    assert_contract_error(flatten(result), errors::INVALID_ROLE);

    // `EXECUTOR` is not a governance role (audit finding I-02); like any name
    // outside GUARDIAN and ORACLE it is refused.
    for role in ["PROPOSER", "EXECUTOR"] {
        let result = gov.try_revoke_role_immediate(&admin, &Symbol::new(&t.env, role));
        assert_contract_error(flatten(result), errors::INVALID_ROLE);
    }
}

fn register_curator(t: &LendingTest) -> Address {
    let path = "target/wasm32v1-none/release/curator.wasm";
    let bytes = std::fs::read(path)
        .or_else(|_| std::fs::read(format!("../{path}")))
        .or_else(|_| std::fs::read(format!("../../{path}")))
        .expect("curator WASM not found. Run 'make build' first.");
    let curator = t.env.register(bytes.as_slice(), (t.governance.clone(),));
    for role in ["CURATOR", "PROPOSER"] {
        t.gov_client().execute_immediate(
            &t.admin,
            &AdminOperation::GrantGovRole(RoleArgs {
                account: curator.clone(),
                role: Symbol::new(&t.env, role),
            }),
        );
    }
    curator
}

fn open_registrar(t: &LendingTest, curator: &Address, admin: &Address) -> u32 {
    let id = t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::RegisterCuratorRegistrar(curator.clone(), admin.clone()),
    );
    u32::try_from_val(&t.env, &id).unwrap()
}

fn curator_call<T: TryFromVal<soroban_sdk::Env, Val>>(
    t: &LendingTest,
    curator: &Address,
    func: &str,
    args: Vec<Val>,
) -> T {
    t.env
        .invoke_contract(curator, &Symbol::new(&t.env, func), args)
}

fn try_curator_call(
    t: &LendingTest,
    curator: &Address,
    func: &str,
    args: Vec<Val>,
) -> Result<(), soroban_sdk::Error> {
    match t.env.try_invoke_contract::<(), soroban_sdk::Error>(
        curator,
        &Symbol::new(&t.env, func),
        args,
    ) {
        Ok(_) => Ok(()),
        Err(Ok(err)) => Err(err),
        Err(Err(_)) => panic!("expected contract error, got InvokeError"),
    }
}

#[test]
fn curator_registrar_tightens_flags_on_its_spoke() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let env = &t.env;
    let curator = register_curator(&t);
    let radmin = Address::generate(env);
    let r = open_registrar(&t, &curator, &radmin);
    t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::ReassignCuratorSpoke(curator.clone(), HARNESS_SPOKE, Some(r)),
    );
    let key = hub_asset(t.resolve_asset("USDC"));

    let flags = |paused: bool| -> Vec<Val> {
        vec![
            env,
            radmin.into_val(env),
            r.into_val(env),
            HARNESS_SPOKE.into_val(env),
            key.into_val(env),
            paused.into_val(env),
            false.into_val(env),
            false.into_val(env),
        ]
    };
    curator_call::<()>(&t, &curator, "tighten_flags", flags(true));
    let after = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key);
    assert!(after.paused && !after.frozen && !after.no_seize);
    assert_contract_error(
        try_curator_call(&t, &curator, "tighten_flags", flags(true)),
        errors::SPOKE_ASSET_FLAGS_UNCHANGED,
    );
}

#[test]
fn curator_hub_owner_freezes_a_grantee_listing_but_cannot_pause_it() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let env = &t.env;
    let curator = register_curator(&t);
    let (hub_admin, spoke_admin) = (Address::generate(env), Address::generate(env));
    let hub_owner = open_registrar(&t, &curator, &hub_admin);
    let spoke_owner = open_registrar(&t, &curator, &spoke_admin);
    t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::ReassignCuratorHub(curator.clone(), HARNESS_HUB, Some(hub_owner)),
    );
    t.gov_client().execute_immediate(
        &t.admin,
        &AdminOperation::ReassignCuratorSpoke(curator.clone(), HARNESS_SPOKE, Some(spoke_owner)),
    );
    let usdc = t.resolve_asset("USDC");
    let key = hub_asset(usdc.clone());
    let grant: Map<Symbol, Val> = map![
        env,
        (Symbol::new(env, "supply_cap"), 1_000i128.into_val(env)),
        (Symbol::new(env, "borrow_cap"), 1_000i128.into_val(env)),
        (Symbol::new(env, "can_collateral"), true.into_val(env)),
        (Symbol::new(env, "can_borrow"), true.into_val(env)),
        (Symbol::new(env, "ltv"), 5_000u32.into_val(env)),
        (Symbol::new(env, "threshold"), 8_000u32.into_val(env)),
        (Symbol::new(env, "bonus"), 500u32.into_val(env)),
        (Symbol::new(env, "liquidation_fees"), 1_000u32.into_val(env)),
    ];
    curator_call::<()>(
        &t,
        &curator,
        "set_cap_grant",
        vec![
            env,
            hub_admin.into_val(env),
            hub_owner.into_val(env),
            spoke_owner.into_val(env),
            HARNESS_HUB.into_val(env),
            HARNESS_SPOKE.into_val(env),
            usdc.into_val(env),
            grant.into_val(env),
        ],
    );
    let read_grant = || -> Option<Map<Symbol, Val>> {
        curator_call(
            &t,
            &curator,
            "cap_grant",
            vec![
                env,
                spoke_owner.into_val(env),
                HARNESS_HUB.into_val(env),
                HARNESS_SPOKE.into_val(env),
                usdc.into_val(env),
            ],
        )
    };
    assert!(read_grant().is_some());

    let before = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key);
    let flags = |paused: bool, frozen: bool| -> Vec<Val> {
        vec![
            env,
            hub_admin.into_val(env),
            hub_owner.into_val(env),
            HARNESS_SPOKE.into_val(env),
            key.into_val(env),
            paused.into_val(env),
            frozen.into_val(env),
            false.into_val(env),
        ]
    };
    assert_contract_error(
        try_curator_call(&t, &curator, "tighten_flags", flags(true, true)),
        errors::NOT_SPOKE_REGISTRAR,
    );
    curator_call::<()>(&t, &curator, "tighten_flags", flags(false, true));
    let after = t.ctrl_client().get_spoke_asset(&HARNESS_SPOKE, &key);
    assert!(after.frozen && !after.paused && !after.no_seize);
    assert_eq!(after.supply_cap, before.supply_cap);
    assert!(read_grant().is_some());
}

#[test]
fn curator_registrar_creates_a_market_in_its_hub() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let env = &t.env;
    let curator = register_curator(&t);
    let radmin = Address::generate(env);
    let r = open_registrar(&t, &curator, &radmin);
    let hub: u32 = curator_call(
        &t,
        &curator,
        "create_hub",
        vec![env, radmin.into_val(env), r.into_val(env)],
    );
    assert_ne!(hub, HARNESS_HUB);
    let owner: Option<u32> =
        curator_call(&t, &curator, "hub_registrar", vec![env, hub.into_val(env)]);
    assert_eq!(owner, Some(r));

    let usdc = t.resolve_asset("USDC");
    let params = DEFAULT_MARKET_PARAMS.to_market_params(&usdc, 7);
    let pool: Address = curator_call(
        &t,
        &curator,
        "create_market",
        vec![
            env,
            radmin.into_val(env),
            r.into_val(env),
            hub.into_val(env),
            params.into_val(env),
        ],
    );
    assert_eq!(pool, t.markets["USDC"].pool);

    let spoke: u32 = curator_call(
        &t,
        &curator,
        "add_spoke",
        vec![env, radmin.into_val(env), r.into_val(env)],
    );
    let listing = AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
        hub_id: hub,
        asset: usdc,
        spoke_id: spoke,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 5_000,
        threshold: 8_000,
        bonus: 500,
        liquidation_fees: 1_000,
        supply_cap: 1_000,
        borrow_cap: 1_000,
    });
    let _op_id: BytesN<32> = curator_call(
        &t,
        &curator,
        "propose",
        vec![
            env,
            radmin.into_val(env),
            r.into_val(env),
            listing.into_val(env),
            BytesN::from_array(env, &[9; 32]).into_val(env),
        ],
    );
    let proposed = env
        .events()
        .all()
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(body) = &event.body;
            matches!(
                (body.topics.first(), body.topics.get(1)),
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_utf8_string().as_deref() == Ok("curator")
                        && b.0.to_utf8_string().as_deref() == Ok("proposed")
            )
        })
        .count();
    assert_eq!(proposed, 1);
}
