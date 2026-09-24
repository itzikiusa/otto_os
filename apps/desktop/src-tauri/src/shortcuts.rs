//! Global-shortcut registry — the ONE owner of every system-wide chord.
//!
//! Each shortcut has a stable id (`snip`, `assistant`, `assistant.voice`), a
//! default accelerator, and a user override persisted in
//! `<app-config>/shortcuts.json` (`{"shortcuts": {"<id>": "<accel>"}}`; an
//! empty string = disabled). Registering one id never touches another's chord —
//! this replaced snip.rs's `unregister_all()`, which assumed snip owned every
//! global shortcut in the app.
//!
//! Handlers stay thin: they route to the feature module (snip emits to one
//! window, the assistant toggles the bar panel). Business logic stays in the
//! daemon + SPA; the Rust side deliberately holds no bearer token.
//!
//! Migration: a pre-registry install kept the snip chord in `snip.json`; it is
//! read once as the `snip` override when `shortcuts.json` has none.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// One registrable chord.
pub struct Spec {
    pub id: &'static str,
    pub label: &'static str,
    /// Default accelerator (global-hotkey syntax); `""` = off until the user
    /// picks one.
    pub default: &'static str,
}

/// Every global shortcut Otto knows. `⌘I` stays an IN-APP chord (keys.ts) —
/// the assistant's system-wide summon is `⌥Space`. Voice is off by default
/// until the voice helper ships (a held chord would otherwise swallow ⌥Space
/// presses meant for the bar).
pub const SPECS: &[Spec] = &[
    Spec {
        id: "snip",
        label: "Take a snip",
        default: crate::snip::DEFAULT_SHORTCUT,
    },
    Spec {
        id: "assistant",
        label: "Show or hide the assistant bar",
        default: "Alt+Space",
    },
    Spec {
        id: "assistant.voice",
        label: "Hold to talk to the assistant",
        default: "",
    },
];

fn spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    #[serde(default)]
    shortcuts: BTreeMap<String, String>,
}

/// Legacy `snip.json` shape (pre-registry).
#[derive(Deserialize, Default)]
struct LegacySnipConfig {
    shortcut: Option<String>,
}

/// Live registrations: id → the chord currently held with the OS, plus the
/// last registration error per id (surfaced by `shortcuts_list`).
#[derive(Default)]
struct Live {
    active: HashMap<&'static str, Shortcut>,
    errors: HashMap<&'static str, String>,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn with_live<R>(f: impl FnOnce(&mut Live) -> R) -> R {
    let mut guard = LIVE.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(Live::default))
}

fn config_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

