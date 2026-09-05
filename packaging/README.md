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
| `prune-target.sh` | Delete superseded `<crate>-<hash>` build artifacts under `target/`, keeping the newest generation of each crate. Run by `deploy.sh` step 5; `--dry-run` reports without deleting. |
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

`deploy.sh` reuses an existing frontend build with `SKIP_UI=1 packaging/deploy.sh`.

**Running it from inside Otto (an agent/shell session):** steps 6–7 (install →
relaunch → verify) run as a one-shot launchd agent, `com.otto.deploy-finish`,
because the relaunch restarts the daemon and the daemon hangs up every session
PTY — a script running inline there dies with exit 137 mid-verify. The
foreground only tails `~/Library/Logs/Otto/deploy-finish-<ts>.log`; if it gets
killed, the phase still completes. Read the outcome with
`packaging/deploy.sh --status` (exit 0 = deployed and healthy). `DETACH=0`
runs the phase inline (fine from a plain Terminal).

**Disk:** step 5 prunes the build cache. Cargo never removes the artifacts of a
previous build — each changed feature set or dependency graph writes another
`<crate>-<hash>` file beside the old one, and with ~285 MB test binaries in this
workspace that reached **43 GB of unreachable duplicates against 6.5 GB of live
ones** before it showed up as a full disk.

It prunes by *generation*, not by hash: cargo keeps several hashes of the same
crate live at once (one per feature unification, plus host/build-script builds),
so "keep the newest hash" would delete live artifacts on every run and make the
next build recompile them. Instead it keeps every variant of a crate written
within `PRUNE_WINDOW_SECS` (default 1h) of that crate's newest artifact — the
co-live variants of one build — and drops the older generations. Running right
after the last `cargo` step means the generation kept is the one this deploy just
produced. Anything removed is simply rebuilt if it's ever needed again, and the
script no-ops if a build is in flight. `PRUNE=0` skips it.

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

**Why:** the app self-deploys `ottod` by overwriting
`~/Library/Application Support/Otto/bin/ottod` *in place*. If the launchd agent
(`KeepAlive`) already had that binary mapped and running, overwriting the file
mid-flight invalidates its mapped code pages → macOS kills it. The kernel then
caches code-signing validity **per inode**, so that inode is rejected on every
relaunch — a permanent loop. (Proof: a byte-identical copy at another path,
e.g. `cp … /tmp/ottod && /tmp/ottod`, runs fine.)

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
