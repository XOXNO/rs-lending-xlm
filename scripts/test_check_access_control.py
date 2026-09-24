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

    def test_execute_operation_readiness_is_timelock_evidence(self):
        path = os.path.join("contracts", "governance", "src", "timelock", "mod.rs")
        guard = "exec.require_auth();"
        role = "access_control::ensure_role(env, &Symbol::new(env, EXECUTOR_ROLE), exec);"
        sources = []
        for rel, text in ac.rust_sources():
            if rel == path:
                self.assertIn(guard, text)
                self.assertIn(role, text)
                text = text.replace(guard, " " * len(guard)).replace(role, " " * len(role))
            sources.append((rel, text))
        eps = by_key(classified(sources=sources))
        self.assertEqual(eps["governance::execute"]["category"], "role-timelock")


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


if __name__ == "__main__":
    unittest.main()
