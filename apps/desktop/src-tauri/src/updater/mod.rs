//! Updates for release builds: check the manifest (60 s after launch, every
//! 6 h, and on Otto ▸ Check for Updates…), download + minisign-verify (the
//! updater plugin's `restore_update` / `verify`, version bound), stage and
//! codesign-check the bundle, then — only when the person clicks Restart —
//! hand over to the detached finisher and quit. "Install updates
//! automatically" only downloads and stages; nothing restarts unattended.
//!
//! The SPA drives it through the `update_*` commands and listens to
//! [`EVENT`] (the whole [`UpdateState`] on every change). A source build
//! (deploy.sh, `tauri dev`, a plain cargo build) never fetches anything.

mod fetch;
mod finish;
mod stage;
#[cfg(test)]
mod testkit;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

pub use finish::LastResult;

pub const EVENT: &str = "otto://update";
pub const MENU_ID: &str = "check-updates";
/// The only place a release copy updates itself in.
const INSTALL_PATH: &str = "/Applications/Otto.app";
const META_URL: &str = "http://127.0.0.1:7700/api/v1/meta";
const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Staged,
    Restarting,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Release,
    #[default]
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// No connection to GitHub.
    Offline,
    /// This copy can't install (location / permissions) — see `blocked`.
    Blocked,
    /// Not enough free disk space.
    Disk,
    /// A signature, identity, version or archive check failed.
    Verify,
    Failed,
}

/// Everything the UI shows; sent whole with every [`EVENT`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateState {
    pub phase: Phase,
    pub channel: Channel,
    pub current: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    /// RFC 3339.
    pub date: Option<String>,
    /// Archive bytes, when the manifest or the download says.
    pub size: Option<u64>,
    pub received: u64,
    pub error: Option<String>,
    pub error_kind: Option<ErrorKind>,
    /// Why this copy can't install updates (None = it can).
    pub blocked: Option<String>,
    pub skipped: Option<String>,
    pub auto_install: bool,
    /// Unix seconds of the last finished check.
    pub last_check: Option<i64>,
    /// The last install's outcome, until the UI calls `update_result_seen`.
    pub last_result: Option<LastResult>,
    pub release_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Prefs {
    #[serde(default)]
    auto_install: bool,
    #[serde(default)]
    skipped: Option<String>,
    #[serde(default)]
    last_check: Option<i64>,
}

/// Where the updater keeps its files.
#[derive(Debug, Clone)]
struct Paths {
    /// `~/Library/Application Support/Otto`
    data: PathBuf,
    /// The installed bundle.
    app: PathBuf,
}

impl Paths {
    fn updates(&self) -> PathBuf {
        self.data.join("updates")
    }
    fn prefs(&self) -> PathBuf {
        self.data.join("updater.json")
    }
    fn slot(&self) -> PathBuf {
        stage::staged_app(&self.updates())
    }
    fn previous(&self) -> PathBuf {
        self.updates().join("previous").join(stage::BUNDLE)
    }
}

struct Inner {
    state: UpdateState,
    update: Option<Update>,
    prefs: Prefs,
    paths: Paths,
    signers: Vec<String>,
}

static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();

/// Release builds only: the release workflow compiles with
/// `OTTO_UPDATE_CHANNEL=release`; a debug build, or one built from the TEST
/// release identity, never counts.
pub fn channel() -> Channel {
    release_channel(
        cfg!(debug_assertions) || test_identity(RELEASE_IDENTITY),
        option_env!("OTTO_UPDATE_CHANNEL"),
    )
}

fn release_channel(not_release: bool, flag: Option<&str>) -> Channel {
    if !not_release && flag == Some("release") {
        Channel::Release
    } else {
        Channel::Source
    }
}

