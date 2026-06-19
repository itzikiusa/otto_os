# Release checklist

Deterministic, gated steps to cut a local Otto release (macOS-only). Each gate
must pass before moving on — stop on the first failure. There is no Makefile;
these are the real commands (mirrors `README.md` + `packaging/`).

> Prerequisites: macOS (Apple Silicon or Intel), Rust stable, Node 20+, and a
> one-time signing identity (`packaging/make-cert.sh`). The Tauri CLI is invoked
> via `npx --yes @tauri-apps/cli@^2` — no global install needed.

## 0. Pre-flight

- [ ] On a clean tree (`git status` shows nothing unexpected) and the right branch.
- [ ] Version bumped consistently if releasing a new version: root
      `Cargo.toml` (`[workspace.package].version`),
      `apps/desktop/src-tauri/Cargo.toml`, and
      `apps/desktop/src-tauri/tauri.conf.json` (`version`).
- [ ] `docs/contracts/*.md` and `ui/src/lib/api/types.ts` are in sync (no drift).

## 1. Verify (gates — all must pass)

```bash
# Rust workspace (daemon crates + ottod)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace
cargo test --workspace

# UI type-check
cd ui && npm ci && npm run check && cd ..
```

- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy … -D warnings` clean
- [ ] `cargo check --workspace` clean
- [ ] `cargo test --workspace` green
- [ ] `npm run check` clean (svelte-check + tsc)

## 2. Build artifacts

```bash
# 1. Frontend → ui/dist
cd ui && npm run build && cd ..

# 2. Daemon (release)
cargo build --release -p ottod
#   For REMOTE / phone access (serve the SPA from the daemon over HTTP/Cloudflare
#   Tunnel), build with the `embed-ui` feature INSTEAD — it bakes ui/dist into the
#   binary so https://<host>/ returns the app same-origin (hash-router deep links /
#   refresh / back all resolve correctly). Build order matters: step 1 (npm run
#   build → ui/dist) MUST run first, because rust-embed reads ui/dist at compile time.
#       cargo build --release -p ottod --features embed-ui
#   See docs/remote-access-runbook.md for exposure (Cloudflare Tunnel) + PWA install.

# 3. Bundle the daemon as the app's sidecar (Tauri externalBin).
#    The filename MUST carry the host target triple.
cp target/release/ottod \
   apps/desktop/src-tauri/binaries/ottod-$(rustc -vV | sed -n 's/host: //p')

# 4. Build the desktop app. `cargo-tauri` is not installed in this repo — invoke
#    the Tauri CLI via npx. Omitting `--bundles` builds both the app and the dmg
#    (the targets configured in tauri.conf.json); pass `--bundles app` for app only.
cd apps/desktop/src-tauri && npx --yes @tauri-apps/cli@^2 build && cd -
#   → apps/desktop/src-tauri/target/release/bundle/macos/Otto.app
#   → apps/desktop/src-tauri/target/release/bundle/dmg/Otto_<version>_<arch>.dmg
```

- [ ] `ui/dist` rebuilt
- [ ] `ottod` release binary built
- [ ] Sidecar copied to `binaries/ottod-<host-triple>` (e.g. `ottod-aarch64-apple-darwin`)
- [ ] `Otto.app` produced under the bundle dir

## 3. Sign (macOS requires it)

Signing with the **same** stable identity each time keeps macOS TCC approvals
(network, accessibility, …) persistent across rebuilds.

```bash
# One-time, if not already created:
packaging/make-cert.sh

# Sign the app and the standalone ottod (path the sidecar copy came from):
packaging/sign.sh <path-to>/Otto.app target/release/ottod
```

- [ ] Signing identity `Otto Dev Signing` present
      (`security find-identity -p codesigning`)
- [ ] `packaging/sign.sh` reports the bundle + sidecar signed
- [ ] `codesign -dv <Otto.app>` shows the expected Identifier/Authority

## 4. DMG

`cargo tauri build` above already emits a `.dmg` (the `dmg` target is enabled in
`tauri.conf.json`). To repackage a *signed* app into a DMG explicitly:

```bash
packaging/dmg.sh <path-to>/Otto.app Otto.dmg
```

- [ ] DMG created and opens; dragging to `/Applications` installs cleanly

## 5. Install & smoke test

Install with `ditto` (it preserves the signature). The app self-deploys the
`ottod` daemon under the `com.otto.daemon` launchd agent on launch.

```bash
rm -rf /Applications/Otto.app && ditto <path-to>/Otto.app /Applications/Otto.app
open /Applications/Otto.app
```

> If the app was already running, **quit it first** (`osascript -e 'quit app
> "Otto"'`) and re-`open` — the daemon is redeployed only at app start.

- [ ] First run prompts for a root password and workspace setup.
- [ ] The daemon is live on loopback:
      `curl -s localhost:7700/api/v1/health` → `{"ok":true}`.
- [ ] The freshly deployed `~/Library/Application Support/Otto/bin/ottod` matches
      the bundled `Contents/MacOS/ottod` (a new `ottod` pid is running).
- [ ] Open a session (claude/codex/shell), a git repo, and a DB connection — no
      crashes; no permission prompts that should have been auto-trusted.

## 6. Publish (only when explicitly approved)

- [ ] Tag the release and push the tag (`main` is protected — open a PR for code).
- [ ] Attach the signed DMG to the release.

---

**Background daemon (optional, for a standalone install):**
`packaging/com.otto.daemon.plist` is the `launchd` template that runs `ottod`
on port `7700`.
