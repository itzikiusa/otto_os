//! The shared webview background-throttling policy. Its own module (not a
//! `main.rs` item) so the native probe examples, which `#[path]`-include the
//! window/pane/room modules that reference `crate::NO_THROTTLE`, can include
//! this file too instead of re-declaring the value.

/// Every Otto webview (main, pop-out, native pane child, room host) opts out
/// of WebKit's inactive-view scheduling. xterm parses each PTY frame on a
/// `setTimeout` and paints on rAF; once WebKit decides a window is occluded
/// or "in the background" those clamp to ~1 s, which shows up as second-scale
/// echo/scroll lag with an idle CPU. The UI's own pollers already pause on
/// `visibilitychange`, so leaving timers live costs nothing while hidden.
/// macOS 14+ only (`WKPreferences.inactiveSchedulingPolicy = .none`); older
/// systems ignore it. The main window sets the same in `tauri.conf.json`.
pub(crate) const NO_THROTTLE: tauri::utils::config::BackgroundThrottlingPolicy =
    tauri::utils::config::BackgroundThrottlingPolicy::Disabled;
