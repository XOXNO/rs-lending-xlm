"""Regression checks for the production math boundary."""

import tomllib
import unittest

from check_fp_boundary import CONTROLLER_RAW_BOUNDARIES, ROOT, violations


class FpBoundaryTest(unittest.TestCase):
    def test_reference_access_preserves_each_layers_execution_model(self):
        common = tomllib.loads((ROOT / "common/Cargo.toml").read_text())["features"]
        self.assertEqual(common["math-test"], [])
        self.assertEqual(common["math-proof"], ["math-test"])
        for contract in ("controller", "price-aggregator"):
            features = tomllib.loads((ROOT / f"contracts/{contract}/Cargo.toml").read_text())["features"]
            self.assertIn("common/math-proof", features["certora"])
            self.assertFalse(any("common/certora" in edges for edges in features.values()))
        pool = tomllib.loads((ROOT / "contracts/pool/Cargo.toml").read_text())["features"]
        self.assertIn("common/certora", pool["certora"])

    def test_core_imports_and_grouped_verification_imports_fail(self):
        for source in (
            "use common::math::fp_core as raw;",
            "use common::math::{\n reference as raw\n};",
            "use common::math::reference::mul_div_floor as floor;",
            "let x = mul_div_floor(env, a, b, c);",
        ):
            with self.subTest(source=source):
                self.assertTrue(violations("contracts/pool/src/guards.rs", source))

    def test_typed_api_and_noncode_mentions_pass(self):
        source = '''
            // fp_core::mul_div_floor(env, a, b, c)
            /* nested /* fp_core */ U256 */
            let label = r#"fp_core U256"#;
            let value = amount.div_floor(env, index);
        '''
        self.assertEqual(violations("common/src/rates/index.rs", source), [])

    def test_only_math_can_access_core(self):
        self.assertEqual(
            violations("common/src/math/fp.rs", "fp_core::mul_div_floor(env, a, b, c)"),
            [],
        )
        self.assertTrue(
            violations("common/src/math/escape.rs", "fp_core::mul_div_floor(env, a, b, c)")
        )

    def test_wide_exceptions_do_not_allow_raw_helpers(self):
        file = "common/src/oracle/lp.rs"
        self.assertEqual(violations(file, "U256::from_u128(env, n)"), [])
        self.assertTrue(violations(file, "fp_core::mul_div_floor(env, a, b, c)"))
        self.assertTrue(violations("common/src/rates/compound.rs", "I256::from_i128(env, n)"))

    def test_controller_raw_extraction_needs_a_reviewed_boundary(self):
        errors = violations(
            "contracts/controller/src/risk/unreviewed.rs",
            "let unhealthy = health_factor.raw() < WAD;",
        )
        self.assertTrue(any("reviewed serialization boundary" in error for error in errors))

    def test_reviewed_controller_raw_boundary_inventory_matches_source(self):
        for relative in CONTROLLER_RAW_BOUNDARIES:
            with self.subTest(relative=relative):
                source = (ROOT / relative).read_text()
                raw_errors = [
                    error
                    for error in violations(relative, source)
                    if "raw fixed-point" in error or "reviewed raw boundary" in error
                ]
                self.assertEqual(raw_errors, [])


if __name__ == "__main__":
    unittest.main()
