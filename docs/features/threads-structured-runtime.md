# Threads — structured agent runtime (how T3 Code does it → how Otto mirrors it)

Status: the daemon side is done: the event model, contracts, runtime, and
drivers for Claude, Codex, Gemini and Grok. Tests use a fake ACP agent and a
fake `codex app-server`. New Claude / Codex / Gemini / Grok threads in a
workspace (Scratch included) run on `/runtime-sessions` (§5); other
harnesses and existing threads stay on the PTY view, which is unchanged.

**Attribution.** Ported from [T3 Code](https://github.com/pingdotgg/t3code)
(MIT, Copyright (c) 2026 T3 Tools Inc.). The licence is reproduced in
`crates/otto-agent-protocol/NOTICE`, and every ported module says so in its
header. Citations are `path:line` in the clone studied (commit `cefb544af3`,
2026-11-02).

## Why

Threads runs the agent CLI in a PTY, types the prompt into it, and rebuilds
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
| **Grok** | `grok` from `binaryPath` (`packages/provider-grok/src/server/acpSupport.ts:67`). Argv per runtime mode (`:47-58`): `agent stdio`; `--permission-mode auto agent stdio`; `agent --always-approve stdio`; for approval-required, `--permission-mode default agent stdio`. Env `GROK_OAUTH2_REFERRER=t3code` (`:66-74`). `_meta` `{clientType:"extension"}` on initialize and `{cancelTrigger:"ctrl_c"}` on cancel (`:98-101`). Auth method `xai.api_key` when `XAI_API_KEY` is set, else `cached_token` (`:21-25`, `:77-81`) | `grok --permission-mode default [-m model] [--rules=<context>] agent stdio` (`AcpDriver::grok`). The context flags must come before `agent`: the subcommand rejects a trailing `--rules` (verified against grok 1.0.70). Same `_meta` and auth preference. Otto omits the T3-specific referrer and never launches the `auto` / `--always-approve` variants |

Installed CLIs it was built against: claude 2.1.318, codex-cli 0.166.0,
gemini (with `--acp`), grok 1.0.70 (`agent stdio`, ACP protocol v1).

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
   the `user` frame's `tool_result` settles the tool.
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
   `queryRuntime.interrupt()`, `:1298-1300`).
6. **Turn end.** The `result` message, mapped by `terminalStatusFromResult`
   (`:3708-3738`):

   | `result` | Turn status |
   |---|---|
   | `terminal_reason` `aborted_*` | interrupted |
   | `success` without `is_error` | completed |
   | errors mentioning interrupt / cancel | interrupted / cancelled |
   | anything else | failed |

   Usage: input is `input` + cache read + cache creation, and output is
   `output` tokens.
7. **Tool → item.** `CLAUDE_KNOWN_TOOL_CLASSIFICATIONS` (`:2819-2860`):

   | Claude tool | Item type |
   |---|---|
   | Bash | `command_execution` |
   | Edit, MultiEdit, Write, NotebookEdit | `file_change` |
   | WebFetch, WebSearch | `web_search` |
   | anything else | `dynamic_tool` |
   | a root `TodoWrite` | the plan (`:9313`) |

   Subagent frames (`parent_tool_use_id` set) are not projected. T3 Code
   builds a subagent tree that Otto does not port. Subagent permission
   questions still reach a person.

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
   | `error` with `willRetry:false` | Kept as the failure |
   | `turn/completed` | `completed`, `interrupted` or `failed` (`mapCodexTurnStatus`, `:359-372`) |

   Item status follows `codexItemStatus` (`:392-414`): `inProgress` →
   running, `declined` → cancelled.
6. **Approvals** (`:5277-5640`).

   | Codex request | Kind | Answer |
   |---|---|---|
   | `item/commandExecution/requestApproval` | command | `{decision: accept \| acceptForSession \| decline \| cancel}` |
   | `item/fileChange/requestApproval` | file-change | same as above |
   | `item/permissions/requestApproval` | permission | the requested profile with scope `turn` / `session`, or an empty profile (`:609-621`) |

   `serverRequest/resolved` closes a request that Codex settled itself.
   `requestUserInput`, elicitations, dynamic tool calls and auth refresh get
   JSON-RPC `-32601`; Otto's client does not offer them.
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

### What Otto does NOT port: policy auto-approval

