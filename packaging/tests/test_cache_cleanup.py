"""Run deploy cleanup blocks and the real pruner only in disposable target trees."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

REPO = Path(__file__).resolve().parents[2]


class CacheCleanup(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="otto-cache-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        packaging = self.root / "packaging"
        packaging.mkdir()
        shutil.copyfile(REPO / "packaging/prune-target.sh", packaging / "prune-target.sh")
        mocks = self.root / "mocks"
        mocks.mkdir()
        pgrep = mocks / "pgrep"
        pgrep.write_text('#!/bin/bash\n[[ "${TEST_BUILD_RUNNING:-0}" == 1 ]]\n')
        pgrep.chmod(0o755)
        self.env = {**os.environ, "PATH": str(mocks) + ":" + os.environ["PATH"],
                    "ROOT": str(self.root), "HERE": str(packaging)}
        for key in ("PRUNE", "PRUNE_WINDOW_SECS", "TEST_BUILD_RUNNING"):
            self.env.pop(key, None)
        self.old_artifacts = []
        self.build_outputs = []
        self.protected = []
        now = time.time()
        for workspace in ("target", "apps/desktop/src-tauri/target"):
            for profile in ("debug", "release"):
                base = self.root / workspace / profile
                # Two feature/host variants can both still be reachable even
                # when Cargo has reused the older variant for several days.
                for digest, timestamp in (("aaaaaaaaaaaaaaaa", now - 7 * 86400),
                                          ("bbbbbbbbbbbbbbbb", now)):
                    for suffix in ("rlib", "rmeta"):
                        artifact = base / "deps" / f"libshared-{digest}.{suffix}"
                        self.write_at(artifact, timestamp)
                        if digest.startswith("a"):
                            self.old_artifacts.append(artifact)
                    for directory, filename in (("build", "out/bindgen.rs"),
                                                (".fingerprint", "lib-shared.json"),
                                                ("incremental", "session/dep-graph.bin")):
                        output = base / directory / f"shared-{digest}" / filename
                        self.write_at(output, timestamp)
                        # The pruner weighs the directory's own mtime too.
                        parent = output.parent
                        while parent != base / directory:
                            os.utime(parent, (timestamp, timestamp))
                            parent = parent.parent
                        self.build_outputs.append(output)
                for relative in ("ottod", "deploy-build.receipt", "bundle/macos/Otto.app/Contents/MacOS/ottod"):
                    path = base / relative
                    self.write_at(path, now - 7 * 86400)
                    self.protected.append(path)

    @staticmethod
    def write_at(path, timestamp):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("usable cached artifact\n")
        os.utime(path, (timestamp, timestamp))

    def run_script(self, script, **env):
        result = subprocess.run(["/bin/bash", "-c", script], env={**self.env, **env},
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def deploy_cleanup(self, path, **env):
        source = (REPO / path).read_text()
        block = source.split('echo "==> 5/7 ', 1)[1].split('\n[[ "$(git rev-parse HEAD)"', 1)[0]
        block = 'echo "==> 5/7 ' + block
        return self.run_script('ok() { :; }; warn() { :; };\n' + block + '\n:', **env)

    def assert_preserved(self, paths):
        for path in paths:
            self.assertTrue(path.is_file(), f"evicted usable artifact: {path.relative_to(self.root)}")

    def test_root_deploy_is_a_wrapper_for_the_one_deploy_script(self):
        source = (REPO / "deploy.sh").read_text()
        self.assertIn('/packaging/deploy.sh" "$@"', source)
        self.assertNotIn("cargo build", source)

    def test_packaging_deploy_preserves_old_live_artifacts_by_default(self):
        self.deploy_cleanup("packaging/deploy.sh")
        self.assert_preserved(self.old_artifacts + self.build_outputs + self.protected)

    def test_pruner_defaults_to_preview(self):
        self.run_script('bash "$HERE/prune-target.sh"')
        self.assert_preserved(self.old_artifacts + self.build_outputs + self.protected)

    def test_explicit_cleanup_preserves_build_outputs_and_deliverables(self):
        self.run_script('bash "$HERE/prune-target.sh" --apply')
        self.assertTrue(all(not path.exists() for path in self.old_artifacts))
        self.assert_preserved(self.build_outputs + self.protected)

    def test_dry_run_preserves_every_artifact(self):
        self.run_script('bash "$HERE/prune-target.sh" --dry-run')
        self.assert_preserved(self.old_artifacts + self.build_outputs + self.protected)

    def test_explicit_cleanup_skips_running_build(self):
        self.run_script('bash "$HERE/prune-target.sh" --apply', TEST_BUILD_RUNNING="1")
        self.assert_preserved(self.old_artifacts + self.build_outputs + self.protected)

    def test_prune_zero_override_preserves_cache(self):
        for path in ("packaging/deploy.sh",):
            with self.subTest(path=path):
                self.deploy_cleanup(path, PRUNE="0")
                self.assert_preserved(self.old_artifacts + self.build_outputs + self.protected)

    def test_prune_one_override_requests_explicit_cleanup(self):
        for path in ("packaging/deploy.sh",):
            with self.subTest(path=path):
                # Restore old dependencies before testing the second entrypoint.
                for artifact in self.old_artifacts:
                    self.write_at(artifact, time.time() - 7 * 86400)
                self.deploy_cleanup(path, PRUNE="1")
                self.assertTrue(all(not artifact.exists() for artifact in self.old_artifacts))
                self.assert_preserved(self.build_outputs + self.protected)


if __name__ == "__main__":
    unittest.main()
