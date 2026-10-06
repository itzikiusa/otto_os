# packaging/ — local macOS build, signing & deploy

Scripts to build, code-sign, and install the Otto desktop app (Tauri shell +
`ottod` daemon sidecar) on this machine. macOS only. There is no Makefile — these
are the real, chained steps; the full gated checklist lives in
[`docs/RELEASE.md`](../docs/RELEASE.md).

| File | What it does |
|------|--------------|
| `make-cert.sh` | One-time: create the long-lived self-signed code-signing cert **"Otto Dev Signing"** in your login keychain and **trust it for code signing**. |
| `sign.sh` | Sign `ottod` + `Otto.app` with that cert. **Also re-asserts code-signing trust** (idempotent) so it can't silently drift. |
| `deploy.sh` | One command: rebuild → bundle → sign → replace `/Applications/Otto.app` → relaunch → verify. |
| `prune-target.sh` | Preview older hashed dependency artifacts under `target/*/deps/`; `--apply` opts into eviction. Deploy step 5 invokes it only with `PRUNE=1`. |
| `dmg.sh` | Package a signed `Otto.app` into a `.dmg`. |
| `publish-walkthroughs.sh` | Re-encode `marketing/videos/out/*.mp4` to 720p and upload them (`--clobber`) to the rolling `walkthroughs` GitHub release the in-app Walkthroughs page streams from. Not part of the DMG. |
| `com.otto.daemon.plist` | `launchd` user-agent template for the daemon (port `7700`). |

## Quick start

```bash
# First time on a new machine (creates + trusts the signing cert):
packaging/make-cert.sh

# Every deploy after that — one command:
packaging/deploy.sh
```

`deploy.sh` always rebuilds the frontend: the build receipt binds the installed
app to a fresh `ui/dist`, so `SKIP_UI=1` is rejected. It runs `npm ci` first when
`ui/package-lock.json` is newer than `ui/node_modules` (`--force-ci` forces it).
The repo-root `./deploy.sh` is a thin wrapper that forwards to
`packaging/deploy.sh` — there is one deploy implementation.

**Safety.** The signed build is staged to `/Applications/.Otto.app.staging.<pid>`
and verified before the running app is touched; the install is two same-volume
renames, so a killed deploy never leaves a half-copied `Otto.app`. If the
post-install verification fails (or the phase dies after the swap), the previous
app is renamed back and relaunched — its supervisor reinstalls the previous
daemon. A verified deploy keeps the previous app as
`/Applications/.Otto.app.previous` and the previous daemon as
`~/Library/Application Support/Otto/bin/ottod.prev`; a daemon that finds pending
schema migrations first snapshots `otto.db` to
`~/Library/Application Support/Otto/backups/` (newest 3 kept). One deploy runs at
a time (`~/Library/Logs/Otto/deploy.lock`). Before building, the script prints
how many live sessions the daemon restart will end and, on a terminal, asks to
continue (`--yes` / `OTTO_DEPLOY_YES=1` skips the prompt). Deploy logs beyond the
newest 20 (`KEEP_DEPLOY_LOGS`) are pruned, and exited `com.otto.deploy-once.*`
launchd jobs from the old root script are booted out.

**Running it from inside Otto (an agent/shell session):** steps 6–7 (install →
relaunch → verify) run as a one-shot launchd agent, `com.otto.deploy-finish`,
because the relaunch restarts the daemon, which hangs up every session PTY it
owns itself — a script running inline there can die with exit 137 mid-verify
(sessions kept in PTY holders survive, see below; the detached phase stays the
safe default). The
foreground only tails `~/Library/Logs/Otto/deploy-finish-<ts>.log`; if it gets
killed, the phase still completes. Read the outcome with
`packaging/deploy.sh --status` (exit 0 = deployed and healthy; non-zero when
the phase failed, is still running, or `/Applications/Otto.app` is missing). `DETACH=0`
runs the phase inline (fine from a plain Terminal).

**Disk:** both deploy entrypoints preserve Cargo caches by default (`PRUNE=0`).
Inspect `du -sh target apps/desktop/src-tauri/target` when disk space is tight.
Cargo reuses dependencies without updating their modification time, so an old
artifact can still belong to the current build.

`packaging/prune-target.sh` (or `--dry-run`) previews optional eviction;
`--apply` deletes selected older hashed dependency variants. `PRUNE=1` opts into
this cleanup after a deploy build. It keeps variants within `PRUNE_WINDOW_SECS`
(default 1h) of the newest dependency artifact, but this is a timestamp
heuristic, not a determination of which artifacts are unused. Eviction can
force recompilation on the next build.

Build-script outputs, fingerprints, incremental state, top-level binaries,
bundles and receipts are preserved. Run explicit cleanup only while builds are
idle; the helper's process check is best-effort. See
[build measurements](../docs/testing/build-performance.md) for the cache policy
and other build-time improvements.

## The recurring "enter your password" keychain prompt — why, and the fix

**Symptom:** macOS keeps popping *"ottod wants to use the confidential information
stored in your keychain — enter your password"*, and it **comes back after every
new build**, even though you click **Always Allow**.

