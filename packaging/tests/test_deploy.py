"""Exercise finish verification with disposable files and mocked OS actions only."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import unittest

REPO = Path(__file__).resolve().parents[2]


class FinishVerification(unittest.TestCase):
    def run_finish(self, scenario, action="finish", previous_app=False, inspect=None, prepare=None):
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
            if previous_app:
                (installed / "Contents/MacOS").mkdir(parents=True)
                (installed / "Contents/MacOS/otto-desktop").write_text("previous desktop")
                (installed / "Contents/MacOS/ottod").write_text("previous daemon")
            if prepare:
                prepare(root)
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
pkill() { echo "UNEXPECTED pkill $*"; RUNNING=0; return 0; }
kill() { if [[ "$1" == -TERM ]]; then echo "KILLED $2"; RUNNING=0; fi; return 0; }
OPENS=0
SLEEPS=0
sleep() {
 SLEEPS=$((SLEEPS+1))
 if [[ "$SCENARIO" == delayed_daemon && "$SLEEPS" == 3 ]]; then
  command cp "$APP/Contents/MacOS/ottod" "$(deployed_daemon_path)"
 fi
 return 0
}
rm() { return 0; }
ditto() { command cp -R "$1" "$2"; }
open() {
 RUNNING=1
 OPENS=$((OPENS+1))
 # The launched app's supervisor installs ITS bundled sidecar.
 if [[ "$SCENARIO" != hash_mismatch && "$SCENARIO" != delayed_daemon ]]; then
  command cp "$INSTALLED_APP/Contents/MacOS/ottod" "$TEST_ROOT/Library/Application Support/Otto/bin/ottod"
 fi
 if [[ "$SCENARIO" == app_mismatch && -z "${MISMATCHED:-}" ]]; then MISMATCHED=1; echo wrong > "$INSTALLED_APP/Contents/MacOS/otto-desktop"; fi
}
launchctl() {
 if [[ "$RUNNING" == 0 || "$SCENARIO" == stale_pid ]]; then echo 'pid = 111'
 elif [[ "$OPENS" -ge 2 && "$SCENARIO" != *same_pid ]]; then echo 'pid = 444'
 else echo 'pid = 333'; fi
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
   [[ "$SCENARIO" != placeholder* ]] || { echo 'UI not embedded'; return; }
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
            if inspect:
                inspect(root, result)
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

    def test_failed_verify_rolls_back_to_the_previous_app(self):
        seen = {}

        def inspect(root, result):
            desktop = root / "installed/Otto.app/Contents/MacOS/otto-desktop"
            seen["installed"] = desktop.read_text() if desktop.exists() else None
            seen["leftovers"] = sorted(p.name for p in (root / "installed").iterdir())

        for scenario in ("unhealthy", "placeholder", "app_mismatch"):
            with self.subTest(scenario=scenario):
                result = self.run_finish(scenario, previous_app=True, inspect=inspect)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("ROLLED BACK", result.stdout + result.stderr)
                self.assertEqual(seen["installed"], "previous desktop")
                # The failed app is set aside (rm is mocked out here); no staging
                # copy and no orphaned .old sibling remain.
                self.assertNotIn(".Otto.app.old", " ".join(seen["leftovers"]))
                self.assertFalse(any(n.startswith(".Otto.app.staging") for n in seen["leftovers"]))

    def test_verified_install_keeps_the_previous_app(self):
        seen = {}

        def inspect(root, result):
            desktop = root / "installed/Otto.app/Contents/MacOS/otto-desktop"
            seen["installed"] = desktop.read_text()
            kept = root / "installed/.Otto.app.previous/Contents/MacOS/otto-desktop"
            seen["kept"] = kept.read_text() if kept.exists() else None

        result = self.run_finish("ok", previous_app=True, inspect=inspect)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("DEPLOY-RECEIPT", result.stdout)
        self.assertEqual(seen["installed"], "new desktop")
        self.assertEqual(seen["kept"], "previous desktop")

    def test_bad_staged_signature_never_touches_the_installed_app(self):
        seen = {}

        def inspect(root, result):
            seen["installed"] = (root / "installed/Otto.app/Contents/MacOS/otto-desktop").read_text()

        result = self.run_finish("invalid_signature", previous_app=True, inspect=inspect)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(seen["installed"], "previous desktop")
        self.assertNotIn("ROLLBACK", result.stdout)

    def test_rejects_false_success(self):
        for scenario in ("hash_mismatch", "app_mismatch", "stale_pid", "wrong_app_path", "wrong_daemon_path", "invalid_signature", "unhealthy", "failed_ui_fetch", "placeholder"):
            with self.subTest(scenario=scenario):
                result = self.run_finish(scenario)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn("DEPLOY-RECEIPT", result.stdout)
                self.assertNotIn("DEPLOY-FINISH EXIT=0", result.stdout)


    def test_rollback_verifies_the_previous_daemon_took_over(self):
        result = self.run_finish("placeholder", previous_app=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("previous daemon verified (pid 444", result.stdout)

    def test_rollback_never_reports_success_while_the_failed_daemon_still_runs(self):
        # S10-01: health alone passed against the failed build's daemon
        # before the restored supervisor booted it out.
        result = self.run_finish("placeholder_same_pid", previous_app=True)
        out = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, out)
        self.assertIn("previous daemon was NOT verified", out)
        self.assertNotIn("previous daemon verified", out)

    def test_killed_mid_swap_restores_the_aside_app_first(self):
        # S10-09: the finish job died between the two renames — no Otto.app,
        # the previous app at .Otto.app.old.<pid>.
        seen = {}

        def prepare(root):
            old = root / "installed/.Otto.app.old.4242/Contents/MacOS"
            old.mkdir(parents=True)
            (old / "otto-desktop").write_text("previous desktop")
            (old / "ottod").write_text("previous daemon")

        def inspect(root, result):
            seen["installed"] = (root / "installed/Otto.app/Contents/MacOS/otto-desktop").read_text()
            kept = root / "installed/.Otto.app.previous/Contents/MacOS/otto-desktop"
            seen["kept"] = kept.read_text() if kept.exists() else None
            seen["old"] = (root / "installed/.Otto.app.old.4242").exists()

        result = self.run_finish("ok", prepare=prepare, inspect=inspect)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("killed mid-swap", result.stdout)
        self.assertEqual(seen["installed"], "new desktop")
        # It became this run's rollback target, then the kept previous app.
        self.assertEqual(seen["kept"], "previous desktop")
        self.assertFalse(seen["old"])

    def test_several_aside_copies_and_no_app_refuses_to_guess(self):
        def prepare(root):
            for pid in ("1", "2"):
                (root / f"installed/.Otto.app.old.{pid}/Contents/MacOS").mkdir(parents=True)

        result = self.run_finish("ok", prepare=prepare)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("several .Otto.app.old", result.stderr)

    def test_quit_only_signals_the_installed_app(self):
        # S10-10: a `tauri dev` otto-desktop at another path is never killed.
        result = self.run_finish("ok", previous_app=True)
        self.assertNotIn("UNEXPECTED pkill", result.stdout)
        result = self.run_finish("wrong_app_path", previous_app=True)
        self.assertNotIn("KILLED", result.stdout)
        self.assertNotIn("UNEXPECTED pkill", result.stdout)


def functions_script():
    return (REPO / "packaging/deploy.sh").read_text().split("\nLOG_DIR=")[0]


def run_snippet(body, env=None, root=None):
    """Source deploy.sh's helpers and run `body` with REAL commands (tests
    only use ones that touch the temp dir)."""
    with tempfile.TemporaryDirectory(prefix="otto-deploy-fn-") as directory:
        base = Path(root or directory)
        (base / "functions.sh").write_text(functions_script())
        return subprocess.run(
            ["/bin/bash", "-c", 'source "$FN"\n' + body],
            env={**os.environ, "FN": str(base / "functions.sh"), "T": str(base), **(env or {})},
            capture_output=True, text=True,
        )


class DeployLock(unittest.TestCase):
    """S10-12: pid + start-time lock, stale sweep, handoff, concurrency."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory(prefix="otto-deploy-lock-")
        self.root = Path(self.dir.name)
        self.lock = self.root / "deploy.lock"

    def tearDown(self):
        self.dir.cleanup()

    def acquire(self, extra_env=None, body='lock_acquire && echo ACQUIRED && cat "$LOCK_DIR/pid"'):
        return run_snippet('LOCK_DIR="$T/deploy.lock"\n' + body, env=extra_env, root=self.root)

    def write_lock(self, content, age=60):
        self.lock.mkdir(exist_ok=True)
        (self.lock / "pid").write_text(content)
        past = time.time() - age
        os.utime(self.lock, (past, past))

    def test_fresh_lock_records_pid_and_start_time(self):
        result = self.acquire()
        self.assertIn("ACQUIRED", result.stdout, result.stderr)
        pid, start = result.stdout.split("ACQUIRED\n")[1].strip().split(" ", 1)
        self.assertTrue(pid.isdigit())
        self.assertRegex(start, r"\d{4}$")  # `ps -o lstart=` ends with the year

    def test_live_holder_refuses(self):
        holder = subprocess.Popen(["/bin/sleep", "30"])
        try:
            start = subprocess.run(["ps", "-p", str(holder.pid), "-o", "lstart="],
                                   capture_output=True, text=True).stdout.strip()
            self.write_lock(f"{holder.pid} {start}\n")
            result = self.acquire()
            self.assertNotIn("ACQUIRED", result.stdout)
            self.assertIn("another deploy is running", result.stderr)
        finally:
            holder.kill()
            holder.wait()

    def test_dead_holder_is_swept(self):
        dead = subprocess.Popen(["/usr/bin/true"])
        dead.wait()
        self.write_lock(f"{dead.pid} Thu Jan  1 00:00:00 2026\n")
        result = self.acquire()
        self.assertIn("ACQUIRED", result.stdout, result.stderr)
        self.assertIn("sweeping stale deploy lock", result.stdout)

    def test_reused_pid_is_stale(self):
        # Alive pid (this test process), but the recorded start time is not its own.
        self.write_lock(f"{os.getpid()} Thu Jan  1 00:00:00 2026\n")
        result = self.acquire()
        self.assertIn("ACQUIRED", result.stdout, result.stderr)

    def test_handoff_takes_over_a_live_lock(self):
        self.write_lock(f"{os.getpid()} \n")
        result = self.acquire({"OTTO_DEPLOY_LOCK_HANDOFF": str(os.getpid())})
        self.assertIn("ACQUIRED", result.stdout, result.stderr)
        self.assertNotIn("sweeping", result.stdout)

    def test_lock_being_created_is_not_stolen(self):
        # mkdir done, pid not written yet: young and empty → held.
        self.lock.mkdir()
        result = self.acquire()
        self.assertNotIn("ACQUIRED", result.stdout)

    def test_two_deploys_racing_a_stale_lock_never_both_win(self):
        for _ in range(5):
            dead = subprocess.Popen(["/usr/bin/true"])
            dead.wait()
            if self.lock.exists():
                subprocess.run(["rm", "-rf", str(self.lock)])
            self.write_lock(f"{dead.pid} Thu Jan  1 00:00:00 2026\n")
            (self.root / "functions.sh").write_text(functions_script())
            body = 'source "$FN"\nLOCK_DIR="$T/deploy.lock"\nlock_acquire && echo ACQUIRED && sleep 1'
            env = {**os.environ, "FN": str(self.root / "functions.sh"), "T": str(self.root)}
            procs = [subprocess.Popen(["/bin/bash", "-c", body], env=env, stdout=subprocess.PIPE,
                                      stderr=subprocess.PIPE, text=True) for _ in range(2)]
            outs = [p.communicate()[0] for p in procs]
            self.assertLessEqual(sum("ACQUIRED" in o for o in outs), 1, outs)

    def test_release_only_removes_our_own_lock(self):
        result = self.acquire(body='lock_acquire && lock_release && [[ ! -d "$LOCK_DIR" ]] && echo RELEASED')
        self.assertIn("RELEASED", result.stdout, result.stderr)


