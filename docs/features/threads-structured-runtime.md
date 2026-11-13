# Threads — structured agent runtime (how T3 Code does it → how Otto mirrors it)

Status: new Claude, Codex, Gemini, Grok, Antigravity (`agy acp`), opencode
and pi threads use `/runtime-sessions` (§5), including Scratch. Other
harnesses and existing PTY sessions (older Antigravity threads included) keep
their terminal-backed view. Structured threads support recoverable
composer drafts, typed provider questions, fresh context in the same visible
thread, tool-result image previews and selected review/diff context. See the
[Threads walkthrough](threads.md) for the user-facing flows and limits.

Provider comparison citations below describe the historical T3 Code snapshot;
Otto's current wire contracts are [api.md](../contracts/api.md) and
[ws.md](../contracts/ws.md).

**Attribution.** Ported from [T3 Code](https://github.com/pingdotgg/t3code)
(MIT, Copyright (c) 2026 T3 Tools Inc.). The licence is reproduced in
`crates/otto-agent-protocol/NOTICE`, and every ported module says so in its
header. Citations are `path:line` in the clone studied (commit `cefb544af3`,
2026-11-02).

## Why

The terminal-backed Threads view runs the agent CLI in a PTY, types the prompt
into it, and rebuilds
the chat by tailing the CLI's transcript file (`transcript_tail.rs` →
`transcript_appended`). The CLI writes a transcript line only after a block
*completes*. The result is no token streaming, and tool calls, approvals and
"working" state are inferred after the fact.

T3 Code does not scrape a terminal. It drives each CLI over its structured
stdio protocol and renders a live timeline from one normalized event stream.
Otto ports that design rather than inventing its own.

## 1. Providers: what T3 Code runs → what Otto runs

