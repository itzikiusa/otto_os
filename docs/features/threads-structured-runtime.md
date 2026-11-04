# Threads — structured agent runtime (how T3 Code does it → how Otto mirrors it)

Status: **phase 1** — event model, contracts, runtime plumbing, the ACP
driver (Gemini) and a fake ACP agent. Claude and Codex drivers are phase 2.
PTY sessions are unchanged.

**Attribution.** The design is ported from [T3 Code](https://github.com/pingdotgg/t3code)
(MIT, Copyright (c) 2026 T3 Tools Inc.); the licence is reproduced in
`crates/otto-agent-protocol/NOTICE`. Citations below are `path:line` in the
clone studied (commit `cefb544af3`). Ported modules say so in their headers.

## Why

Threads runs the agent CLI in a PTY, types the prompt into it, and rebuilds
the chat by tailing the CLI's transcript (`transcript_tail.rs` →
`transcript_appended`). The transcript is written when a block *completes*:
no token streaming, tool calls / approvals / "working" inferred after the
fact. T3 Code drives the same CLIs over their structured stdio protocols and
renders a live timeline from a normalized event stream. Otto ports that.

## 1. What T3 Code runs, per provider

| Provider | What T3 Code runs | What Otto runs |
|---|---|---|
| Claude | **Claude Agent SDK** (`@anthropic-ai/claude-agent-sdk ^0.3.284`, `apps/server/package.json:37`) in the first-class `claudeAgent` adapter (`apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts:37,221`); the SDK spawns the `claude` CLI itself (or `pathToClaudeCodeExecutable` when set, `:1649-1651`) and speaks its stream-json + control protocol. An ACP path via the `@agentclientprotocol/claude-agent-acp` registry package exists but is hidden for Claude (`packages/provider-acp-registry/src/server/adapter.ts:195-208`, `docs/user/providers-acp.md:8-10`) | **Phase 2, decision needed** (below): either a Rust port of the SDK's transport against `claude -p --input-format stream-json --output-format stream-json …` (no new dependency), or the ACP adapter (npm package, needs approval) |
| Codex | `codex app-server` with `["app-server", …launchArgs]` (`apps/server/src/provider/codexLaunchArgs.ts:12-15`), env `CODEX_HOME` (shadow home) + `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` (`CodexAdapterV2.ts:1556-1559`, `CodexProvider.ts:452-457`), typed client `packages/effect-codex-app-server` | **Phase 2**: `codex app-server`, same lifecycle; wire types from the installed CLI's own `codex app-server generate-json-schema` (see §6) |
| Gemini | No dedicated adapter: the ACP registry entry `gemini` (npx `@google/gemini-cli@<exact>`, `packages/provider-acp-registry/src/server/AcpRegistrySupport.test.ts:1693-1720`), launched with the registry's argv/env (`AcpRegistrySupport.ts:1906-1915`) into the generic ACP runtime | **Phase 1**: `gemini --acp` (the installed CLI; `--model <m>` when set) into Otto's ACP driver (`crates/otto-agent-protocol/src/acp.rs`) |
| Grok (reference) | `grok agent stdio`, `--permission-mode auto|default` or `agent --always-approve stdio` per runtime mode (`packages/provider-grok/src/server/acpSupport.ts:47-58`), env `GROK_OAUTH2_REFERRER` (`:66-74`) | Not planned; fits the ACP driver unchanged |

## 2. Session lifecycle

**ACP (T3 Code `packages/provider-acp/src/server/AcpSessionRuntime.ts`)** —
mirrored 1:1 in `acp.rs`:

1. `initialize` with protocol v1 and **no fs, no terminal** client
   capabilities (`:1789-1803`; the registry flavour enables terminals only
   for Devin, `provider-acp-registry/src/server/adapter.ts:299-306`). Otto
   answers any `fs/*` / `terminal/*` request with JSON-RPC `-32601`.
2. `authenticate` only after an auth-required error (`:2164-2209`). Otto
   phase 1 surfaces the error as an `error` item (sign in with the CLI once).