class LiveSessionCounts(unittest.TestCase):
    """S10-04: shells survive (kind=agent, provider=shell) — connections and
    background agents are what a deploy terminates."""

    def classify(self, rows):
        return run_snippet("classify_live_sessions <<'JSON'\n" + json.dumps(rows) + "\nJSON")

    def test_classifies_against_real_session_shapes(self):
        rows = [
            {"kind": "agent", "provider": "shell", "status": "running", "meta": {}},
            {"kind": "agent", "provider": "claude", "status": "idle", "meta": {"work": {"origin": "manual"}}},
            {"kind": "connection", "provider": "ssh", "status": "running", "meta": {}},
            {"kind": "agent", "provider": "claude", "status": "working", "meta": {"source": "workflow"}},
            {"kind": "agent", "provider": "codex", "status": "running", "meta": {"work": {"origin": "swarm"}}},
            {"kind": "agent", "provider": "claude", "status": "exited", "meta": {}},
        ]
        result = self.classify(rows)
        self.assertEqual(result.stdout.split(), ["5", "2", "1", "2"], result.stderr)

    def test_background_list_matches_otto_core(self):
        domain = (REPO / "crates/otto-core/src/domain.rs").read_text()
        body = domain[domain.index("pub const BACKGROUND_SESSION_SOURCES"):]
        body = body[body.index("= [") + 3:body.index("];")]
        core = set(re.findall(r'"([^"]+)"', body))
        script = (REPO / "packaging/deploy.sh").read_text()
        deploy = set(re.search(r'^BACKGROUND_SOURCES="([^"]*)"', script, re.M).group(1).split())
        self.assertEqual(deploy, core)


