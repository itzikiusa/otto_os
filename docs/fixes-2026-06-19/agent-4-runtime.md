# Agent 4 — Runtime hardening (TLS listener, workflow reaper + timeout, Tauri CSP)

Pass: 6-agent parallel hardening of the Otto repo. Source audits:
`docs/deep-dive-improvements-2026-06-19.md` (§2 S3 TLS, D7 workflow reaper;
§1 S10 Tauri CSP) + the research audit.

## Checklist

- [x] **T3a** — TLS on the `0.0.0.0` network listener (audit S3)
- [x] **T7** — Workflow crash recovery: startup reaper + global per-run timeout (audit D7)
- [x] **T6** — Real Tauri Content-Security-Policy (audit S10)

---

## T3a — TLS on the network listener (rustls)

**Files / functions changed**

- `crates/ottod/Cargo.toml` — added TLS deps (see below).
- `crates/ottod/src/main.rs`
  - `run()` — the `network_listener` block (was plain `axum::serve` over a
    `TcpListener`) now serves over TLS via `axum_server::bind_rustls(addr, cfg)`,
    driven by `router.into_make_service()`. Graceful shutdown is bridged from the
    existing `shutdown_rx` watch channel into `axum_server::Handle::graceful_shutdown`,
    so the TLS listener drains in step with the loopback listener; the tail
    `network_task.await` still blocks until it finishes before PTYs are killed.
  - `load_or_make_tls_config(data_dir) -> Result<RustlsConfig, String>` (new) —
    loads `<data_dir>/tls/cert.pem` + `key.pem`; on first use auto-generates a
    persisted **self-signed** cert (`rcgen::generate_simple_self_signed` for
    `localhost`, `otto.local`, `127.0.0.1`), `chmod 600` on the key, and logs the
    SHA-256 fingerprint so operators can pin it. Installs the **ring** crypto
    provider explicitly (`rustls::crypto::ring::default_provider().install_default()`)
    because both `ring` and `aws-lc-rs` are linked into rustls in this tree, which
    makes the process-default provider ambiguous and would otherwise panic at
    first TLS use. If the cert/key can't be generated or loaded, the network
    listener is **not** started (clear error log) — it never silently falls back
    to plain HTTP.
  - `cert_fingerprint(der)` / `pem_cert_fingerprint(pem_bytes)` (new helpers) —
    colon-separated hex SHA-256 of a DER cert; the PEM variant parses the first
    cert from a PEM file for the "existing cert" log line.

**Behaviour preserved**: the loopback `127.0.0.1:<port>` listener is unchanged
(plain HTTP — fine for loopback). Both listeners share the same router clone and
the same graceful-shutdown signal; shutdown still drains both then kills PTYs.

**Deps added** (`crates/ottod/Cargo.toml`):

```toml
axum-server = { version = "0.8", default-features = false, features = ["tls-rustls-no-provider"] }
rustls = { version = "0.23", default-features = false, features = ["ring"] }
rustls-pemfile = "2"
rcgen = "0.13"
sha2 = "0.10"
```

All resolve to versions already in `Cargo.lock` (rustls 0.23.40, rustls-pki-types
1.14.1) or new leaf crates (axum-server 0.8.0, rcgen 0.13.2) — **no version bumps
to existing deps**. `tls-rustls-no-provider` is the key feature: it lets us pick
the provider rather than letting rustls guess (and panic).

---

## T7 — Workflow crash recovery (reaper + timeout)

**Files / functions changed**

- `crates/otto-server/src/workflow_engine.rs`
  - `reap_orphaned_runs(pool) -> Result<u64, sqlx::Error>` (new) — inline
    `UPDATE workflow_runs SET status='error', error='Interrupted by a daemon
    restart — re-run the workflow.', finished_at=COALESCE(finished_at, ?) WHERE
    status IN ('pending','running')`. No new repo method (per instruction); SQL
    mirrors the proven `ReviewsRepo::fail_running` / `SkillEvalsRepo::fail_running`
    pattern. Reaps **both** `pending` (run row created but the background task
    died before flipping to `running`) and `running` orphans.
  - `run_workflow(...)` run loop — added `RUN_WALL_CLOCK_TIMEOUT` (30 min) +
    `run_started: Instant` captured before the loop and a `timed_out` flag. The
    loop checks `run_started.elapsed() >= RUN_WALL_CLOCK_TIMEOUT` at each node
    boundary (next to the existing cancel check) and breaks. On timeout the run is
    marked `error` with a clear message and any not-yet-run nodes become
    `skipped`. A node already executing finishes first, bounded by the existing
    per-node `NODE_AGENT_TIMEOUT` (120 s).