/// Why the bundle at `bundle` can't replace itself, if it can't.
fn install_blocker(bundle: &Path, install_path: &Path) -> Option<String> {
    if bundle != install_path {
        return Some(format!(
            "Move Otto to the Applications folder to get updates (it runs from {}).",
            bundle.display()
        ));
    }
    let writable = |p: &Path| {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::CString::new(p.as_os_str().as_bytes())
            // SAFETY: a valid NUL-terminated path.
            .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
    };
    let parent = install_path.parent().unwrap_or(Path::new("/"));
    if !writable(parent) || !writable(install_path) {
        return Some(format!(
            "This account can't replace apps in {}. Install the new version from the DMG, or ask an administrator.",
            parent.display()
        ));
    }
    None
}

/// `…/Otto.app` of the running executable (`…/Otto.app/Contents/MacOS/x`).
fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.parent()?.to_path_buf())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn load_prefs(path: &Path) -> Prefs {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_prefs(path: &Path, prefs: &Prefs) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(prefs).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// `plugins.updater` of tauri.conf.json: the plugin reads `pubkey` from it;
/// Otto fetches the first of its `endpoints`.
fn plugin_config<R: Runtime>(app: &AppHandle<R>) -> serde_json::Value {
    app.config()
        .plugins
        .0
        .get("updater")
        .cloned()
        .unwrap_or_default()
}

fn endpoint(cfg: &serde_json::Value) -> Option<reqwest::Url> {
    cfg.get("endpoints")?.get(0)?.as_str()?.parse().ok()
}

/// The accepted code-signing identities: `acceptedLeaves` (SHA-1 of each
/// leaf certificate) of the file packaging/release-artifacts.sh signs and
/// checks with, so the TEST identity is replaced in that one place. A later
/// identity (a new certificate, Developer ID) is added there next to the old
/// one for at least one release.
const RELEASE_IDENTITY: &str = include_str!("../../release-identity.json");

/// `"test": false` is the only way out of TEST (as release-manifest.py checks).
fn test_identity(identity: &str) -> bool {
    let v: serde_json::Value = serde_json::from_str(identity).unwrap_or_default();
    v.get("test").and_then(|t| t.as_bool()) != Some(false)
}

fn signers(identity: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(identity).unwrap_or_default();
    v.get("acceptedLeaves")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()))
                .map(|leaf| format!("certificate leaf = H\"{}\"", leaf.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default()
}

/// `https://github.com/<o>/<r>/releases/tag/v<version>` from the endpoint.
fn release_url(endpoint: &reqwest::Url, version: &str) -> Option<String> {
    let mut segs = endpoint.path_segments()?;
    let (owner, repo) = (segs.next()?, segs.next()?);
    (endpoint.host_str() == Some("github.com") && segs.next() == Some("releases"))
        .then(|| format!("https://github.com/{owner}/{repo}/releases/tag/v{version}"))
}

struct Fail(ErrorKind, String);

impl From<fetch::FetchError> for Fail {
    fn from(e: fetch::FetchError) -> Fail {
        match e {
            fetch::FetchError::Offline(m) => Fail(
                ErrorKind::Offline,
                format!("Otto couldn't reach GitHub ({m}). Check the connection and try again."),
            ),
            fetch::FetchError::Refused(m) => Fail(ErrorKind::Failed, m),
        }
    }
}

/// Fetch the manifest (allowed host, capped) and let the plugin parse it,
/// compare versions and pick this platform's entry. The download URL must be
/// an allowed host too, before anything is downloaded.
async fn find_update<R: Runtime>(
    app: &AppHandle<R>,
    endpoint: &reqwest::Url,
) -> Result<Option<Update>, Fail> {
    let bytes = fetch::get(endpoint, fetch::MANIFEST_MAX, |_, _| {}).await?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
        Fail(
            ErrorKind::Failed,
            format!("the update manifest is not valid JSON: {e}"),
        )
    })?;
    let update = app
        .updater_builder()
        .build()
        .and_then(|u| u.restore_update(json))
        .map_err(|e| {
            Fail(
                ErrorKind::Failed,
                format!("the update manifest was refused: {e}"),
            )
        })?;
    if let Some(u) = &update {
        if !fetch::allowed(&u.download_url) {
            return Err(Fail(
                ErrorKind::Verify,
                format!(
                    "the update points at {}, which is not an allowed download host",
                    u.download_url.host_str().unwrap_or("?")
                ),
            ));
        }
    }
    Ok(update)
}

