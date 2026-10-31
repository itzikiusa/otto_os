#!/usr/bin/env python3
"""Unit tests for scripts/ci-cargo-cache.py."""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

MOD_PATH = Path(__file__).resolve().parent / "ci-cargo-cache.py"
SPEC = importlib.util.spec_from_file_location("ci_cargo_cache", MOD_PATH)
assert SPEC and SPEC.loader
ci_cargo_cache = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci_cargo_cache)


class CiCargoCacheTests(unittest.TestCase):
    def _init_repo(self, root: Path) -> None:
        subprocess.run(["git", "init", "-q"], cwd=root, check=True)
        (root / "crates" / "foo" / "src").mkdir(parents=True)
        (root / "crates" / "foo" / "migrations").mkdir(parents=True)
        (root / "crates" / "foo" / "src" / "lib.rs").write_text("pub fn x() {}\n")
        (root / "crates" / "foo" / "migrations" / "001.sql").write_text("SELECT 1;\n")
        subprocess.run(["git", "add", "."], cwd=root, check=True)

    def test_cold_restore_stamps_baseline_and_warm_restore_preserves_exact_mtime_ns(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._init_repo(root)

            # Cold cache: no manifest yet -> stamps deterministic baseline mtime_ns.
            self.assertEqual(ci_cargo_cache.cmd_restore(root), 0)
            lib_rs = root / "crates" / "foo" / "src" / "lib.rs"
            mig_sql = root / "crates" / "foo" / "migrations" / "001.sql"
            self.assertEqual(lib_rs.stat().st_mtime_ns, ci_cargo_cache.STABLE_MTIME_NS)
            self.assertEqual(mig_sql.stat().st_mtime_ns, ci_cargo_cache.STABLE_MTIME_NS)
            self.assertEqual(mig_sql.parent.stat().st_mtime_ns, ci_cargo_cache.STABLE_MTIME_NS)

            # Give lib.rs a custom mtime_ns (as after an incremental edit) and save manifest.
            custom_ns = 1700000123_456789000
            os.utime(lib_rs, ns=(custom_ns, custom_ns))
            self.assertEqual(ci_cargo_cache.cmd_prepare_save(root), 0)

            # Simulate a fresh checkout stamping everything with future mtime.
            future = time.time() + 10
            os.utime(lib_rs, (future, future))
            os.utime(mig_sql, (future, future))
            os.utime(mig_sql.parent, (future, future))

            self.assertEqual(ci_cargo_cache.cmd_restore(root), 0)
            self.assertEqual(lib_rs.stat().st_mtime_ns, custom_ns)
            self.assertEqual(mig_sql.stat().st_mtime_ns, ci_cargo_cache.STABLE_MTIME_NS)
            self.assertEqual(mig_sql.parent.stat().st_mtime_ns, ci_cargo_cache.STABLE_MTIME_NS)

            # Adding a new migration updates both the file and the watched directory mtime.
            mig2 = root / "crates" / "foo" / "migrations" / "002.sql"
            mig2.write_text("SELECT 2;\n")
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            before = time.time() - 2
            self.assertEqual(ci_cargo_cache.cmd_restore(root), 0)
            self.assertEqual(lib_rs.stat().st_mtime_ns, custom_ns)
            self.assertGreaterEqual(mig2.stat().st_mtime, before)
            self.assertGreaterEqual(mig2.parent.stat().st_mtime, before)

    def test_v1_manifest_backwards_compatibility(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._init_repo(root)
            cur_files, cur_dirs = ci_cargo_cache.snapshot(root)
            manifest = root / ci_cargo_cache.MANIFEST_REL
            manifest.parent.mkdir(parents=True, exist_ok=True)
            manifest.write_text(json.dumps({"version": 1, "files": cur_files, "dirs": cur_dirs}))

            lib_rs = root / "crates" / "foo" / "src" / "lib.rs"
            os.utime(lib_rs, (time.time() + 20, time.time() + 20))
            self.assertEqual(ci_cargo_cache.cmd_restore(root), 0)
            self.assertEqual(lib_rs.stat().st_mtime_ns, ci_cargo_cache.STABLE_MTIME_NS)

    def test_prepare_save_prunes_incremental_and_expired_deps_but_keeps_examples(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self._init_repo(root)
            debug = root / "target" / "debug"
            (debug / "incremental" / "foo").mkdir(parents=True)
            (debug / "examples").mkdir(parents=True)
            example_bin = debug / "examples" / "fold_bench"
            example_bin.write_text("bin")
            (debug / "deps").mkdir(parents=True)
            fresh_dep = debug / "deps" / "libfoo-111.rlib"
            fresh_dep.write_text("fresh")
            old_dep = debug / "deps" / "libfoo-000.rlib"
            old_dep.write_text("old")
            expired = time.time() - (8 * 24 * 3600)
            os.utime(old_dep, (expired, expired))

            self.assertEqual(ci_cargo_cache.cmd_prepare_save(root), 0)
            self.assertFalse((debug / "incremental").exists())
            self.assertTrue(example_bin.exists())
            self.assertTrue(fresh_dep.exists())
            self.assertFalse(old_dep.exists())


if __name__ == "__main__":
    unittest.main()