3. Resume: `session/load` when a native session id is known and the agent
   advertised `loadSession`, else `session/new` (`:2217-2260`); a failed load
   starts a fresh session (`adapter.ts:6997-7019`). History the agent
   replays during `session/load` is dropped (Otto keeps its own).
4. `session/prompt`; its **response** carries `stopReason`
   (`AcpSessionRuntime.ts:2619-2621`).
5. Cancel: `session/cancel`, and every open `session/request_permission` is
   answered `cancelled` (ACP rule; `:2366-2405`).
6. Permission round-trip (`adapter.ts:6353-6440`, `1255-1276`): the request
   becomes a pending runtime request + an `approval_request` item; a person's
   decision maps to exactly one option kind — accept → `allow_once`,
   accept-for-session → `allow_always`, decline → `reject_once`; no matching
   option or `cancel` → `{outcome: "cancelled"}`. Never a wider grant than
   asked for.

T3 Code also auto-answers some requests by runtime-mode policy
(`clientPolicy.ts:76-192`: reads always allowed, full-access allows all).
**Otto does not**: every request reaches a person (see Safety).

**Codex (`CodexAdapterV2.ts`)**, for phase 2: one `initialize` per process
(`clientInfo`, `capabilities.experimentalApi`, `:1765-1803`, `:233-237`) →
`thread/start {cwd, model, config}` / `thread/resume {threadId,
excludeTurns:true, …}` (`:7061-7108`) → `turn/start {threadId, input, cwd,
model, approvalPolicy, sandboxPolicy, …}` (`:744-818`) → `turn/interrupt
{threadId, turnId}` (`:7567`). Runtime mode → `approvalPolicy` /
`sandboxPolicy` (`:703-742`: approval-required = `untrusted` + read-only,
auto-accept-edits = `on-request` + workspace-write, full-access = `never` +
danger-full-access). Server→client `item/commandExecution/requestApproval`
and `item/fileChange/requestApproval` are answered `{decision: accept |
acceptForSession | decline | cancel}` (`:5277-5418`).

**Claude (SDK)**, for phase 2: `resume: nativeThreadId` or `sessionId`,
`includePartialMessages: true`, the `claude_code` system-prompt preset,
`permissionMode` from the runtime mode (`ClaudeAdapterV2.ts:1526-1660`,
`:2573-2620`), approvals through the SDK's `canUseTool` callback (`:10136`).

## 3. The normalized event model

T3 Code's adapters emit `ProviderAdapterV2Event`
(`packages/provider-core/src/server/ProviderAdapter.ts:83-183`) — **entity
upserts**, not deltas: an item is re-sent whole with its accumulated text and
a `streaming` flag. Otto's `otto_core::agent_runtime` is that model, renamed
only to snake_case fields (event `type` tags unchanged):

| T3 Code event | Otto `type` | Payload |
|---|---|---|
| `provider_session.updated` | same | `provider_session {status: starting\|ready\|running\|waiting\|stopped\|error, cwd, model, updated_at, last_error}` (`orchestrationV2.ts:946`) |
| `provider_thread.updated` | same | `provider_thread {native_thread_id, context_usage?, updated_at}` |
| `provider_turn.updated` | same | `provider_turn {id, ordinal, status: pending\|running\|completed\|interrupted\|failed\|cancelled, started_at, completed_at, turn_token_usage?}` (`:1168-1188`) |
| `turn_item.updated` | same | `turn_item {id, turn_id, ordinal, status, title, started_at, completed_at, updated_at, type, …}` with `type` ∈ `user_message`, `assistant_message {message_id, text, streaming}`, `reasoning {text, streaming}`, `todo_list`, `file_change`, `command_execution {input, output?, exit_code?}`, `dynamic_tool`, `approval_request {request_id, request_kind, prompt?, options[{decision,label}]}`, `system_notice`, `error {failure}` (`:1495-1760`) |
| `runtime_request.updated` | same | `runtime_request {id, turn_id, kind, status: pending\|resolved\|expired\|cancelled, response_capability, created_at, resolved_at, decision?}` (`:1205-1236`) |
| `plan.updated` | same | `plan {id, turn_id, steps[{id,text,status}], explanation?}` (`:1320-1370`) |
| `command_output.delta` | same | `{item_id, chunk}` — the only delta, never persisted |
| `turn.terminal` | same | `{turn_id, status: completed\|interrupted\|cancelled\|failed, failure?, reason?}` |