T3 Code auto-answers by runtime mode. Reads are always allowed and
full-access allows everything (`clientPolicy.ts:143-200`, `adapter.ts:1268-1276`).
Codex and Claude get `on-request` / `never` / `bypassPermissions` in the
wider modes. Otto runs **only the approval-required mode**: every question
reaches a person. Only a provider's own config allow-rules (for example
`~/.claude/settings.json` `permissions.allow`) apply, exactly as in a PTY
session.

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
| `turn_item.updated` | `turn_item {id, turn_id, ordinal, status, title, started_at, completed_at, updated_at, type, …}`; `type` is one of the item types below (`:1495-1819`) |
| `runtime_request.updated` | `runtime_request {id, turn_id, kind: command\|file_read\|file_change\|mcp_elicitation\|permission, status: pending\|resolved\|expired\|cancelled, response_capability: live\|not_resumable, created_at, resolved_at, decision?}` (`:1205-1231`) |
| `plan.updated` | `plan {id, turn_id, steps[{id, text, status: pending\|running\|completed}], explanation?}` (`:1320-1370`) |
| `command_output.delta` | `{item_id, chunk}`. The only delta, never persisted |
| `turn.terminal` | `{turn_id, status: completed\|interrupted\|cancelled\|failed, failure?, reason?}` |

Turn item types:

| Item type | Fields |
|---|---|
| `user_message` | |
| `assistant_message` | `message_id`, `text`, `streaming` |
| `reasoning` | `text`, `streaming` |
| `todo_list` | |
| `file_change` | `file_name`, `diff_str?`, `old_str?`, `new_str?` |
| `command_execution` | `input`, `output?`, `exit_code?` |
| `dynamic_tool` | |
| `web_search` | `patterns` |
| `approval_request` | `request_id`, `request_kind`, `prompt?`, `reason?`, `options[{decision, label}]` |
| `system_notice` | |
| `error` | `failure` |

Approval decisions (`ProviderApprovalDecision`, `providerPolicy.ts:58-64`)
are `accept | accept_for_session | accept_always | decline | cancel`.

**Envelope.** Each event carries `{session_id, sequence, ts}` flattened
beside the payload.

**Not ported** (no Otto consumer yet): `app_thread.created`,
`node.updated`, `subagent.updated`, `background_work.updated`, and the item
types for checkpoints, handoffs, forks, user-input requests and
file_search. `message.updated` is also not ported, because the
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
  | ACP | session id | `session/load` |

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
  (`RestartContinuation.ts:33-36`); Otto does not.

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
   codex, `--rules` for grok);
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

None of the four drivers passes a skip-permissions flag. Claude:
`--permission-mode default`. Codex: `untrusted`. Grok:
`--permission-mode default`. Gemini: its default.

## 8. Code map

| Path | What |
|---|---|
| `crates/otto-core/src/agent_runtime.rs` | event model + REST DTOs |
| `crates/otto-agent-protocol/src/driver.rs` | `StructuredDriver` contract (sans-IO) |
| `crates/otto-agent-protocol/src/{claude,codex,acp}.rs` | the drivers |
| `crates/otto-agent-protocol/src/runtime.rs` | processes, turns, sequencing, coalescing, replay, persistence, recovery |
| `crates/otto-agent-protocol/src/{host,process,ring,http}.rs` | safety host, child I/O, replay ring, `/runtime-sessions` router |
| `crates/otto-agent-protocol/src/bin/otto-fake-{agent,codex}.rs` | test doubles (ACP agent, `codex app-server`) |
| `crates/otto-state/src/agent_runtime.rs`, migration `0197_agent_runtime.sql` | persistence |
| `ui/src/lib/api/types/agent-runtime.ts` | TS mirror |
| `ui/src/modules/threads/runtime/projection.ts` | event fold + derivations (T3 Code's projection reducer, pending requests, phase) |
| `ui/src/modules/threads/runtime/runtimeThreads.svelte.ts` | session rows, live subscription, replay after a gap / reconnect |
| `ui/src/modules/threads/runtime/Runtime{Thread,WorkRow,ApprovalRow}.svelte` | timeline, work-log rows, inline approvals, composer with Stop and a queued follow-up |
| `ui/e2e/desktop-threads-runtime.spec.ts` | E2E against `otto-fake-codex` (global-setup's codex shim execs it for `app-server`) |

## 9. Gaps and next steps

- **Threads UI.** Runtime threads raise the shared "needs you" flag while
  an approval waits, sit in History once closed (read-only, renamable),
  and take the view's slash commands (`/btw` side chat — itself a PTY
  thread — `/diff`, `/files`, `/terminal`, `/new`, `/fork`, `/review`);
  any other `/command` goes to the agent as typed (T3 Code's provider
  commands). Not yet: image attachments (the prompt contract is text only;
  the Claude and Codex drivers would need image content), model / effort
  switching mid-thread, and the per-thread terminal drawer.
- **`ottod` teardown** calling `AgentRuntime::shutdown()`, and eviction of
  idle live entries.
- **Runtime modes.** auto-accept-edits and full-access are unported; they
  would need an admin setting, as `agent_skip_permissions` is for PTY
  sessions.
- **Features T3 Code has that Otto does not yet:** restart continuation,
  images and attachments in prompts, steering a running turn
  (`turn/steer`), Codex `thread/unarchive`, Claude thinking summaries and
  the subagent tree, ACP v2.
- **Named provider accounts and network profiles** (refused today).
- **Otto MCP tools** for structured sessions.
