//! Remote live browser: a daemon-owned Chromium (Chrome for Testing,
//! downloaded on first use — never bundled) whose screen is streamed to the
//! Browser page as a CDP screencast, with the viewer's input sent back.
//! Contract: `docs/contracts/api.md` "Browser — remote live view" and
//! `docs/contracts/ws.md` §1b.
//!
//! Security model (as built):
//! - CDP over `--remote-debugging-pipe` only — no debugging port exists;
//! - Chrome's own sandbox stays on; private `--user-data-dir` per process;
//! - one browser context per ephemeral session (disposed on close) and one
//!   process per persistent profile scoped to (workspace, owner, name);
//! - every request paused by CDP `Fetch` (browser-level when available) and
//!   vetted through `otto-netguard` for the session's whole life, AND every
//!   TCP connection dialled through a netguarded SOCKS5 proxy (pinned DNS);
//! - agent-driven outward document requests are held for a human approval
//!   with a pre-action screenshot;
//! - downloads denied or quarantined (GUID-named, xattr, no exec bit);
//! - navigations, opens/closes, control changes and outward decisions audited.
//!
//! Module map: [`types`] wire/settings types · [`install`] pinned builds +
//! download · [`chrome`] command line + pipe spawn · [`conn`] CDP-over-pipe
//! client · [`process`] one Chromium + its browser-level event loop ·
//! [`session`] one tab's page, screencast, input, control · [`runtime`] the
//! pool + janitor · [`guard`] per-request SSRF / outward decisions ·
//! [`proxy`] the guard proxy · [`flow`] screencast backpressure ·
//! [`input`] input → CDP mapping · [`control`] the take-over lock ·
//! [`protocol`] the viewer WS protocol · [`hooks`] what the host provides.

pub mod chrome;
pub mod conn;
pub mod control;
pub mod flow;
pub mod guard;
pub mod hooks;
pub mod input;
pub mod install;
pub mod process;
pub mod protocol;
pub mod proxy;
pub mod runtime;
pub mod session;
pub mod types;

pub use hooks::{LiveAudit, LiveHooks, NoopHooks, OutwardAction};
pub use install::{InstallJob, InstallState};
pub use protocol::{ClientMsg, ControlAction, NavAction, ServerMsg};
pub use runtime::{LiveError, LiveRuntime, LiveStatus};
pub use session::{
    ImageFormat, LiveSession, OpenParams, Screenshot, ScreenshotMode, ScreenshotRequest,
    ViewerHandle, ViewerOut,
};
pub use types::{
    ChromeBuild, ControllerKind, DownloadPolicy, LiveSessionInfo, LiveSettings, LiveSettingsPatch,
    SessionState, Viewport, EPHEMERAL_PROFILE, SETTINGS_KEY,
};
