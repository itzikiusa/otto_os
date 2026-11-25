//! The install step: hand the staged bundle to a detached launchd one-shot
//! (`finish.sh`) that swaps it in after the app quits, verifies it and
//! rolls back on failure; and, at the next launch, finish whatever such a
//! run left behind (D-015).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

pub const LABEL: &str = "com.otto.update-finish";
const SCRIPT: &str = include_str!("finish.sh");
const LOGS_KEPT: usize = 5;

/// What the finisher (or [`recover`]) recorded about the last install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastResult {
    pub ok: bool,
    /// The version itself failed (auto-skip it); false when nothing was tried.
    #[serde(default)]
    pub skip: bool,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub reason: String,
    /// The finisher's log file.
    #[serde(default)]
    pub log: String,
}

/// One install run.
pub struct Job<'a> {
    pub updates: &'a Path,
    pub app: &'a Path,
    pub slot: &'a Path,
    pub previous: &'a Path,
    pub from: &'a str,
    pub to: &'a str,
    pub app_pid: u32,
    pub meta_url: &'a str,
}

fn result_path(updates: &Path) -> PathBuf {
    updates.join("last-result.json")
}
fn journal_path(updates: &Path) -> PathBuf {
    updates.join("finish.state")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The one-shot agent: RunAtLoad, no KeepAlive, a pinned PATH, everything
/// the script needs in its environment.
pub fn plist(job: &Job, script: &Path, log: &Path) -> String {
    let env = [
        ("OTTO_APP", job.app.to_string_lossy().into_owned()),
        ("OTTO_SLOT", job.slot.to_string_lossy().into_owned()),
        ("OTTO_PREVIOUS", job.previous.to_string_lossy().into_owned()),
        ("OTTO_UPDATES", job.updates.to_string_lossy().into_owned()),
        ("OTTO_FROM", job.from.to_string()),
        ("OTTO_TO", job.to.to_string()),
        ("OTTO_APP_PID", job.app_pid.to_string()),
        ("OTTO_META_URL", job.meta_url.to_string()),
        // The real app: LaunchServices opens it, and its executable is what
        // the finisher waits for. Tests run the script with stubs for both.
        ("OTTO_OPEN", "/usr/bin/open".to_string()),
        (
            "OTTO_APP_EXE",
            job.app
                .join("Contents/MacOS/otto-desktop")
                .to_string_lossy()
                .into_owned(),
        ),
        ("OTTO_LOG", log.to_string_lossy().into_owned()),
        ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin".to_string()),
    ];
    let env: String = env
        .iter()
        .map(|(k, v)| format!("    <key>{k}</key><string>{}</string>\n", xml(v)))
        .collect();
    let (script, log) = (xml(&script.to_string_lossy()), xml(&log.to_string_lossy()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>/bin/sh</string><string>{script}</string></array>
  <key>EnvironmentVariables</key><dict>
{env}  </dict>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
</dict></plist>
"#
    )
}

fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    pid > 0 && unsafe { libc::kill(pid as i32, 0) } == 0
}

/// The journal: (finisher pid, phase, from, to).
/// The journal: (finisher pid, phase, from, to, log path — may hold spaces).
fn read_journal(updates: &Path) -> Option<(u32, String, String, String, String)> {
    let text = fs::read_to_string(journal_path(updates)).ok()?;
    let mut it = text.trim_end_matches('\n').splitn(5, ' ');
    Some((
        it.next()?.parse().ok()?,
        it.next()?.into(),
        it.next()?.into(),
        it.next()?.into(),
        it.next().unwrap_or("").into(),
    ))
}

/// Whether a finisher is running right now.
pub fn in_flight(updates: &Path) -> bool {
    read_journal(updates).is_some_and(|(pid, ..)| pid_alive(pid))
}

/// Write the script and the agent, then bootstrap it. The caller quits the
/// app right after; the script waits for `job.app_pid` to exit.
pub fn launch(job: &Job) -> Result<PathBuf, String> {
    if in_flight(job.updates) {
        return Err("an update is already being installed".into());
    }
    let script = job.updates.join("finish.sh");
    let plist_path = job.updates.join(format!("{LABEL}.plist"));
    let stamp = chrono_stamp();
    let log = job.updates.join(format!("finish-{stamp}.log"));
    let write = |p: &Path, s: &str, mode: u32| -> std::io::Result<()> {
        fs::write(p, s)?;
        fs::set_permissions(p, fs::Permissions::from_mode(mode))
    };
    write(&script, SCRIPT, 0o700)
        .and_then(|_| write(&plist_path, &plist(job, &script, &log), 0o600))
        .map_err(|e| format!("could not write the installer: {e}"))?;
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    // Never boot out a finisher that is running (its journal may not exist yet).
    let running = Command::new("/bin/launchctl")
        .args(["print", &format!("{domain}/{LABEL}")])
        .output();
    if running.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("state = running")) {
        return Err("an update is already being installed".into());
    }
    // A previous run leaves the exited job registered; bootstrap would then
    // refuse and RunAtLoad never fire.
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &format!("{domain}/{LABEL}")])
        .output();
    let out = Command::new("/bin/launchctl")
        .args(["bootstrap", &domain])
        .arg(&plist_path)
        .output()
        .map_err(|e| format!("could not run launchctl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(log)
}

/// `YYYYmmdd-HHMMSS` (UTC) without a date crate.
fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

fn write_result(updates: &Path, r: &LastResult) {
    let path = result_path(updates);
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec(r).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

pub fn read_result(updates: &Path) -> Option<LastResult> {
    serde_json::from_slice(&fs::read(result_path(updates)).ok()?).ok()
}

pub fn clear_result(updates: &Path) {
    let _ = fs::remove_file(result_path(updates));
}

/// At launch: finish what a finisher killed mid-run left behind. The swap is
/// atomic, so the app path always holds a whole app; the journal and the
/// version THIS app runs say which one. The other copy (old or failed) is in
/// the slot unless it was already moved to `previous` — never lose the
/// rollback copy. A live finisher is left alone (it opened THIS app and is
/// verifying it). Returns whether one is in flight.
pub fn recover(updates: &Path, slot: &Path, previous: &Path, running_version: &str) -> bool {
    let Some((pid, phase, from, to, log)) = read_journal(updates) else {
        return false;
    };
    if pid_alive(pid) {
        return true;
    }
    let done = read_result(updates).is_some_and(|r| r.to == to);
    let new_runs = running_version == to;
    let old_runs = running_version == from;
    let mut r = LastResult {
        ok: false,
        skip: false,
        from,
        to,
        reason: String::new(),
        log,
    };
    match phase.as_str() {
        // The new version runs, so the swap happened (even if the journal
        // still said "waiting"): keep it; the slot holds the old app.
        "waiting" | "swapped" if new_runs => {
            if slot.exists() {
                let _ = fs::remove_dir_all(previous);
                if let Some(p) = previous.parent() {
                    let _ = fs::create_dir_all(p);
                }
                let _ = fs::rename(slot, previous);
            }
            r.ok = true;
            r.reason = format!("Updated to {}.", r.to);
        }
        // Killed before the swap: the old app is still installed.
        "waiting" => {
            let _ = fs::remove_dir_all(slot);
            r.reason =
                "The update was interrupted before Otto was replaced. Nothing changed.".into();
        }
        // Swapped back (or the swap-back finished before the journal said
        // so): the old app runs, the slot holds the failed one.
        "rolled_back" | "swapped" if old_runs || phase == "rolled_back" => {
            let _ = fs::remove_dir_all(slot);
            r.skip = true;
            r.reason = format!("The update to {} failed and was undone.", r.to);
        }
        _ => {
            r.reason = format!(
                "The update to {} was interrupted while it was being checked.",
                r.to
            );
        }
    }
    if !done {
        write_result(updates, &r);
    }
    let _ = fs::remove_file(journal_path(updates));
    false
}

/// Keep the newest [`LOGS_KEPT`] finisher logs.
pub fn prune_logs(updates: &Path) {
    let Ok(it) = fs::read_dir(updates) else {
        return;
    };
    let mut logs: Vec<PathBuf> = it
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("finish-") && n.ends_with(".log"))
        })
        .collect();
    logs.sort();
    for old in logs.iter().rev().skip(LOGS_KEPT) {
        let _ = fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::stage::bundle_version;
    use crate::updater::stage::BUNDLE;
    use crate::updater::testkit::{temp_dir, waiter, Kit, Server};

    #[test]
    fn the_plist_is_a_one_shot_that_carries_the_job() {
        let job = Job {
            updates: Path::new("/u/Otto & co/updates"),
            app: Path::new("/Applications/X.app"),
            slot: Path::new("/u/s/X.app"),
            previous: Path::new("/u/p/X.app"),
            from: "0.5.4",
            to: "0.5.5",
            app_pid: 42,
            meta_url: "http://127.0.0.1:7700/api/v1/meta",
        };
        let p = plist(
            &job,
            Path::new("/u/finish.sh"),
            Path::new("/u/finish-1.log"),
        );
        assert!(p.contains("<key>Label</key><string>com.otto.update-finish</string>"));
        assert!(
            p.contains("<key>RunAtLoad</key><true/>") && p.contains("<key>KeepAlive</key><false/>")
        );
        assert!(p.contains("<key>OTTO_APP_PID</key><string>42</string>"));
        assert!(
            p.contains("<string>/u/Otto &amp; co/updates</string>"),
            "escaped: {p}"
        );
        assert!(
            p.contains("<key>OTTO_OPEN</key><string>/usr/bin/open</string>"),
            "only the real job opens through LaunchServices"
        );
        assert!(p.contains(
            "<key>OTTO_APP_EXE</key><string>/Applications/X.app/Contents/MacOS/otto-desktop</string>"
        ));
        assert!(chrono_stamp().len() == 15);
    }

    /// One finisher run against throwaway INERT bundles (no executable, not
    /// Otto's name or id — testkit::Kit::plain_bundle): `APP` (installed,
    /// FROM) and `SLOT` (staged, TO) in a temp dir. Nothing goes through
    /// LaunchServices or launchd: the script runs under /bin/sh directly,
    /// `OTTO_OPEN` is a stub that execs a 60 s waiter OUTSIDE any bundle
    /// (`OTTO_APP_EXE`), and a fake daemon answers /api/v1/meta with
    /// `daemon_version`. (osascript runs only the in-process JXA
    /// `renamex_np` swap of the two temp dirs.)
    struct Run {
        dir: PathBuf,
        app: PathBuf,
        exe: PathBuf,
        slot: PathBuf,
        prev: PathBuf,
        updates: PathBuf,
        _srv: Server,
        meta: String,
    }

    impl Run {
        fn new(tag: &str, daemon_version: &str, open_starts_app: bool) -> Run {
            let body = format!(r#"{{"version":"{daemon_version}","x":1}}"#);
            Self::with_meta(tag, &body, open_starts_app)
        }

        fn with_meta(tag: &str, meta_body: &str, open_starts_app: bool) -> Run {
            let dir = temp_dir(&format!("finish-{tag}"));
            let app = Kit::plain_bundle(&dir.join("installed"), "1.0.0", false, false);
            let slot = Kit::plain_bundle(&dir.join("updates/staging"), "1.0.1", false, false);
            let updates = dir.join("updates");
            let prev = updates.join("previous").join(BUNDLE);
            // ≤ 15 chars: pgrep -x matches the kernel's 16-byte process name.
            let exe = dir.join("bin/otto-upd-fake");
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::copy(waiter(), &exe).unwrap();
            let srv = Server::start(vec![(
                "/api/v1/meta",
                200,
                meta_body.as_bytes().to_vec(),
                None,
            )]);
            let meta = srv.url("/api/v1/meta").to_string();
            let opener = dir.join("open.sh");
            let start = if open_starts_app {
                format!("'{}' </dev/null >/dev/null 2>&1 &\n", exe.display())
            } else {
                String::new()
            };
            // Each open is logged with whether the outcome was already
            // recorded at that moment (C1-012: always before a reopen).
            let body = format!("#!/bin/sh\n{start}d=\"$(dirname \"$0\")\"\nif [ -f \"$d/updates/last-result.json\" ]; then r=result; else r=none; fi\necho \"$1 $r\" >> \"$d/opened\"\n");
            fs::write(&opener, body).unwrap();
            fs::set_permissions(&opener, fs::Permissions::from_mode(0o700)).unwrap();
            Run {
                dir,
                app,
                exe,
                slot,
                prev,
                updates,
                _srv: srv,
                meta,
            }
        }

        /// Run the script with `old_app_pid`; returns its exit code.
        fn finish(&self, old_app_pid: u32, verify_secs: u32) -> i32 {
            self.finish_with(old_app_pid, verify_secs, &[])
        }

        fn finish_with(&self, old_app_pid: u32, verify_secs: u32, extra: &[(&str, &str)]) -> i32 {
            let script = self.updates.join("finish.sh");
            fs::create_dir_all(&self.updates).unwrap();
            fs::write(&script, SCRIPT).unwrap();
            let out = Command::new("/bin/sh")
                .arg(&script)
                .env_clear()
                .envs(extra.iter().copied())
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .env("OTTO_APP", &self.app)
                .env("OTTO_SLOT", &self.slot)
                .env("OTTO_PREVIOUS", &self.prev)
                .env("OTTO_UPDATES", &self.updates)
                .env("OTTO_FROM", "1.0.0")
                .env("OTTO_TO", "1.0.1")
                .env("OTTO_APP_PID", old_app_pid.to_string())
                .env("OTTO_META_URL", &self.meta)
                .env("OTTO_OPEN", self.dir.join("open.sh"))
                .env("OTTO_APP_EXE", &self.exe)
                .env("OTTO_QUIT_SECS", "2")
                .env("OTTO_VERIFY_SECS", verify_secs.to_string())
                .env("OTTO_LOG", self.dir.join("finish.log"))
                .output()
                .unwrap();
            eprintln!("{}", String::from_utf8_lossy(&out.stdout));
            out.status.code().unwrap_or(-1)
        }

        fn result(&self) -> LastResult {
            read_result(&self.updates).expect("a result was recorded")
        }

        fn new_app_running(&self) -> bool {
            let ps = Command::new("/bin/ps")
                .args(["-axo", "comm="])
                .output()
                .unwrap();
            String::from_utf8_lossy(&ps.stdout)
                .lines()
                .any(|l| l.trim() == self.exe.to_string_lossy())
        }
    }

    impl Drop for Run {
        fn drop(&mut self) {
            let _ = Command::new("/usr/bin/pkill")
                .arg("-f")
                .arg(&self.exe)
                .output();
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// The old app: exits by itself after 1 s. Not our child (a child that
    /// exited stays a zombie until reaped, and `kill -0` still finds it).
    fn quitting_app() -> u32 {
        let out = Command::new("/bin/sh")
            .args(["-c", "sleep 1 </dev/null >/dev/null 2>&1 & echo $!"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    #[test]
    fn a_healthy_update_is_kept_and_the_old_app_becomes_the_rollback_copy() {
        let run = Run::new("ok", "1.0.1", true);
        let old = quitting_app();
        assert_eq!(run.finish(old, 20), 0);
        assert_eq!(bundle_version(&run.app).unwrap(), "1.0.1");
        assert_eq!(
            bundle_version(&run.prev).unwrap(),
            "1.0.0",
            "one previous copy kept"
        );
        assert!(!run.slot.exists());
        let r = run.result();
        assert!(
            r.ok && !r.skip && r.from == "1.0.0" && r.to == "1.0.1",
            "{r:?}"
        );
        assert!(
            !journal_path(&run.updates).exists(),
            "journal ends with the result"
        );
        assert!(run.new_app_running());
    }

    #[test]
    fn a_daemon_that_never_reports_the_new_version_rolls_back() {
        let run = Run::new("stale", "1.0.0", true);
        let old = quitting_app();
        assert_eq!(run.finish(old, 4), 1);
        assert_eq!(
            bundle_version(&run.app).unwrap(),
            "1.0.0",
            "old app restored"
        );
        assert!(!run.slot.exists(), "the failed copy is removed");
        assert!(!run.prev.exists());
        let r = run.result();
        assert!(!r.ok && r.skip, "{r:?}");
        assert!(
            r.reason.contains("did not report version 1.0.1") && r.reason.contains("undone"),
            "{}",
            r.reason
        );
        let opened = fs::read_to_string(run.dir.join("opened")).unwrap();
        assert_eq!(
            opened.lines().count(),
            2,
            "new app opened, then the old one: {opened}"
        );
    }

    /// C1-012: the reopened old app finds the outcome already recorded (so it
    /// skips the failed version); C1-027: the reason survives quotes,
    /// backslashes and a newline as real JSON.
    #[test]
    fn a_rollback_records_its_outcome_as_json_before_reopening() {
        let run = Run::with_meta("json", r#"{"version":"1.0.0 \"beta\" \\ x\ny"}"#, true);
        assert_eq!(run.finish(quitting_app(), 3), 1);
        let opened = fs::read_to_string(run.dir.join("opened")).unwrap();
        let lines: Vec<_> = opened.lines().collect();
        assert_eq!(lines.len(), 2, "{opened}");
        assert!(
            lines[0].ends_with(" none") && lines[1].ends_with(" result"),
            "{opened}"
        );
        let r = run.result();
        assert!(!r.ok && r.skip, "{r:?}");
        assert!(
            r.reason.contains(r#"1.0.0 "beta" \ x"#) && r.reason.contains('\n'),
            "{}",
            r.reason
        );
    }

    /// C1-027: `version` is read as JSON, not by the last "version" in the
    /// text (a tool's version after it used to win).
    #[test]
    fn the_daemon_version_is_parsed_not_grepped() {
        let body = r#"{"version":"1.0.1","tools":[{"name":"git","version":"2.47.0"}]}"#;
        let run = Run::with_meta("tools", body, true);
        assert_eq!(run.finish(quitting_app(), 10), 0);
        assert!(run.result().ok);
    }

    /// C1-011: osascript warnings on stderr (here: the ObjC runtime's help
    /// text) never turn a swap that happened into "Nothing changed".
    #[test]
    fn stderr_noise_from_osascript_does_not_fail_a_swap() {
        let run = Run::new("noise", "1.0.1", true);
        assert_eq!(
            run.finish_with(quitting_app(), 10, &[("OBJC_HELP", "1")]),
            0
        );
        assert_eq!(bundle_version(&run.app).unwrap(), "1.0.1");
        assert!(run.result().ok);
    }

    /// C1-026: when the swap itself fails, the old app is reopened.
    #[test]
    fn a_failed_swap_reopens_the_old_app() {
        let run = Run::new("noswap", "1.0.1", true);
        fs::remove_dir_all(&run.slot).unwrap();
        assert_eq!(run.finish(quitting_app(), 3), 1);
        assert_eq!(bundle_version(&run.app).unwrap(), "1.0.0");
        let opened = fs::read_to_string(run.dir.join("opened")).unwrap();
        assert!(
            opened.starts_with(&run.app.to_string_lossy().into_owned()),
            "{opened}"
        );
        assert!(run.result().reason.contains("Nothing changed"));
    }

    /// The finisher as a state machine: killed at every crash point (power
    /// loss is the same), then the next launch — the app at APP — recovers.
    /// Always: APP is a whole app, the rollback copy of the OLD app is never
    /// lost while the new one is installed, the journal is gone and the
    /// outcome is recorded; a failed version is skipped.
    #[test]
    fn every_crash_point_recovers_without_losing_an_app() {
        let points = [
            ("after_swap", "1.0.1"),
            ("after_journal", "1.0.1"),
            ("after_open", "1.0.1"),
            ("after_keep", "1.0.1"),
            ("before_swap_back", "1.0.0"),
            ("after_swap_back", "1.0.0"),
            ("after_rolled_back", "1.0.0"),
        ];
        for (point, daemon) in points {
            let run = Run::new(&format!("crash-{point}"), daemon, true);
            let code = run.finish_with(quitting_app(), 2, &[("OTTO_FINISH_DIE_AT", point)]);
            assert_ne!(code, 0, "{point}: killed");
            let installed = bundle_version(&run.app).unwrap();
            assert!(
                installed == "1.0.0" || installed == "1.0.1",
                "{point}: {installed}"
            );
            assert!(
                !recover(&run.updates, &run.slot, &run.prev, &installed),
                "{point}"
            );
            assert!(!journal_path(&run.updates).exists(), "{point}");
            let r = run.result();
            if installed == "1.0.1" {
                assert!(r.ok, "{point}: {r:?}");
                assert_eq!(
                    bundle_version(&run.prev).unwrap(),
                    "1.0.0",
                    "{point}: rollback copy kept"
                );
            } else {
                assert!(!r.ok && r.skip, "{point}: {r:?}");
                assert!(!run.slot.exists(), "{point}: failed copy removed");
            }
            assert_eq!(
                r.log,
                run.dir.join("finish.log").to_string_lossy(),
                "{point}: log kept"
            );
        }
    }
    #[test]
    fn a_new_app_that_never_starts_rolls_back() {
        let run = Run::new("crash", "1.0.1", false);
        let old = quitting_app();
        assert_eq!(run.finish(old, 4), 1);
        assert_eq!(bundle_version(&run.app).unwrap(), "1.0.0");
        let r = run.result();
        assert!(
            !r.ok && r.skip && r.reason.contains("did not start"),
            "{r:?}"
        );
    }

    #[test]
    fn an_app_that_will_not_quit_leaves_everything_as_it_was() {
        let run = Run::new("stuck", "1.0.1", true);
        let mut stuck = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; while :; do sleep 0.2; done"])
            .spawn()
            .unwrap();
        assert_eq!(run.finish(stuck.id(), 4), 1);
        let _ = stuck.kill();
        let _ = stuck.wait();
        assert_eq!(bundle_version(&run.app).unwrap(), "1.0.0");
        assert_eq!(
            bundle_version(&run.slot).unwrap(),
            "1.0.1",
            "staged copy untouched"
        );
        let r = run.result();
        assert!(
            !r.ok && !r.skip && r.reason.contains("did not quit"),
            "{r:?}"
        );
        assert!(!run.dir.join("opened").exists(), "nothing was opened");
    }

    fn journal(updates: &Path, pid: u32, phase: &str) {
        fs::create_dir_all(updates).unwrap();
        fs::write(
            journal_path(updates),
            format!("{pid} {phase} 1.0.0 1.0.1\n"),
        )
        .unwrap();
    }

    fn dead_pid() -> u32 {
        let mut c = Command::new("/usr/bin/true").spawn().unwrap();
        let pid = c.id();
        c.wait().unwrap();
        pid
    }

    #[test]
    fn recovery_finishes_an_interrupted_run_by_its_phase() {
        let dir = temp_dir("recover");
        let updates = dir.join("updates");
        let slot = updates.join("staging").join(BUNDLE);
        let prev = updates.join("previous").join(BUNDLE);

        // A live finisher (it opened us): hands off.
        let mut live = Command::new("/bin/sleep").arg("5").spawn().unwrap();
        journal(&updates, live.id(), "swapped");
        assert!(recover(&updates, &slot, &prev, "1.0.1"));
        assert!(journal_path(&updates).exists());
        let _ = live.kill();
        let _ = live.wait();

        // Killed while verifying, and the new version is what runs: keep it.
        fs::create_dir_all(&slot).unwrap();
        journal(&updates, dead_pid(), "swapped");
        assert!(!recover(&updates, &slot, &prev, "1.0.1"));
        let r = read_result(&updates).unwrap();
        assert!(r.ok, "{r:?}");
        assert!(
            prev.is_dir() && !slot.exists(),
            "the old app became the rollback copy"
        );
        assert!(!journal_path(&updates).exists());
        clear_result(&updates);

        // Killed before the swap: nothing changed, the version is NOT skipped.
        fs::create_dir_all(&slot).unwrap();
        journal(&updates, dead_pid(), "waiting");
        recover(&updates, &slot, &prev, "1.0.0");
        let r = read_result(&updates).unwrap();
        assert!(
            !r.ok && !r.skip && r.reason.contains("Nothing changed"),
            "{r:?}"
        );
        assert!(!slot.exists());
        clear_result(&updates);

        // Killed after swapping back: failed, skipped.
        journal(&updates, dead_pid(), "rolled_back");
        recover(&updates, &slot, &prev, "1.0.0");
        let r = read_result(&updates).unwrap();
        assert!(!r.ok && r.skip, "{r:?}");

        // A result the finisher already wrote is kept as it is.
        journal(&updates, dead_pid(), "rolled_back");
        let before = read_result(&updates).unwrap();
        recover(&updates, &slot, &prev, "1.0.0");
        assert_eq!(read_result(&updates).unwrap(), before);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_newest_logs_are_kept() {
        let dir = temp_dir("logs");
        for i in 0..8 {
            fs::write(dir.join(format!("finish-2026010{i}-000000.log")), "x").unwrap();
        }
        fs::write(dir.join("other.log"), "x").unwrap();
        prune_logs(&dir);
        let mut left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left.len(), 6, "{left:?}");
        assert!(
            left.contains(&"other.log".to_string())
                && left.contains(&"finish-20260107-000000.log".to_string())
        );
        assert!(!left.contains(&"finish-20260102-000000.log".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }
}
