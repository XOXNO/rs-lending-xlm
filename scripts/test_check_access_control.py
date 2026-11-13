"""Checks for the access-control gate: cfg handling, contractimpl discovery,
pinned test-only symbols, guard patterns and declaration-file validation."""

import os
import tempfile
import unittest

import check_access_control as ac

NFT_MUTATORS = ("transfer", "transfer_from", "approve", "approve_for_all")


def classified(sources=None, mutate=None):
    """Discover and classify the real tree; `mutate(eps)` edits entrypoints first."""
    eps = ac.discover_entrypoints()
    if mutate:
        mutate(eps)
    ac.classify(eps, ac.CallGraph(sources if sources is not None else ac.rust_sources()))
    return eps


def by_key(eps):
    return {f"{fn['contract']}::{fn['name']}": fn for fn in eps}


class TestOnlyCfgTest(unittest.TestCase):
    def test_only_a_positive_test_cfg_counts_as_test_only(self):
        for cfg in (
            "#[cfg(test)]",
            '#[cfg(feature = "testing")]',
            '#[cfg(any(test, feature = "testing"))]',
        ):
            self.assertTrue(ac.is_test_only({"cfg": cfg}), cfg)
        for cfg in (
            "",
            "#[cfg(not(test))]",
            '#[cfg(not(feature = "testing"))]',
            '#[cfg(not(any(test, feature = "testing")))]',
            '#[cfg(feature = "certora")]',
            '#[cfg(any(test, feature = "production"))]',
            '#[cfg(any(feature = "testing", not(test)))]',
            '#[cfg(feature = "production] #[cfg(test)]")]',
        ):
            self.assertFalse(ac.is_test_only({"cfg": cfg}), cfg)

    def test_dropping_the_cfg_from_a_gated_test_only_entrypoint_fails(self):
        def drop_cfg(eps):
            by_key(eps)["governance::execute_immediate"]["cfg"] = ""

        eps = classified(mutate=drop_cfg)
        violations = ac.check(eps, ac.load_allowlist(ac.ALLOWLIST))
        self.assertTrue(any("execute_immediate" in v for v in violations), violations)
        self.assertTrue(ac.test_only_violations(eps))

    def test_the_real_tree_matches_the_pinned_test_only_set(self):
        self.assertEqual(ac.test_only_violations(classified()), [])


class ContractimplDiscoveryTest(unittest.TestCase):
    def test_array_returns_and_literal_delimiters_do_not_hide_functions(self):
        src = '''#[contractimpl] impl X {
            fn first() { let text = r##"} ) // /* fn fake() {"##; }
            fn second() -> [u32; 1] { let ch = '}'; [1] }
        }'''
        eps = ac._entrypoints_in_file("x", "contracts/x/src/lib.rs", src)
        self.assertEqual([fn["name"] for fn in eps], ["first", "second"])
        self.assertEqual(set(ac.CallGraph([("contracts/x/src/lib.rs", src)]).by_name), {"first", "second"})
        with self.assertRaises(ac.ParseError):
            ac._entrypoints_in_file("x", "contracts/x/src/lib.rs", "#[contractimpl] impl X { fn ok() {} fn missing(); }")

    def test_contracttrait_nft_mutators_are_discovered_as_caller_auth(self):
        eps = by_key(classified())
        for name in NFT_MUTATORS:
            self.assertEqual(eps[f"position-nft::{name}"]["category"], "caller-auth", name)
            self.assertTrue(eps[f"position-nft::{name}"]["mutates"], name)
        for name in ("owner_of", "balance", "token_uri", "total_supply", "get_token_id"):
            self.assertEqual(eps[f"position-nft::{name}"]["category"], "view", name)

    def test_unknown_contractimpl_argument_is_a_parse_error(self):
        with self.assertRaises(ac.ParseError):
            ac._entrypoints_in_file(
                "x",
                "contracts/x/src/lib.rs",
                "#[contractimpl(bogus)]\nimpl T for X { fn f(e: Env) {} }",
            )

    def test_unknown_contracttrait_is_a_parse_error(self):
        with self.assertRaises(ac.ParseError):
            ac._entrypoints_in_file(
                "x",
                "contracts/x/src/lib.rs",
                "#[contractimpl(contracttrait)]\nimpl Mystery for X {}",
            )

    def test_custom_contract_type_is_a_parse_error(self):
        for body in ("type ContractType = MyOverrides;", ""):
            with self.assertRaises(ac.ParseError, msg=body):
                ac._entrypoints_in_file(
                    "x",
                    "contracts/x/src/lib.rs",
                    "#[contractimpl(contracttrait)]\nimpl NonFungibleToken for X { " + body + " }",
                )
        with self.assertRaises(ac.ParseError):
            ac._entrypoints_in_file(
                "x",
                "contracts/x/src/overrides.rs",
                "pub struct Enumerable;\nimpl stellar_tokens::ContractOverrides for Enumerable {}",
            )

    def test_a_contracttrait_override_is_classified_from_its_body(self):
        src = (
            "#[contractimpl(contracttrait)]\n"
            "impl NonFungibleToken for X {\n"
            "    type ContractType = Base;\n"
            "    fn transfer(e: &Env, from: Address, to: Address, token_id: u32) {\n"
            "        e.storage().persistent().set(&token_id, &to);\n"
            "    }\n"
            "}\n"
        )
        eps = ac._entrypoints_in_file("x", "contracts/x/src/lib.rs", src)
        names = {fn["name"] for fn in eps}
        self.assertEqual(names, set(ac.CONTRACTTRAIT_METHODS["NonFungibleToken"]))
        ac.classify(eps, ac.CallGraph([]))
        cats = {fn["name"]: fn["category"] for fn in eps}
        self.assertEqual(cats["transfer"], "UNGATED-MUTATOR")
        self.assertEqual(cats["transfer_from"], "caller-auth")
        self.assertEqual(cats["owner_of"], "view")