class Hygiene(unittest.TestCase):
    def test_prune_keeps_newest_of_each_kind_including_foreground(self):
        with tempfile.TemporaryDirectory(prefix="otto-deploy-logs-") as directory:
            logs = Path(directory)
            for kind in ("deploy-finish-", "deploy-foreground-", "deploy-"):
                for i in range(4):
                    f = logs / f"{kind}2026010{i}-000000.log"
                    f.write_text("x")
                    os.utime(f, (1_000_000 + i, 1_000_000 + i))
            run_snippet(f'KEEP_DEPLOY_LOGS=2 prune_deploy_logs "{logs}"')
            left = sorted(p.name for p in logs.iterdir())
            self.assertEqual(sum(n.startswith("deploy-foreground-") for n in left), 2, left)
            self.assertEqual(sum(n.startswith("deploy-finish-") for n in left), 2, left)

    def test_heal_never_resigns_the_receipted_build(self):
        # S10-11: a sidecar failing verification is not installed and sign.sh
        # (which would change $APP after the receipt) never runs.
        with tempfile.TemporaryDirectory(prefix="otto-deploy-heal-") as directory:
            root = Path(directory)
            (root / "side").write_text("sidecar")
            (root / "dep").write_text("deployed")
            result = run_snippet(
                'codesign() { return 1; }\nlaunchctl() { echo "LAUNCHCTL $*"; }\n'
                'HERE=/nonexistent\nside="$T/side"\ndep="$T/dep"\n'
                'heal_codesign_inode || echo "RC=$?"',
                root=root,
            )
            self.assertIn("RC=1", result.stdout, result.stderr)
            self.assertEqual((root / "dep").read_text(), "deployed")
            self.assertFalse((root / "dep.fresh").exists())
            self.assertNotIn("LAUNCHCTL", result.stdout)


if __name__ == "__main__":
    unittest.main()
