# Channels — Slack & Telegram bridges (+ Broadcast)

> **Summary.** A *channel* bridges a Slack or Telegram conversation to an Otto
> agent session. Inbound messages (and, for Slack, file attachments) are relayed
> into a live agent's terminal; the agent's working trail and final reply (and
> any files it produces) are relayed back to the same chat/thread. One agent per
> conversation thread, reused across follow-ups, auto-reclaimed when idle. The
> Slack side runs over **Socket Mode** (an outbound WebSocket — no public
> webhook URL, no inbound ports); the Telegram side runs over **long-poll
> `getUpdates`**. Bot/app tokens live in the **macOS Keychain**; the SQLite state
> DB stores only opaque key references plus presence flags. This page is the
> end-user + operator guide, including a **copy-pasteable Slack app manifest** and
> the full Telegram BotFather flow. **Broadcast** (a separate, AI-free feature)
> is documented in §6.

---

## 1. Overview

A channel integration is **per-workspace** and **per-channel** — each Otto
workspace can have at most one Slack integration and one Telegram integration.
You configure them in **Settings → Channels** (`ui/src/modules/settings/Channels.svelte`).

The runtime is the `otto-channels` crate. A long-lived supervisor
(`ChannelManager`, `crates/otto-channels/src/manager.rs`) scans every enabled
integration, resolves its token(s) from the Keychain, and spawns one listener
task per integration:

- **Slack** → `crates/otto-channels/src/slack.rs::run` — opens a Socket Mode
  WebSocket and forwards `message` / `app_mention` events.
- **Telegram** → `crates/otto-channels/src/telegram.rs::run` — long-polls
  `getUpdates` and forwards text messages.

Both feed a shared `Bridge` (`crates/otto-channels/src/bridge.rs`), which maps
the conversation to an agent session and pastes the message into its PTY. A
`Mirror` (`crates/otto-channels/src/mirror.rs`) tails the agent's transcript and
streams a rolling "🧠 working…" feed back, then posts the final reply.

```
Slack thread  ─Socket Mode WSS─┐
                               ├─▶ Bridge ──▶ SessionManager ──▶ agent PTY (claude/codex/…)
Telegram chat ─getUpdates poll─┘                │
                                                ▼
   chat  ◀────────── Mirror (tails transcript) ─┘
        "🧠 working…" feed (edited in place) → "🧠 done — N steps"
        + final reply (inline, or head + investigation.md when long)
        + any ⟦otto-file⟧ attachments the agent asked to upload
```

The supervisor re-scans every **15 s**. Because the generation signature
includes each integration's `updated_at` timestamp, **saving a token/config edit
in the UI takes effect live** — the affected listener is cancelled and respawned
without restarting `ottod`.

### Where it lives

| Concern | Location |
|---|---|
| Crate root / module list | `crates/otto-channels/src/lib.rs` |
| Supervisor (spawn/rescan/respawn) | `crates/otto-channels/src/manager.rs` (`ChannelManager`, `RESCAN_INTERVAL = 15s`) |
| Transport abstraction | `crates/otto-channels/src/adapter.rs` (`Adapter` trait, `Inbound`) |
| Slack adapter + Socket Mode listener | `crates/otto-channels/src/slack.rs` |
| Telegram adapter + long-poll listener | `crates/otto-channels/src/telegram.rs` |
| Inbound routing → session | `crates/otto-channels/src/bridge.rs` (`Bridge`, quick-commands) |
| Outbound feed / reply / files | `crates/otto-channels/src/mirror.rs` (`Mirror`) |
| Transcript tailer | `crates/otto-channels/src/transcript.rs` |
| Self-improvement → channel notifier | `crates/otto-channels/src/improve_notify.rs` |
| Loom bulk seed | `crates/otto-channels/src/seed.rs` |
| HTTP routes | `crates/otto-channels/src/http.rs` |
| Settings UI | `ui/src/modules/settings/Channels.svelte` |
| Broadcast endpoint | `crates/otto-server/src/modules.rs::workspace_broadcast` |
| Broadcast UI | `ui/src/lib/components/BroadcastModal.svelte` |
| Domain / request types | `crates/otto-core/src/domain.rs::Integration`, `crates/otto-core/src/api.rs::{UpsertIntegrationReq,BroadcastReq,BroadcastResp}` |
| Contract (REST) | `docs/contracts/api.md` — *Channel integrations* + *Orchestrator & broadcast* |

