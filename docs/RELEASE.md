# Release checklist

Two ways to make an Otto build (macOS only; there is no Makefile):

- **Published release (CI).** A `v<version>` tag builds, signs and drafts a
  GitHub Release with the DMG and the in-app updater's files
  (`.github/workflows/release.yml`). This is what people download and what
  installed release builds update to. One-time setup and the one-step release
  are below.
- **Local build** with your own certificate: the gated checklist from
  [§0](#0-pre-flight) on (`packaging/deploy.sh` chains it). A local build never
  updates itself.

## Published releases

### What the workflow does

1. **ci-gate.** The tagged commit must be on `main` with a green `ci.yml` run.
2. **build** (no secrets in this job: it runs the npm and cargo build).
   - The tag must equal the version in the root `Cargo.toml`,
     `apps/desktop/src-tauri/Cargo.toml` and `tauri.conf.json`.
   - `README.md` must have a `### What's new in <version>` section.
   - Then UI → `ottod --release --features embed-ui` → sidecar → `tauri build
     --bundles app`, as `deploy.sh` does.
3. **sign** runs in the GitHub Environment `release`. It waits for your approval
   and is the only job that sees the secrets.
   - It imports the release `.p12` into a throwaway keychain.
   - [`packaging/release-artifacts.sh`](../packaging/release-artifacts.sh) then
     does the packaging:
     1. signs with `sign.sh`;
     2. checks the app, `ottod` (and `kiri`) each against its own identifier and
        the accepted certificates in
        `apps/desktop/src-tauri/release-identity.json`;
     3. packs `Otto_<v>_aarch64.app.tar.gz` with no AppleDouble files, and
        proves its signature survives a literal extraction;
     4. signs the archive with the update key, bound to `<v>`;
     5. makes the DMG and its `.sha256`;
     6. writes `latest.json` and the notes.
   - It creates a **draft** release with those five files. A `-rc` version
     becomes a prerelease draft.

Nothing reaches anyone until you **publish** the draft. Installed apps read
`https://github.com/itzikiusa/otto_os/releases/latest/download/latest.json`,
which resolves only to the newest published release that is not a prerelease.

### One-time setup (owner)

Run these on your Mac. The private files never go into the repository.

1. **Signing certificate** (self-signed; see the note on Developer ID below):

   ```bash
   OTTO_SIGN_IDENTITY="Otto Release Signing" OTTO_P12_PASSWORD='<a strong password>' \
     packaging/make-cert.sh --export ~/otto-release
   # prints: Leaf hash: <40 hex>
   ```

   - It writes `~/otto-release/Otto Release Signing.p12` (private) and `.cer`
     (public).
   - Keep the `.p12` and its password in your password manager.
   - Losing it means the next release is signed differently, and every user
     re-grants macOS permissions once.

2. **Update key** (minisign; Tauri asks for a password):

   ```bash
   npx --yes @tauri-apps/cli@2.12.6 signer generate -w ~/otto-release/otto-update.key
   ```

   Back up `otto-update.key` and its password too. Without them no installed
   copy can ever update again; it would need a manual DMG install.

3. **Put the public halves in the repository** (one PR). Today they hold TEST
   values that exist only on the machine that made them:
   - `apps/desktop/src-tauri/tauri.conf.json` → `plugins.updater.pubkey` = the
     content of `~/otto-release/otto-update.key.pub`.
   - `apps/desktop/src-tauri/release-identity.json` → `"signingIdentity":
     "Otto Release Signing"`, `"acceptedLeaves": ["<leaf hash from step 1>"]`.
     Installed apps refuse an update signed by any other certificate. To move
     to another certificate later (for example Developer ID), first ship a
     release that lists both.

4. **GitHub Environment, secrets and tag protection** (`gh` as the repository
   admin):

   ```bash
   R=itzikiusa/otto_os
   gh api -X PUT repos/$R/environments/release --input - <<JSON
   {"reviewers":[{"type":"User","id":$(gh api user --jq .id)}],
    "deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
   JSON
   gh api -X POST repos/$R/environments/release/deployment-branch-policies -f name='v*' -f type=tag
   gh api -X POST repos/$R/environments/release/deployment-branch-policies -f name=main -f type=branch
   base64 -i ~/otto-release/"Otto Release Signing.p12" | gh secret set OTTO_SIGN_P12_BASE64 --repo $R --env release
   gh secret set OTTO_SIGN_P12_PASSWORD --repo $R --env release            # paste the .p12 password
   gh secret set TAURI_SIGNING_PRIVATE_KEY --repo $R --env release < ~/otto-release/otto-update.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo $R --env release  # paste the key password
   # Only admins may create, move or delete v* tags:
   gh api -X POST repos/$R/rulesets --input - <<'JSON'
   {"name":"release tags","target":"tag","enforcement":"active",
    "conditions":{"ref_name":{"include":["refs/tags/v*"],"exclude":[]}},
    "rules":[{"type":"creation"},{"type":"update"},{"type":"deletion"}],
    "bypass_actors":[{"actor_id":5,"actor_type":"RepositoryRole","bypass_mode":"always"}]}
   JSON
   ```

   - The `main` policy is for re-runs started from the Actions tab: those run
     on `main`, with the tag as input.
   - The rolling `latest` prerelease from the retired monthly workflow is no
     longer updated. Delete it when you like:
     `gh release delete latest --repo $R --cleanup-tag`.

> **Developer ID instead** (Apple, $99/year): the first open is then
> Gatekeeper-clean, and no one sees a Keychain prompt. Export the Developer ID
> Application identity as the `.p12`. Set `signingIdentity` to its full name.
> Add `(anchor apple generic and certificate leaf[subject.OU] = "<TEAM ID>")` to
> the accepted list, through a release that still accepts the old certificate.
> Notarization and `--timestamp` are not wired in yet.

### Releasing a version (one step)

1. Bump the version and add `### What's new in <version>` to `README.md` in the
   release PR, as usual (§0). Merge it.
2. Tag the merged commit and push the tag:

   ```bash
   git fetch origin && git tag v0.5.4 origin/main && git push origin v0.5.4
   ```

3. In **Actions ▸ Release**, approve the `release` deployment. About 40
   minutes later the draft is ready.
4. Open the draft.
   - Check the notes and attach the tour film by hand.
   - Optionally check an asset: `codesign -d -r- Otto.app` should show the
     release leaf.
   - Click **Publish**.
5. Check what installed apps will see:
   `curl -sL https://github.com/itzikiusa/otto_os/releases/latest/download/latest.json | jq .version`.

A failed or wrong draft:
- `gh release delete v0.5.4 --repo itzikiusa/otto_os --yes` deletes the draft
  and keeps the tag.
- Fix the problem, then **Actions ▸ Release ▸ Run workflow** with tag `v0.5.4`.

The first release that contains the updater is 0.5.4. The first real update is
therefore 0.5.4 → 0.5.5.

## Local build

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
cargo build --release -p ottod --features embed-ui
#   `embed-ui` bakes ui/dist into the binary so the daemon serves the SPA
#   same-origin — https://<host>/ returns the app (hash-router deep links /
#   refresh / back all resolve correctly). REQUIRED for remote / phone access
#   over HTTP or a Cloudflare Tunnel; without it those requests get the
#   "UI not embedded" placeholder while /api/v1/health still reports ok, so a
#   redeploy that drops the feature breaks sharing with no obvious symptom.
#   Drop the flag only for a local desktop-only build (the Tauri webview loads
#   the frontend from the app bundle, not from the daemon).
#   Build order matters: step 1 (npm run build → ui/dist) MUST run first,
#   because rust-embed reads ui/dist at compile time.
#   See docs/remote-access-runbook.md for exposure (Cloudflare Tunnel) + PWA install.
#   `packaging/deploy.sh` does all of this with embed-ui on by default.

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
(network, accessibility, …) persistent across rebuilds. `sign.sh` also ensures
the cert is **trusted for code signing**, which is what keeps login-keychain
"Always Allow" grants from re-prompting on every rebuild — see
[`packaging/README.md`](../packaging/README.md).

```bash
# One-time, if not already created (also trusts the cert for code signing):
packaging/make-cert.sh

# Sign the app and the standalone ottod (path the sidecar copy came from):
packaging/sign.sh <path-to>/Otto.app target/release/ottod
```

- [ ] Signing identity `Otto Dev Signing` present
      (`security find-identity -p codesigning`)
- [ ] Cert trusted for code signing
      (`security dump-trust-settings | grep -i "Otto Dev Signing"`)
- [ ] `packaging/sign.sh` reports the bundle + sidecar signed
- [ ] `codesign -dv <Otto.app>` shows the expected Identifier/Authority

> **Shortcut:** steps 2–5 (build → sign → install → relaunch → verify) are
> chained in [`packaging/deploy.sh`](../packaging/deploy.sh) — run that for a
> one-command local redeploy.

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

### Rolling back

A bad local deploy rolls back automatically when verification fails. To roll
back by hand, follow
[packaging/README.md — Recovery](../packaging/README.md#recovery-rolling-back-a-bad-deploy). Roll back
the app and the daemon together: the app's supervisor reinstalls its bundled
`ottod` at every launch, so restoring `bin/ottod.prev` alone does not stick
while a newer app is installed.

Before rolling back below 0.3.0, close and delete any Muse threads: a 0.2.x
daemon can't read a Muse thread row, and its Threads list then fails to load.
Builds from 0.3.0 on skip (and log) a thread row with a provider they don't
know.

## Walkthrough videos (separate from the DMG)

The in-app Walkthroughs page streams its MP4s from the rolling GitHub release
tagged **`walkthroughs`** (`https://github.com/itzikiusa/otto_os/releases/download/walkthroughs/<Id>.mp4`);
they are **not** in `ui/dist`, `ottod`, or `Otto.app` (that was ~135 MB in each).
Only when the videos were re-rendered (`marketing/videos/`):

```bash
packaging/publish-walkthroughs.sh   # 720p re-encode → gh release upload --clobber
```

- [ ] `curl -sI -r 0-1023 https://github.com/itzikiusa/otto_os/releases/download/walkthroughs/Intro.mp4`
      → `302`, then `206` with `Accept-Ranges: bytes` from the CDN.
- [ ] The release stays a **pre-release** so it never becomes "Latest".
- [ ] `ui/dist/` contains no `walkthroughs/` directory (`du -sh ui/dist` ≈ 22 MB).

## 6. Publish (only when explicitly approved)

- [ ] Publishing is the CI flow in [Published releases](#published-releases):
      push the `v<version>` tag, approve, review the draft, Publish. A locally
      signed DMG is signed with your own certificate, so installed release
      builds would refuse it as an update; don't attach it to a release.

---

**Background daemon (optional, for a standalone install):**
`packaging/com.otto.daemon.plist` is the `launchd` template that runs `ottod`
on port `7700`. Both it and the plist the app's supervisor installs set
`ProcessType=Interactive`: launchd's default (and `Background`) throttles CPU
and I/O for the daemon and every PTY child it spawns, which shows up as
keystroke-echo lag. Verify with
`launchctl print gui/$UID/com.otto.daemon | grep -i 'process type'`.
Both also set `ExitTimeOut=30` (ottod caps its HTTP drain at ~3 s and bounds
each teardown step, so launchd never has to SIGKILL a stopping daemon) and send
stdout/stderr to `~/Library/Logs/Otto/ottod.{stdout,stderr}.log` — panics land
there (and in `ottod.log`, with a backtrace). Under launchd only warnings and
errors are mirrored to stderr; `ottod.log.*` keeps the newest 30 daily files.