/// Download (capped), verify the minisign signature and its signed version
/// (the plugin), then unpack and codesign-check the bundle.
async fn download_and_stage(
    update: &Update,
    updates: PathBuf,
    signers: Vec<String>,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, Fail> {
    let bytes = fetch::get(&update.download_url, fetch::ARCHIVE_MAX, progress).await?;
    update.verify(&bytes).map_err(|e| {
        Fail(
            ErrorKind::Verify,
            format!("the download's signature did not check out: {e}"),
        )
    })?;
    let version = update.version.clone();
    tauri::async_runtime::spawn_blocking(move || stage::stage(&bytes, &updates, &version, &signers))
        .await
        .map_err(|e| Fail(ErrorKind::Failed, e.to_string()))?
        .map_err(|e| Fail(ErrorKind::Verify, format!("the update was refused: {e}")))
}

fn with<T>(f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
    INNER
        .get()
        .map(|m| f(&mut m.lock().unwrap_or_else(|p| p.into_inner())))
}

fn publish(app: &AppHandle) {
    if let Some(state) = with(|i| i.state.clone()) {
        let _ = app.emit(EVENT, state);
    }
}

fn fail(app: &AppHandle, Fail(kind, msg): Fail) {
    crate::lifecycle::durable(&format!("update: {msg}"));
    with(|i| {
        i.state.phase = Phase::Error;
        i.state.error_kind = Some(kind);
        i.state.error = Some(msg);
    });
    publish(app);
}

/// Call once in `setup`: finish an interrupted install, apply its outcome
/// (a failed version is skipped), sweep stale staging, start the timers.
pub fn init(app: &AppHandle) {
    let data = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Application Support/Otto");
    let paths = Paths {
        data,
        app: PathBuf::from(INSTALL_PATH),
    };
    let current = app.package_info().version.to_string();
    let updates = paths.updates();
    let in_flight = finish::recover(&updates, &paths.slot(), &paths.previous(), &current);
    if !in_flight {
        // The slot holds the OLD app while a finisher verifies this one.
        let _ = std::fs::remove_dir_all(updates.join("staging"));
        finish::prune_logs(&updates);
    }
    let mut prefs = load_prefs(&paths.prefs());
    let last_result = finish::read_result(&updates);
    if let Some(r) = last_result.as_ref().filter(|r| !r.ok && r.skip) {
        prefs.skipped = Some(r.to.clone());
        save_prefs(&paths.prefs(), &prefs);
    }
    let channel = channel();
    let blocked = match running_bundle() {
        Some(b) => install_blocker(&b, &paths.app),
        None => Some("Otto could not tell where it is installed.".into()),
    };
    let state = UpdateState {
        channel,
        current,
        blocked,
        skipped: prefs.skipped.clone(),
        auto_install: prefs.auto_install,
        last_check: prefs.last_check,
        last_result,
        ..Default::default()
    };
    let _ = INNER.set(Mutex::new(Inner {
        state,
        update: None,
        prefs,
        paths,
        signers: signers(RELEASE_IDENTITY),
    }));
    if channel != Channel::Release {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // This app was opened by a finisher that is still verifying it:
        // pick its result up when it lands.
        if in_flight {
            for _ in 0..120 {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if !finish::in_flight(&updates) {
                    let r = finish::read_result(&updates);
                    with(|i| i.state.last_result = r);
                    publish(&handle);
                    break;
                }
            }
        }
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            check(&handle, false).await;
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// One check. Single-flight: a no-op while checking, downloading, staged or
/// restarting. An automatic check hides a skipped version and, with
/// "Install updates automatically", goes on to download and stage.
async fn check(app: &AppHandle, manual: bool) {
    let go = with(|i| {
        let busy = matches!(
            i.state.phase,
            Phase::Checking | Phase::Downloading | Phase::Staged | Phase::Restarting
        );
        if i.state.channel != Channel::Release || busy {
            return None;
        }
        i.state.phase = Phase::Checking;
        i.state.error = None;
        i.state.error_kind = None;
        Some(plugin_config(app))
    })
    .flatten();
    let Some(cfg) = go else { return };
    publish(app);
    let Some(url) = endpoint(&cfg) else {
        return fail(
            app,
            Fail(ErrorKind::Failed, "no update endpoint is configured".into()),
        );
    };
    let found = find_update(app, &url).await;
    let auto = with(|i| {
        i.prefs.last_check = Some(now_secs());
        i.state.last_check = i.prefs.last_check;
        save_prefs(&i.paths.prefs(), &i.prefs);
        match &found {
            Ok(Some(u)) if manual || i.prefs.skipped.as_deref() != Some(u.version.as_str()) => {
                i.state.phase = Phase::Available;
                i.state.version = Some(u.version.clone());
                i.state.notes = u.body.clone();
                i.state.date = u
                    .raw_json
                    .get("pub_date")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                i.state.size = u
                    .raw_json
                    .pointer(&format!(
                        "/platforms/{}/size",
                        tauri_plugin_updater::target().unwrap_or_default()
                    ))
                    .and_then(|v| v.as_u64());
                i.state.received = 0;
                i.state.release_url = release_url(&url, &u.version);
                i.update = Some(u.clone());
                !manual && i.prefs.auto_install && i.state.blocked.is_none()
            }
            Ok(_) => {
                i.state.phase = Phase::UpToDate;
                i.update = None;
                false
            }
            Err(_) => false,
        }
    })
    .unwrap_or(false);
    match found {
        Err(e) => fail(app, e),
        Ok(_) => {
            publish(app);
            if auto {
                download(app);
            }
        }
    }
}

/// Start the download + staging of the announced update (returns at once;
/// progress arrives as events).
fn download(app: &AppHandle) {
    let job = with(|i| {
        if i.state.channel != Channel::Release {
            return Err(None);
        }
        if let Some(why) = &i.state.blocked {
            return Err(Some(Fail(ErrorKind::Blocked, why.clone())));
        }
        let ready = matches!(i.state.phase, Phase::Available)
            || (i.state.phase == Phase::Error && i.update.is_some());
        let Some(update) = i.update.clone().filter(|_| ready) else {
            return Err(None);
        };
        let need = stage::needed_bytes(&i.paths.app);
        let _ = stage::private_dir(&i.paths.updates());
        if let Some(free) = stage::free_bytes(&i.paths.updates()).filter(|f| *f < need) {
            let gb = |b: u64| b as f64 / 1e9;
            return Err(Some(Fail(
                ErrorKind::Disk,
                format!("Not enough free disk space: the update needs about {:.1} GB and {:.1} GB is free.", gb(need), gb(free)),
            )));
        }
        i.state.phase = Phase::Downloading;
        i.state.received = 0;
        i.state.error = None;
        i.state.error_kind = None;
        Ok((update, i.paths.updates(), i.signers.clone()))
    });
    let (update, updates, signers) = match job {
        Some(Ok(j)) => j,
        Some(Err(Some(e))) => return fail(app, e),
        _ => return,
    };
    publish(app);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_pct = 0;
        let res = download_and_stage(&update, updates, signers, |got, total| {
            with(|i| {
                i.state.received = got;
                if total.is_some() {
                    i.state.size = total;
                }
            });
            // ≤ 100 events per download.
            let pct = total.map(|t| got * 100 / t.max(1)).unwrap_or(got >> 22);
            if pct != last_pct {
                last_pct = pct;
                publish(&handle);
            }
        })
        .await;
        match res {
            Ok(_) => {
                with(|i| i.state.phase = Phase::Staged);
                crate::lifecycle::durable(&format!("update: {} staged", update.version));
                publish(&handle);
            }
            Err(e) => fail(&handle, e),
        }
    });
}

/// Hand the staged bundle to the finisher and quit. The UI has already shown
/// what a restart ends (`GET /api/v1/system/restart-impact`) and got the
/// person's click.
fn restart(app: &AppHandle) -> Result<(), String> {
    let job = with(|i| {
        let to = i
            .state
            .version
            .clone()
            .filter(|_| i.state.phase == Phase::Staged);
        to.map(|to| (i.paths.clone(), i.state.current.clone(), to))
    })
    .flatten();
    let Some((paths, from, to)) = job else {
        return Err("no update is ready to install".into());
    };
    let (updates, slot, previous) = (paths.updates(), paths.slot(), paths.previous());
    let launched = finish::launch(&finish::Job {
        updates: &updates,
        app: &paths.app,
        slot: &slot,
        previous: &previous,
        from: &from,
        to: &to,
        app_pid: std::process::id(),
        meta_url: META_URL,
    });
    if let Err(e) = launched {
        fail(
            app,
            Fail(
                ErrorKind::Failed,
                format!("Could not start the installer: {e}"),
            ),
        );
        with(|i| i.state.phase = Phase::Staged);
        publish(app);
        return Err(e);
    }
    with(|i| i.state.phase = Phase::Restarting);
    publish(app);
    crate::quit::quit_app(app, &format!("update restart {from} → {to}"));
    Ok(())
}

/// Otto ▸ Check for Updates…: bring the main window forward (also when
/// hidden or minimized), ask its SPA to open the update dialog, and check.
pub fn on_menu(app: &AppHandle) {
    let _ = crate::popout::open_in_otto(app.clone(), None);
    if let Some(win) = crate::windows::primary_window(app) {
        let _ = app.emit_to(
            tauri::EventTarget::webview(win.label()),
            "otto://menu",
            MENU_ID,
        );
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move { check(&handle, true).await });
}

fn snapshot() -> UpdateState {
    with(|i| i.state.clone()).unwrap_or_default()
}

#[tauri::command]
pub fn update_state() -> UpdateState {
    snapshot()
}

/// A manual check (shows a skipped version too); resolves after it ends.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> UpdateState {
    check(&app, true).await;
    snapshot()
}

