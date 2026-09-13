"""Exercise finish verification with disposable files and mocked OS actions only."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class FinishVerification(unittest.TestCase):
    def run_finish(self, scenario, action="finish"):
        with tempfile.TemporaryDirectory(prefix="otto-deploy-test-") as directory:
            root = Path(directory)
            app = root / "built/Otto.app/Contents/MacOS"
            app.mkdir(parents=True)
            (app / "ottod").write_text("new daemon")
            (app / "otto-desktop").write_text("new desktop")
            deployed = root / "Library/Application Support/Otto/bin/ottod"
            deployed.parent.mkdir(parents=True)
            deployed.write_text("old daemon")
            installed = root / "installed/Otto.app"
            installed.parent.mkdir()
            (root / "functions.sh").write_text((REPO / "packaging/deploy.sh").read_text().split("\nLOG_DIR=")[0])
            script = r'''
source "$TEST_ROOT/functions.sh"
ROOT="$REPO"
HERE="$REPO/packaging"
APP="$TEST_ROOT/built/Otto.app"
INSTALLED_APP="$TEST_ROOT/installed/Otto.app"
deployed_daemon_path() { echo "$TEST_ROOT/Library/Application Support/Otto/bin/ottod"; }
RUNNING=0
osascript() { return 0; }
pgrep() { if [[ "$RUNNING" == 1 ]]; then echo 222; else return 1; fi; }
pkill() { return 0; }
SLEEPS=0
sleep() {
 SLEEPS=$((SLEEPS+1))
 if [[ "$SCENARIO" == delayed_daemon && "$SLEEPS" == 3 ]]; then
  command cp "$APP/Contents/MacOS/ottod" "$(deployed_daemon_path)"
 fi
 return 0
}
rm() { return 0; }
ditto() { command cp -R "$APP" "$INSTALLED_APP"; }
open() {
 RUNNING=1
 if [[ "$SCENARIO" != hash_mismatch && "$SCENARIO" != delayed_daemon ]]; then
  command cp "$APP/Contents/MacOS/ottod" "$TEST_ROOT/Library/Application Support/Otto/bin/ottod"
 fi
 if [[ "$SCENARIO" == app_mismatch ]]; then echo wrong > "$INSTALLED_APP/Contents/MacOS/otto-desktop"; fi
}
launchctl() {
 if [[ "$RUNNING" == 0 || "$SCENARIO" == stale_pid ]]; then echo 'pid = 111'; else echo 'pid = 333'; fi
}
ps() {
 if [[ "$2" == 222 ]]; then
  [[ "$SCENARIO" != wrong_app_path ]] || { echo /tmp/otto-desktop; return; }
  echo "$INSTALLED_APP/Contents/MacOS/otto-desktop"
 else
  [[ "$SCENARIO" != wrong_daemon_path ]] || { echo /tmp/ottod; return; }
  echo "$TEST_ROOT/Library/Application Support/Otto/bin/ottod"
 fi
}
codesign() { [[ "$SCENARIO" != invalid_signature ]]; }
curl() {
 case "$*" in
  *api/v1/health*)
   [[ "$SCENARIO" != unhealthy ]] || { echo '{"ok":false}'; return; }
   echo '{"ok":true}' ;;
  *)
   [[ "$SCENARIO" != failed_ui_fetch ]] || return 22
   [[ "$SCENARIO" != placeholder ]] || { echo 'UI not embedded'; return; }
   echo '<!doctype html><title>Otto</title><script type="module" src="/assets/app.js"></script><div id="app"></div>' ;;
 esac
}
trap 'echo "DEPLOY-FINISH EXIT=$?"' EXIT
case "$TEST_ACTION" in
 finish) finish_phase ;;
 receipt)
  BUILD_RECEIPT="$TEST_ROOT/receipt"
  git() {
   case "$*" in
    *status*) [[ "$SCENARIO" != dirty_source ]] || echo ' M source.rs' ;;
    *rev-parse*) echo "$TEST_COMMIT" ;;
   esac
   return 0
  }
  TEST_COMMIT=abc123
  ORIGINAL_SCENARIO="$SCENARIO"
  SCENARIO=ok
  build_manifest > "$BUILD_RECEIPT"
  SCENARIO="$ORIGINAL_SCENARIO"
  case "$SCENARIO" in
   changed_commit) TEST_COMMIT=def456 ;;
   changed_artifact) echo changed > "$APP/Contents/MacOS/ottod" ;;
   changed_embed) EMBED_UI=0 ;;
   missing_receipt) BUILD_RECEIPT="$TEST_ROOT/missing" ;;
  esac
  validate_build_receipt ;;
 delay) FINISH_DELAY_SECONDS="$SCENARIO"; validate_delay ;;
esac
'''
            result = subprocess.run(
                ["/bin/bash", "-c", script],
                env={**os.environ, "TEST_ROOT": str(root), "REPO": str(REPO), "SCENARIO": scenario, "TEST_ACTION": action},
                capture_output=True, text=True,
            )
            return result

    def test_build_only_stops_before_any_install_action(self):
        with tempfile.TemporaryDirectory(prefix="otto-build-only-test-") as directory:
            root = Path(directory)
            (root / "packaging").mkdir()
            (root / "ui").mkdir()
            bundle = root / "apps/desktop/src-tauri/target/release/bundle/macos/Otto.app/Contents/MacOS"
            bundle.mkdir(parents=True)
            (bundle / "ottod").write_text("signed daemon")
            (bundle / "otto-desktop").write_text("signed desktop")
            (root / "target/release").mkdir(parents=True)
            (root / "target/release/ottod").write_text("signed daemon")
            (root / "packaging/deploy.sh").write_text((REPO / "packaging/deploy.sh").read_text())
            (root / "packaging/sign.sh").write_text("#!/bin/bash\nexit 0\n")
            mocks = root / "mocks"
            mocks.mkdir()
            commands = {
                "git": 'case "$*" in *rev-parse*) echo abc123 ;; esac',
                "rustc": 'echo "host: test-apple-darwin"',
                "cargo": ":", "npm": ":", "npx": ":", "codesign": ":",
                # Even a regressed branch cannot reach a real OS operation or
                # create the launchd/log directories under the actual HOME.
                "mkdir": 'for arg in "$@"; do case "$arg" in -*) ;; "$TEST_ROOT"/*) ;; *) exit 99 ;; esac; done; /bin/mkdir "$@"',
            }
            for command in ("launchctl", "open", "osascript", "pkill", "rm", "ditto", "ln", "curl"):
                commands[command] = "exit 99"
            for name, body in commands.items():
                path = mocks / name
                path.write_text("#!/bin/bash\n" + body + "\n")
                path.chmod(0o755)
            result = subprocess.run(
                ["/bin/bash", str(root / "packaging/deploy.sh")],
                env={**os.environ, "PATH": str(mocks) + ":/usr/bin:/bin", "TEST_ROOT": str(root),
                     "BUILD_ONLY": "1", "PRUNE": "0", "SKIP_UI": "0", "OTTO_DEPLOY_PHASE": "", "FINISH_DELAY_SECONDS": "0"},
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            receipt = root / "apps/desktop/src-tauri/target/release/deploy-build.receipt"
            self.assertIn("commit=abc123", receipt.read_text())
            self.assertNotIn("DEPLOY-QUEUED", result.stdout)
            self.assertNotIn("DEPLOY-RECEIPT", result.stdout)
            stale = subprocess.run(
                ["/bin/bash", str(root / "packaging/deploy.sh")],
                env={**os.environ, "PATH": str(mocks) + ":/usr/bin:/bin", "TEST_ROOT": str(root),
                     "BUILD_ONLY": "1", "PRUNE": "0", "SKIP_UI": "1", "OTTO_DEPLOY_PHASE": "", "FINISH_DELAY_SECONDS": "0"},
                capture_output=True, text=True,
            )
            self.assertNotEqual(stale.returncode, 0)
            self.assertIn("fresh UI build", stale.stderr)
            self.assertNotIn("BUILD-RECEIPT", stale.stdout)

    def test_receipt_binds_source_and_signed_artifacts(self):
        self.assertEqual(self.run_finish("ok", "receipt").returncode, 0)
        for scenario in ("dirty_source", "changed_commit", "changed_artifact", "changed_embed", "missing_receipt", "invalid_signature"):
            with self.subTest(scenario=scenario):
                result = self.run_finish(scenario, "receipt")
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_delay_is_bounded_and_decimal(self):
        for delay in ("0", "08", "15", "60"):
            self.assertEqual(self.run_finish(delay, "delay").returncode, 0, delay)
        for delay in ("61", "100000000000000000000", "-1", "abc", "1+1"):
            self.assertNotEqual(self.run_finish(delay, "delay").returncode, 0, delay)

    def test_waits_for_supervisor_to_replace_old_healthy_daemon(self):
        result = self.run_finish("delayed_daemon")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("DEPLOY-RECEIPT", result.stdout)

    def test_valid_install_emits_receipt(self):
        result = self.run_finish("ok")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("DEPLOY-RECEIPT", result.stdout)
        self.assertIn("DEPLOY-FINISH EXIT=0", result.stdout)

    def test_rejects_false_success(self):
        for scenario in ("hash_mismatch", "app_mismatch", "stale_pid", "wrong_app_path", "wrong_daemon_path", "invalid_signature", "unhealthy", "failed_ui_fetch", "placeholder"):
            with self.subTest(scenario=scenario):
                result = self.run_finish(scenario)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn("DEPLOY-RECEIPT", result.stdout)
                self.assertNotIn("DEPLOY-FINISH EXIT=0", result.stdout)


if __name__ == "__main__":
    unittest.main()