### Per-integration settings (the `Integration` record)

These map 1:1 to the **Settings → Channels** edit form and to
`UpsertIntegrationReq`:

| Field | Meaning |
|---|---|
| `enabled` | Listener runs only when this is on. |
| `bot_token` *(write-only)* | Slack `xoxb-…` / Telegram `123456:ABC…`. Stored in Keychain; never returned. |
| `app_token` *(write-only, Slack only)* | Slack Socket Mode `xapp-…` token. Stored in Keychain; never returned. |
| `channel_id` | **Default chat ID** — the chat the `/test` button and notifications post to (Slack `C…` channel id; Telegram numeric chat id). Not required for relaying inside a live thread. |
| `allowed_users` | Comma-separated **channel-native** user IDs allowed to drive the bot. Blank = everyone. (Slack `U…` ids; Telegram numeric user ids.) |
| `agent_reply` | `false` (default) → **Otto relays** the agent's final reply for you. `true` → the agent marks the exact text to send; the agent never posts on its own (Otto still does the posting). See §5. |
| `reply_instructions` | Free-text guidance injected into the trusted-context block (e.g. tone/format). Only surfaced in the UI when `agent_reply` is on. |
| `preferred_cli` | Agent CLI for this channel's sessions (`claude`, `codex`, `shell`, …). Blank → workspace default → global default → `claude`. |
| `has_bot_token` / `has_app_token` | Read-only presence flags (the only token info ever returned to the UI). |

---

## 2. The relay, in detail (read this before configuring)

What actually crosses the bridge — derived directly from `bridge.rs` and
`mirror.rs`:

1. **Inbound message arrives** (Slack `message`/`app_mention`, or a Telegram text
   message). A leading bot `<@U…>` mention is stripped on Slack.
2. **Allowed-users gate.** If `allowed_users` is set and the sender's id isn't in
   it, the message is silently dropped.
3. **Quick commands.** A message that starts with `/` may be a quick command
   (`/help`, `/sessions`, `/who`, `/stop`, `/new`, `/restart`) and is handled
   locally without touching an agent — see §5.
4. **Session resolution.** The bridge keys on `(workspace_id, chat, thread)`. If a
   live, non-archived session exists for that key it is **reused**; otherwise a
   new agent session is spawned (`SessionKind::Agent`) with the chosen
   `preferred_cli`, titled from the first line of the opening message, tagged
   `meta.source = "channel"`.
5. **Trusted-context wrapping.** The user's text is wrapped in an
   `⟦otto relay⟧ … ⟦/otto relay⟧` block that tells the agent where the message
   came from and how replies are handled. This block is *trusted context added by
   Otto* — the agent is told it is not user input. The agent is explicitly
   instructed **not** to read `.env`, run commands to post, or use any token to
   reply itself; Otto does all the posting.