**Why:** Otto stores all secrets (channel tokens, DB/SSH passwords, share tokens,
email creds…) in the login keychain under the service `com.otto.daemon`, and the
daemon reads them from background tasks. "Always Allow" binds the grant to the
requesting app's **code identity**:

- When the signing cert is **trusted for code signing**, macOS anchors a *stable*
  identity — `identifier "ottod"` + the *Otto Dev Signing* cert — which is
  **identical across every rebuild**. The grant survives rebuilds.
- When the cert is **not trusted**, macOS can't anchor that identity, so it falls
  back to pinning *this specific build's* signature. Every re-sign produces a new
  signature → the prior "Always Allow" no longer matches → the prompt returns.

So a recurring prompt that returns on each build = **the cert is not trusted for
code signing.** (This is separate from TCC/network/accessibility grants, which key
off the designated requirement and already persist.)

**The fix is automatic now.** `make-cert.sh` trusts the cert at setup, and
`sign.sh` re-asserts it on every build (idempotent — it only acts, and only
prompts once, when trust is missing). So `packaging/deploy.sh` always leaves the
cert trusted, and the prompt stops coming back.

### One-time cleanup after enabling trust
Grants recorded *before* the cert was trusted were pinned to an old signature, so
the **next** access of each secret may prompt once more — click **Always Allow**.
From then on it's anchored to the stable identity and won't return on rebuilds.

### Verify / fix manually
```bash
# Is the cert trusted for code signing?
security dump-trust-settings | grep -i "Otto Dev Signing"        # user domain
security dump-trust-settings -d | grep -i "Otto Dev Signing"     # admin domain

# Trust it by hand (what sign.sh does):
security find-certificate -c "Otto Dev Signing" -p > /tmp/otto.pem
security add-trusted-cert -r trustRoot -p codeSign \
  -k "$HOME/Library/Keychains/login.keychain-db" /tmp/otto.pem
```
Or in **Keychain Access → login → "Otto Dev Signing" → Trust → Code Signing:
Always Trust**.

**Scope/safety:** the trust is for one self-signed cert that signs only Otto, in
your own login keychain — nothing system-wide, no effect on other software. Keep
signing every build with the *same* cert (all scripts here do) so the identity
stays stable.

## Daemon won't start after a deploy: `OS_REASON_CODESIGNING` crash-loop

Symptom: after `deploy.sh`, the app launches but `curl localhost:7700/api/v1/health`
never responds, and:

```bash
launchctl print "gui/$(id -u)/com.otto.daemon" | grep -iE 'runs =|last exit'
#   runs = 60                       (climbing — respawning)
#   last exit reason = OS_REASON_CODESIGNING
```

The crash report (`~/Library/Logs/DiagnosticReports/ottod-*.ips`) says
`SIGKILL (Code Signature Invalid)` / namespace `CODESIGNING` / indicator
`Invalid Page` — **even though `codesign --verify <deployed ottod>` passes**.

**Why (older installed versions):** the app self-deployed `ottod` by overwriting
`~/Library/Application Support/Otto/bin/ottod` *in place*. If the launchd agent
(`KeepAlive`) already had that binary mapped and running, overwriting the file
mid-flight invalidates its mapped code pages → macOS kills it. The kernel then
caches code-signing validity **per inode**, so that inode is rejected on every
relaunch — a permanent loop. (Proof: a byte-identical copy at another path,
e.g. `cp … /tmp/ottod && /tmp/ottod`, runs fine.)

Current app supervisors already publish the daemon with an atomic rename.
The deploy script retains recovery for inodes poisoned by older versions.

**Fix (now automatic):** `deploy.sh` step 6 detects this and self-heals by
giving the deployed binary a **fresh inode** (atomic rename of a clean copy of
the signed bundle sidecar), then kickstarting the agent. To do it by hand:

```bash
BIN="$HOME/Library/Application Support/Otto/bin"
cp -f "/Applications/Otto.app/Contents/MacOS/ottod" "$BIN/ottod.fresh"
mv -f "$BIN/ottod.fresh" "$BIN/ottod"     # atomic → NEW inode, fresh CS evaluation
launchctl kickstart -k "gui/$(id -u)/com.otto.daemon"
curl -s localhost:7700/api/v1/health      # {"ok":true}
```


## Recovery: rolling back a bad deploy

A deploy leaves these recovery artifacts:

| Artifact | What it is | Written by |
|---|---|---|
| `/Applications/.Otto.app.previous` | The app the last **verified** deploy replaced | `deploy.sh` (`keep_previous_app`) |
| `/Applications/.Otto.app.old.<pid>` | The previous app, only while a deploy is mid-swap. A leftover means a finish job was killed between the two renames; the next deploy puts a lone copy back automatically, and `--status` names it | `deploy.sh` (`swap_app`) |
| `~/Library/Application Support/Otto/bin/ottod.prev` | The daemon binary the app's supervisor last **replaced** (only on a real byte change, never on a slow-boot reinstall) | the app supervisor |
| `~/Library/Application Support/Otto/backups/otto.db.pre-<version>-<UTC stamp>` | A `VACUUM INTO` copy of the state DB taken just before pending migrations ran (newest 3 kept, owner-only `0600`) | `ottod` at boot |