Envelope: `{session_id, sequence, ts}` flattened beside the payload. Not
ported (no Otto use yet): `app_thread.created`, `node.updated`,
`subagent.updated`, `message.updated` (redundant with the
`assistant_message` item), `background_work.updated`, and the item types for
checkpoints, handoffs, compaction, web/file search.

**ACP mapping** (T3 Code `runtimeModel.ts:1363-1620` + `adapter.ts`), as in
`acp.rs`: `agent_message_chunk` / `agent_thought_chunk` accumulate into one
`assistant_message` / `reasoning` item (`emitTextSegment`,
`adapter.ts:2593-2700`), any non-text update closes the segment; `tool_call`
/ `tool_call_update` merge into one item (`emitTool`, `:3711`, `:4041-4197`):
`execute` → `command_execution`, `edit|delete|move` → `file_change`, other →
`dynamic_tool`; `plan` → `plan.updated` + a `todo_list` item (`:4238-4345`);
prompt end → `turn.terminal` (`:7547-7602`; `cancelled` → interrupted).

**Who owns what.** In T3 Code adapters own their processes; the
orchestration layer (`ProviderEventIngestor.ts:458-640`) sequences, persists
and turns a failed terminal into an `error` item. Otto splits the same way
but keeps drivers **sans-IO** (`StructuredDriver`, `driver.rs`): the runtime
owns the child (so every spawn passes Otto's safety pipeline), turns
(ids, ordinals, status), item ordinals, sequencing, request resolution /
cancellation, and the process-death terminal (`transport_error`, as T3 Code,
`adapter.ts:2245-2277`). A driver returns `DriverStep {events: DriverEvent[],
writes, signal}`.

## 4. History, replay and restarts

* T3 Code stores every domain event with a global sequence
  (`persistence/OrchestrationEventStore.ts:468-490`) plus projection tables
  (`Migrations/055_OrchestrationV2.ts:55-298`); a client subscribes with
  `afterSequence` and gets the events since, or — past 128 events / 1 MiB or
  an unknown sequence — a snapshot then live (`apps/server/src/ws.ts:916-1000`,
  `ThreadStream.ts:18-20`).
* **Otto**: per-session `sequence` (strictly increasing, never reused); an
  in-memory ring (4096 events / 8 MiB) and an entity projection kept in
  memory and persisted (`agent_runtime_entities`, migration 0197: latest JSON
  per entity; streaming text written once it settles).
  `GET /runtime-sessions/{id}/events?after_sequence=` returns
  `{kind:"events"}` within the ring and T3 Code's 128 / 1 MiB limits, else
  `{kind:"snapshot"}` — the same contract.
* Sequence floor: each session row keeps `seq_reserved`; before a child is
  used 65 536 sequences are durably reserved (again whenever one reaches the
  reservation), and a restarted daemon resumes above
  `max(last_seq, seq_reserved)`, so no sequence a client saw is reused.
* **Restart** — T3 Code `ProviderRuntimeRecoveryService.reconcileProjection`
  (`:199`, `:369-450`): pending requests expire as `not_resumable`,
  non-terminal turns / items become `cancelled`. Otto does exactly that at
  boot (`AgentRuntime::recover`, called from `recover_before_serve`) and marks
  the provider session `stopped`. The next prompt respawns the CLI and
  resumes the provider's own session (`session/load`; Codex
  `thread/resume`; Claude `resume`). T3 Code can additionally auto-continue
  a restarted run (`RestartContinuation.ts:33-86`); Otto does not (yet).
* Weaker than PTY sessions, stated plainly: a PTY agent with
  `session_persistence` keeps running in its PTY holder across a daemon
  restart. A structured child is a direct child of `ottod` with piped stdio;
  when the daemon exits its stdin closes and the CLI exits, so an in-flight
  turn always ends `cancelled`. Surviving would need a stdio-proxy holder.