class GuardPatternTest(unittest.TestCase):
    def test_literal_text_cannot_supply_authorization_or_call_edges(self):
        graph = ac.CallGraph([("contracts/x/src/lib.rs",
                              'fn guard() { enforce_owner_auth(&env); } '
                              'fn note() { let text = "enforce_owner_auth(&env);"; }')])
        for literal in (
            '"enforce_owner_auth(&env); guard(); }"',
            'r###"enforce_owner_auth(&env); guard(); \\" }"###',
            'b"enforce_owner_auth(&env); guard();"',
            'c"enforce_owner_auth(&env); guard();"',
        ):
            fn = {"contract": "x", "name": "entry", "attrs": [], "cfg": "",
                  "body": f"{{ note(); let text = {literal}; env.storage().instance().set(&1, &2); }}"}
            ac.classify([fn], graph)
            self.assertEqual(fn["category"], "UNGATED-MUTATOR", literal)

    def test_storage_parameters_aliases_and_array_return_helpers_are_writes(self):
        for storage in ("Instance", "soroban_sdk::storage::Persistent<'_>", "Temporary"):
            for body in ("store.set(&1, &2);", "let a = store; let b = &a; b.remove(&1);"):
                src = f"#[contractimpl] impl X {{ fn entry(store: &{storage}) {{ {body} }} }}"
                eps = ac._entrypoints_in_file("x", "contracts/x/src/lib.rs", src)
                ac.classify(eps, ac.CallGraph([]))
                self.assertEqual(eps[0]["category"], "UNGATED-MUTATOR", src)
        src = "fn write(env: &Env) -> [u32; 1] { env.storage().instance().set(&1, &2); [1] }"
        fn = {"contract": "x", "name": "entry", "attrs": [], "cfg": "", "body": "{ write(&env); }"}
        ac.classify([fn], ac.CallGraph([("contracts/x/src/lib.rs", src)]))
        self.assertEqual(fn["category"], "UNGATED-MUTATOR")
        self.assertTrue(ac.storage_writes("let a = env.storage().instance(); let b = a; b.set(&1, &2);"))
        self.assertFalse(ac.storage_writes("fn read(store: &Instance) { store.get::<_, u32>(&1); store.extend_ttl(1, 2); }"))

    def test_write_walk_has_no_depth_cutoff_and_stops_at_cycles(self):
        src = " ".join(f"fn helper{i}() {{ helper{i + 1}(); }}" for i in range(15))
        src += " fn helper15() { env.storage().instance().set(&1, &2); helper0(); }"
        fn = {"contract": "x", "name": "entry", "attrs": [], "cfg": "", "body": "{ helper0(); }"}
        graph = ac.CallGraph([("contracts/x/src/lib.rs", src)])
        ac.classify([fn], graph)
        self.assertEqual(fn["category"], "UNGATED-MUTATOR")
        self.assertFalse(graph.reaches(fn["body"], "x", ac.OWNER_PATTERNS))

    def test_owner_patterns_match_the_locked_oz_name(self):
        for text in ("ownable::enforce_owner_auth(&e);", "enforce_owner_auth(env);"):
            self.assertTrue(any(p.search(text) for p in ac.OWNER_PATTERNS), text)

    def test_an_ambiguous_same_name_helper_is_not_gate_evidence(self):
        graph = ac.CallGraph(
            [
                ("contracts/x/src/lib.rs", "fn helper(env: &Env) {}"),
                (
                    "contracts/x/src/other.rs",
                    "fn helper(env: &Env) { access_control::ensure_role(env, &role, &caller); }",
                ),
            ]
        )
        fn = {
            "contract": "x",
            "name": "entry",
            "attrs": [],
            "cfg": "",
            "body": "fn entry(env: Env) { helper(&env); "
            "env.storage().instance().set(&1u32, &2u32); }",
        }
        ac.classify([fn], graph)
        self.assertEqual(fn["category"], "UNGATED-MUTATOR")

    def test_a_turbofish_invoke_contract_is_an_arbitrary_target(self):
        graph = ac.CallGraph(
            [
                (
                    "contracts/x/src/lib.rs",
                    "fn begin_immediate(env: &Env) { access_control::ensure_role(env, &role, &caller); }",
                )
            ]
        )
        fn = {
            "contract": "x",
            "name": "entry",
            "attrs": [],
            "cfg": "",
            "body": "fn entry(env: Env, t: Address, s: Symbol, args: Vec<Val>) { "
            "begin_immediate(&env); env.invoke_contract::<()>(&t, &s, args); }",
        }
        ac.classify([fn], graph)
        real = by_key(classified())
        self.assertTrue(fn["mutates"])
        self.assertEqual(fn["category"], real["governance::pause"]["category"])
        self.assertTrue(real["governance::suspend_curator_registrar"]["mutates"])
        self.assertEqual(
            real["governance::suspend_curator_registrar"]["category"],
            real["governance::pause"]["category"],
        )

    def test_execute_operation_readiness_is_timelock_evidence(self):
        # Execution authorizes no caller and checks no role (audit finding
        # I-02), so the timelock primitive is the only evidence the classifier
        # can find on these three entrypoints.
        path = os.path.join("contracts", "governance", "src", "timelock", "mod.rs")
        texts = [text for rel, text in ac.rust_sources() if rel == path]
        self.assertEqual(len(texts), 1)
        self.assertNotIn("EXECUTOR", texts[0])
        prepare = texts[0].split("fn prepare_execute(", 1)[1].split("\n}\n", 1)[0]
        self.assertNotIn("require_auth", prepare)
        self.assertNotIn("ensure_role", prepare)
        eps = by_key(classified())
        for name in ("execute", "execute_self", "execute_canceller_reset"):
            self.assertEqual(eps[f"governance::{name}"]["category"], "role-timelock")