| Provider | T3 Code runs | Otto runs |
|---|---|---|
| **Claude** | The **Claude Agent SDK** (`@anthropic-ai/claude-agent-sdk ^0.3.284`, `apps/server/package.json:37`), in-process: `query()` (`apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts:1090`). The SDK spawns the `claude` CLI (`binaryPath` setting, default `claude`, `packages/contracts/src/settings.ts:715`, passed as `pathToClaudeCodeExecutable`, `ClaudeAdapterV2.ts:1651-1653`) and speaks its stream-json control protocol. Options: `permissionMode` from the runtime mode (`:2573-2620`), `includePartialMessages: true` (`:1617`), the `claude_code` system-prompt preset (`:1660`), `sessionId` on the first run and `resume` afterwards (`:1578-1580`). The ACP adapter `claude-agent-acp` is used only as a Registry agent (`packages/provider-acp-registry/src/server/adapter.ts:195-210`); T3 Code bundles no npm ACP adapter (`packages/provider-acp-registry/package.json:35-42`) | **The same SDK transport, ported to Rust** (`crates/otto-agent-protocol/src/claude.rs`). It runs the installed `claude` directly with the SDK's argv (`sdk.mjs` `ProcessTransport.initialize`): `--output-format stream-json --verbose --input-format stream-json --permission-prompt-tool stdio --permission-mode default --include-partial-messages [--model m] (--session-id <uuid> \| --resume <id>)`, env `CLAUDE_CODE_ENTRYPOINT=sdk-ts`. No Node, and **no npm package is needed** |
| **Codex** | `codex app-server` (`["app-server", …launchArgs]`, `apps/server/src/provider/codexLaunchArgs.ts:12-15`). The binary is the `binaryPath` setting (default `codex`, `CodexAdapterV2.ts:1561`) or an optional managed install of 0.156.1 (`apps/server/src/provider/CodexInstallation.ts:61-98`). Env: `CODEX_HOME` (a shadow home that overlays `auth.json`, `Drivers/CodexHomeLayout.ts:11-77`) and `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` (`CodexProvider.ts:452-457`). Typed client: `packages/effect-codex-app-server`, generated from `openai/codex` `687a119f` (`scripts/generate.ts:16-19`) | `codex [context flags] app-server` (`src/codex.rs`). The context hook adds `--add-dir=…`, `-c developer_instructions=…` (top-level options `app-server` rejects after it) and `CODEX_HOME` (the Otto context bundle's shadow home), exactly as for a PTY codex session. No originator override. Wire shapes were checked against `codex app-server generate-json-schema` of the installed **0.166.0** |
| **Gemini** | No first-party adapter. It is the ACP Registry entry `gemini` (npm `@google/gemini-cli`, `AcpRegistrySupport.test.ts:1693-1720`). The registry is fetched from `cdn.agentclientprotocol.com` (`AcpRegistrySupport.ts:51`), npm-installed into T3's own prefix (`:1252-1294`), and launched with the registry's argv/env (`:1877-1917`). The exact flag comes from the live registry JSON and is not in the repo | `gemini --acp` (the installed CLI's own flag) plus `--model m`, into the ACP driver (`src/acp.rs`, `AcpDriver::gemini`) |
| **Grok** | `grok` from `binaryPath` (`packages/provider-grok/src/server/acpSupport.ts:67`). Argv per runtime mode (`:47-58`): `agent stdio`; `--permission-mode auto agent stdio`; `agent --always-approve stdio`; for approval-required, `--permission-mode default agent stdio`. Env `GROK_OAUTH2_REFERRER=t3code` (`:66-74`). `_meta` `{clientType:"extension"}` on initialize and `{cancelTrigger:"ctrl_c"}` on cancel (`:98-101`). Auth method `xai.api_key` when `XAI_API_KEY` is set, else `cached_token` (`:21-25`, `:77-81`) | `grok --permission-mode default [-m model] [--rules=<context>] agent stdio` (`AcpDriver::grok`). The context flags must come before `agent`: the subcommand rejects a trailing `--rules` (verified against grok 1.0.70). Same `_meta` and auth preference. Otto omits the T3-specific referrer and never launches the `auto` variant; full-access is `agent --always-approve stdio` |
| **Antigravity** | Only as the ACP Registry agent `antigravity-acp`, an npm adapter (`packages/client-runtime/src/providerClients.ts:39-43`) | **agy's own ACP server** (`AcpDriver::agy`): `agy acp [--dangerously-skip-permissions]`, the flag only for full access. Mode, model and Otto's context bundle go over the protocol (below). No npm adapter |

Installed CLIs it was built against: claude 2.1.318, codex-cli 0.166.0,
gemini (with `--acp`), grok 1.0.70 (`agent stdio`, ACP protocol v1), agy
1.5.8 (`acp`), opencode 1.19.5 (`acp`), pi 1.5.0 (`--mode rpc`).

**Claude, ACP or SDK?** T3 Code's own Claude provider is the SDK path. The
ACP adapter (`@agentclientprotocol/claude-agent-acp`) is only a Registry
agent, and it would need an npm install. Otto therefore mirrors the SDK path.
The SDK is a thin TypeScript layer over `claude`'s stdio protocol, so the
Rust port speaks that protocol to the CLI already on the machine. Otto
installs nothing.

### Live check (2026-11-04)

Each provider was run once through the full runtime: an isolated `ottod`
(temp data dir, port 7813), a scratch git repo, the prompts "reply with ok",
"create a file with a shell command" (answering the approval) and a long
count that was interrupted after the first delta, then `close`.

| Provider | Result | First delta |
|---|---|---|
| Claude (`haiku`) | all three turns as expected; the Bash approval arrived and was answered; interrupt ended the turn `interrupted` in ~0.1 s | 1.9–2.9 s (cold start included) |
| Codex | the same. The first run failed: the context hook's `--add-dir=` must precede `app-server`, so the subcommand is now a `tail_arg` | 1.8–5.3 s |
| Grok | text, reasoning, the tool and the interrupt all worked, but **no approval was asked**: with the user's own `~/.grok/config.toml` `[ui] permission_mode = "always-approve"`, `grok agent stdio` ignores `--permission-mode default` and `_meta.yoloMode: false`. It follows the user's grok config, as a PTY grok session does (T3 Code passes the same argv) | 2.1–4.8 s |
| Antigravity (`agy acp` 1.5.8, `gemini-3.8-flash-low`; 2026-11-13) | text, the context bundle (AGENTS.md rule obeyed, the indexed skill read on demand), a shell approval (`request_permission`, options `allow` / `deny`) answered and run **in the session cwd**, a full-access respawn (`--dangerously-skip-permissions` + `session/load` kept the history), `session/cancel` → `stopReason: cancelled`, and the prompt response's `usage` all worked | 2–6 s |
| Gemini | not run: the account is refused at `session/new` (`-32000`, "This client is no longer supported for Gemini Code Assist for individuals … migrate to Antigravity"). Each prompt fails with that message, and the next prompt respawns the CLI | — |

## 2. Session lifecycle

### Claude (SDK control protocol; `claude.rs`)

1. **Spawn and initialize.** Spawn with the argv above. Send
   `{type:"control_request", request_id, request:{subtype:"initialize"}}`
   (the SDK's `Query.initialize`). The prompt is queued until the
   `control_response` arrives. `system/init` reports the session id and
   model (`ClaudeAdapterV2.ts:3000`).
2. **Prompt.** Send `{type:"user", message:{role:"user", content:[{type:"text"}]},
   parent_tool_use_id:null}` (the SDK's `send`).
3. **Stream.** The CLI emits `stream_event`s. Text and thinking deltas grow
   one item per content block. The full `assistant` frame adds the
   `tool_use` blocks (text that was already streamed is not re-emitted), and
   the `user` frame's `tool_result` settles the tool. A call the CLI marks
   `tool_result_meta[].non_execution_kind: "interrupted"` (a new message or
   Stop cut it short) settles `interrupted`, shown as stopped, not failed;
   `"cancelled"` settles `cancelled` (T3 Code #34183).
4. **Permission round-trip.** The CLI sends
   `control_request {subtype:"can_use_tool", tool_name, input,
   permission_suggestions, tool_use_id}`, which becomes a pending runtime
   request plus an `approval_request` item. The answer is a
   `control_response {subtype:"success", response: PermissionResult}`,
   built by `permissionResultFromDecision` (`ClaudeAdapterV2.ts:3502-3545`):

   | Decision | Answer |
   |---|---|
   | accept | `allow` + the original input |
   | accept for session | `allow` + `updatedPermissions`: the suggestions re-targeted to `session`, else an `addRules` allow rule for the tool |
   | decline | `deny` |
   | cancel | `deny` + `interrupt:true` |

   Otto drops `setMode` suggestions, because a session-wide mode switch is a
   wider grant than the one tool approved. T3 Code drops them under a
   managed ceiling. A `control_cancel_request` (the CLI withdrew the
   question) closes the request `cancelled`.
5. **Interrupt.** Every open question is answered `cancel`, then
   `control_request {subtype:"interrupt"}` is sent (T3 Code:
   `queryRuntime.interrupt()`, `:1298-1300`). A Stop that lands while a
   prompt is still starting (its agent spawning or resuming) is kept and
   stops that turn as soon as it runs (T3 Code #34912).
6. **Turn end.** The `result` message, mapped by `terminalStatusFromResult`
   (`:3708-3738`):

   | `result` | Turn status |
   |---|---|
   | `terminal_reason` `aborted_*` | interrupted |
   | `success` without `is_error` | completed |
   | errors mentioning interrupt / cancel | interrupted / cancelled |
   | anything else | failed |

   Usage: input is `input` + cache read + cache creation, and output is
   `output` tokens. A turn that ends interrupted or cancelled settles the
   text still streaming as `interrupted`, not completed; on an
   `aborted_streaming` result the reply shown last is re-marked too
   (T3 Code #34597).

   Context meter: the last root message's `usage` (input + cache + output,
   not the turn's sum) against `result.modelUsage[<that message's
   model>].contextWindow`, the window this Claude process enforces
   (settings such as `CLAUDE_CODE_DISABLE_1M_CONTEXT` shrink it). Another
   model's entry never stands in (T3 Code #31370, open upstream).
7. **Tool → item.** `CLAUDE_KNOWN_TOOL_CLASSIFICATIONS` (`:2819-2860`):

   | Claude tool | Item type |
   |---|---|
   | Bash | `command_execution` |
   | Edit, MultiEdit, Write, NotebookEdit | `file_change` |
   | WebFetch, WebSearch | `web_search` |
   | anything else | `dynamic_tool` |
   | a root `TodoWrite` | the plan (`:9313`) |

   An `Agent` / `Task` call is a **`subagent` item** (a card) instead of a
   `dynamic_tool`. The subagent's own frames (`parent_tool_use_id` = that
   call) become items with `parent_item_id` = the card, in the card's turn:
   text from its complete `assistant` frames (its partial stream events would
   interleave with the root's, so they are skipped) and tool calls settled by
   its `user` tool results. Its permission questions reach a person and point
   at its own tool. T3 Code builds a separate subagent tree; Otto keeps one
   flat item list with parent links (`claude_tasks.rs`).

8. **Background work and the agent's own turns** (`claude_tasks.rs`, frames
   of Claude Code 2.1.318). `system` frames `task_started {task_id,
   tool_use_id, description, task_type, is_backgrounded}`, `task_progress`,
   `task_updated {patch: {status, is_backgrounded}}` and `task_notification
   {task_id, status, summary}` track the CLI's tasks; older CLIs are covered
   by the launch tool results ("Command running in background with ID: …",
   "Async agent launched … agentId: …"). A task counts as **background** when
   the CLI says so or its tool call asked for `run_in_background`; those are
   the provider session's `background_tasks` until they finish, then a
   `system_notice` ("Background command finished: …"). A background
   subagent's card stays `running` past its turn until its task ends.

   When a background task finishes after the turn, the CLI feeds its
   notification to the model and runs a turn **on its own**. Root output
   that arrives with no prompt of ours running opens an **agent turn**
   (`DriverEvent::TurnStarted` → `provider_turn.origin: "agent"`, no user
   message); its `result` ends it. It runs like any turn: the thread is
   working, Stop interrupts it, and a message sent meanwhile is queued, never
   interleaved. A session is busy while a turn runs **or** background work is
   pending (`latest_reply` / `PeerReply.busy`, `background_tasks`).

9. **Steering.** A message with `delivery: "steer"` is written to the CLI's
   stdin while the turn runs; the CLI hands it to the model at its next step
   (the SDK's streaming input). It is a `user_message` of the running turn.
   A model or effort picked with it switches first (`set_model` /
   `apply_flag_settings`), so the rest of the turn uses it (T3 Code #34564);
   the session's stored model follows on the next prompt.
   The other drivers do not steer; such a message is queued instead.
10. **Resume.** A resumed CLI (`--resume`) gets
   `MCP_CONNECTION_NONBLOCKING=0` unless the daemon's environment sets it:
   Claude Code then connects its MCP servers before the first request
   instead of telling the agent they all disconnected (T3 Code #34822).
11. **Escaped jobs.** Claude Code starts some jobs in a process group of
   their own, which the group kill misses. While the session is busy its
   process tree is recorded once a second (`ps`); when the process is
   gone, each recorded job still running (same pid and start time) is
   killed (`runtime/descendants.rs`, T3 Code #34505).

### Codex (`codex app-server`; `codex.rs`)

The framing is newline-delimited JSON-RPC **without** a `"jsonrpc"` member:
`{id, method, params}` (`packages/effect-codex-app-server/src/protocol.ts:106-110`,
`:517-537`).

1. **Initialize.** `initialize {clientInfo, capabilities:{experimentalApi:true,
   optOutNotificationMethods:["turn/diff/updated"]}}`, then the
   `initialized` notification (`CodexAdapterV2.ts:1763-1801`, `:233-237`).
2. **Thread.** Send `thread/start {cwd, model?, approvalPolicy, sandbox}`.
   With a known thread, send
   `thread/resume {threadId, excludeTurns:true, …}` instead (`:7061-7108`).
   If the resume fails, Otto starts a fresh thread. T3 Code unarchives an
   archived thread and retries (`:6725-6741`); Otto does not.
3. **Prompt.** `turn/start {threadId, input:[{type:"text", text}],
   approvalPolicy, sandboxPolicy}` (`buildCodexTurnStartParams`, `:744-818`).
4. **Runtime mode.** T3 Code's approval-required mode is
   `approvalPolicy:"untrusted"` + `sandboxPolicy:{type:"readOnly"}`
   (`:703-742`). When Otto's Seatbelt confines the process, the inner
   sandbox becomes `{type:"externalSandbox"}`, with thread-level
   `danger-full-access` inside the outer jail. A nested Seatbelt fails, and
   PTY codex drops its own sandbox under Otto's in the same way
   (`otto-sessions` `interaction_mode_apply`). Approvals still go to a
   person.
5. **Stream** (`:4364-4955`).

   | Codex notification | Result |
   |---|---|
   | `item/started`, `item/completed` | Upsert one item per Codex item: `agentMessage`, `reasoning`, `commandExecution`, `fileChange`, `mcpToolCall` / `dynamicToolCall`, `webSearch`, `contextCompaction` |
   | `item/agentMessage/delta`, `item/reasoning/summaryTextDelta`, `item/reasoning/textDelta` | Grow the text |
   | `item/commandExecution/outputDelta` | `command_output.delta` |
   | `turn/plan/updated` | The plan |
   | `thread/tokenUsage/updated` | Usage (`last` → turn, `total` + `modelContextWindow` → context) |
   | `mcpServer/startupStatus/updated` `failed` | A `system_notice` "MCP server <name> failed to start" with Codex's error, once, in the running turn or the next one (T3 Code #34771) |
   | `error` with `willRetry:false` | Kept as the failure |
   | `turn/completed` | `completed`, `interrupted` or `failed` (`mapCodexTurnStatus`, `:359-372`) |

   Item status follows `codexItemStatus` (`:392-414`): `inProgress` →
   running, `declined` → cancelled.

   Effort rides `turn/start`, so changing it never resumes the loaded thread
   (T3 Code #32915), and Otto never sends a reasoning `summary`, so Codex
   uses the user's own `model_reasoning_summary` (T3 Code #35089).
6. **Approvals** (`:5277-5640`).

   | Codex request | Kind | Answer |
   |---|---|---|
   | `item/commandExecution/requestApproval` | command | `{decision: accept \| acceptForSession \| decline \| cancel}` |
   | `item/fileChange/requestApproval` | file-change | same as above |
   | `item/permissions/requestApproval` | permission | the requested profile with scope `turn` / `session`, or an empty profile (`:609-621`) |

   `serverRequest/resolved` closes a request that Codex settled itself.
   `item/tool/requestUserInput` produces a typed question request (see §3).
   Its answers preserve native question IDs and arrays of selected values;
   skip returns an empty native answer map. Other unsupported server requests,
   including Codex elicitations, client-executed dynamic tool calls and auth
   refresh, get JSON-RPC `-32601`. Completed MCP/dynamic tool result items are
   displayed independently of that unsupported server-call mechanism.
   The v1 `execCommandApproval` / `applyPatchApproval` (`:576-590`) belong
   to the conversation API that Otto does not use.
7. **Interrupt.** Open requests are answered `cancel`. Then Otto
   sends `turn/interrupt {threadId, turnId}` (`:7567`); if the Codex turn id
   is not known yet, it is sent once `turn/start` answers.

### ACP: Gemini and Grok (`acp.rs`)

T3 Code's ACP runtime is `packages/provider-acp/src/server/AcpSessionRuntime.ts`
plus `adapter.ts`.

1. **Initialize.** Send `initialize` with **no fs and no terminal**
   capabilities. T3 Code advertises none for Claude-ACP, Gemini or Grok
   (`AcpSessionRuntime.ts:1789-1803`, `adapter.ts:2297-2306`). Otto answers
   any `fs/*` / `terminal/*` request with `-32601`. T3 Code prefers the ACP
   v2 preview and falls back to v1 (`packages/effect-acp/src/client.ts:679-703`).
   Otto speaks **v1**, which both installed agents answer.
2. **Session.** With a known id, send `session/load` if the agent advertised
   `loadSession`; otherwise send `session/new {cwd, mcpServers:[]}`
   (`AcpSessionRuntime.ts:2217-2260`). A failed load starts a fresh session
   (`adapter.ts:6997-7019`). Otto drops the history the agent replays
   during `session/load` (`AcpSessionRuntime.ts:1736-1765`).
3. **Auth.** Authentication is **lazy**. Only after `session/new|load` fails
   with `-32000` does Otto send `authenticate {methodId}`, with the
   flavour's preferred advertised method or else the first `agent`-type
   method, and then retry once (`:1065-1078`, `:2164-2208`). Terminal and
   env-var methods are never sent. A second failure is reported, and the
   person signs in with the CLI.
4. **Prompt.** `session/prompt`; the response carries `stopReason`.
   `agent_message_chunk`s with the same `messageId` append to one message,
   also when a thought or another message came in between: it keeps its
   streamed text and its place (T3 Code #33604). An opencode model written
   as a config option that failed for any reason but a refusal (`-32602`)
   is written again before the next prompt (T3 Code #34782).

   | `stopReason` | Turn status |
   |---|---|
   | `cancelled` | interrupted |
   | `end_turn`, `max_tokens`, `max_turn_requests`, `refusal` | completed (the last three also as `reason`) |

   T3 Code: `adapter.ts:8181-8230`.
5. **Permission round-trip** (`adapter.ts:6353-6527`).
   `session/request_permission` becomes a pending request plus an
   `approval_request` item. The answer uses `selectPermissionOptionId`
   (`adapter.ts:1255-1266`):

   | Decision | ACP option kind |
   |---|---|
   | accept | `allow_once` |
   | accept for session | `allow_always` |
   | decline | `reject_once` |
   | cancel, or no matching option | `{outcome:"cancelled"}` |

   The answer is never a wider grant than asked for.
6. **Interrupt.** Every open permission is answered `cancelled`, then
   `session/cancel` is sent (`AcpSessionRuntime.ts:2366-2405`;
   `adapter.ts:5754-5790`).

### Antigravity (`agy acp`; `AcpDriver::agy`)

Probed against agy 1.5.8 by speaking ACP from a script, then end to end
through the driver. It is a plain ACP v1 agent:

1. **Handshake.** `initialize` advertises `loadSession`, image and
   embedded-context prompts, `sessionCapabilities {resume, close, load, list,
   additionalDirectories}`, `_meta.antigravity.systemPrompt.append` and four
   auth methods (`oauth-personal`, …; a signed-in CLI never asks).
   `session/new|load` return `models {availableModels, currentModelId}`,
   `modes {currentModeId: "default", availableModes: default, accept-edits,
   auto, plan}` and `configOptions` (model family, thought level, mode).
2. **Model.** `session/set_model {modelId}` with the full id
   (`gemini-4-argon-high`); `agy acp` has no `--model`. Sent once the
   session is ready, so a switch respawns + loads.
3. **Access.** approval-required / auto-accept-edits are `session/set_mode
   default` / `accept-edits`, sent right after the session opens when it is
   not already in that mode, and live afterwards. Full access is the launch
   flag `--dangerously-skip-permissions` (`auto` still asks before commands
   that leave agy's sandbox), so entering or leaving it respawns and loads.
   An unknown mode is `-32602`.
4. **Context bundle.** agy (PTY, print mode and ACP alike) runs a shell
   command in the alphabetically-first workspace root, whatever the session
   cwd: with `--add-dir=~/.otto/context/agy/<cwd>` (or
   `additionalDirectories`) every command ran inside the bundle. So the
   bundle is no workspace root: otto-context names it in
   `OTTO_AGY_CONTEXT_DIR`, the driver takes that out of the spawn's env
   (`StructuredDriver::adopt_spec`, on a blocking thread) and sends the
   bundle's `AGENTS.md` plus an index of its skills (name, description,
   absolute `SKILL.md`) as `session/new|load`
   `_meta.antigravity.systemPrompt.append`. A PTY agy gets no bundle at all
   (correct cwd over Otto's context; its own `~/.gemini` rules and skills
   still load).
5. **Stream.** `agent_thought_chunk`, `agent_message_chunk`, `tool_call` /
   `tool_call_update` (`kind: execute` with `rawInput.CommandLine`; a denied
   tool ends `failed` with empty content), `plan`; also
   `available_commands_update`, `usage_update`, `session_info_update` and an
   echoed `user_message_chunk`, which Otto ignores. `session/load` replays
   the history (dropped). The prompt response carries a draft `usage`
   (`inputTokens` excludes `cachedReadTokens`), reported as the turn's usage.
6. **Interrupt.** `session/cancel` → the prompt answers `stopReason:
   cancelled` at once; the process stays up.

### opencode (`opencode acp`) and pi (`pi --mode rpc`)

Verified live on 2026-11-05 against **opencode 1.19.5** (npm `opencode-ai`)
and **pi 1.5.0** (npm `@earendil-works/pi-coding-agent`), by speaking each
protocol from a script.

**opencode** runs through the ACP driver (`AcpDriver::opencode`):

- `initialize` reports `loadSession: true`, stdio + HTTP MCP and image /
  embedded-context prompts. `session/new {cwd, mcpServers}` returns the
  `ses_…` id and one config option, `model`. `session/set_config_option
  {configId: "model", value: "provider/model"}` selects the model. There is
  no model flag and no modes.
- A stdio `mcpServers` entry with its `env` reached the server: a probe tool
  returned its env secret to the model.
- `session/load` with the earlier id remembered the earlier turn. The replayed
  history is dropped.
- The access mode is the `OPENCODE_PERMISSION` env. opencode deep-merges this
  JSON object over the user's config:

  | Mode | `OPENCODE_PERMISSION` |
  |---|---|
  | full access | `{"*":"allow"}` |
  | auto-accept edits | `{"*":"ask", "read"/"glob"/"grep"/"list"/"edit": "allow"}` |
  | approval required | the same without `edit` |

  A bare string such as `"allow"` makes `session/new` fail with "OpenCode
  service failure (directory)". Without the env, opencode allows everything,
  so every mode sets it. In approval-required mode the bash call arrived as
  `session/request_permission` with the options `once` / `always` /
  `reject`.
- A non-JSON line can appear on stdout (`[auto-update-checker] …`). The
  driver skips it.
- Free models (`opencode/big-pickle`, `…-free`) work without a login.

**pi** runs through its own RPC mode (`PiDriver`):

- `get_state` reports `data.sessionId` (a UUID) and `data.model`. A prompt
  answers `{disposition: "started"}`. Events then follow:
  `message_update.assistantMessageEvent` (`text_delta`, `toolcall_*`),
  `tool_execution_start|end {toolCallId, toolName, args, result.content,
  isError}`, `agent_end {messages, willRetry}`, then `agent_settled
  {aborted}`.
- A turn ends at `agent_settled` once the process has sent one. pi 0.x has
  no `agent_settled`, so there the turn ends at a `willRetry: false`
  `agent_end`.
- A provider failure is not a refused prompt. The prompt is `started`, and
  the run ends with the last assistant message's `stopReason: "error"` +
  `errorMessage`. Seen live: an expired openai-codex OAuth refresh and the
  OpenCode Zen free-tier 429.
- Resume is `--session <id>`.
- Thinking is `set_thinking_level` (`off` … `xhigh`, `max` in 1.x).
- No approvals: the access mode is the tool set. Restricted modes add
  `--tools … --exclude-tools mcp__*`, because MCP tools can write and pi
  cannot ask first.

**pi MCP.** pi 1.x reads `~/.pi/agent/mcp.json` and, in trusted projects
only, `.pi/mcp.json`. Otto edits neither. For each spawn it writes a private
extension, `<tmp>/otto-mcp/<session>.pi-mcp.js` (mode 0600), and passes
`-e <file>`. The extension calls `pi.registerMcpServer(name, config)` for:

- `otto`;
- `otto-browser`, when the session opts in;
- the workspace's enabled user servers.

These registrations last only for that session. Details:

- **Exposure.** Each server is `deferred`, so pi's `tool_search` loads its
  tools on demand.
- **Credentials.** `otto` and `otto-browser` carry no env. pi starts stdio
  servers with its own environment, which holds the session's Otto
  credential.
- **User env.** User-server env values are escaped for pi's interpolation
  (`$` → `$$`, `!` → `$!`).
- **Names.** Names pi refuses, or duplicates once `-` and `_` count as the
  same, are listed in `mcp_skipped`.
- **Cleanup.** The file is removed with the session's credentials.

Terminal pi sessions get the same `-e` file.

Verified live with real pi 1.5.0 and a scripted OpenAI-compatible model:

- `tool_search` loaded `mcp__otto__otto_list_workspaces` from the real
  `ottod mcp-tools`. The call returned the workspaces using the
  credential inherited from the environment.
- A probe server received its escaped env (`s3cr$$et$!x` → `s3cr$et!x`).
- The pi TUI started with `--session-id … -e …` and connected the server.

The model had to be scripted because neither of pi's own providers had a
working login on the test machine.

**Terminal sessions** (`crates/otto-sessions/src/providers.rs`):

- **pi:** `pi --session-id <otto sid> [--model p/m] [--thinking level]`. pi
  1.x creates the cwd's session with that id, or reopens it, so the same
  argv starts and resumes it. pi 0.x has no `--session-id`.
- **opencode:** `opencode [--auto] [--model p/m]`. `-s <id>` only opens an
  existing session (an unknown id is "Session not found"), and opencode mints
  its own ids. `-c` would resume the cwd's most recent session and cross-wire
  sessions that share a folder. So terminal opencode sessions are **not
  resumable** and start fresh on every respawn. The ACP path resumes
  exactly.
- **Models:** `opencode models` lists one `provider/model` per line. It takes
  about 2 s warm, so its probe gets 10 s. `pi --offline --list-models` prints
  a `provider model context …` table, read as `provider/model` ids.

### Access modes

The session's `access_mode` is T3 Code's `RuntimeMode` without `auto`:
`approval-required`, `auto-accept-edits`, `full-access`. The creation default
follows Settings › Providers: full access while skip-permission prompts is
enabled, approval-required otherwise. Each driver passes it the way T3 Code does (table in `docs/contracts/api.md`
"Structured agent runtime"): Claude's `--permission-mode` (`default` /
`acceptEdits` / `bypassPermissions` + `--allow-dangerously-skip-permissions`,
`ClaudeAdapterV2.ts:2573-2681`), switched live with the control protocol's
`set_permission_mode`; Codex's per-turn `approvalPolicy` + `sandboxPolicy`
(`codexRuntimeModeTurnDefaults`, `CodexAdapterV2.ts:703-742`, the sandbox
kept `externalSandbox` under Otto's Seatbelt); Grok's argv
(`grokAcpSpawnArgs`, `acpSupport.ts:47-58`; no accept-edits variant, so that
mode asks); Gemini's `--approval-mode` and live `session/set_mode` when
advertised; agy's `session/set_mode` plus its launch-only full access. A driver that cannot switch
live (`StructuredDriver::switches_access_live`) is respawned in the new mode
and resumes its thread, as for a model switch. Otto does NOT port T3 Code's
client-side auto-answering (`clientPolicy.ts:143-200`): whatever the CLI
still asks reaches a person. Otto's Seatbelt, folder trust and prompt guard
apply in every mode.

## 3. The normalized event model

T3 Code's adapters emit `ProviderAdapterV2Event`
(`packages/provider-core/src/server/ProviderAdapter.ts:84-182`). These are
**entity upserts**, not deltas: an item is re-sent whole, with its
accumulated text and a `streaming` flag. The ingestor sequences and persists
them as domain events (`apps/server/src/orchestration-v2/ProviderEventIngestor.ts:464-640`).

`otto_core::agent_runtime` is that model. Event `type` tags are T3 Code's;
fields are snake_case (Otto's wire convention):

| T3 Code event | Otto payload |
|---|---|
| `provider_session.updated` | `provider_session {status: starting\|ready\|running\|waiting\|stopped\|error, cwd, model, updated_at, last_error}` (`orchestrationV2.ts:950`) |
| `provider_thread.updated` | `provider_thread {native_thread_id, context_usage?, updated_at}` (`:1074-1083`) |
| `provider_turn.updated` | `provider_turn {id, ordinal, status: pending\|running\|completed\|interrupted\|failed\|cancelled, started_at, completed_at, turn_token_usage?}` (`:1168-1189`) |
| `turn_item.updated` | `turn_item {id, turn_id, ordinal, status, title, started_at, completed_at, updated_at, error?, type, …}` (`error`: a failed tool's text, ≤ ~2 KB); `type` is one of the item types below (`:1495-1819`) |
| `runtime_request.updated` | `runtime_request {id, turn_id, kind: command\|file_read\|file_change\|mcp_elicitation\|permission\|question, status: pending\|resolved\|expired\|cancelled, response_capability: {type: live\|not_resumable, reason?}, created_at, resolved_at, decision?, cancel_reason?, questions?, question_title?, answers?}` (`:1205-1231`; question fields are Otto additions) |
| `plan.updated` | `plan {id, turn_id, steps[{id, text, status: pending\|running\|completed}], explanation?}` (`:1320-1370`) |
| `command_output.delta` | `{item_id, chunk}`. The only delta, never persisted |
| `turn.terminal` | `{turn_id, status: completed\|interrupted\|cancelled\|failed, failure?, reason?}` |

Turn item types:

| Item type | Fields |
|---|---|
| `user_message` | `text`, `attachments?[{id, kind: image\|file, mime, name?}]` (bytes at `GET /runtime-sessions/{id}/attachments/{attachment_id}`) |
| `assistant_message` | `message_id`, `text`, `streaming` |
| `reasoning` | `text`, `streaming` |
| `todo_list` | |
| `file_change` | `file_name`, `diff_str?`, `old_str?`, `new_str?` |
| `command_execution` | `input`, `output?`, `exit_code?` |
| `dynamic_tool` | `tool_name`, `input`, `output?`, `images?[{mime, data_base64}]` |
| `web_search` | `patterns` |
| `approval_request` | `request_id`, `request_kind`, `prompt?`, `reason?`, `options[{decision, label}]`, `tool_item_id?`, `cancel_reason?` (`agent` / `interrupt` / `closed`) |
| `session_boundary` | No extra fields; rendered as **New session** before the accepted fresh prompt |
| `system_notice` | `message` |
| `error` | `failure` |

Approval decisions (`ProviderApprovalDecision`, `providerPolicy.ts:58-64`)
are `accept | accept_for_session | accept_always | decline | cancel`.

**Envelope.** Each event carries `{session_id, sequence, ts}` flattened
beside the payload.

**Otto additions.** `queue.updated {queued_prompts}` (the session's queued
messages, whole), `provider_session.background_tasks`,
`provider_turn.origin`, the `subagent` item and `parent_item_id`. T3 Code
carries subagents and background work as their own entities
(`subagent.updated`, `background_work.updated`); Otto folds them into the
session and the item list.

**Not ported** (no Otto consumer yet): `app_thread.created`,
`node.updated`, and the item
types for checkpoints, handoffs, forks and file_search. Typed questions live
on `runtime_request.updated` with `kind: "question"`; they do not require a
separate user-input turn item. `message.updated` is also not ported, because the
`assistant_message` item carries the same text.

**Who owns what.** In T3 Code, adapters own their processes and the
orchestration layer sequences, persists, and turns a failed terminal into an
`error` item. Otto splits the work the same way, but its drivers are
**sans-IO** (`StructuredDriver`, `driver.rs`). The runtime owns the child,
so every spawn passes Otto's safety pipeline. It also owns turns (ids,
ordinals, status), item ordinals, sequencing, request resolution and
cancellation, and the process-death terminal. A dead process fails the turn
as `transport_error`, as in T3 Code (`adapter.ts:2245-2277`). A driver
returns `DriverStep {events, writes, signal}`.

**Coalescing.** T3 Code flushes Codex deltas every 50 ms
(`CodexAdapterV2.ts:212`) and coalesces live thread events over a 50 ms
window (`ThreadLiveEventCoalescer.ts:18`). Otto's runtime coalesces every
streaming upsert of an item to one per `RuntimeConfig::stream_coalesce`
(50 ms):

- the first chunk goes out at once;
- the latest held text is flushed when the window ends, or before any other
  event;
- text only grows.

### Typed provider questions

`runtime_request.updated` with `kind: "question"` carries the full form:
`questions[{id, header, prompt, kind: single|multi|text, required,
allow_free_text, allow_empty?, options[{value, label, description?}]}]` and
optional `question_title`. It remains available in snapshots even when its
tool row or the event replay window is no longer visible. The UI renders
**Needs your answer** separately from permission approvals; settled forms
remain under **Previous questions**.

| Provider path | Supported input and native reply |
|---|---|
| Claude `AskUserQuestion` | Text, single choice and multi-select, with free-text alternatives. The original question text keys `updatedInput.answers`; multi-select values use Claude's comma-separated format. |
| Codex `item/tool/requestUserInput` | Text and single choice, with free-text alternatives. Native question IDs map to `{answers: [...]}` arrays. |
| ACP `elicitation/create` | Session-scoped `mode: "form"` with flat text, string enum/titled single choices, and arrays of string choices. Replies preserve schema property names and values. An optional single choice `<id>` with an optional `<id>__other` text field (OMP's ask dialog) is one question with a free-text alternative; typed text is returned as `<id>__other` (T3 Code #34867). This is the upstream draft extension; not every ACP provider emits it. |

`POST /runtime-sessions/{id}/requests/{request_id}` accepts `{answers: [...]}`
with tagged `single`, `multi` or `text` entries, or `{question_action:
"decline"|"cancel"}`. The UI's **Skip question** uses decline. ACP preserves
the native dismissal action, Claude denies only the question tool, and Codex
returns an empty answer map. Dismissal marks the request cancelled without
approving a tool or sending ordinary chat text.

Required IDs, answer types, duplicate entries and offered values are validated
before consuming the request. Native Claude/Codex required answers must be
nonempty. Unconstrained ACP text and multi-select fields allow empty values,
including when required: the property must be present. Numeric/boolean fields,
nested schemas, extra validation constraints, URL/auth/request-scoped ACP
elicitations and secret questions are refused with an explicit provider
error/denial. No answer is fabricated. The pi driver has no typed question
adapter.

Public request IDs are scoped to the live process generation; replies map back
to the preserved native RPC ID. Pending requests can be answered out of order.
Withdrawn, interrupted or closed requests become cancelled; daemon recovery
expires them as not resumable. An old card cannot answer a replacement
process's request. Question payloads and responses are separate from permission
decisions, so one cannot be used in place of the other.

### Tool-result images

`dynamic_tool.images?` contains `{mime, data_base64}` records alongside
`output?`, preserving accompanying text and structured MCP metadata. Claude
(including nested subagent tools), Codex MCP, and ACP/OpenCode image content
use the shared `tool_images.rs` extraction and validation path. OpenCode inline
file attachments can also arrive in ACP `rawOutput`.

Only inline PNG, JPEG, GIF and WebP are retained: valid base64 and matching
raster signatures, at most four images, 1 MiB decoded per image and 2 MiB decoded
total per result. Unsupported, malformed, remote-only or over-limit images
produce a text omission notice. No provider image URL is fetched and no HTML
or SVG is rendered. Old stored tool items without `images` continue to work.
`RuntimeToolImages` rechecks payloads at the rendering boundary, shows
thumbnails in expanded work rows, and opens the shared modal for enlargement,
including keyboard close/focus return and phone layouts. Decoder failures show
an image error without losing the tool text.

### Per-turn model, effort and images

`POST …/prompt` takes `{text, model?, effort?, images?: [{mime,
data_base64}]}`. T3 Code sends its model selection with every turn
(`ClaudeAdapterV2.ts:1224-1260`, `CodexAdapterV2.ts` `turn/start`, ACP
`session/set_model` / `set_config_option`, `AcpSessionRuntime.ts:2698-2715`);
Otto takes the cheapest correct path per provider:

| Provider | Model switch | Effort | Images |
|---|---|---|---|
| Claude | live: `control_request {subtype:"set_model"}` (the SDK's `setModel`) | live: `apply_flag_settings {effortLevel}` | `image` blocks, `source: {type:"base64"}` (`ClaudeAdapterV2.ts:2410-2414`) |
| Codex | `turn/start {model}` (this turn and later ones) | `turn/start {effort}` | `{type:"image", url:"data:<mime>;base64,…"}` inputs (`CodexAdapterV2.ts:3268-3269`) |
| Gemini, Grok (ACP) | restart with `--model` / `-m`, then `session/load` | 400 | `image` blocks when `promptCapabilities.image`, else 400 (`adapter.ts:7035`, `:7777`) |
| Antigravity (ACP) | restart, `session/load`, then `session/set_model` | 400 | `image` blocks |

The model sticks: it becomes the session's `model`, so a later respawn
launches on it. A refused Claude switch arrives as an `error` event; the turn
still runs.

Live check (same isolated daemon): a 16 px red PNG, then a switch, then a
follow-up. Claude (haiku → `sonnet` + `low`) and Codex (`gpt-6.1-sol` →
`gpt-6-luna` + `low`) both answered "Red", switched (the Claude transcript
and the Codex rollout show the new model on the second and third turns)
and still remembered the image.

## 4. History, replay and restarts

- **T3 Code.** Every domain event is stored with a sequence
  (`OrchestrationV2StoredEvent`, `orchestrationV2.ts:2264-2269`). A client
  subscribes with `afterSequence` and gets the events since. Past 128 events
  or 1 MiB, or for an unknown sequence, it gets a snapshot and then live
  events (`apps/server/src/orchestration-v2/ThreadStream.ts:18-20`, `:91-118`).
- **Otto.** Each session has its own `sequence`, strictly increasing and
  never reused. An in-memory ring holds 4096 events / 8 MiB, and an entity
  projection is persisted in `agent_runtime_entities` (migration 0197):
  the latest JSON per entity, with streaming text written once it settles.
  `GET /runtime-sessions/{id}/events?after_sequence=` returns
  `{kind:"events"}` within the ring and T3 Code's 128 / 1 MiB limits, and
  `{kind:"snapshot"}` otherwise. That is the same contract.
- **Sequence floor.** Each session row keeps `seq_reserved`. Before a child
  is used, 65 536 sequences are durably reserved, and again whenever the
  count reaches the reservation. A restarted daemon resumes above
  `max(last_seq, seq_reserved)`.
- **What is resumed.** The provider's own id is persisted as
  `native_thread_id`:

  | Provider | Resume key | How Otto resumes |
  |---|---|---|
  | Claude | session id | `--resume` |
  | Codex | thread id | `thread/resume` |
  | ACP (Antigravity included) | session id | `session/load` |

  T3 Code persists the same `nativeThreadRef` (`adapter.ts:861-903`,
  `orchestrationV2.ts:145-151`).
- **Daemon restart, stated plainly.** A structured child is a direct child
  of `ottod` with piped stdio, so it dies with the daemon. A PTY agent with
  `session_persistence` survives in its PTY holder; a structured child does
  not. At boot (`AgentRuntime::recover`, from `recover_before_serve`):
  - every in-flight turn ends **`interrupted`**, and so do its open items;
  - every pending request becomes `expired` / `not_resumable`;
  - the provider session becomes `stopped`.

  T3 Code's `ProviderRuntimeRecoveryService` does the same, except that it
  marks runs `cancelled` (`:271`, `:369-396`). Otto says `interrupted`
  because the daemon stopped the turn. The next prompt respawns the CLI and
  resumes the provider's own session id. The interrupted turn's partial work
  is whatever the provider itself saved. T3 Code can also auto-continue
  (`RestartContinuation.ts:33-36`); Otto does not. Background work dies with
  the child: recovery clears `background_tasks`, and open background
  subagent cards settle `interrupted` (also when the process exits or the
  thread closes).
- **The queue survives a restart.** A message sent with `delivery: "queue"`
  (or `"interrupt"`) while a turn runs is stored whole (text, images, files)
  as the projection's `queue` entity. After a restart `recover` sends what is
  queued when a PERSON queued any of it (respawning the CLI, resuming its
  thread). A queue holding only child threads' replies does not respawn the
  agent: they wait for the person's next message or **Revive** (`reopen`),
  then go out in order. Messages go out one per turn, oldest first, each
  exactly once (one drain at a time per session). A child thread whose turn
  the restart cut off tells its parent: "Codex was interrupted when Otto
  restarted: …" (`stopped` on its card) and, when the parent's agent asked
  it, one queued reply saying so — once, without respawning the parent.
- **Stall watchdog.** A turn that emits nothing for 15 min (45 min while one
  of its tools runs) is interrupted with `turn.terminal reason: "stalled"`
  and a `system_notice`. A pending approval or pending background work is
  never a stall (`RuntimeConfig::stall_after` / `stall_tool_after`).
- **Revive and delete.** `POST /runtime-sessions/{id}/reopen` clears
  `closed` (in order with the close's own write) and respawns a closed or dormant agent with
  its resume key, without sending a prompt of its own (what waits in its
  queue — a child thread's reply — then goes out). An already-live process is preserved. If the process exits before it is ready, the runtime resumes
  once more unless the exit says the conversation is gone ("No
  conversation found"): a passing failure keeps the provider's conversation
  (T3 Code #34762). If that fails too (the provider lost
  the conversation), the runtime starts a fresh provider thread, keeps the
  timeline, and adds a notice that the agent does not remember it; the old
  key is kept until the fresh one is up. Concurrent reopens and prompts share
  the per-session spawn lock: one process per thread. `DELETE
  /runtime-sessions/{id}` closes the session, then removes its row (entities
  cascade) and attachments.
- **Personal-agent runs** (`personal_agent_threads.rs`) wait until the
  thread is idle — no turn, its own included, no background work — and stays
  idle for 5 s. Only then do they take the report (the report file, else the
  last answer across turns) and close the thread. Background work counts as
  progress, with a 45 min stuck allowance while it is pending. A closed run
  thread can be reopened as a normal conversation (its meta is kept).

### Explicit fresh context (`/clear`)

`/clear` arms the next prompt with `fresh_context: true`; Cancel clears only
that intent. It does not send a prompt, terminate work or create another Otto
thread. `/new` retains its separate-thread behavior. Fresh intent is part of
the saved composer draft.

The runtime serializes admission with ordinary prompts, queue delivery, close
and reopen. A running turn, background task, queued message or pending request
causes **409**, before the old process is removed. Fresh prompts are never
queued, steered or used to interrupt current work. An otherwise idle closed
thread can reopen straight into a fresh conversation.

On admission, the old process is stopped, the native identity/context projection
is temporarily cleared and a new provider is initialized without a resume key.
Only after initialization and local `driver.prompt` acceptance does the runtime
persist the new identity and emit one `session_boundary` item before the new
user message. Existing history, files, session ID, ownership and workspace stay
intact. Failed initialization or rejected local prompt input restores the old
native identity and closed state when applicable, with no boundary. A later
provider error after acceptance is a failed turn in the newly committed
conversation. The frontend keeps the submitted draft and fresh intent on a
rejected send.

### Durable composer state

`GET/PUT /runtime-sessions/{id}/draft` uses `{revision, draft}` records backed by
`runtime_composer_drafts`, scoped to user, workspace and runtime session. The
human-only routes reject managed-agent, MCP and share credentials. Draft
content includes text, attachments (bytes and names), model, effort,
`access_mode`, `fresh_context`, and attributed review/diff context chips. Action
flags such as sending, stopping and replay sequence are live UI state only.
No draft content is put in browser local storage.

The frontend uses one serialized writer per draft with a 200 ms debounce,
plus a best-effort flush on page hide. **Restoring / Saving / Saved** and an
inline **Retry** report persistence. Send waits for recovery; edits during a
late load survive, and added context merges by stable ID. A revision mismatch
returns **409**, suspending automatic writes; explicit Retry reloads the
revision and saves this window's local version. It does not merge competing
text. An accepted prompt or queue operation acknowledges only the submitted
snapshot, retaining later edits and model/effort/access preferences. Failed
sends retain the draft; recovery never sends it automatically.

Limits: 1 MiB text, 12 attachments at 10 MiB decoded each, 32 context chips at
128 KiB text each, and 24 MiB total serialized draft/request size. Attachment
MIME types are PNG, JPEG, GIF, WebP and PDF, validated against base64 bytes and
file signatures; prompt-time provider restrictions still apply. Each
person/workspace has a 64 MiB / 128 nonempty-draft quota. Clearing content stays
possible at quota; preference-only records do not consume a nonempty-draft
slot. Payloads older than 30 days since their last save expire during recovery
and draft reads/writes, including preferences and attachment bytes. Revision
tombstones remain until thread deletion, which cascades the row. Quota and
validation failures are explicit; content is not silently truncated.

### Selected review and diff context

The inspector's Pull request surface loads complete comment conversations;
**Add to chat** preserves the comment/replies, author, PR URL, location and
outdated/resolved status. Changes offers whole-file, hunk and line selection
in unified or split view, preserving the diff target and old/new line numbers.
Selections are independent chips; repeat selection replaces the matching chip
without touching typed text. The chips can be removed or opened in a readable
modal on desktop or phone. They are appended with attribution only on send,
without an automatic prompt or a canned fix instruction. Loaders abort and
clear old results on owner changes; the inspector rechecks the focused runtime
thread before updating its draft or returning focus.

## 5. How the web client consumes it (T3 Code → Otto Threads)

T3 Code loads a snapshot, then subscribes with `afterSequence`, and folds
each event into a projection (`packages/client-runtime/src/state/threads.ts:937-1027`,
`orchestrationV2Projection.ts:270-420`).

| Element | T3 Code | Otto |
|---|---|---|
| Streaming text | `turn_item.updated` with `streaming: true`, replaced by id (`apps/web/src/session-logic.ts:635-669`, `:867-895`) | same |
| Tool and work-log cards | `command_execution` / `file_change` / `web_search` / `dynamic_tool` items (`session-logic.ts:400-545`) plus live command output (`orchestration.subscribeCommandOutput`) | the same items plus `command_output.delta` |
| Approvals | Pending `runtimeRequests` paired with their `approval_request` item (`packages/client-runtime/src/state/threadRequests.ts:229-284`), answerable only while `responseCapability` is `live`; answered by `runtime-request.respond {requestId, decision}` (`operations/commands.ts:879-890`) | `POST /runtime-sessions/{id}/requests/{request_id} {decision}` |
| Plan | `todo_list` / `plan.updated` (`session-logic.ts:517`, `:672-690`) | same |
| Stop | `thread.stop` / `run.interrupt` (`commands.ts:784-876`), shown while the phase is running (`session-logic.ts:1124-1146`) | `POST …/interrupt` |
| Working | provider-session / run status (`session-logic.ts:1124-1134`) | provider-session status |

## 6. Official crates

- **ACP.** `agent-client-protocol` **5.0.0** (schema 1.15.1). Every message
  Otto sends or parses uses its v1 wire types (`InitializeRequest`,
  `AuthenticateRequest`, `NewSessionRequest`, `LoadSessionRequest`,
  `PromptRequest`, `CancelNotification`, `SessionNotification`,
  `RequestPermissionRequest/Response`, …). Otto does not use the crate's
  connection runtime: Otto's runtime must own the process (for the safety
  pipeline), and the driver stays sans-IO and unit-testable. Otto's own
  code is only the JSON-RPC envelope and id correlation.
- **Codex.** crates.io `codex-app-server-protocol` is **0.63.0**, published
  from a fork (`namastexlabs/codex`). It lags the installed codex-cli
  **0.166.0** by about a hundred releases (`thread/*`, `turn/*` and `item/*`
  are not in it), so it is not used. T3 Code generates its own client from
  an upstream commit (`scripts/generate.ts:16-19`). Otto keeps a minimal
  typed surface in `codex.rs`, checked against
  `codex app-server generate-json-schema` of 0.166.0. When upstream
  publishes a current crate, swap it in.
- **Claude.** There is no official Rust SDK. `claude.rs` ports the
  TypeScript SDK's transport: its argv and the control-protocol messages
  `initialize`, `can_use_tool`, `interrupt` and `control_cancel_request`.

## 7. Safety: identical to PTY agent sessions

Every spawn goes through `SessionManager::prepare_runtime_spawn`
(`crates/otto-sessions/src/manager/runtime_spawn.rs`). It calls the same
private helpers as `SessionManager::create`, in the same order:

1. `refuse_while_shutting_down`;
2. `check_session_cwd`, which refuses `/`, `$HOME` while confined, its
   parents, and Otto's data dir;
3. `trust::ensure_trusted_async` (claude, codex and grok pre-trust);
4. the context hook (`--add-dir` / `--append-system-prompt-file` /
   `--settings` for claude, `developer_instructions` + `CODEX_HOME` for
   codex, `--rules` for grok, the `OTTO_AGY_CONTEXT_DIR` env for agy, which
   the driver turns into the system-prompt append);
5. a driver's `tail_args` (grok's `agent stdio`);
6. `apply_sandbox`, **last**, over the fully built argv: the
   `process_sandbox` Seatbelt profile, same `sandbox_decision`.

A named provider account or a workspace network profile is **refused**, not
skipped.

The PTY prompt guard types keystrokes to accept folder-trust or "press
Enter" dialogs on a terminal screen. A structured child has no terminal:
trust is pre-seeded as above, and every approval arrives as a pending
runtime request **that only a person can answer**. Access rules:

- the answer route is person-only and is an `Admin`-class approval gate in
  `policy::route_class`;
- every other write is person-only too;
- reads need workspace `viewer` + owner-or-admin;
- WS delivery is session-family (owner, workspace admin, root).

Provider permissions follow the selected access mode (§2), including explicit
full-access settings. Remaining permission requests still require a person.
Typed question answers use a separate payload and never grant a permission;
secret question forms are refused instead of being collected into history.

## 8. Code map

| Path | What |
|---|---|
| `crates/otto-core/src/agent_runtime.rs` | event model + REST DTOs |
| `crates/otto-agent-protocol/src/driver.rs` | `StructuredDriver` contract (sans-IO) |
| `crates/otto-agent-protocol/src/{claude,codex,acp,pi}.rs` | the drivers |
| `crates/otto-agent-protocol/src/claude_tasks.rs` | Claude background tasks, subagent cards, their nested activity |
| `crates/otto-agent-protocol/src/runtime.rs`, `runtime/{fresh,request_ids}.rs` | processes, turns, sequencing, coalescing, replay, recovery, fresh context and process-scoped request IDs |
| `crates/otto-agent-protocol/src/{questions,tool_images}.rs` | native question schema normalization and bounded tool image extraction |
| `crates/otto-agent-protocol/src/http/drafts.rs`, `crates/otto-state/src/agent_runtime/drafts.rs` | draft validation, authorization, revision checks, quotas and expiry |
| `crates/otto-agent-protocol/src/{host,process,ring,http}.rs` | safety host, child I/O, replay ring, `/runtime-sessions` router |
| `crates/otto-agent-protocol/src/bin/otto-fake-{agent,codex,claude}.rs` | test doubles (ACP agent, `codex app-server`, Claude SDK stream-json with background work / subagents / steering) |
| `crates/otto-state/src/agent_runtime.rs`, migration `0197_agent_runtime.sql` | persistence |
| `ui/src/lib/api/types/agent-runtime.ts` | TS mirror |
| `ui/src/modules/threads/runtime/projection.ts`, `timeline.ts` | event fold (T3 Code's projection reducer); what the view derives (pending requests, phase, rows) |
| `ui/src/modules/threads/runtime/runtimeThreads.svelte.ts` | session rows (live status) and actions |
| `ui/src/modules/threads/runtime/runtimeView.svelte.ts` | an open thread's projection: live events, replay after a gap / reconnect (loads with the view) |
| `ui/src/modules/threads/runtime/Runtime{Thread,WorkRow,ApprovalRow}.svelte` | timeline, work-log rows, inline approvals, composer with Stop (sending to a closed thread reopens it) |
| `ui/src/modules/threads/runtime/{RuntimeWorkFold.svelte,RuntimeWorkGroup.svelte,workLog.ts}` | the quiet work log: a finished stretch folds into "Worked for …", a running group names its live call "+N more", labels without `cd <dir> &&` and with paths relative to the thread's folder, durations, "exit N" |
| `ui/src/modules/threads/runtime/{RuntimeBannerStack.svelte,banners.ts}` | everything above the composer in one priority order (error > question / approval > plan > queue > notices), at most two in full |
| `ui/src/modules/threads/runtime/Runtime{SubagentCard,AgentRow,ThreadCallRow}.svelte`, `agentRows.ts` | one compact row for another agent: a subagent, or the agent's `thread_open` / `thread_message` call |
| `ui/src/modules/threads/runtime/Runtime{QueueBar,TurnStatus}.svelte` | background work, the queue (Steer / Send now / Edit / Remove) and Closed · Reopen; the turn's status line (working, own turn, stopped, stalled, partly done) |
| `ui/src/modules/threads/{RelatedThreadsChip,RelatedThreadsList,ThreadPopover}.svelte`, `relatedThreads.ts`, `relatedCollect.ts`, `relatedMessage.ts` | the header's one Related threads control (linked threads, workers, side chat, rooms; open, open beside, message, start a room) |
| `ui/src/modules/threads/threadsPageMenu.ts` | the toolbar ⋯: the thread's actions, then New thread, Related threads…, Project card, Terminal drawer and the layout |
| `ui/src/lib/stores/runtimeDraftPersistence.ts`, `runtimeDraftRecovery.svelte.ts`, `runtimeDrafts.svelte.ts` | draft snapshot acknowledgement, serialized persistence and recovery |
| `ui/src/modules/threads/runtime/Runtime{Questions,QuestionCard,ToolImages,ContextChips,DraftStatus}.svelte` | typed questions, image enlargement, context preview/removal and save status |
| `ui/src/modules/threads/runtime/composerContext.ts` | attributed review conversations and file/hunk/line excerpts assembled on send |
| `ui/src/modules/threads/runtime/runtimeMenu.ts` | the thread menu: Reopen, Close, Delete (confirmed unless "Closing a session tab" is Always delete) |
| `crates/otto-agent-protocol/src/runtime/council.rs` (+ `council/rounds.rs`), `ui/src/modules/threads/runtime/{council.ts,councilRounds.ts,councilStore.svelte.ts,Council*.svelte}` | Council: seats created all or none, follow-up rounds, the judge's prompt per round; the stage multi-select, columns, rounds strip, composer, sidebar row |
| `ui/e2e/desktop-threads-runtime.spec.ts` | E2E against `otto-fake-codex` and `otto-fake-claude` (global-setup's shims exec them for `app-server` / `--input-format stream-json`) |

## 9. Gaps and next steps

- **Threads UI.** Runtime threads raise the shared "needs you" flag while
  an approval or typed question waits, sit in History once closed (renamable;
  opening or messaging one reopens it),
  and take the view's slash commands (`/btw` side chat — itself a PTY
  thread — `/diff`, `/files`, `/terminal`, `/new`, `/clear`, `/fork`, `/review`);
  any other `/command` goes to the agent as typed (T3 Code's provider
  commands). The thread's terminal drawer is a plain shell in its folder
  (`meta.terminal_of`). The composer's model / effort pick and attached
  images ride `POST …/prompt` (`model`, `effort`, `images`).
- **Idle live-entry eviction** remains a runtime lifecycle follow-up. Daemon
  shutdown already calls `AgentRuntime::shutdown()`.
- **Features T3 Code has that Otto does not yet:** restart continuation,
  steering for Codex (`turn/steer`) and ACP, Codex `thread/unarchive`,
  Claude thinking summaries, subagent / background tracking for the other
  providers, ACP v2.
- **Known limits.** A Claude resume key is persisted once the CLI answers its
  handshake, so a daemon that dies before that loses the resume (the next
  spawn starts fresh). A subagent's streamed text is not shown live (its
  complete messages are).
- **Named provider accounts and network profiles** (refused today).
- ~~Otto MCP tools for structured sessions~~ — done: `prepare_runtime_spawn`
  mints the per-session `otto` credential and applies the workspace's enabled
  MCP servers. Claude / Codex get them as a terminal session does
  (`.mcp.json` + `claude --mcp-config` with Otto's managed entries, codex
  `-c`); ACP agents (Gemini, Grok, Antigravity, opencode) get
  them in `session/new|load.mcpServers` with each server's env. pi (1.x) gets
  them through a private per-session `-e` extension that registers each
  server (see "opencode and pi" above). The credential is revoked when the
  process exits or the thread closes.
- **Agent UI control** (done): a thread's agent can drive Otto beside it — open a DB connection, run queries in your Database Explorer, detach the pane into its own window — with the same per-thread grant as terminal sessions (see [Agent UI control](./agent-ui-control.md#threads)).
- **Linked threads** (`/runtime-sessions/{id}/peers`, `otto.thread_open` /
  `_list` / `_message` / `_read`): a thread's agent opens other threads
  (`parent_id` = the group's root), messages them and reads their replies —
  an orchestrator driving codex / gemini / grok workers. The Threads sidebar
  nests the group; the toolbar's **Related threads** chip lists it (see
  [Threads layout](#threads-layout)); an embed shows a small "N linked" chip
  of its own.

### Council: one prompt, several agents

T3 Code lets you pick several models in the new-thread picker; one send then
starts one thread per model with the same prompt
(`apps/web/src/components/chat/useMultipleModelSend.ts`, `chatViewSend.ts`
`startFanoutThread`). Otto's **Council** does the same and keeps the answers
together, then adds the step T3 Code leaves to you: a judge.

**Starting one.** In the new-thread stage, **Ask several** (beside the agent
chip) turns the agent menu into a multi-select: every model picked becomes a
seat, and picking it again takes it out. A ⌘- or ⇧-click on a model starts
the selection with the current pick. With 2–4 seats the chip reads e.g.
"Opus, Sol, +1" and the send button "Ask 3"; one seat sends nothing and
asks for another. The same provider may sit twice (another model or
effort). One model is today's single thread, unchanged.

**What the daemon makes** (`POST /workspaces/{id}/runtime-sessions/council`,
`crates/otto-agent-protocol/src/runtime/council.rs`):

- one linked group: seat 1 is the root, seats 2–4 are linked to it, every
  seat carries `meta.council {id, seat, role: "seat", effort?}`;
- every seat is **read-only** (`meta.read_only`: the read-only Seatbelt
  profile and tool list) and approval-required, so parallel agents in one
  folder never clash on files;
- all seats or none: a seat that cannot start removes the ones already
  made, and the error names it ("seat 2 (codex): …"); the stage keeps the
  picks so a retry is one click;
- the prompt goes to every seat at once. No new WebSocket events: each seat
  streams on its own session channel.

**The view.** A thread with `meta.council` opens as columns
(`ui/src/modules/threads/runtime/CouncilView.svelte`): 2–4 compact embeds,
each headed by the provider, model, effort, status, elapsed time and tokens
(tokens, not cost — most people run on subscriptions), with **Continue
here** to take that seat on as a normal thread (the sidebar's council row
brings the columns back). A seat that did not finish the current round
(its turn failed, a Stop or a daemon restart cut it off, or the follow-up was
removed from its queue) says so and offers **Ask again**, which sends the
round's question to that seat once more (queued if it is busy). On a phone the columns become a segmented control, one seat at a
time. The sidebar shows the council as one row, "Council · 3" with the
seats' provider icons, that unfolds into the seats and the synthesis.

**Rounds: keep talking to the council.** Under the columns sits one
composer (`CouncilComposer.svelte`) with a target chooser — **All seats**
(the default) or one seat; clicking a seat's name in its header aims the
next follow-up at that seat only. **Ask round N** sends it
(`POST /runtime-sessions/{id}/council/rounds`, `council/rounds.rs`): each
chosen seat gets the text as its own thread's next turn, so every seat keeps
its own context, with the seat's effort; a seat still working **queues** it
exactly like a message sent mid-turn (the seat's queue bar shows it). A
closed seat is listed as not reached, with the reason. **Ask the others** on
a seat that answered the round drafts "Seat X answered: <its answer>. Where
do you disagree, and why?" for the other seats — the person reads and can
edit it before sending: the cheap way to get real cross-examination. Each
column shows the new question and answer under the earlier ones (the seat's
own thread, newest in view); above the columns, from round 2, a **Rounds**
strip lists "Round 2 · <question>" with every seat's state in that round
(working, queued, done, interrupted, failed, not delivered, not reached, sat
out) and time on it, and the syntheses judged on it. **Stop all** stops the
round's working seats. A council runs at most **10 rounds**; the composer
then says so and asks for a new council. Nothing is ever sent by itself:
every round starts with a click (the tokens are the person's).

The rounds are one `council_rounds` entity on seat 1
(`GET /runtime-sessions/{id}/council/rounds`), so they survive a reload and
a daemon restart without a migration; a council from before rounds reads as
one round. Which message answers which round is read off each seat's own
timeline by the round's text (a re-ask of the same text before the next
round takes its place), in the daemon for the judge and in
`councilRounds.ts` for the view. After a restart a seat that was mid-turn
shows **interrupted** and the round can be asked again.

**Synthesis.** The footer's one primary action is **Synthesize with
<judge>** — from round 2 **Synthesize round N with <judge>** (the judge
defaults to seat 1's provider; any runtime provider can judge). It judges
the CURRENT round: it is disabled, with the daemon's own reason, while a
seat of that round or an earlier synthesis still works, while seat 1 is
closed, or while no seat has an answer in the round. **What counts as an
answer:** any answer text the seat has on record in the round, whole or cut
off by a Stop (the judge is told it may be incomplete). Text still streaming
when the daemon stopped is not kept, so a seat a restart cut mid-answer has
none: a round a restart cut for every seat reads "No seat has answered round
N yet — nothing to synthesize", and **Ask again** (or a new round) is the way
on; a seat that finished before the restart keeps its answer and counts. `POST /runtime-sessions/{id}/council/synthesize` opens a thread linked
to seat 1 (`role: "synthesis"`, read-only, titled "… · synthesis of round
N" after round 1, recorded under the round) whose prompt the daemon builds:
the round's question (after round 1 with the council's first question), each
of the round's seats' answer in that round, a line naming every seat of the
round **without** an answer (no reply, never reached, still queued) — never
silently dropped; the footer shows the same list before you click — and the
task: where they agree, where they disagree and who is right, a pick or a
merge, open questions. A cut-off answer is marked as possibly incomplete. It
is sent without the "Opened by …" header (the daemon wrote it, not an
agent). In the Synthesis column that prompt folds to one quiet line —
"Asked the judge to compare N answers · Show prompt" (a button; it unfolds
the full prompt) — so the column opens on the verdict. The answer is the
last, full-width column, labelled with its round;
earlier syntheses stay in the Rounds strip, one click away.

**Limits.** 4 seats, 10 rounds; seats + syntheses count against the group
cap (`MAX_LINKED_OPEN`, 8 besides the root), so close an old synthesis to
judge again once the group is full. A council is started and watched by a
person, so it is not a background source and takes no admission slot. The
Rounds strip loads when the council opens and after your own sends; a round
sent from another device shows after a reopen.

**Left for later, on purpose:** a worktree per seat so seats can edit;
scored rubrics; merging the chosen seat's work back; an `otto.council` MCP
tool; cost in dollars.

### Threads inside a thread

T3 Code shows a thread that belongs to another one in three places, the
same for every provider, and Otto now does too.

**What a person sees in T3 Code** (read from its source:
`apps/web/src/components/chat/V2LifecycleRow.tsx`, `ProviderSubagentBar.tsx`,
`ThreadLineageWorkflowRow.tsx`, `ThreadRelationshipsControl.tsx`,
`packages/client-runtime/src/state/subagentRuntime.ts`,
`packages/contracts/src/orchestratorMcp.ts` `delegate_task`,
`docs/user/composer.md`, `docs/user/thread-sidebar.md`):

- *Inside the parent's timeline*: a delegated child is one compact row
  (`SubagentTimelineLink`) — the provider's avatar with a status ring, its
  title, one line of progress while it runs and of its result once settled,
  the status word, elapsed time and a chevron; the whole row opens the
  child, a hover card adds model, effort and the full result. When the child
  ends, a notification row says "Finished" / "Failed" / "Stopped" with the
  time. Agents a workflow runs group under one header ("2/3 agents",
  phases, "Stop entire workflow"). Inside the child, the composer gives way
  to a bar: provider, model, effort, live status, "Runs on its own", **Open
  parent**.
- *On the side*: the header's **Related threads** / **Agents** control lists
  the parent and its subagents with status icons; the sidebar keeps one row
  per thread and folds working threads into a "Working" section.
- *Starting one*: the agent calls `delegate_task` (provider, model, effort);
  its completion wakes the parent ("Completion wakes this thread, so end your
  turn"). A person types `@opus` / `@gpt` in the composer to name a model
  for the agent; Shift-click several models in a new thread's picker sends
  one prompt to each.

**Otto's version.** One visual language for every child — a thread a person
asked from the composer, one the agent opened with `otto.thread_open`, a
council's seats — on Claude, Codex and the ACP providers (Gemini, Grok,
opencode, pi) alike:

- **Start one** — in a runtime thread's composer, **Ask another agent**
  (the people icon beside Attach) opens the agent menu; each model picked
  becomes a chip above the text ("Ask Codex · GPT-5"), and picking 2–4 is
  **Ask several**. Typing `@` and part of a provider or model name (`@cod`,
  `@opus`) offers the same agents (↑ ↓, ↵ / Tab picks, Esc closes). With
  chips set, Send asks them instead of this thread's agent:
  `POST /runtime-sessions/{id}/children` opens one new thread per agent,
  all or none, linked to this one, in its folder. The chip row says what
  they may do: this thread's access ("asks before it edits", "can edit
  files", "full access", or "read-only"); the composer's access chip narrows
  it, never widens it (a wider pick is refused in words). A full group (8
  open threads beside the root) is refused above the composer with how many
  still fit.
- **Inside the thread** — where they were asked, the parent shows a live
  card per child (`RuntimeChildCard.svelte`): provider icon, title, model ·
  effort, Starting / Working / Needs you / Done / Failed / Stopped, elapsed
  time, its latest step while it works and its answer's first line once it
  ends, with **Stop**, **Open beside** and **Open**; clicking the row opens
  its last answer inline, read-only. Several asked together sit under one
  header ("3 agents · 2 working · Stop all"). An agent's `thread_open` call
  becomes the same card once it returned (a stretch holding one never folds
  into "Worked for …"); a provider's own subagent keeps its card, drawn with
  the same row. Cards follow each child's own session events (its projection
  stays live while the card is on screen) — no polling.
- **On the side** — the sidebar nests threads under the thread they belong
  to (indented, provider icon, status dot); the parent row reads "· 3", its
  dot speaks for the family (one needs you → needs you, one works →
  working), and its chevron folds them. A child whose parent is closed,
  archived or filtered out still shows, marked **detached**. A council's
  seats nest the same way under its "Council · N" row. History and search
  are unchanged.
- **Hearing back** — each child turn's end writes one line in the parent:
  "Codex finished: <first line> · Open". When the parent's **agent** opened
  the child and the turn answered what it asked (its opening prompt or an
  `otto.thread_message`), the answer is also queued for that agent (T3's
  wake-on-completion): `[Reply from thread "…" (codex, id …): its turn
  finished …]` + the answer (cut at 16,000 characters). It waits in the
  parent's queue — the queue bar shows it, with Send now / Remove — and goes
  out at the parent's next idle moment; one entry per child (a newer answer
  replaces an older one), never twice for one turn. A dormant parent is not
  respawned for it, not after a restart either (the person's next message or
  Revive sends it); a closed one gets nothing — closing a parent does not
  stop its children (they stay, marked detached, with their answers; Stop
  them in their own thread) — and a delivered reply never wakes a
  grandparent. The parent's turn runs under
  the parent's own access mode, so every approval it would need is asked as
  usual. A person's ask never wakes the parent's agent.
- **Both at once** — **Open beside** (card, sidebar row) puts the child in
  the split next to its parent (1-up → 2-up → grid). On a phone the child
  opens in place and the header shows **‹ <parent title>** to go back.

| Word | Where | What |
|---|---|---|
| **Ask another agent** | a thread's composer (people icon, or `@`) | one new linked thread on the chosen provider / model / effort |
| **Ask several** | the new-thread stage (a council), or 2–4 chips in a thread's composer | the same message to each, side by side |
| `otto.thread_open` | the agent itself | a linked thread whose answers come back to the agent |

Meta and markers: a child carries `meta.child_of {id, by: "person" |
"agent"}` (the opener; `parent_id` stays the group's root). The parent's
notices end in `[otto-thread:<id>[,<id>…]:started|finished|failed|stopped]`
(`threadNoticeOf` in `runtime/childThreads.ts`). Code:
`crates/otto-agent-protocol/src/runtime/children.rs`;
`ui/src/modules/threads/runtime/{childThreads.ts,askAgents.ts,askTargets.svelte.ts,childNav.ts,RuntimeAskAgent.svelte,RuntimeChildCard.svelte,RuntimeChildGroup.svelte,RuntimeChildNotice.svelte,ThreadParentChip.svelte}`,
`ui/src/modules/threads/threadFamilies.ts`.

### Threads layout

After T3 Code's header and work log, so the conversation stays the loudest
thing on screen:

- **Toolbar: four controls** — sidebar, the breadcrumb (project / thread),
  right panel, ⋯. The ⋯ holds the thread's own actions, then **New thread**
  (also the sidebar's New button and ⌘K "New thread"), **Related threads…**,
  **Project card**, **Terminal drawer** (also `/terminal` and ⌘K) and the
  layout (Single / Split / Grid, also in ⌘K). Back and Forward are the
  shell's own (⌘⇧← / ⌘⇧→, ⌘K "Go back" / "Go forward"). Continue on phone is
  a ⋯ item, in the toolbar and in a split pane's header alike.
- **Related threads**: one chip beside the title, shown only while the thread
  has related threads (linked by `otto.thread_open`, a delegated thread's
  parent and workers, its side chat). Its popover lists them with their
  status, the thread's session rooms and its recent cross-thread messages;
  each row opens, opens beside or messages; **Message a thread…** reaches any
  thread (or all of them) and **Start a room…** starts a session room. It
  replaced the Peers tab, the pane's "To:" relay bar and the ⋯ items "Send
  message to peer…" / "Session room…". A sidebar row's "Send message from
  active thread…" opens the same message sheet directly.
- **Right panel**: Browser, Files, Diff, Pull request and Side chat; the
  terminal is the drawer (no Terminal surface); **Connections** joins the +
  menu once the thread has database activity (or its tab is open).
- **Work log**: a finished stretch of tool calls and thinking folds into one
  "Worked for 2m 14s · 6 steps" line that opens to every call. A stretch that
  is still running, or holds a failure, an approval (pending or decided), a
  notice, a subagent or a task list never folds. Thinking reads inline (three
  lines, then More). A running group reads "npm test +2 more". Rows drop a
  leading `cd <dir> &&` / `cd <dir>;`, show paths relative to the thread's
  folder and each call's duration, and a failed command reads "exit 1".
- **Other agents**: a subagent and the agent's `thread_open` /
  `thread_message` calls share one compact row — provider, name, model ·
  effort, status, elapsed time, a one-line result and Open (⇧-click: beside).
- **Above the composer**: one stack in one order — error, then a question
  or a UI-control request, then the plan, the queue (or Revive), then the
  fresh-context notice. Errors, questions and approvals always show; the rest
  fill up to two and fold into "N more: …".
- **Composer**: the draft's save state shows only when saving failed (with
  Retry). One model chip; effort is picked inside its popover.

### Revive and responsive width

A stopped agent shows **Revive** above the composer and **Revive thread** in its menu. This starts the agent again without sending a message, keeps the same Otto thread and history, and resumes the provider conversation when available. A live agent is left alone. Closed threads in History have the same action; sending a message also revives them. If the provider has lost its conversation, Otto keeps the visible history and explicitly says the provider started fresh.

The conversation and composer use the available pane width with matching, responsive gutters. Wide screens can show wider answers and code; split panes and phones size themselves independently.

### Embedding a thread in another module

One runtime thread can live inside another module's chrome — today a personal
agent's chat; next, a pull request's left column and a connection's
"diagnose" panel — so the person does not switch windows. The host loads the
view through `ui/src/lib/runtimeThreadPort.ts` (`loadRuntimeThreadView()`,
bound by the shell at boot) and renders it with these props:

| Prop | Default | Meaning |
|------|---------|---------|
| `sessionId`, `workspaceId` | — | The runtime thread and its workspace. The embed reads only that row (`GET /runtime-sessions/{id}`) and its linked group (`GET /runtime-sessions/{id}/peers`), never the workspace's whole thread list. A failed read shows inline with Retry. |
| `agentName` | — | A personal agent's chat: "Message Scout…", and the model/access pickers give way to the agent's settings. |
| `focusOnMount` | `false` | Take keyboard focus when mounted. Embeds beside other work leave it off. |
| `compact` | `false` | Denser gutters for a narrow column (≥ 280 px, usable at 390 px), plus a slim title bar with **Open in Threads**. |
| `onmaximize()` | — | The host grows the embed. The maximise button renders only when given. |
| `ondetach(id)` | — | The host moves the thread out (a window, a tab). The detach button renders only when given; without it a compact embed (or a linked thread shown in place) offers **Open in Threads** (`openInThreads`). |
| `onstatus(status)` | — | Called on mount with the host's thread state and on every change: `'idle'`, `'working'` (a turn, a starting session or background work), `'needs-you'` (an approval or question waits; outranks working) or `'closed'`. Lets a folded host header show a dot without reading the Threads stores. |
| `hostCommands` | — | The view-level slash commands the host carries out (`RuntimeHostCommands`). |

`hostCommands` handlers (all optional): `openDiff(id)` for `/diff`,
`openFiles(id)` for `/files`, `openTerminal(id)` for `/terminal`,
`newThread(id, beside)` for `/new` (`beside` = false) and `/fork` (true), and
`sideChat(id, question)` for `/btw`. A command with no handler is hidden from
the slash popup, and typed by hand it goes to the agent like any other
`/command` — never a silent no-op. `/clear` (fresh context) and `/review` (a
prompt) belong to the thread and always work. Threads is just another host
(`ui/src/modules/threads/runtime/threadsHost.ts`).

Threads an embed starts should carry a `meta.source` from
`EMBEDDED_THREAD_SOURCES` (`ui/src/modules/threads/threadOrigins.ts`:
`pr-agent`, `connection-diagnose`): the Threads sidebar keeps them out of the
projects and History, in their own folded group ("Pull requests",
"Connections", "Helpers"). A personal agent's routine runs fold into one
**Agent runs** group the same way (newest first, 20 at a time); its chat
stays under the agent's name.

The daemon side of that tag. `meta` is server-owned, so a page names the
source in the create body instead — `POST /workspaces/{id}/runtime-sessions`
with `source` (`pr-agent` or `connection-diagnose`) and an optional
`origin: {kind, id, label?}` (`kind` `pr` for `pr-agent`, `connection` for
`connection-diagnose`; `id` 1–200 characters, e.g. `<repo>#<number>` or the
connection id; `label` ≤ 200). The daemon validates them in one place
(`otto_core::agent_runtime::embedded_meta`: any other source, an `origin`
without a `source`, a mismatched kind or a bad length is a 400) and stores
exactly `meta.source` / `meta.origin`; nothing else in the body reaches
`meta`. The page finds its thread again with
`GET /workspaces/{id}/runtime-sessions?source=pr-agent&origin_id=<repo>#<n>`
(URL-encode the id): the caller's own matching threads (all, for an admin),
newest activity first, closed ones included so the page can offer to reopen
rather than start over. A thread the agent opens from an embedded one
(`…/peers`) inherits its `source` / `origin` and stays in the same group.

Embedded threads close themselves. Shortly after boot and then hourly, the
runtime closes every open thread with an embedded source that has had no
activity for `embedded_thread_auto_close_hours` (daemon setting, `PUT
/settings`; default `12`, `0` = never) — the same Close a person clicks: the
agent stops, the thread stays in History and reopens on demand. It never
closes one with a turn or background work running, or with an approval or
question waiting on the person, and closes at most 50 per pass (the rest wait
for the next hour); each close is logged at info. The sweep lives in
`crates/otto-agent-protocol/src/runtime/auto_close.rs`; the
`json_extract(meta, '$.source')` index keeps both the lookup and the sweep
off a full scan.