fn load_config(app: &AppHandle) -> Config {
    let Some(dir) = config_dir(app) else {
        return Config::default();
    };
    let mut cfg: Config = fs::read(dir.join("shortcuts.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    if !cfg.shortcuts.contains_key("snip") {
        let legacy = fs::read(dir.join("snip.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice::<LegacySnipConfig>(&raw).ok())
            .and_then(|c| c.shortcut);
        if let Some(accel) = legacy {
            cfg.shortcuts.insert("snip".into(), accel);
        }
    }
    cfg
}

/// Atomic write (temp + rename), like windows.rs — a crash mid-write must not
/// lose every chord.
fn save_config(app: &AppHandle, cfg: &Config) {
    let Some(dir) = config_dir(app) else { return };
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("shortcuts.json");
    let tmp = path.with_extension("json.tmp");
    if let Ok(raw) = serde_json::to_vec_pretty(cfg) {
        if fs::write(&tmp, raw).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

/// The accelerator an id should hold: the user's override, else the default.
pub fn accel_for(app: &AppHandle, id: &str) -> String {
    let Some(s) = spec(id) else {
        return String::new();
    };
    load_config(app)
        .shortcuts
        .get(id)
        .cloned()
        .unwrap_or_else(|| s.default.to_string())
}

/// Route a fired chord to its feature. Kept tiny: every branch just hands off.
fn dispatch(app: &AppHandle, id: &'static str, state: ShortcutState) {
    match (id, state) {
        ("snip", ShortcutState::Pressed) => crate::snip::emit_snip(app),
        ("assistant", ShortcutState::Pressed) => crate::bar::toggle(app),
        ("assistant.voice", ShortcutState::Pressed) => crate::bar::voice(app, true),
        ("assistant.voice", ShortcutState::Released) => crate::bar::voice(app, false),
        _ => {}
    }
}

fn parse(accel: &str) -> Result<Shortcut, String> {
    accel
        .parse::<Shortcut>()
        .map_err(|e| format!("invalid shortcut {accel:?}: {e}"))
}

/// (Re)bind `id` to `accel` with the OS. Empty `accel` = disable. Only this
/// id's previous chord is released; a failed new registration re-arms the old
/// one so a typo in Settings never leaves the feature without its chord.
pub fn register(app: &AppHandle, id: &str, accel: &str) -> Result<(), String> {
    let spec = spec(id).ok_or_else(|| format!("unknown shortcut id {id:?}"))?;
    let id: &'static str = spec.id;
    let next = if accel.trim().is_empty() {
        None
    } else {
        Some(parse(accel.trim())?)
    };

    // Two ids on one chord would make the second registration fail at the OS
    // level (or, worse, silently shadow) — refuse it with a clear message.
    if let Some(sc) = next {
        let clash = with_live(|l| {
            l.active
                .iter()
                .find(|(other, held)| **other != id && **held == sc)
                .map(|(other, _)| *other)
        });
        if let Some(other) = clash {
            let label = spec_label(other);
            return Err(format!("{accel} is already used by \"{label}\""));
        }
    }

    let gs = app.global_shortcut();
    let prev = with_live(|l| l.active.remove(id));
    if let Some(p) = prev {
        if Some(p) == next {
            // Same chord: keep the existing registration.
            with_live(|l| {
                l.active.insert(id, p);
                l.errors.remove(id);
            });
            return Ok(());
        }
        let _ = gs.unregister(p);
    }
    let Some(sc) = next else {
        with_live(|l| {
            l.errors.remove(id);
        });
        return Ok(());
    };

    let result = gs.on_shortcut(sc, move |app, _sc, event| dispatch(app, id, event.state()));
    match result {
        Ok(()) => {
            with_live(|l| {
                l.active.insert(id, sc);
                l.errors.remove(id);
            });
            Ok(())
        }
        Err(e) => {
            let msg = format!("could not register {accel:?}: {e}");
            if let Some(p) = prev {
                if gs
                    .on_shortcut(p, move |app, _sc, event| dispatch(app, id, event.state()))
                    .is_ok()
                {
                    with_live(|l| {
                        l.active.insert(id, p);
                    });
                }
            }
            with_live(|l| {
                l.errors.insert(id, msg.clone());
            });
            Err(msg)
        }
    }
}

fn spec_label(id: &str) -> &'static str {
    spec(id).map(|s| s.label).unwrap_or("another shortcut")
}

/// Startup: arm every chord from config/defaults. Non-fatal per id — another
/// app holding a chord only disables that one feature's global trigger (the
/// in-app triggers still work) and the error shows in `shortcuts_list`.
pub fn init(app: &AppHandle) {
    for s in SPECS {
        let accel = accel_for(app, s.id);
        if let Err(e) = register(app, s.id, &accel) {
            eprintln!("shortcut {}: global shortcut unavailable: {e}", s.id);
        }
    }
}

/// Persist + arm a new chord for `id` (register first: nothing is saved
/// unless the OS accepted it).
pub fn set(app: &AppHandle, id: &str, accel: &str) -> Result<(), String> {
    register(app, id, accel)?;
    let mut cfg = load_config(app);
    cfg.shortcuts
        .insert(id.to_string(), accel.trim().to_string());
    save_config(app, &cfg);
    Ok(())
}

/// One row of `shortcuts_list` (the Settings UI renders these).
#[derive(Serialize)]
pub struct ShortcutInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// The configured accelerator (`""` = disabled).
    pub accel: String,
    pub default: &'static str,
    /// True when the OS accepted the registration.
    pub active: bool,
    /// Last registration failure (e.g. another app holds the chord).
    pub error: Option<String>,
}

#[tauri::command]
pub fn shortcuts_list(app: AppHandle) -> Vec<ShortcutInfo> {
    SPECS
        .iter()
        .map(|s| {
            let (active, error) =
                with_live(|l| (l.active.contains_key(s.id), l.errors.get(s.id).cloned()));
            ShortcutInfo {
                id: s.id,
                label: s.label,
                accel: accel_for(&app, s.id),
                default: s.default,
                active,
                error,
            }
        })
        .collect()
}

#[tauri::command]
pub fn shortcuts_set(app: AppHandle, id: String, accel: String) -> Result<(), String> {
    set(&app, &id, &accel)
}

/// Back to the built-in default for `id`.
#[tauri::command]
pub fn shortcuts_reset(app: AppHandle, id: String) -> Result<(), String> {
    let s = spec(&id).ok_or_else(|| format!("unknown shortcut id {id:?}"))?;
    set(&app, s.id, s.default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_have_unique_ids_and_parseable_defaults() {
        let mut seen = std::collections::HashSet::new();
        for s in SPECS {
            assert!(seen.insert(s.id), "duplicate shortcut id {}", s.id);
            if !s.default.is_empty() {
                assert!(parse(s.default).is_ok(), "default for {} must parse", s.id);
            }
        }
        // The assistant's summon is ⌥Space; ⌘I stays in-app.
        assert_eq!(spec("assistant").map(|s| s.default), Some("Alt+Space"));
    }

    #[test]
    fn defaults_never_collide() {
        let parsed: Vec<Shortcut> = SPECS
            .iter()
            .filter(|s| !s.default.is_empty())
            .map(|s| parse(s.default).unwrap())
            .collect();
        for (i, a) in parsed.iter().enumerate() {
            for b in &parsed[i + 1..] {
                assert_ne!(a, b, "two default chords collide");
            }
        }
    }
}
