# Feature — Proactive self-improvement → Telegram/Slack

Implements R1 from `docs/research/2026-06-19-hermes-vs-otto-self-improvement.md` §4: when the
self-improvement engine applies or queues an edit (or finishes a run), push a concise,
human-readable line to the user's configured channel(s) **immediately** — without the user
having to open the UI or have the improvement happen inside a channel-driven session.

## What it does

A notifier task subscribes to the same daemon event broadcast the WS uses, filters to the three
`Improvement*` events, formats one line, and posts it to the configured chat of every enabled
integration whose workspace matches the event's `workspace_id`.

Message formats (one line, names + counts only — no diff bodies, no secrets):

| Event | Line |
|---|---|
| `ImprovementEditApplied` | `💾 Self-improvement: skill \`<name>\` — applied` (memory targets → `memory \`<file>\``) |
| `ImprovementApprovalPending` | `📝 Self-improvement: proposed edit to skill \`<name>\` — needs approval` |
| `ImprovementRunFinished` | `🧠 Self-improvement run: <applied> applied, <pending> queued` |

The skill-vs-memory distinction is derived from `target_ref` shape (`*.md` ⇒ memory file, otherwise
a skill name), mirroring `otto-improve`'s `pathsafe.rs` rules. A `RunFinished` with `0 applied / 0
pending` is suppressed (nothing learned isn't worth a ping).

### Design note vs the brief

The event payloads carry `target_ref` (a skill name or memory filename) but **not** a separate
change-count. So the applied/pending lines name the target without an `(N changes)` suffix — the
real events don't expose that number, and inventing one would be wrong. Counts still appear in the
run-finished summary (`applied` / `pending`), which is where the engine actually reports them.

## Settings flag (opt-in, default OFF)

Gated on the existing key/value settings store (the same `SettingsRepo` used for `network_listener`
/ `providers`) — **no migration, no DTO field**:

- Key: `channels.notify_self_improvement`
- Type: bool, default `false` (missing / non-bool / read-error ⇒ off)
- Re-read **per event**, so toggling it takes effect live without a daemon restart.

### How to turn it on

Settings live in the key/value `settings` table, exposed by the root-only bulk
`PUT /api/v1/settings` endpoint (`routes/settings.rs`), which upserts every key in a flat
`{ "<key>": <value> }` body. Using the `otto-api` token:

```bash
curl -sS -X PUT "$OTTO_URL/api/v1/settings" \
  -H "Authorization: Bearer $OTTO_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"channels.notify_self_improvement": true}'
```

Set it to `false` to silence it. The notifier re-reads the flag per event, so no daemon restart is
needed either way. (There's no UI toggle for this key yet — it's a settings-store flag.)

## Target-chat resolution

Best-effort, MVP per the design doc: post to the integration's **configured default chat**
(`Integration.channel_id` — the UI field "Default channel ID" / "Default chat ID"), which is the
chat the bot already operates in. For each event:

1. `IntegrationsRepo::list_all_enabled()` → filter to the event's `workspace_id`.
2. Skip any integration whose `channel_id` is empty (logged at `debug`, no send).
3. Build the outbound adapter from the bot token in the secret store (same refs the manager uses:
   `chan-bot-<ws>-telegram` / `chan-bot-<ws>-slack`); skip if the token is missing.
4. `Adapter::send(channel_id, None, line)`.

If nothing is deliverable for the workspace, the event is skipped silently (`debug` log). Slack
posts to its bot's default channel; Telegram to the configured chat id. Threads aren't used (these
are top-level FYIs).

## Safety / non-blocking

- **Additive + off by default** — zero behavior change unless the flag is set.
- **Never blocks/crashes the engine or bus**: the notifier is its own `tokio` task; broadcast lag →
  log + skip (no panic); broadcast closed → task exits; a slow/failed channel send is logged at
  `warn` and swallowed. Nothing here can stall the improvement engine.
- **No secrets, no diff bodies** — only target names and applied/queued counts are posted.
- Shares the `ChannelManager`'s top-level cancel flag, so it stops on daemon shutdown.

## Files changed

- **`crates/otto-channels/src/improve_notify.rs`** (new) — the notifier: `spawn()`, the event
  loop, `render()` (event → line), `describe_target()`, `notify_enabled()`, `deliver()`,
  `build_adapter()`, plus 6 unit tests. Exposes `NOTIFY_SETTING_KEY`.
- **`crates/otto-channels/src/lib.rs`** — `pub mod improve_notify;`.
- **`crates/otto-channels/src/manager.rs`** — `ChannelManager` gains an
  `events: Option<broadcast::Sender<Event>>` field (and `new(...)` arg); `start()` spawns the
  notifier (via `events.subscribe()`) sharing the cancel flag when `events` is `Some`.
- **`crates/ottod/src/main.rs`** — pass `Some(events.clone())` into `ChannelManager::new` (the same
  `broadcast::Sender<Event>` the WS, usage and improvement engine already share).

No migration, no otto-core DTO change, no UI change, no improvement-engine change.

## Verification

| Check | Result |
|---|---|
| `cargo check --workspace` | ✅ pass (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ pass (exit 0, no warnings) |
| `cargo test -p otto-channels` | ✅ 14 passed / 0 failed (6 new `improve_notify` tests) |

New tests cover: applied skill edit → exact one-line string (and asserts no newline), applied memory
edit → `memory` wording, pending → approval wording, finished → counts, finished-with-no-changes →
suppressed, and non-improvement events (`Notice`, `ImprovementRunStarted`) → ignored.