#[tauri::command]
pub fn update_download(app: AppHandle) -> UpdateState {
    download(&app);
    snapshot()
}

#[tauri::command]
pub fn update_restart(app: AppHandle) -> Result<(), String> {
    restart(&app)
}

/// "Install updates automatically" (download + stage in the background).
#[tauri::command]
pub fn update_set_auto(app: AppHandle, on: bool) -> UpdateState {
    with(|i| {
        i.prefs.auto_install = on;
        i.state.auto_install = on;
        save_prefs(&i.paths.prefs(), &i.prefs);
    });
    publish(&app);
    snapshot()
}

/// Skip `version` (hidden from automatic checks), or unskip with `null`.
#[tauri::command]
pub fn update_skip(app: AppHandle, version: Option<String>) -> UpdateState {
    with(|i| {
        i.prefs.skipped = version.clone();
        i.state.skipped = version;
        save_prefs(&i.paths.prefs(), &i.prefs);
        if i.state.phase == Phase::Available && i.state.skipped == i.state.version {
            i.state.phase = Phase::Idle;
        }
    });
    publish(&app);
    snapshot()
}

/// The UI showed `last_result` (toast / inline notice): forget it.
#[tauri::command]
pub fn update_result_seen(app: AppHandle) -> UpdateState {
    with(|i| {
        i.state.last_result = None;
        finish::clear_result(&i.paths.updates());
    });
    publish(&app);
    snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use testkit::{temp_dir, Kit, Minisign, Server};

    #[test]
    fn the_test_identity_is_never_the_release_channel() {
        assert!(test_identity(r#"{"test": true, "acceptedLeaves": []}"#));
        assert!(test_identity("{}"), "no marker counts as TEST");
        assert!(!test_identity(r#"{"test": false}"#));
        // The repo carries the TEST identity until the owner's replaces it.
        assert!(test_identity(RELEASE_IDENTITY));
        assert_eq!(
            release_channel(test_identity(RELEASE_IDENTITY), Some("release")),
            Channel::Source
        );
    }

    #[test]
    fn only_a_release_compile_is_on_the_release_channel() {
        assert_eq!(release_channel(false, Some("release")), Channel::Release);
        assert_eq!(
            release_channel(true, Some("release")),
            Channel::Source,
            "debug builds never update"
        );
        assert_eq!(
            release_channel(false, None),
            Channel::Source,
            "deploy.sh / cargo build"
        );
        assert_eq!(release_channel(false, Some("Release")), Channel::Source);
        // This test binary is itself a source build.
        assert_eq!(channel(), Channel::Source);
    }

    #[test]
    fn a_copy_outside_applications_or_without_write_access_is_blocked() {
        let dir = temp_dir("blocker");
        let apps = dir.join("apps");
        let app = apps.join("X.app");
        std::fs::create_dir_all(&app).unwrap();
        assert_eq!(install_blocker(&app, &app), None);
        let elsewhere = dir.join("Downloads/X.app");
        assert!(install_blocker(&elsewhere, &app)
            .unwrap()
            .contains("Move Otto to the Applications folder"));
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&apps, std::fs::Permissions::from_mode(0o555)).unwrap();
        let why = install_blocker(&app, &app).unwrap();
        assert!(
            why.contains("can't replace apps") && why.contains("DMG"),
            "{why}"
        );
        std::fs::set_permissions(&apps, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_gives_the_endpoint_the_signers_and_the_release_page() {
        let cfg = serde_json::json!({
            "pubkey": "x",
            "endpoints": ["https://github.com/acme/otto/releases/latest/download/latest.json"],
        });
        let url = endpoint(&cfg).unwrap();
        let leaf = "E2FD4736E55446F39C4151E258A6F260B1AE40CC";
        let two = format!(
            r#"{{"acceptedLeaves": ["{leaf}", "{}", "not-a-hash\" or anchor apple"]}}"#,
            "a".repeat(40)
        );
        assert_eq!(
            signers(&two),
            [
                format!("certificate leaf = H\"{}\"", leaf.to_lowercase()),
                format!("certificate leaf = H\"{}\"", "a".repeat(40))
            ],
            "hex leaves only — nothing else reaches the requirement"
        );
        assert_eq!(
            signers(RELEASE_IDENTITY).len(),
            1,
            "the shipped file parses"
        );
        assert_eq!(
            release_url(&url, "0.5.5").unwrap(),
            "https://github.com/acme/otto/releases/tag/v0.5.5"
        );
        assert!(signers("{}").is_empty(), "missing = none = fail closed");
        assert!(release_url(&"https://example.com/a/b/releases/x".parse().unwrap(), "1").is_none());
    }

    /// A mock Tauri app hosting the updater plugin with `pubkey`, running
    /// `version`.
    fn mock_app(pubkey: &str, version: &str) -> tauri::App<tauri::test::MockRuntime> {
        let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
        ctx.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({"pubkey": pubkey, "endpoints": ["https://github.com/o/r/releases/latest/download/latest.json"]}),
        );
        ctx.package_info_mut().version = version.parse().unwrap();
        tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(ctx)
            .unwrap()
    }

    fn manifest(version: &str, url: &str, sig: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": version,
            "notes": "## What's new\n- things",
            "pub_date": "2026-11-25T12:00:00Z",
            "platforms": {"darwin-aarch64": {"url": url, "signature": sig, "size": 1234}},
        }))
        .unwrap()
    }

    /// End to end on loopback: TEST-signed latest.json + archive served
    /// locally, the plugin's parsing/comparison/minisign + version binding,
    /// then staging with codesign checks — and each way it must refuse.
    #[tokio::test]
    async fn a_served_update_is_found_verified_and_staged_and_bad_ones_are_refused() {
        let keys = Minisign::new();
        let kit = Kit::new("e2e");
        let leaf = kit.identity("Otto Updater TEST E2E");
        let signers = vec![format!("certificate leaf = H\"{leaf}\"")];
        let archive = kit.archive(&kit.bundle("v123", "1.2.3", "Otto Updater TEST E2E", false));
        let srv = Server::start(vec![("/update.tar.gz", 200, archive.clone(), None)]);
        let archive_url = srv.url("/update.tar.gz").to_string();
        let app = mock_app(&keys.pubkey(), "1.2.2");
        let handle = app.handle().clone();
        let latest = srv.url("/latest.json");
        let updates = temp_dir("e2e-updates");

        // Happy path.
        srv.set(
            "/latest.json",
            200,
            manifest("1.2.3", &archive_url, &keys.sign(&archive, Some("1.2.3"))),
        );
        let u = find_update(&handle, &latest)
            .await
            .ok()
            .unwrap()
            .expect("1.2.3 > 1.2.2");
        assert_eq!(u.version, "1.2.3");
        let mut seen = 0;
        let app_dir = download_and_stage(&u, updates.clone(), signers.clone(), |got, _| seen = got)
            .await
            .ok()
            .unwrap();
        assert_eq!(seen, archive.len() as u64);
        assert_eq!(stage::bundle_version(&app_dir).unwrap(), "1.2.3");

        // Not newer: nothing to do.
        srv.set(
            "/latest.json",
            200,
            manifest("1.2.2", &archive_url, &keys.sign(&archive, Some("1.2.2"))),
        );
        assert!(find_update(&handle, &latest).await.ok().unwrap().is_none());

        // A newer version number paired with an older signed archive (replay).
        srv.set(
            "/latest.json",
            200,
            manifest("9.0.0", &archive_url, &keys.sign(&archive, Some("1.2.3"))),
        );
        let u = find_update(&handle, &latest).await.ok().unwrap().unwrap();
        let Err(Fail(kind, msg)) =
            download_and_stage(&u, updates.clone(), signers.clone(), |_, _| {}).await
        else {
            panic!("replay accepted")
        };
        assert_eq!(kind, ErrorKind::Verify, "{msg}");

        // A signature that binds no version at all.
        srv.set(
            "/latest.json",
            200,
            manifest("1.2.3", &archive_url, &keys.sign(&archive, None)),
        );
        let u = find_update(&handle, &latest).await.ok().unwrap().unwrap();
        assert!(
            download_and_stage(&u, updates.clone(), signers.clone(), |_, _| {})
                .await
                .is_err()
        );

        // Another key's signature.
        let other = Minisign::new();
        srv.set(
            "/latest.json",
            200,
            manifest("1.2.3", &archive_url, &other.sign(&archive, Some("1.2.3"))),
        );
        let u = find_update(&handle, &latest).await.ok().unwrap().unwrap();
        assert!(
            download_and_stage(&u, updates.clone(), signers.clone(), |_, _| {})
                .await
                .is_err()
        );

        // A tampered archive under a valid manifest.
        let mut bad = archive.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0xff;
        srv.set("/update.tar.gz", 200, bad);
        srv.set(
            "/latest.json",
            200,
            manifest("1.2.3", &archive_url, &keys.sign(&archive, Some("1.2.3"))),
        );
        let u = find_update(&handle, &latest).await.ok().unwrap().unwrap();
        let Err(Fail(kind, _)) =
            download_and_stage(&u, updates.clone(), signers.clone(), |_, _| {}).await
        else {
            panic!("tampered accepted")
        };
        assert_eq!(kind, ErrorKind::Verify);

        // A download URL off the allowed hosts is refused before any download.
        srv.set(
            "/latest.json",
            200,
            manifest(
                "1.2.3",
                "https://evil.example/update.tar.gz",
                &keys.sign(&archive, Some("1.2.3")),
            ),
        );
        let Err(Fail(kind, msg)) = find_update(&handle, &latest).await else {
            panic!("evil host accepted")
        };
        assert!(
            kind == ErrorKind::Verify && msg.contains("evil.example"),
            "{msg}"
        );

        // An oversized manifest is refused before it is parsed.
        srv.set(
            "/latest.json",
            200,
            vec![b' '; fetch::MANIFEST_MAX as usize + 1],
        );
        let Err(Fail(_, msg)) = find_update(&handle, &latest).await else {
            panic!("huge manifest parsed")
        };
        assert!(msg.contains("larger"), "{msg}");
        let _ = std::fs::remove_dir_all(&updates);
    }
}