class DeclarationFileTest(unittest.TestCase):
    def load(self, invariants):
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
            fh.write(
                f"controller::supply | caller-auth | {invariants} | "
                "Anyone may top up an account they already supply to.\n"
            )
        try:
            return ac.load_allowlist(fh.name)
        finally:
            os.unlink(fh.name)

    def test_a_cited_invariant_must_exist(self):
        self.assertIn("controller::supply", self.load("INV-AUTH-03"))
        with self.assertRaises(ac.AllowlistError):
            self.load("INV-AUTH-03, INV-STRAT-99")


class DependencyRevisionTest(unittest.TestCase):
    def test_manifest_and_lock_must_both_match_reviewed_release(self):
        ac.check_dependency_revision()
        checksum = ac.REVIEWED_OZ_CHECKSUMS["stellar-tokens"]
        pin = f'stellar-tokens = "={ac.REVIEWED_OZ_VERSION}"'
        changes = (
            ("Cargo.toml", pin, 'stellar-tokens = "=0.8.1"'),
            ("Cargo.toml", pin, f'stellar-tokens = "{ac.REVIEWED_OZ_VERSION}"'),
            ("Cargo.lock", checksum, "0" * 64),
        )
        with tempfile.TemporaryDirectory() as root:
            for changed, old, new in changes:
                for name in ("Cargo.toml", "Cargo.lock"):
                    with open(os.path.join(ac.REPO_ROOT, name), encoding="utf-8") as fh:
                        content = fh.read()
                    if name == changed:
                        self.assertIn(old, content)
                        content = content.replace(old, new)
                    with open(os.path.join(root, name), "w", encoding="utf-8") as fh:
                        fh.write(content)
                with self.assertRaises(ac.ParseError, msg=f"{changed}: {new}"):
                    ac.check_dependency_revision(root)


if __name__ == "__main__":
    unittest.main()
