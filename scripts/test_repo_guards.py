#!/usr/bin/env python3
"""Unit tests for scripts/repo-guards.py."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("repo_guards", Path(__file__).with_name("repo-guards.py"))
assert SPEC and SPEC.loader
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)


class RepoGuardsTests(unittest.TestCase):
    def test_keyed_lock_flags_new_and_allows_allowlisted(self) -> None:
        bad = "locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,"
        self.assertEqual(mod.check_keyed_lock({"crates/otto-core/src/keyed_lock.rs": bad}), [])
        self.assertEqual(mod.check_keyed_lock({"crates/otto-sessions/src/manager.rs": bad}), [])
        errs = mod.check_keyed_lock({"crates/otto-new/src/lib.rs": bad})
        self.assertEqual(len(errs), 1)
        self.assertIn("crates/otto-new/src/lib.rs", errs[0])
        # Multi-line unrelated HashMap + Mutex must not trigger false positives.
        unrelated = "struct A { m: HashMap<String, usize>, }\nstruct B { l: Arc<Mutex<()>> }"
        self.assertEqual(mod.check_keyed_lock({"crates/otto-new/src/lib.rs": unrelated}), [])

    def test_otto_e2e_allows_owner_and_flags_other_crates(self) -> None:
        code = 'let x = std::env::var("OTTO_E2E").is_ok();'
        self.assertEqual(mod.check_otto_e2e({"crates/otto-core/src/e2e.rs": code}), (0, []))
        self.assertEqual(mod.check_otto_e2e({"crates/otto-server/tests/e2e.rs": code}), (0, []))
        cnt, errs = mod.check_otto_e2e({"crates/otto-server/src/lib.rs": code})
        self.assertEqual(cnt, 1)
        self.assertEqual(len(errs), 1)
        self.assertIn("crates/otto-server/src/lib.rs:1", errs[0])

    def test_prompt_submit_allows_otto_sessions_and_tests_flags_others(self) -> None:
        code = r'write(b"\r");'
        self.assertEqual(mod.check_prompt_submit({"crates/otto-sessions/src/submit.rs": code}), [])
        self.assertEqual(mod.check_prompt_submit({"crates/otto-server/tests/it.rs": code}), [])
        errs = mod.check_prompt_submit({"crates/otto-ide/src/term.rs": code})
        self.assertEqual(len(errs), 1)
        self.assertIn("crates/otto-ide/src/term.rs", errs[0])

    def test_c0_bytes_flags_control_bytes_allows_tab_lf_cr(self) -> None:
        self.assertEqual(mod.check_c0_bytes({"a.rs": b"fn a() {\t\r\n}\n"}), [])
        errs = mod.check_c0_bytes({"a.rs": b"let s = \"a\x00b\";", "b.ts": b"const x = '\x1b';"})
        self.assertEqual(len(errs), 1)
        self.assertIn("a.rs", errs[0])
        self.assertIn("b.ts", errs[0])

    def test_ui_lib_shims_ratchet(self) -> None:
        shim = "export * from '../../lib/api/types/foo';\n"
        type_shim = "export type * from '../../../lib/api/types/bar';\n"
        under_files = {"ui/src/modules/a/x.ts": shim}
        cnt, notes, errs = mod.check_ui_lib_shims(under_files)
        self.assertEqual(cnt, 1)
        self.assertEqual(errs, [])
        self.assertTrue(notes, "should suggest lowering UI_LIB_SHIM_MAX when under cap")

        at_cap_files = {
            "ui/src/modules/a/x.ts": shim,
            "ui/src/modules/b/y.ts": type_shim,
        }
        cnt, notes, errs = mod.check_ui_lib_shims(at_cap_files)
        self.assertEqual(cnt, 2)
        self.assertEqual(errs, [])
        self.assertEqual(notes, [])

        over_files = {f"ui/src/modules/m{i}/t.ts": shim for i in range(mod.UI_LIB_SHIM_MAX + 1)}
        cnt, _, errs = mod.check_ui_lib_shims(over_files)
        self.assertEqual(cnt, mod.UI_LIB_SHIM_MAX + 1)
        self.assertEqual(len(errs), 1)

    def test_live_repo_passes_all_guards(self) -> None:
        self.assertEqual(mod.main(["--all"]), 0)


if __name__ == "__main__":
    unittest.main()
