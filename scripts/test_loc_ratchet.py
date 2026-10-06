"""Unit tests for loc-ratchet.py (S12-307)."""

import importlib.util
import json
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location("loc_ratchet", Path(__file__).with_name("loc-ratchet.py"))
mod = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(mod)

BASE = {"new_file_max": 100, "files": {"a.rs": 150, "gone.rs": 120}, "totals": {"otto-server": 1000}}


class RatchetTest(unittest.TestCase):
    def test_growth_fails_shrink_passes(self):
        self.assertEqual(mod.check(BASE, {"a.rs": 150}, {"otto-server": 1000}), [])
        self.assertEqual(mod.check(BASE, {"a.rs": 90}, {"otto-server": 900}), [])
        errs = mod.check(BASE, {"a.rs": 151}, {"otto-server": 1001})
        self.assertEqual(len(errs), 2)
        self.assertIn("a.rs: 151 lines > ratchet 150", errs[0])
        self.assertIn("otto-server src total", errs[1])

    def test_new_untracked_god_module_fails(self):
        errs = mod.check(BASE, {"a.rs": 1, "new.rs": 100}, {"otto-server": 1})
        self.assertEqual(len(errs), 1)
        self.assertIn("new.rs", errs[0])
        self.assertEqual(mod.check(BASE, {"new.rs": 99}, {"otto-server": 1}), [])

    def test_update_only_lowers_and_drops_deleted(self):
        new = mod.lowered(BASE, {"a.rs": 200}, {"otto-server": 900})
        self.assertEqual(new["files"], {"a.rs": 150})  # never raised; gone.rs dropped
        self.assertEqual(new["totals"], {"otto-server": 900})

    def test_raise_is_explicit_per_path(self):
        new = mod.raised(BASE, {"a.rs": 200}, {"otto-server": 1100}, ["a.rs"])
        self.assertEqual(new["files"]["a.rs"], 200)
        self.assertEqual(new["totals"]["otto-server"], 1000)
        new = mod.raised(BASE, {"a.rs": 200}, {"otto-server": 1100}, ["otto-server"])
        self.assertEqual(new["totals"]["otto-server"], 1100)

    def test_repo_baseline_shape(self):
        b = json.loads(mod.BASELINE.read_text())
        self.assertGreater(b["new_file_max"], 0)
        self.assertIn("crates/otto-server/src/workflow_engine.rs", b["files"])
        self.assertIn("crates/otto-server/src/modules.rs", b["files"])
        self.assertIn("otto-server", b["totals"])
        for f, n in b["files"].items():
            self.assertGreaterEqual(n, b["new_file_max"], f"{f} is below the cap; drop it")


if __name__ == "__main__":
    unittest.main()
