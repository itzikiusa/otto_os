---
id: channels
title: Channels
group: Infrastructure
route: settings/channels
summary: Bridge a Slack thread, a Telegram chat or an inbound webhook to an Otto agent, and get the agent's reply back in the same place.
---
## What it's for

Channels let you talk to Otto from outside the app. A message in a Slack
thread or a Telegram chat starts (or continues) an agent session on your Mac.
While the agent works, Otto posts a live progress message; when it finishes,
Otto posts the reply — and any files the agent asks to share — back to the same
thread. A third channel, **Webhook**, lets another system trigger an agent with
an HTTP `POST` and optionally receive the reply at a callback URL.

Channels are set up per workspace in **Settings → Channels**. Each workspace can
have one Slack, one Telegram and one Webhook integration.

## Getting started

**Slack** (Socket Mode — no public URL or open port needed):

1. At api.slack.com/apps, create an app with **Socket Mode** turned on. Give the
   bot the `chat:write`, `channels:history`, `groups:history`, `im:history`,
   `mpim:history`, `files:read`, `files:write` and `app_mentions:read` scopes,
   and subscribe to the `app_mention` and `message.*` bot events. The full
   manifest is in `docs/features/channels-slack-telegram.md`.
2. Install the app to your Slack workspace and copy the **Bot User OAuth
   Token** (`xoxb-…`).
3. Under **Basic information → App-level tokens**, create a token with the
   `connections:write` scope and copy it (`xapp-…`).
4. In Otto, open **Settings → Channels**, choose **Edit** on the Slack card and
   paste the bot token and the app token. Optionally set a **Default channel
   ID** (`C…`) and **Allowed users** (`U…` ids).
5. Tick **Enabled** and choose **Save**. Within about 15 seconds the listener
   starts — no daemon restart needed.
6. Invite the bot to a channel (`/invite @Otto`), then mention it or post in a
   thread. Choose **Test** on the card to post an "Otto is connected" message to the
   default channel.

**Telegram** (long polling — no webhook needed):

1. In Telegram, message **@BotFather**, send `/newbot` and copy the bot token
   (`123456:ABC…`).
2. For group use, send `/setprivacy` to BotFather and choose **Disable** so the
   bot sees every message in the group.
3. In **Settings → Channels**, choose **Edit** on the Telegram card, paste the
   token, optionally add a **Default chat ID** and **Allowed users** (numeric
   ids), tick **Enabled** and choose **Save**.
4. Message the bot directly, or add it to a group.

**Webhook**:

1. Choose **Edit** on the Webhook card. Copy the **Inbound URL**
   (`…/api/v1/webhooks/<workspace id>`).
2. Paste your own key or choose **Generate**, then **Copy** it — it is masked
   once you save.
3. Optionally set a **Default reply callback URL** and **Allowed callers**.
4. Tick **Enabled** and choose **Save**. Callers `POST` JSON `{ "text": "…" }`
   with the header `X-Otto-Webhook-Key: <key>`.

## Everything it can do

**Conversations and sessions**
- One agent session per conversation: a Slack thread, a Telegram chat (or forum
  topic), or a webhook conversation. Follow-up messages go to the same agent.
- Otto finds the thread's agent again after a daemon restart, so a live thread
  keeps its agent.
- New sessions are titled from the first line of the opening message and show
  up in Agents like any other session.
- Each channel can pick a **Preferred CLI** (for example Claude Code or Codex);
  blank uses the workspace's default agent.
- Channel sessions are archived automatically after 1 hour without activity
  (checked every 10 minutes), never while the agent is working. The next
  message starts a fresh agent.

**What the chat sees**
- A progress message that Otto edits in place as the agent works: steps, a
  rotating "still working" header, and short previews of terminal commands.
  Telegram also shows "typing…".
- Your home folder is shortened to `~`, and secrets (tokens, passwords, keys,
  auth headers, JWTs, e-mails in the progress feed) are replaced with
  `[redacted]` before anything is posted.
- The final reply, formatted for the channel. A reply longer than about 1,800
  characters is posted as a short head plus an `investigation.md` attachment.

