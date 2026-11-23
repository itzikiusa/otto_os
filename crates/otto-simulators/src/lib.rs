//! `otto-simulators` — iOS Simulator (`xcrun simctl`) and Android Emulator
//! (`adb` / `emulator` / `avdmanager` / `sdkmanager`) control plane for Otto.
//!
//! - [`detect`] — dynamic host auto-detection + actionable readiness diagnostics
//!   (never fails startup when Xcode / iOS runtimes / Android SDK are absent).
//! - [`ios`] — iOS Simulator driver (`xcrun simctl`, optional `idb`, `osascript`).
//! - [`android`] — Android Emulator & ADB driver (AVDs, connected devices,
//!   auto `adb reverse` localhost forwarding, touch/key input, rotation, dark mode,
//!   screenshots, app install/launch, logcat).
//! - [`service`] — unified [`SimulatorsService`] with discovery caching and
//!   per-device single-flight screenshot locking.
//! - [`http`] — `/simulators/*` Axum REST router generic over [`SimulatorsCtx`].
//! - [`ws`] — live screencast + bidirectional input (`WS /ws/simulators/{id}/stream`)
//!   and live log streaming (`WS /ws/simulators/{id}/logs`).

pub mod android;
pub mod android_env;
pub mod detect;
pub mod exec;
pub mod http;
pub mod install;
pub mod ios;
pub mod service;
pub mod types;
pub mod ws;

use std::sync::Arc;

use axum::Router;

pub use service::SimulatorsService;
pub use types::*;

/// Server-side context required by the `/simulators/*` REST router.
pub trait SimulatorsCtx: Clone + Send + Sync + 'static {
    fn simulators(&self) -> &Arc<SimulatorsService>;
}

/// All `/simulators/*` REST routes (handler-relative; nested under `/api/v1`).
pub fn api_router<S: SimulatorsCtx>() -> Router<S> {
    http::api_router::<S>()
}