- `crates/ottod/src/main.rs`
  - `run()` — calls `otto_server::workflow_engine::reap_orphaned_runs(&pool)` at
    startup, immediately after the review + skill-eval recovery blocks, logging the
    count of reaped runs (same shape as the others).

`crates/otto-server/src/routes/workflows.rs` (owned) needed **no change**: its
`cancel_run` already gates on `Pending | Running`, and the lifecycle changes are
entirely engine-side.

---

## T6 — Real Tauri CSP

**File changed**: `apps/desktop/src-tauri/tauri.conf.json` — `app.security.csp`
went from `null` to:

```
default-src 'self';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' data: blob: https: http://127.0.0.1:7700;
font-src 'self' data:;
connect-src 'self' http://127.0.0.1:7700 ws://127.0.0.1:7700 https://127.0.0.1:7700 wss://127.0.0.1:7700 http://localhost:7700 ws://localhost:7700 blob: data: ipc: http://ipc.localhost;
worker-src 'self' blob:;
frame-src 'self' blob: data: https: http:;
media-src 'self' blob: data:;
object-src 'none';
base-uri 'self'
```

**Why each clause** (verified against the UI/daemon, not guessed):

- `script-src 'self'` — vite bundles ES modules; `index.html` has no inline
  scripts; xterm WebGL addon + CodeMirror need neither `eval` nor `wasm`.
- `style-src 'unsafe-inline'` — Svelte injects `<style>` at runtime and
  xterm/CodeMirror set inline `style=` attributes.
- `connect-src` — the daemon REST + WS on `127.0.0.1:7700` (the SPA's `baseUrl()`
  / LSP WS), `blob:`/`data:` for object-URL fetches, and `ipc:` +
  `http://ipc.localhost` for Tauri 2 IPC (`@tauri-apps/api` `invoke`).
- `img-src` / `frame-src blob:` / `media-src blob:` — `URL.createObjectURL`
  images, the insights report iframe (blob URL), CSV download blobs.
- `frame-src https: http:` — the **BrowserPanel** is a real inline browser: it
  frames external pages directly and routes take-over mode through the daemon
  `GET /browser/proxy` (covered by `http://127.0.0.1:7700`, itself within
  `http:`). FileTree/insights `srcdoc`/blob iframes are covered by `'self'`/`blob:`.
- `object-src 'none'` + `base-uri 'self'` — defense-in-depth, no plugins / no
  base-tag hijack.

As strict as possible while keeping the app fully functional (local assets,
daemon http+ws, xterm/Monaco-class editors, the browser-proxy flow).

---

## Build results

`cargo check -p otto-server -p ottod`:

- New TLS deps resolved and compiled cleanly: `rcgen v0.13.2`, `axum-server v0.8.0`.
- **My files compile with zero errors.** Verified: `cargo check -p otto-server`
  reports no error in `workflow_engine.rs`. The TLS helper and the
  `bind_rustls().handle().serve(into_make_service())` wiring were each
  compile-tested in isolation against the exact workspace dep versions (axum 0.8,
  axum-server 0.8, rustls 0.23 + ring, rcgen 0.13, sha2 0.10, rustls-pemfile 2) —
  both build green.
- The crate currently fails to finish compiling due to **7 errors in files I do
  not own** — another agent is adding a `user_id` field to `NewNotice` /
  `Event::Notification` (working-tree-only; not on HEAD) and hasn't updated all
  call sites yet: `monitor.rs` (×4), `routes/activity.rs`, `skill_eval.rs`,
  `state.rs`. Noted, not fixed (out of my ownership). Once that agent's change
  lands, `cargo check -p ottod` should go green with no action on my files.

`tauri.conf.json` validated as well-formed JSON.
