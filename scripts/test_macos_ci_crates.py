"""Unit tests for macos-ci-crates.py (S12-306 / S12-310)."""

import importlib.util
import shutil
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "macos_ci_crates", Path(__file__).with_name("macos-ci-crates.py")
)
mod = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(mod)

WF = """
on:
  pull_request:
    paths:
      - 'crates/a/**'
      - 'crates/b/**'
  push:
    branches: [main]
    paths:
      - 'crates/a/**'
      - 'crates/b/**'
permissions:
  contents: read
jobs:
  daemon:
    steps:
      - name: cargo clippy (macOS cfg)
        run: >-
          cargo clippy
          -p a
          --all-targets
      - name: cargo nextest
        run: >-
          cargo nextest run
          -p a
"""


class CheckTest(unittest.TestCase):
    dirs = {"a": Path("/r/crates/a"), "b": Path("/r/crates/b"), "c": Path("/r/crates/c")}

    def test_in_sync(self):
        self.assertEqual(mod.check(WF, {"a"}, {"a", "b"}, self.dirs), [])

    def test_a_macos_crate_missing_from_the_test_lists_fails(self):
        # The S12-306 shape: otto-automation has a macOS-only test but no
        # macOS job names it.
        errs = mod.check(WF, {"a", "c"}, {"a", "b", "c"}, self.dirs)
        self.assertTrue(any("missing -p c" in e for e in errs), errs)
        self.assertTrue(any("crates/c/**" in e for e in errs), errs)

    def test_push_paths_must_mirror_pr_paths(self):
        wf = WF.replace("    paths:\n      - 'crates/a/**'\n      - 'crates/b/**'\npermissions", "    paths:\n      - 'crates/a/**'\npermissions")
        errs = mod.check(wf, {"a"}, {"a", "b"}, self.dirs)
        self.assertTrue(any("push paths differ" in e for e in errs), errs)

    def test_macos_cfg_detection(self):
        self.assertTrue(mod.MACOS_CFG.search('#[cfg(target_os = "macos")]'))
        self.assertTrue(mod.MACOS_CFG.search('cfg!(all(target_os="macos", x))'))
        self.assertFalse(mod.MACOS_CFG.search('#[cfg(target_os = "linux")]'))

    @unittest.skipUnless(shutil.which("cargo"), "needs cargo metadata")
    def test_repo_workflow_is_in_sync(self):
        macos, trigger, dirs = mod.compute()
        # The two crates S12-306 found untested on macOS.
        self.assertIn("otto-automation", macos)
        self.assertIn("otto-ssh", macos)
        self.assertEqual(mod.check(mod.WORKFLOW.read_text(), macos, trigger, dirs), [])


if __name__ == "__main__":
    unittest.main()
