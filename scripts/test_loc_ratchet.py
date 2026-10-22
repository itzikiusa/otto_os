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

    def test_production_lines_skip_top_level_test_items(self):
        src = "\n".join([
            "fn a() {",
            "    1",
            "}",
            "#[cfg(test)]",
            "fn helper() {",
            "    // test-only",
            "}",
            "fn b() {}",
            "#[cfg(test)]",
            "mod tests;",
            "#[cfg(test)]",
            "#[allow(dead_code)]",
            "mod tests {",
            "    #[test]",
            "    fn t() {",
            "    }",
            "}",
        ])
        self.assertEqual(mod.production_lines(src), ["fn a() {", "    1", "}", "fn b() {}"])
        # An indented #[cfg(test)] (inside an impl) is not a top-level item.
        self.assertEqual(len(mod.production_lines("impl X {\n    #[cfg(test)]\n    fn t() {}\n}")), 4)
        # A bare top-level `mod tests { … }` counts as tests too.
        self.assertEqual(mod.production_lines("fn a() {}\nmod tests {\n    fn t() {}\n}\nfn b() {}"), ["fn a() {}", "fn b() {}"])

    def test_moving_tests_out_no_longer_creates_headroom(self):
        with_tests = "fn a() {}\n" * 10 + "#[cfg(test)]\nmod tests {\n" + "    // t\n" * 50 + "}\n"
        self.assertEqual(len(mod.production_lines(with_tests)), 10)

    def test_function_lengths(self):
        src = "\n".join([
            "pub(crate) async fn big(",
            "    x: u8,",
            ") -> u8 {",
            "    let s = \"{\";",
            "    x",
            "}",
            "impl Y {",
            "    fn big(&self) {",
            "    }",
            "    fn decl(&self);",
            "}",
        ])
        lens = mod.function_lengths(src)
        self.assertEqual(lens["big"], 6, "longest definition wins; braces in strings are ignored")
        self.assertNotIn("decl", lens)

    def test_function_caps(self):
        base = {**BASE, "functions": {"execute_node": 100}}
        self.assertEqual(mod.check(base, {}, {"otto-server": 1}, {"execute_node": 100}), [])
        errs = mod.check(base, {}, {"otto-server": 1}, {"execute_node": 101})
        self.assertEqual(len(errs), 1)
        self.assertIn("fn execute_node: 101 lines > ratchet 100", errs[0])
        errs = mod.check(base, {}, {"otto-server": 1}, {})
        self.assertIn("not found", errs[0])
        self.assertEqual(mod.lowered(base, {}, {"otto-server": 1}, {"execute_node": 80})["functions"], {"execute_node": 80})
        self.assertEqual(mod.raised(base, {}, {"otto-server": 1}, ["fn:execute_node"], {"execute_node": 120})["functions"], {"execute_node": 120})

    def test_test_files_are_not_production(self):
        self.assertTrue(mod.is_test_file("crates/otto-server/tests/it.rs"))
        self.assertTrue(mod.is_test_file("crates/otto-sessions/src/manager/tests/mod.rs"))
        self.assertTrue(mod.is_test_file("crates/x/src/foo_tests.rs"))
        self.assertTrue(mod.is_test_file("crates/x/src/tests.rs"))
        self.assertFalse(mod.is_test_file("crates/x/src/testsuite.rs"))

    def test_lowered_drops_files_under_the_cap(self):
        new = mod.lowered(BASE, {"a.rs": 99, "gone.rs": 110}, {"otto-server": 900})
        self.assertEqual(new["files"], {"gone.rs": 110})

    def test_repo_baseline_shape(self):
        b = json.loads(mod.BASELINE.read_text())
        self.assertGreater(b["new_file_max"], 0)
        self.assertIn("crates/otto-server/src/workflow_engine.rs", b["files"])
        self.assertIn("crates/otto-server/src/modules.rs", b["files"])
        self.assertIn("otto-server", b["totals"])
        for name in ("execute_node", "run_workflow_with_budget", "run_review_core"):
            self.assertIn(name, b["functions"])
        for f, n in b["files"].items():
            self.assertGreaterEqual(n, b["new_file_max"], f"{f} is below the cap; drop it")


if __name__ == "__main__":
    unittest.main()
