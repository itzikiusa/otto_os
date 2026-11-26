# Updates — Check for Updates and automatic download

Otto's desktop app updates itself from the project's GitHub releases. The
desktop shell (`apps/desktop/src-tauri/src/updater.rs`) checks, downloads,
verifies and stages a release; the UI only shows it and asks before the
restart. Nothing restarts on its own.

Who sees it: the **root** account, in the **desktop app**, on a **release
build**. The phone and the remote web never show any update UI, and a copy
built from source only says where its updates come from (git).

## Setup

Nothing to set up. A release build checks a minute after launch and then every
six hours. A source build (`packaging/deploy.sh`, `tauri dev`) never installs
anything: pull and run `packaging/deploy.sh` instead.

## Walkthrough

1. **A new version is out** — a quiet row appears in the sidebar footer above
   **Help**: **Update to 0.6.1**. In the collapsed rail it is one icon with a
   dot (its tooltip and accessible name say the same). Up to date, checking, a
   failed automatic check and a skipped version show nothing.
2. **Open it** — the row, **Otto → Check for Updates…** or **Settings →
   Updates → Check for updates** open the **Update** dialog: the version, its
   date and size, and the release notes.
3. **One button** that follows the state: **Download and install** →
   **Downloading… 42 %** (the sidebar row shows the same) → **Restart to
   finish**. **Later** closes the dialog; the download keeps going.
4. **Restart** — the dialog lists what keeps running and what ends, counted by
   the daemon (`GET /api/v1/system/restart-impact`): sessions by kind,
   connection terminals, background agents, IDE runs and debug sessions,
   containers, tunnels, phone/remote clients. When nothing ends the button is
   **Restart now** (one click). When something ends it reads **Restart and end
   3** and asks once more, naming what ends. If the count fails, it asks.
5. **After the relaunch** — a toast **Updated to 0.6.1**. If the new version
   did not start, the shell put the old app back; the row reads **Update failed
   and was undone · Details** and the dialog shows why, with **Show the update
   log** (reveals it in Finder). That version is skipped so it is not tried again.

**⋯ in the dialog**: **Skip this version** (the row hides it; a newer version
shows again), **View release on GitHub**, **Install updates automatically**
(download and prepare in the background, then show **Restart to update** — no
unattended restart).

**Otto → Check for Updates…** checks now and always answers: **You're up to
date**, the new version, **Couldn't check for updates** with the cause
(offline, GitHub unreachable) and **Retry**, or, on a source build, "This is a
source build — updates come from git".

**Settings → Updates** (System group; root; desktop app only): the running
version and channel, the last check, **Check for updates**, **Install updates
automatically**, and the skipped version with **Unskip**.

## Safety

- Release notes come from an unsigned manifest: they are rendered as untrusted
  markdown (allowlist sanitiser, no scripts, no remote images) and their links
  open in your browser.
- Verification (signature, signing identity, version, staging outside
  `/Applications`, atomic swap, rollback) is the shell's — see
  `apps/desktop/src-tauri/src/updater.rs`.

## Surface

| Piece | Where |
|---|---|
| State store (row label, dialog state machine, restart plan) | `ui/src/lib/stores/updates.svelte.ts` |
| Sidebar row / rail icon | `ui/src/shell/UpdateRow.svelte` (in `Navigator.svelte`, `Rail.svelte`) |
| Update dialog, per-window host | `ui/src/shell/UpdateDialog.svelte`, `UpdateHost.svelte` (lazy, `overlays.svelte.ts`) |
| Menu item | `check-updates` in `ui/src/lib/menu.ts` |
| Settings | `ui/src/modules/settings/UpdatesSettings.svelte` (`sections.ts` id `updates`, `desktopOnly`) |
| Shell commands | `update_state`, `update_check`, `update_download`, `update_restart`, `update_set_auto`, `update_skip`, `update_result_seen`; event `otto://update` (the finisher's log is revealed with the opener's `reveal_item_in_dir`) |
| Daemon | `GET /api/v1/system/restart-impact` (see `docs/contracts/api.md`) |

## Troubleshooting

- **No row although a release is out** — the account is not root, this is a
  source build, or the version is skipped (Settings → Updates → Unskip).
- **"Couldn't check for updates"** — the Mac is offline or GitHub is
  unreachable; **Retry**.
- **Update failed and was undone** — **Details → Show the update log**.