`deploy.sh` rolls back automatically when verification fails: it restores the
previous app, relaunches it, and reports success only once a **different**
daemon process, running the restored bundle's sidecar, answers `/health`
(`ROLLED BACK: … previous daemon verified`). Otherwise it prints
`previous daemon was NOT verified` — follow the manual steps below.

**The schema stays migrated.** Migrations are append-only and additive, and
an older `ottod` boots on a newer schema (it logs a WARN listing the newer
versions and leaves them in place). Restore a DB snapshot only when the new
build damaged data, not as part of an ordinary rollback: it discards
everything written since the snapshot.

**The app and the daemon roll back together.** At every launch the app's
supervisor installs its bundled sidecar whenever the bytes differ from
`bin/ottod`, so restoring `ottod.prev` alone is silently undone the next time
the (newer) app starts. Roll back the app, and the daemon follows.

Manual rollback:

```bash
# 1. Quit the app, then stop the daemon (launchd would respawn it otherwise).
osascript -e 'quit app "Otto"'
launchctl bootout "gui/$(id -u)/com.otto.daemon"

# 2. Restore the previous app (or the .Otto.app.old.<pid> copy --status names).
mv /Applications/Otto.app "/Applications/.Otto.app.failed.$(date +%s)"
mv /Applications/.Otto.app.previous /Applications/Otto.app

# 3. ONLY if the new build damaged data: restore the newest snapshot, and
#    remove the WAL/SHM files so SQLite can't replay the newer log onto it.
D="$HOME/Library/Application Support/Otto"
ls -1t "$D/backups"/otto.db.pre-*            # newest first
# Keep the damaged DB for forensics WITH its WAL/SHM, under one stamp — the
# newest commits (what you want to investigate) live in the -wal file.
TS=$(date +%s)
for f in otto.db otto.db-wal otto.db-shm; do
  [ -e "$D/$f" ] && cp "$D/$f" "$D/$f.broken.$TS"
done
cp "$D/backups/otto.db.pre-<version>-<stamp>" "$D/otto.db"
rm -f "$D/otto.db-wal" "$D/otto.db-shm"
chmod 600 "$D/otto.db"

# 4. Launch the restored app: its supervisor reinstalls ITS ottod and
#    bootstraps the launchd job. Verify:
open /Applications/Otto.app
packaging/deploy.sh --status
curl -s localhost:7700/api/v1/health      # {"ok":true}
```

## Staged local deployment from an Otto session

Deploy from a clean, committed checkout. To finish the build and signing before
interrupting the current daemon's sessions:

```bash
PRUNE=0 EMBED_UI=1 BUILD_ONLY=1 packaging/deploy.sh
# After the build succeeds, queue the existing detached installer and return:
PRUNE=0 EMBED_UI=1 OTTO_DEPLOY_PHASE=queue-finish FINISH_DELAY_SECONDS=15 packaging/deploy.sh
# After restart, inspect the durable result:
packaging/deploy.sh --status
```

`BUILD_ONLY=1` writes `apps/desktop/src-tauri/target/release/deploy-build.receipt`
with the source commit, signed app/daemon SHA-256 hashes, and embedded-UI mode.
Queuing checks that receipt against the current clean source tree and signed
artifacts; the detached worker checks again immediately before installation.
Rebuild if the source, artifacts, or `EMBED_UI` setting changes. `SKIP_UI=1` is
rejected so a receipt cannot label an older frontend as the current source. The optional
install delay accepts integers from 0 to 60 seconds. Queue mode returns after
launchd accepts the job; that means **queued**, not installed successfully.

The detached log records `DEPLOY-RECEIPT` only after the installed bundle matches
the build, its signatures verify, the deployed daemon matches the sidecar,
the app and daemon run from their installed paths, the daemon PID changes when
the binary changes, the health API returns success, and the embedded SPA can be
fetched when enabled. `DEPLOY-FINISH EXIT=0` marks successful completion;
`--status` returns that exit code. No provider session is created to verify the
installation. Replacing the daemon terminates the PTYs it owns itself — engine
sessions, and any session spawned with `session_persistence` off — including an
agent running this command; launchd and the durable log survive that interruption.

**Sessions kept across the restart.** With `session_persistence` on (the
default), sessions you start from the Agents page run in `ottod pty-holder`
processes (one per session, `setsid`, sockets in `<data dir>/pty-holders/`) and
keep running while the daemon is replaced; the new daemon re-adopts them on
boot. A holder keeps executing the binary it was started from (the old inode
stays mapped), so swapping `ottod` in place is safe for it, and the protocol
version handshake lets a newer daemon adopt it. The holders leave the launchd
job's process group, which is why neither plist sets `AbandonProcessGroup` —
see docs/features/agent-sessions.md → *Sessions survive daemon restarts*.

Run the portable regression suite with `python3 packaging/tests/test_deploy.py`.
It uses temporary files and mocks OS/process/network operations, without building,
signing, installing, or contacting a running daemon. CI runs this suite on Ubuntu.