**Reply modes**
- **Off (default):** Otto posts the agent's whole final message.
- **Agent posts the final reply itself** (Slack/Telegram) or **Relay only the
  agent's marked reply** (Webhook): the agent marks exactly what to send with
  `⟦otto-send⟧ … ⟦/otto-send⟧`; unmarked replies are sent as-is. In this mode you
  can add **Reply instructions** (tone, format). Either way, Otto does the
  posting — the agent never uses the tokens.

**Files**
- Agent → chat on all channels: the agent references a file as
  `⟦otto-file⟧/absolute/path⟦/otto-file⟧` and Otto uploads it.
- Chat → agent on Slack: attached files are downloaded and the agent is told
  where to read them. Telegram only relays text.

**Quick commands** (Slack and Telegram; handled by Otto, no agent involved)

| Command | Effect |
|---|---|
| `/help` | List the commands |
| `/sessions` | List live agents started from this chat |
| `/who` | Show which session this conversation is mapped to |
| `/stop` | Stop the session bound to this chat or thread |
| `/new` or `/restart` | Detach the current session; the next message starts fresh |

**Launch other Otto work from a message**
- **Run with Otto:** `/run <Jira key | GitHub or Confluence URL | finding:/story:/test:/report:<id>>`,
  `/run <describe what you want>`, or "run with otto …". Reply `approve` or
  `reject` in the run's thread to resolve its approval gate.
- **Workflows:** `run <workflow name>: <prompt>`, a structured
  `Action: Workflow` message, or a chat binding set on the workflow. In a
  running workflow's thread, `status`, `skip` and `abort` control that run.
- **Swarm:** a swarm with a channel trigger (optional keyword and chat) launches
  from a matching message and reports progress back to the chat.

**Card actions**
- Enable or disable with the toggle, **Test** (needs a default chat), **Edit**,
  and **Remove** (deletes the tokens from the Keychain). Right-click a card for
  the same actions.

**Notifications to the default chat** (opt-in, in **Settings →
Notifications**): self-improvement events, code review finished, swarm finished,
insights report ready, and budget cap exceeded.

## Keyboard shortcuts

None specific to this page. See [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
for the global ones.

## Tips and limits

- **Tokens stay in the macOS Keychain.** The database keeps only a reference,
  and the UI only ever learns whether a token is set. Leave a token field blank
  to keep the existing one.
- **Slack needs both tokens.** Without the `xapp-` app token the Slack listener
  does not start. Bot token = `xoxb-`, app token = `xapp-`.
- **The bot must be in the channel.** Slack only delivers messages from
  channels the bot is a member of. For Telegram groups, turn privacy mode off
  and re-add the bot.
- **Messages from bots are ignored on Slack.** Any message carrying a bot id —
  Otto's own feed edits included — is skipped to prevent loops, so posts by
  other Slack bots do not reach an agent. Join/leave notices are ignored too.
- **Allowed users** is a comma-separated allowlist of channel user ids; other
  senders are dropped silently. Blank allows everyone in the chat. Actions that
  start runs, workflows or swarms run as the daemon's owner.
- **Webhook**: slash commands are not treated as quick commands. The endpoint is
  reachable wherever the daemon listens — loopback (`127.0.0.1`) by default.
  Callback URLs pass Otto's outbound guard, so loopback, private and cloud
  metadata addresses are refused. With no callback URL, a webhook only triggers
  the agent.
- **Shared files are confined.** Uploads must live under the session's working
  folder, `/tmp` or Otto's artifact folders; credential locations, `.git`
  internals, credential-like names (`.env`, `*.pem`, `id_rsa`) and files over
  20 MB are refused and noted in the thread.
- **Test and notifications need a default chat.** The relay itself replies
  wherever the message came from.
- **Permissions:** viewing integrations needs workspace viewer; editing,
  testing and removing need workspace editor.
- Edits take effect live: the supervisor re-scans about every 15 seconds.
- For one literal message to many live agents at once, use **Broadcast**
  (⌘⇧B) instead — it is not a channel and relays no replies.

## Related

- [Settings](#/walkthroughs/settings)
- [Agents](#/walkthroughs/agents)
- [Run with Otto](#/walkthroughs/run-with-otto)
- [Workflows](#/walkthroughs/workflows)
- [Swarm](#/walkthroughs/swarm)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
