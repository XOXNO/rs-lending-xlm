"""Offline checks for local dependency coverage and freshness invalidation."""

import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

import focused_wasm as focused
import write_wasm_manifest as manifest


class FocusedInputsTests(unittest.TestCase):
    def test_local_dependencies_are_covered(self):
        workspace = tomllib.loads((focused.ROOT / "Cargo.toml").read_text())
        patches = [
            focused.ROOT / dependency["path"]
            for source in workspace["patch"].values()
            for dependency in source.values()
            if "path" in dependency
        ]
        for layer, inputs in focused.BASE_INPUTS.items():
            covered = set(manifest.resolved_inputs(inputs))
            package = focused.ROOT / ("common" if layer == "common" else f"contracts/{layer}")
            pending, seen = [package, *patches], set()
            while pending:
                root = pending.pop().resolve()
                if root in seen:
                    continue
                seen.add(root)
                cargo = root / "Cargo.toml"
                with self.subTest(layer=layer, dependency=str(root)):
                    self.assertIn(cargo, covered)
                    self.assertTrue({p for p in (root / "src").rglob("*") if p.is_file()} <= covered)
                data = tomllib.loads(cargo.read_text())
                workspace_root, workspace_data = root, data
                while "workspace" not in workspace_data:
                    workspace_root = workspace_root.parent
                    workspace_cargo = workspace_root / "Cargo.toml"
                    workspace_data = tomllib.loads(workspace_cargo.read_text()) if workspace_cargo.exists() else {}
                for section in ("dependencies", "build-dependencies"):
                    for name, dependency in data.get(section, {}).items():
                        origin = root
                        if isinstance(dependency, dict) and dependency.get("workspace"):
                            dependency = workspace_data["workspace"]["dependencies"][name]
                            origin = workspace_root
                        if isinstance(dependency, dict) and "path" in dependency:
                            pending.append(origin / dependency["path"])

    def test_model_macro_interface_and_summary_changes_invalidate_fingerprint(self):
        changed = [
            "vendor/cvlr-soroban/Cargo.toml",
            "vendor/cvlr-soroban/cvlr-soroban/src/lib.rs",
            "vendor/cvlr-soroban/cvlr-soroban-macros/src/lib.rs",
            "vendor/cvlr-soroban/cvlr-soroban-derive/src/lib.rs",
        ]
        extras = {
            "controller": ["interfaces/swap-aggregator/src/lib.rs", "interfaces/position-nft/src/lib.rs"],
            "common": ["certora/common/spec/summaries.rs"],
            "pool": ["certora/common/spec/summaries.rs"],
        }
        with tempfile.TemporaryDirectory() as directory, patch.object(manifest, "ROOT", Path(directory)):
            declared = {relative for inputs in focused.BASE_INPUTS.values() for relative in inputs}
            for relative in declared | set(changed) | {p for paths in extras.values() for p in paths}:
                path = manifest.ROOT / relative
                if not path.suffix and path.name != "Makefile":
                    path /= "fixture.rs"
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("original")
            for layer, inputs in focused.BASE_INPUTS.items():
                before = manifest.input_fingerprint(inputs)
                for relative in changed + extras.get(layer, []):
                    path = manifest.ROOT / relative
                    path.write_text("changed")
                    with self.subTest(layer=layer, changed=relative):
                        self.assertNotEqual(before, manifest.input_fingerprint(inputs))
                    path.write_text("original")


if __name__ == "__main__":
    unittest.main()
