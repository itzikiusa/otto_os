#!/usr/bin/env python3
"""Unit tests for scripts/ci-route.py (which PR jobs a change needs)."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("ci_route", Path(__file__).resolve().parent / "ci-route.py")
assert SPEC and SPEC.loader
ci_route = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci_route)
route = ci_route.route


class CiRouteTests(unittest.TestCase):
    def test_ui_only_change_skips_rust_but_keeps_code_gates(self) -> None:
        r = route(["ui/src/modules/git/GitTabs.svelte", "ui/e2e/x.spec.ts"])
        self.assertFalse(r["rust"])
        self.assertTrue(r["code"])
        self.assertFalse(r["plugin"])
        self.assertTrue(r["daemon"])

    def test_rust_change_runs_rust(self) -> None:
        for p in ["crates/otto-git/src/lib.rs", "Cargo.lock", "vendor/vt100/src/lib.rs", ".config/nextest.toml",
                  ".github/workflows/ci.yml", "docs/contracts/ui-commands.json", "scripts/loc-ratchet.py"]:
            self.assertTrue(route([p])["rust"], p)

    def test_docs_only_change_skips_everything_but_routing(self) -> None:
        r = route(["docs/testing/build-performance.md", "README.md", "marketing/videos/out/Git.mp4"])
        self.assertEqual((r["rust"], r["code"], r["plugin"], r["desktop"]), (False, False, False, False))

    def test_contracts_are_code_and_rust(self) -> None:
        r = route(["docs/contracts/api.md"])
        self.assertTrue(r["rust"] and r["code"])

    def test_unknown_top_level_paths_fail_open(self) -> None:
        r = route(["newdir/thing.toml"])
        self.assertTrue(r["rust"] and r["code"])

    def test_plugin_and_desktop_routes(self) -> None:
        self.assertTrue(route(["examples/plugins/team-performance/server.js"])["plugin"])
        self.assertTrue(route(["ui/src/lib/tokens.css"])["plugin"])
        r = route(["apps/desktop/src-tauri/src/main.rs"])
        self.assertTrue(r["desktop"])
        self.assertFalse(r["daemon"])
        self.assertFalse(r["rust"])

    def test_empty_diff_runs_everything(self) -> None:
        self.assertTrue(all(route([]).values()))


if __name__ == "__main__":
    unittest.main()
