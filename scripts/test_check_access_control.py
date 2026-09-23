"""Checks that only a positive test cfg classifies an entrypoint as test-only.

`#[cfg(not(test))]` is true in every release build, so a `#[contractimpl]`
block behind it ships in the deployable WASM. Classified `test-only`, an
ungated mutator in it would pass `check()` with no declared line.
"""

import unittest

import check_access_control as ac


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


if __name__ == "__main__":
    unittest.main()