## 5. How the web client consumes it (T3 Code → Otto Threads, phase 2 UI)

T3 Code subscribes per thread with `afterSequence` and folds every event
into a projection (`packages/client-runtime/src/state/threads.ts:482-614`,
`orchestrationV2Projection.ts:259-373`):

* streaming text — `turn_item.updated` with `streaming: true`, replaced by
  id (`apps/web/src/components/chat/TimelineAssistantRows.tsx:29-53`);
* tool cards — `command_execution` / `file_change` items
  (`session-logic.ts:349-567`, `TimelineWorkEntryRow.tsx`) plus live
  `command_output.delta`;
* approvals — pending `runtime_request`s paired with their item
  (`threadRequests.ts:229`, `ComposerPendingApprovalActions.tsx:20-120`),
  answered with `runtime-request.respond` → Otto
  `POST /runtime-sessions/{id}/requests/{request_id} {decision}`;
* plan — `plan.updated` / `todo_list` (`ProposedPlanCard.tsx`);
* stop — enabled while a turn runs (`session-logic.ts:1137`) → Otto
  `POST …/interrupt`;
* working — the run / provider-session status (`session-logic.ts:1124-1134`).

## 6. Official crates

* ACP: `agent-client-protocol` **5.0.0** (schema 1.15.1) — Otto uses its v1
  wire types for every message it sends and parses; it does not use the
  crate's connection runtime, because the runtime (not the SDK) must own the
  process for the safety pipeline and the driver stays sans-IO and testable.
* Codex: crates.io `codex-app-server-protocol` is **0.63.0**, published from
  a fork (`namastexlabs/codex`), against an installed codex-cli **0.166.0** —
  it lags and is not OpenAI's. Phase 2 uses the installed CLI's own
  `codex app-server generate-json-schema` output (T3 Code generates its
  types the same way from an upstream commit,
  `packages/effect-codex-app-server/scripts/generate.ts:16-19`).

## 7. Safety — identical to PTY agent sessions

Every spawn goes through `SessionManager::prepare_runtime_spawn`
(`crates/otto-sessions/src/manager/runtime_spawn.rs`), which calls the same
private helpers, in the same order, as `SessionManager::create`:
`refuse_while_shutting_down` → `check_session_cwd` (refuses `/`, `$HOME`
while confined, its parents, Otto's data dir) → `trust::ensure_trusted_async`
→ the pre-spawn context hook → `apply_sandbox` **last** (the
`process_sandbox` Seatbelt profile, same `sandbox_decision`; `gemini` is not
a built-in name, so it is confined whenever the setting is on and is never
exempt by omission). A named provider account or a workspace network
profile is **refused**, not skipped.

The PTY prompt guard types keystrokes to accept folder-trust / "press
Enter" dialogs on a terminal screen. A structured child has no terminal:
trust is pre-seeded as above, and every approval arrives as a pending
runtime request **only a person can answer** — Otto does not port T3 Code's
policy auto-approval. The answer route is person-only and an `Admin`-class
approval gate in `policy::route_class`; every other write is person-only
too; reads need workspace `viewer` + owner-or-admin; WS delivery is
session-family (owner, workspace admin, root).

## 8. Phase 2 decisions and gaps

* **Claude** — mirror T3 Code's live path (the Agent SDK) by porting its
  transport to Rust against the `claude` binary already installed (no new
  dependency), or use `@agentclientprotocol/claude-agent-acp` (an npm
  install, which needs the user's approval; T3 Code installs registry npm
  agents with `npm install --global` into its own prefix,
  `AcpRegistrySupport.ts:1251-1288`).
* **Codex** driver (above). Runtime modes (approval-required / auto-accept-edits /
  full-access) mapped like T3 Code, defaulting to approval-required.
* Text coalescing (T3 Code flushes Codex deltas every 50 ms,
  `CodexAdapterV2.ts:212`); Otto re-emits per chunk today.
* Threads UI on `/runtime-sessions`; `ottod` teardown calling
  `AgentRuntime::shutdown()`; eviction of idle live entries; accounts,
  network profiles, Otto MCP tools for structured sessions; restart
  continuation; images in prompts.