6. **PTY submit.** The wrapped text is bracket-pasted into the agent's terminal
   and submitted with Enter. Otto waits for the TUI to settle, then confirms the
   prompt was dispatched (re-sending Enter once if it wasn't).
7. **Working feed.** The `Mirror` tails the agent transcript and posts a rolling
   `🧠 working…` message that it **edits in place** as tool calls happen
   (throttled to one edit per 2.5 s; trimmed to stay under the channel's message
   limit). Telegram additionally shows a "typing…" indicator every 4 s.
8. **Final reply.** On the agent's final message the feed is rewritten to
   `🧠 done — N steps`, and the reply is posted:
   - **Short reply** → posted inline (with channel-native formatting:
     Slack `mrkdwn`, Telegram legacy `Markdown`).
   - **Long reply** (> ~1800 chars) → a short head is posted and the full text is
     attached as `investigation.md`.
   - **Explicit file attachments** the agent wrote on disk are uploaded if it
     wraps the absolute path in `⟦otto-file⟧/abs/path⟦/otto-file⟧`.

**File attachments inbound:** **Slack** downloads every attached file (via
`url_private`, authenticated with the bot token) to a local temp path and tells
the agent where to read it. **Telegram inbound is text-only** — Telegram
documents/photos are *not* downloaded by the listener (only `message.text` is
forwarded). Outbound file uploads work on both channels.

---

## 3. Slack setup

Otto's Slack bridge uses **Socket Mode**. That means:

- **No public URL / no inbound port.** Otto opens an *outbound* WebSocket to
  Slack (`wss://…` obtained from `apps.connections.open`) and receives events
  over it. This is exactly why the daemon can stay **loopback-only** and still
  receive Slack messages.
- **Two tokens are required:**
  - a **Bot User OAuth Token** — `xoxb-…` — used for every Web API call
    (`chat.postMessage`, `chat.update`, file uploads, file downloads);
  - an **App-Level Token** — `xapp-…` — used *only* to open the Socket Mode
    connection (`apps.connections.open`). It carries the `connections:write`
    scope. If this token is missing, the Slack listener **does not start**
    (`manager.rs` logs *"app token missing (needed for Socket Mode), skipping"*).

### 3.1 Create the app from the manifest

1. Go to **https://api.slack.com/apps** → **Create New App** →
   **From an app manifest**.
2. Pick the workspace to install into → **Next**.
3. Paste the manifest below (YAML), review, **Create**.

The manifest enables Socket Mode, subscribes to exactly the events the listener
consumes (`message.channels`, `message.groups`, `message.im`, `message.mpim`,
`app_mention`), and requests exactly the bot scopes the code uses. Scopes not
strictly required by current code are commented inline and marked
`# OPTIONAL`.

```yaml
# Otto — Slack channel bridge.
# Paste at: api.slack.com/apps → Create New App → From an app manifest (YAML).
display_information:
  name: Otto
  description: Bridges a Slack thread to an Otto coding-agent session.
  background_color: "#1f2430"

features:
  bot_user:
    display_name: Otto
    # always_online keeps the bot showing as active while the Socket Mode
    # connection is held open. Cosmetic; safe to leave true.
    always_online: true

oauth_config:
  scopes:
    bot:
      # ---- REQUIRED by the code ----
      # chat.postMessage / chat.update — post the working feed + final reply,
      # and edit the rolling feed message in place (mirror.rs, slack.rs).
      - chat:write
      # Read message events in PUBLIC channels the bot is a member of
      # (handle_event consumes `message` events).
      - channels:history
      # Same, for PRIVATE channels the bot is in.
      - groups:history
      # Same, for 1:1 DMs with the bot.
      - im:history
      # Same, for group DMs (multi-person IMs).
      - mpim:history
      # files.getUploadURLExternal + files.completeUploadExternal — upload the
      # agent's files / investigation.md (slack.rs upload flow). Also used to
      # read `url_private` of inbound attachments (download with the bot token).
      - files:read
      - files:write

      # ---- OPTIONAL (recommended, not strictly required by current code) ----
      # Lets users @-mention the bot to start a thread; pairs with the
      # `app_mention` event below. The code DOES handle app_mention, so if you
      # subscribe to that event (as this manifest does) keep this scope.
      - app_mentions:read
      # Resolve human-readable names for users/channels in logs or future UI.
      # OPTIONAL — current code keys on raw ids only.
      # - users:read
      # - channels:read

settings:
  event_subscriptions:
    bot_events:
      # The listener forwards these two event types (slack.rs::handle_event):
      # everything else is acked and ignored.
      - app_mention          # @Otto … (start/continue a thread by mention)
      - message.channels     # messages in public channels the bot is in
      - message.groups       # messages in private channels the bot is in
      - message.im           # direct messages to the bot
      - message.mpim         # messages in group DMs the bot is in
  interactivity:
    # Not used by Otto (no slash commands / buttons). Leave disabled.
    is_enabled: false
  org_deploy_enabled: false
  # Socket Mode = outbound WebSocket. This is what lets ottod stay loopback-only.
  socket_mode_enabled: true
  token_rotation_enabled: false
```

> **JSON variant.** If you prefer JSON in the manifest editor, the same
> definition is:
>
> ```json
> {
>   "display_information": {
>     "name": "Otto",
>     "description": "Bridges a Slack thread to an Otto coding-agent session.",
>     "background_color": "#1f2430"
>   },
>   "features": {
>     "bot_user": { "display_name": "Otto", "always_online": true }
>   },
>   "oauth_config": {
>     "scopes": {
>       "bot": [
>         "chat:write",
>         "channels:history",
>         "groups:history",
>         "im:history",
>         "mpim:history",
>         "files:read",
>         "files:write",
>         "app_mentions:read"
>       ]
>     }
>   },
>   "settings": {
>     "event_subscriptions": {
>       "bot_events": [
>         "app_mention",
>         "message.channels",
>         "message.groups",
>         "message.im",
>         "message.mpim"
>       ]
>     },
>     "interactivity": { "is_enabled": false },
>     "org_deploy_enabled": false,
>     "socket_mode_enabled": true,
>     "token_rotation_enabled": false
>   }
> }
> ```

### 3.2 Install and collect the two tokens

1. **Install to the workspace.** In the app dashboard: **Install App** →
   **Install to Workspace** → **Allow**.
2. **Copy the Bot User OAuth Token.** **OAuth & Permissions** → *Bot User OAuth
   Token* → starts with **`xoxb-`**. This is the **Bot token**.
3. **Create the App-Level Token.** **Basic Information** → *App-Level Tokens* →
   **Generate Token and Scopes**:
   - Name it e.g. `otto-socket`.
   - Add the scope **`connections:write`**.
   - **Generate** → copy the token — starts with **`xapp-`**. This is the
     **App token**.
   (Creating the app from the manifest with `socket_mode_enabled: true` turns
   Socket Mode on; the `xapp-` token is what actually authorizes the connection.)

### 3.3 Paste the tokens into Otto

In Otto: **Settings → Channels → Slack → Edit**:

- **Bot token** → paste the `xoxb-…`.
- **App token (Socket Mode)** → paste the `xapp-…`.
- **Default channel ID** *(optional)* → the `C…` id of the channel you'll use the
  `Test` button / notifications against (right-click the channel in Slack → *View
  channel details* → copy the **Channel ID** at the bottom).
- **Allowed users** *(optional)* → comma-separated Slack user ids (`U…`).
- **Preferred CLI / Agent reply / Reply instructions** as desired (§5).
- Tick **Enabled** → **Save**.

On save, Otto writes the tokens to the macOS Keychain under the references
`chan-bot-{workspaceId}-slack` (bot) and `chan-app-{workspaceId}-slack` (app) and
returns only `has_bot_token` / `has_app_token` flags — **the tokens are never
sent back to the UI or stored in the DB.** Within ~15 s the supervisor starts the
Socket Mode listener (`"starting Slack Socket Mode listener"` in the daemon log).

### 3.4 Invite the bot and start a thread

A bot only receives `message.*` events from channels it is a **member** of:

- In the target Slack channel, type `/invite @Otto` (or *Add apps* from the
  channel's *Integrations* tab).
- Then either **@-mention** the bot (`@Otto investigate ticket FOO-123`) or just
  post in the channel/thread.

**Thread → session mapping.** The bridge keys on `(workspace, channel, thread)`.
Slack's `thread_ts` (falling back to the message `ts`) becomes the thread key, so
**each Slack thread gets its own agent session** and follow-up replies in that
thread go to the same agent. Start a new top-level message to get a new agent.

### 3.5 Verify

Use **Settings → Channels → Slack → Test** (requires a *Default channel ID*). It
posts **"Otto is connected ✅"** to that channel via `chat.postMessage`. A failure
surfaces the Slack API error string (e.g. `not_in_channel`, `invalid_auth`).

---

## 4. Telegram setup

Otto's Telegram bridge uses **long-poll `getUpdates`** (no webhook, no inbound
port — the daemon stays loopback-only). Only a single **bot token** is needed.

### 4.1 Create the bot with BotFather

1. In Telegram, open a chat with **[@BotFather](https://t.me/BotFather)**.
2. Send **`/newbot`**.
3. Give it a **display name** (e.g. `Otto`) and a **username** ending in `bot`
   (e.g. `my_otto_bot`).
4. BotFather replies with the **HTTP API token** — looks like
   `123456789:AAH...` . That's the **Bot token**.
5. *(Recommended for group use)* Send **`/setprivacy`** → select your bot →
   **Disable**. With privacy *enabled* (the default), the bot only receives
   messages that @-mention it or reply to it in groups; **disabling** lets it see
   all messages in groups it's added to. In a 1:1 chat the bot always sees your
   messages regardless of this setting.

### 4.2 Find the chat id (for the default chat / `Test`)

The relay itself doesn't need a chat id — it replies wherever a message came
from. You only need a **Default chat ID** to use the `Test` button and to receive
notifications (§7):

- **Easiest:** message your bot once, then open
  `https://api.telegram.org/bot<YOUR_TOKEN>/getUpdates` in a browser and read
  `result[].message.chat.id`. (Private chats are positive numbers; groups are
  negative, e.g. `-100…`.)
- Or add a helper like `@RawDataBot` / `@userinfobot` to the chat to print the id.

### 4.3 Paste the token into Otto

**Settings → Channels → Telegram → Edit**:

- **Bot token** → paste `123456789:AAH…`.
- **Default chat ID** *(optional)* → e.g. `-100123…` or your private chat id.
- **Allowed users** *(optional)* → comma-separated Telegram **numeric** user ids.
- **Preferred CLI / Agent reply / Reply instructions** as desired (§5).
- Tick **Enabled** → **Save**.

On save the token is stored in the Keychain under `chan-bot-{workspaceId}-telegram`
and only `has_bot_token` is returned. Within ~15 s the supervisor starts the
long-poll listener (`"starting Telegram listener"` in the daemon log).

### 4.4 Use it

Message the bot directly, or add it to a group and message there.
**Thread mapping:** Telegram forum-topic threads (`message_thread_id`) form part
of the session key, so each forum topic maps to its own agent; a chat with no
topics maps the whole chat to one agent.

### 4.5 Verify

**Settings → Channels → Telegram → Test** (requires *Default chat ID*) posts
**"Otto is connected ✅"** via `sendMessage`. A failure surfaces the Telegram
`description` (e.g. `chat not found`, `Unauthorized`).

---

## 5. Using a channel (lifecycle, reply modes, files, commands)

### Reply modes (`agent_reply`)

Either way, **the agent never posts to the chat itself and never touches tokens**
— Otto's bot does all posting. The flag only changes *what text* gets posted:

- **`agent_reply = false` (default).** Otto relays the agent's **entire final
  message** verbatim. Simplest; good for Telegram and quick Q&A.
- **`agent_reply = true`.** The agent may mark the exact text(s) to send by
  wrapping them in `⟦otto-send⟧ … ⟦/otto-send⟧` (multiple blocks allowed). If it
  marks none, its final message is sent as-is. `reply_instructions` (tone/format
  guidance) is injected into the trusted-context block in this mode.

### File attachments

- **Agent → chat:** the agent writes a file to disk and references it as
  `⟦otto-file⟧/absolute/path⟦/otto-file⟧` in its final message. Otto reads the
  raw bytes and uploads them to the thread (Slack external-upload flow / Telegram
  `sendDocument`). The directive markup is stripped from the posted text. A long
  inline reply is auto-attached as `investigation.md`.
- **Chat → agent (Slack only):** attached files are downloaded to temp paths and
  the agent is told where to read them. (Telegram inbound files are not relayed.)

### Lifecycle & idle auto-archive

- **One agent per `(workspace, chat, thread)`**, reused across follow-ups.
- A new message on a thread whose session has **exited or been archived** starts a
  **fresh** agent. (Sessions are auto-archived after idle by the session manager —
  see `./agent-sessions.md` — so a dormant thread cleanly re-spawns later.)
- A **respawn of the listeners** (e.g. after a token edit) does **not** drop live
  thread→session mappings: the `Bridge` and `Mirror` survive across listener
  generations.

### Quick commands (handled locally, no agent involved)

Send these as a message in the chat/thread:

| Command | Effect |
|---|---|
| `/help` | Show the command list. |
| `/sessions` | List this workspace's agent sessions + status. |
| `/who` | Show which session this conversation is mapped to. |
| `/stop` | Kill the session bound to this chat/thread and drop the mapping. |
| `/new` | Drop the mapping so the **next** message starts a fresh session. |
| `/restart` | Same as `/new` (next message starts fresh). |

---

## 6. Broadcast (separate feature — literal, AI-free fan-out)

**Broadcast is not a channel.** It sends **one literal message to many live agent
sessions at once**, with **no AI, no parsing, no orchestrator, and no fallback** —
the text is submitted to each target session's PTY exactly as if you typed it and
pressed Enter. It is deliberately separate from the ⌘K orchestrator.

- **UI:** the **Broadcast** composer (`ui/src/lib/components/BroadcastModal.svelte`).
  Type a message, pick which live agent sessions to hit (all eligible sessions are
  selected by default; only `running`/`working`/`idle` agents are eligible),
  **Broadcast**. `Enter` sends, `Shift+Enter` newlines.
- **Endpoint:** `POST /api/v1/workspaces/{id}/broadcast` (ws **editor**).
  - Request `BroadcastReq { text, session_ids? }`. `session_ids` absent/empty →
    **every live agent session** in the workspace; otherwise only the listed
    sessions that are live agents.
  - Response `BroadcastResp { session_ids }` — the sessions that **actually**
    received it.
  - Empty `text` → `400 Invalid`.
- **Difference vs channels:** channels bridge an *external* Slack/Telegram thread
  to *one* agent and round-trip the reply with the activity mirror and trusted
  context. Broadcast is an *internal* one-shot push to *many* agents with no reply
  relay and no context wrapping.

---

## 7. Self-improvement & event notifications → channels (opt-in)

The notifier (`crates/otto-channels/src/improve_notify.rs`) can push a concise,
one-line summary of select daemon events to the **default chat** of each enabled
integration (`Integration.channel_id`) — so someone watching only Slack/Telegram
sees activity without opening the UI. It runs alongside the channel supervisor
and shares its shutdown signal.

It posts **only names and counts** — never diff bodies, file contents, or
secrets. Every category is **opt-in and OFF by default**, re-read per event so a
toggle takes effect live. Settings keys (key/value store, boolean):

| Setting key | Fires on |
|---|---|
| `channels.notify_self_improvement` | Self-improvement edit applied / proposed-edit needs approval / run finished (with applied+queued counts). |
| `channels.notify_review_done` | Code review finished (`done` / `failed`). |
| `channels.notify_swarm_done` | Agent-swarm run reached a terminal state (completed / aborted / failed). |
| `channels.notify_insight_ready` | An insights report became available (**global** — delivered to every enabled integration). |
| `channels.notify_budget_exceeded` | A spend cap was crossed (the "exceeded" edge only). |

Delivery is best-effort: an integration with no `channel_id`, or a send failure,
is logged and skipped (never crashes the engine or the event bus). All except
`notify_insight_ready` are scoped to the event's workspace; `InsightReady` is
global. See `./self-improvement.md` for the self-improvement engine itself.

---

## 8. API / contract reference

Authoritative source: `docs/contracts/api.md` (*Channel integrations* and
*Orchestrator & broadcast* sections). Paths below are under `/api/v1`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `GET /workspaces/{id}/integrations` | ws viewer | — | `Integration[]` (presence flags only — no tokens) |
| `PUT /workspaces/{id}/integrations/{channel}` | ws editor | `UpsertIntegrationReq` | `Integration` (upsert; stores any provided tokens in Keychain) |
| `DELETE /workspaces/{id}/integrations/{channel}` | ws editor | — | `204` (also deletes the Keychain secrets) |
| `POST /workspaces/{id}/integrations/{channel}/test` | ws editor | — | `{ ok, error? }` — sends "Otto is connected ✅" to `channel_id` |
| `POST /workspaces/{id}/integrations/seed-from-loom` | ws editor | — | `Integration[]` — bulk-seed from a `.loom.env` (see §9) |
| `POST /workspaces/{id}/broadcast` | ws editor | `BroadcastReq {text, session_ids?}` | `BroadcastResp {session_ids}` |

`{channel}` is `slack` or `telegram`. `UpsertIntegrationReq` carries write-only
`bot_token` / `app_token` (omit or send `null` to keep the existing secret) plus
`enabled`, `allowed_users`, `agent_reply`, `reply_instructions`, `channel_id`,
`preferred_cli` (`crates/otto-core/src/api.rs`).

The same per-integration runtime config flows over the WebSocket only as session
events (the channel-spawned sessions appear like any other agent session); there
is no dedicated channel WS event. See `docs/contracts/ws.md`.

---

## 9. Capabilities & limitations

**Capabilities**
- Per-workspace Slack + Telegram bridges; live config edits (no restart).
- One agent per conversation thread; reuse across follow-ups; clean re-spawn after
  idle/archive.
- Rolling "working…" activity feed (edited in place), Telegram typing indicator.
- Two reply modes (Otto-relays vs agent-marks-text); long-reply auto-attach.
- Outbound file uploads (both channels) and inbound file download (Slack).
- Allowed-users allow-list; local quick commands.
- Opt-in event notifications to the default chat.
- Bulk seed from a Loom `.loom.env` file (`POST …/integrations/seed-from-loom`):
  reads `LOOM_<WORKSPACE>_SLACK_TOKEN` / `…_SLACK_APP_TOKEN` /
  `…_TELEGRAM_TOKEN` / `…_ALLOWED_USERS` (falling back to `LOOM_DEFAULT_*`) from
  `$OTTO_LOOM_ENV` or `~/claude_ade/.loom.env`. Seeded integrations are created
  **disabled** — an operator enables each manually.

**Limitations**
- **One integration per channel per workspace** (the key is `(workspace, channel)`).
- **Telegram inbound is text-only** — Telegram documents/photos sent *to* the bot
  are not relayed to the agent.
- **Slack requires Socket Mode** with both `xoxb-` and `xapp-` tokens; the Events
  API (HTTP webhook) path is not implemented.
- Telegram uses **long-poll**, not webhooks.
- The bot must be a **member** of a Slack channel to receive its messages; for
  Telegram groups, privacy mode must be disabled to see all messages.
- Default-chat-dependent features (`Test`, notifications) need `channel_id` set.
- The activity feed is throttled (~1 edit / 2.5 s) and trimmed to the channel's
  message size limit; very long investigations elide older steps in the feed.

---

## 10. Security

- **Tokens live in the macOS Keychain, never in the repo or the DB.** On
  upsert/seed, `bot_token`/`app_token` are written under opaque references
  (`chan-bot-{ws}-slack`, `chan-app-{ws}-slack`, `chan-bot-{ws}-telegram`); the
  SQLite state DB stores only those references plus `has_bot_token`/`has_app_token`
  flags. The REST API never returns a token. Deleting an integration deletes its
  Keychain secrets.
- **Secrets are scrubbed from logs/errors.** Slack's single-use upload URL and the
  Socket Mode WSS URL are redacted (`redact_url`); the Telegram bot token — which
  reqwest embeds in the request URL — is scrubbed from every error (`redact_token`)
  so it can't leak into a log line.
- **Least-privilege scopes.** The provided manifest requests only the bot scopes
  the code uses (`chat:write`, `*:history`, `files:read`, `files:write`,
  `app_mentions:read`) and subscribes to only the events it consumes. Don't add
  scopes the bot doesn't need.
- **Loopback by default — Socket Mode/long-poll are why.** Because Slack uses an
  **outbound** WebSocket and Telegram uses outbound long-poll, the daemon receives
  channel traffic **without** opening any inbound port. Do not switch the daemon to
  a network listener to "make Slack work" — it isn't needed and weakens the default
  posture (see `AGENTS.md`).
- **Trusted-context fencing.** Inbound text is wrapped in an `⟦otto relay⟧` block
  and the agent is instructed it is *trusted context, not user input*, and is told
  not to read `.env` or use tokens to post. Loop-prevention drops the bot's own
  Slack messages (including `message_changed` edits of the rolling feed) so the
  relay never feeds its own output back to the agent.
- **Access control.** Configuring integrations requires **ws editor**; reading the
  list requires **ws viewer**; the `allowed_users` list further restricts *who in
  the chat* may drive the bot.

---

## 11. Troubleshooting

**Slack listener never starts / "app token missing".** Socket Mode needs the
`xapp-` App-Level Token *in addition to* the `xoxb-` Bot token. Add it in
**Settings → Channels → Slack → Edit → App token** and Save. The daemon log shows
`starting Slack Socket Mode listener` once both are present and the integration is
enabled.

**Slack `invalid_auth` / `not_authed`.** The bot token is wrong/revoked, or you
pasted the app token into the bot field (or vice-versa). Bot = `xoxb-`,
App = `xapp-`. Re-copy from **OAuth & Permissions** / **Basic Information →
App-Level Tokens** and re-save.

**Slack bot doesn't respond in a channel.** It isn't a member. `/invite @Otto` in
that channel. Also confirm you subscribed to the right `message.*` event for the
conversation type (channel vs DM vs group) — the manifest covers all four.

**Slack `missing_scope` on the `Test` / feed.** The installed app lacks a scope.
Re-create from the manifest (it has the full set) or add the scope under **OAuth &
Permissions** and **reinstall** the app to the workspace.

**Socket Mode keeps disconnecting / reconnecting.** The listener reconnects with
exponential backoff (3 s → 60 s) on a closed socket, a server `disconnect`
envelope, or a transport error — this is normal and self-healing. Persistent
failure to even open a connection (`apps.connections.open not ok`) usually means
the `xapp-` token lacks `connections:write` or Socket Mode is disabled in the app
settings.

**Telegram bot silent in a group.** Privacy mode is on (default). In BotFather:
`/setprivacy` → your bot → **Disable**, then **remove and re-add** the bot to the
group for the change to take effect.

**Telegram `Unauthorized` / `chat not found`.** Wrong bot token, or a *Default
chat ID* the bot can't reach (it must share the chat — message the bot, or add it
to the group, first). Re-copy the token from BotFather; re-fetch the chat id via
`getUpdates`.

**`Test` button disabled / "No default chat ID configured".** Set the **Default
channel/chat ID** field. The relay itself works without it; only the `Test`
button and notifications require it.

**Reply posted twice, or the bot replies to itself.** Shouldn't happen — Slack
loop-prevention drops the bot's own messages and the mirror de-dups repeated Final
events. If you see it, check you don't have two integrations/bots installed in the
same channel.

**Config edit didn't take effect.** The supervisor re-scans every ~15 s; wait a
moment. Saving changes the integration's `updated_at`, which forces the listener
to respawn even when the enabled set is unchanged.

---

## 12. Related docs

- `./agent-sessions.md` — agent sessions (the thing a channel drives), idle
  auto-archive, restart/resume.
- `./self-improvement.md` — the self-improvement engine behind the
  `channels.notify_self_improvement` notifications.
- `docs/contracts/api.md` — authoritative REST contract (*Channel integrations*,
  *Orchestrator & broadcast*).
- `docs/contracts/ws.md` — WebSocket events (channel-spawned sessions surface as
  ordinary session events).
- `../MULTI-USER-RBAC.md` — the ws viewer/editor roles gating these endpoints.
- `../../README.md` — the feature tour.
