"""Regression checks for the production math boundary."""

import unittest

from check_fp_boundary import violations


class FpBoundaryTest(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
