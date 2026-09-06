# Conversation view, History, Tasks board & Outputs

Status: **design, rev 4 (final)** (2026-09-05; after a corpus census of 1,816 Claude files / 311,757 records and 889 Codex rollouts). Turns every agent session into a
Claude/Codex-app-style conversation you can toggle with the terminal, adds a
searchable History of past sessions, a per-session **Tasks** view (the agent's
plan) that the Mission Control board can push sub-tasks into, and an **Outputs**
panel (artifacts the agent produced, with previews).

The single most important decision: the conversation is **rebuilt from the
provider's own transcript on disk** (Claude `~/.claude/projects/<cwd-slug>/<sid>.jsonl`,
Codex `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<sid>.jsonl`), *not* from PTY
scrollback. Otto already stores `sessions.provider_session_id` and knows how to
find Claude transcripts (`otto-sessions/src/lifecycle.rs`); Codex rollouts are
matched by cwd today and get a persisted path here. The parser is **one shared
crate extracted from `otto-usage`**, so cost, usage and the chat agree.

---

## 1. What exists today (and is reused)

| Piece | Where | Reused for |
|---|---|---|
| Session rows + provider ids + transcript path resolution | `otto-state` sessions, `otto-sessions::lifecycle` | locating the transcript |
| PTY prompt submission | `SessionManager::submit_text` (`otto-sessions/src/manager.rs`) / `review_session::submit_prompt` — NOT `send_input`, which pastes `text\n` as one burst and never submits | the chat composer + board nudge |
| Activity trail + `agent_tasks` (TodoWrite) + Claude Stop/PostToolUse hook ingest | `routes/activity.rs`, migration 0016 (`agent_tasks`), `ui/src/lib/stores/activity.svelte.ts`, `panels/ActivityPanel.svelte` | task tracker, live signals |
| Mission Control surfaces (needs_you / working / …) | `routes/mission.rs`, `agents/MissionControl.svelte` | the board that gets sub-tasks |
| Work graph (`work_items`, `work_artifacts`) | `otto-workgraph`, migration 0083 | artifact registration |
| Right panel tabs (Git/Files/Activity/Canvas/Info/Browser) | `ui/src/shell/RightPanel.svelte` | new **Tasks** and **Outputs** tabs |
| Markdown: `marked` + `sanitizeHtml` combined in `ui/src/modules/vault/mdRender.ts` (NOT `lib/md.ts`, which is unsanitized) · sandboxed iframe: `product/MockupViewer.svelte` · diffs: `git/DiffViewer.svelte` (takes `DiffResp`) · windowing: `lib/components/VirtualList.svelte` · per-session persisted keys: `workspace.svelte.ts::winKey` · resizable pane: `shell/RightPanel.svelte:27-44` splitter | rendering |
| Existing transcript readers | `otto-usage` (`parse_claude_line`, `parse_codex_line`, `CursorStore`, `SeenKeys`), `otto-orchestrator/claude_pty.rs` (250 ms JSONL poll), `otto-channels/transcript.rs`, `otto-improve/digest.rs`, `routes/handover.rs`, `review_session.rs` | the new crate is extracted from `otto-usage`'s parsers so cost/usage stay consistent |

## 2. Transcript corpus facts (full local corpus: 1,816 Claude files / 311,757 records; 889 Codex rollouts)

**Claude** record `type`s: `assistant`, `user`, `attachment`, `system`
(`turn_duration`, `stop_hook_summary`), `queue-operation` (`enqueue|dequeue|remove`),
`ai-title`, `last-prompt`, `mode`, `permission-mode`, `atis-latch`, `bridge-session`,
`file-history-snapshot/delta`, `cost-state`, `pr-link`. Content blocks: `tool_use`,
`tool_result`, `thinking` (empty text, see below), `text`, `image` (base64 PNG).
`toolUseResult` is on the **user** record and is per-tool structured (Bash:
`stdout/stderr/interrupted`; Edit: `structuredPatch`; Write: `content`; Read:
`file{filePath,content,numLines,startLine,totalLines}`; Agent: `agentId`;
ToolSearch: `matches: string[]`; TaskCreate: `{task}`; TaskUpdate:
`{taskId,statusChange,updatedFields}`) or a **bare string** (1.5%). Pseudo-tags in
user text: `<system-reminder>`, `<task-notification>…`, `<command-name>/
<command-message>/<command-args>`, `<local-command-stdout>`, `<local-command-caveat>`,
`[Image #N]`. `attachment` records (`total_tokens_reminder` = 72% of them,
`output_style`, `skill_listing`, `deferred_tools_delta`, `queued_command`,
`edited_text_file`, `task_reminder`, `hook_*`, `remote_session_change`, …) are
system noise: hidden by default, one collapsed chip per turn.

**Codex**: `session_meta{cli_version,…}` · `turn_context` · `world_state` · `event_msg`
(`task_started`, `item_completed{item.type ∈ UserMessage|AgentMessage|Reasoning|
CommandExecution|FileChange|McpToolCall|ContextCompaction|Extension}`, `token_count`,
`task_complete{last_agent_message}`, `agent_message`, `user_message`,
`patch_apply_end`, `mcp_tool_call_end`) · `response_item` (`message` role
user/assistant/**developer**, `reasoning`, `custom_tool_call`/`_output` (`exec`),
`function_call`/`_output`) · `token_usage_record` · `compacted`.

**agy**: history is NOT JSONL — it is SQLite (`~/.gemini/antigravity-cli/conversation_summaries.db`:
`conversation_id, title, preview, step_count, parent_conversation_id, nesting_depth,
raw_summary BLOB`; 0 rows here) with encrypted usage (`ottod/src/usage_tailer.rs`).
The adapter is a stub yielding `unsupported`; the UI shows the terminal only.

**Census facts that shape the parser** (full local corpus):
- Claude `thinking` blocks are **empty** (4,186 of 4,192 have `thinking: ""`, only a
  signature is persisted) → thinking is a *marker*, never a body.
- Codex reasoning is **never recoverable** (`summary: []` in 36,201/36,201;
  `Reasoning.summary_text/raw_content` empty in 26,486/26,486) and it is the most
  common item type → dropped but **counted** (`stats.reasoning_steps`).
- Codex has **two eras**: `item_completed` items exist only for CLI ≥ 0.147
  (2026/06–07: 193 rollouts with none). Older rollouts must be read from
  `event_msg agent_message/user_message/patch_apply_end` + `response_item
  function_call/function_call_output` (4,290 each). **Era is decided per FILE**:
  new era iff the file contains any `item_completed`; in a new-era file
  `response_item` records are used only for stats (never rendered) so nothing is
  double-counted. `ordinal` is absent on old-era records.
- Subagents: `<sid>/subagents/*.jsonl` is flat and includes depth-2/3 agents spawned
  by *sibling* subagents (45% unreachable from the parent's `Agent` results). The
  tree comes from the `<agent-id>.meta.json` sidecars
  (`{agentType, description, toolUseId, parentAgentId, spawnDepth, model}`); inside
  a subagent file `sessionId` is the PARENT's id — key on the top-level `agentId`.
- Assistant `message.content` always has exactly one block; records of one
  `requestId` interleave with user `tool_result` records (parallel tools) → group
  by `requestId` across the assistant subsequence, never by line adjacency.
- Robustness: `toolUseResult` is a bare string 1.5% of the time; `user.message.content`
  may be a bare string; `ToolSearch.matches` is a list of strings; `Read.file` is
  `{filePath, content, numLines, startLine, totalLines}`; Codex `FileChange.changes`
  is an object keyed by absolute path; `AgentMessage.content` blocks use `"Text"`;
  `sessionId` and `session_id` coexist; sidecar records (`ai-title`, `last-prompt`,
  `mode`, `permission-mode`, `atis-latch`) carry no `uuid/parentUuid/timestamp`.
- Files are **append-only** (prefix sha256 stable across live growth; `ai-title` is
  re-emitted identically up to 145× per session, never rewritten) → the tailer is
  a plain offset reader; redundant sidecars are deduped, not re-read.
- Sizes: Claude 1.08 GiB / 1,816 files (65 > 2 MiB); Codex 2.14 GiB / 889 files
  (**393 > 2 MiB**, largest 18 MB). Codex filename timestamps are LOCAL time, record
  timestamps UTC. Only untyped records in the corpus are 28 synthetic fixtures under
  `/var/folders/**/T--tmp*` project dirs → the corpus test skips those dirs.

## 3. Normalized model (the contract between the parser, the API and the UI)

Lives in `crates/otto-transcript/src/model.rs` (serde) and is mirrored in
`ui/src/lib/api/types.ts` under a `// ── Transcript` section.

```ts
interface Transcript { session_id: string|null; provider: 'claude'|'codex'|'agy';
  title: string|null; cwd: string|null; model: string|null;
  cursor: string;                  // opaque "<record_index>" of the FIRST record of the OLDEST returned turn;
                                   // pass it back as `before` (exclusive) to page earlier. Records are
                                   // append-only so the index is stable; the live tail uses `after_cursor`
                                   // in the WS delta = index of the LAST folded record.
  has_earlier: boolean;            // true when `before` can page further back
  turns: Turn[];
  stats: { turns: number; tool_calls: number; cost_usd: number|null; input_tokens: number|null;
           output_tokens: number|null; duration_ms: number|null; reasoning_steps: number;
           thinking_steps: number; unknown_records: number };
  subagents: SubagentMeta[];       // FULL tree from `subagents/*.meta.json` (§2)
  unavailable_reason: string|null  // set (with turns:[]) when no transcript resolves: 'no_provider_session_id' |
                                   // 'transcript_missing' | 'provider_unsupported' | 'codex_rollout_unresolved'
}
interface SubagentMeta { agent_id: string; parent_agent_id: string|null; depth: number;
  agent_type: string; description: string; model: string|null; tool_use_id: string|null }

interface Turn { id: string;       // Claude: assistant → requestId (else uuid); user → uuid. Codex: new era → turn_id
                                   // + ":u"|":a"; old era → "r<record_index of first record>". Stable across re-parses.
  role: 'user'|'assistant'; ts: string|null; blocks: Block[]; duration_ms: number|null;
  model: string|null; system: SystemNote[] }   // attachments / reminders / hooks, collapsed

type Block =
  | { kind:'text'; md: string }
  | { kind:'thinking'; count: number }                                  // marker only (no text is persisted)
  | { kind:'image'; id: string; media_type: string; alt: string|null }  // GET …/transcript/images/{id}
  | { kind:'tool_call'; id: string; name: string; tool: ToolKind; title: string;
      input: unknown; result: ToolResult|null }
  | { kind:'subagent'; agent_id: string; description: string; agent_type: string;
      status:'running'|'done'|'error'|null }                            // children come from `subagents[]`
  | { kind:'tasks'; tasks: TaskItem[] }                                 // list state AFTER the call
  | { kind:'queued'; op:'enqueue'|'dequeue'|'remove'; text: string; injected: boolean } // Claude queue-operation;
                                                                         // injected = content is a <task-notification>/system payload
  | { kind:'artifact'; artifact: Artifact }
  | { kind:'notice'; note: SystemNote };

type ToolKind = 'shell'|'read'|'edit'|'write'|'search'|'agent'|'mcp'|'skill'|'web'|'ask'|'task'|'other';
// Claude name → kind: Bash→shell · Read→read · Edit/MultiEdit/NotebookEdit→edit · Write→write ·
// Grep/Glob/ToolSearch→search · Agent→agent · mcp__*→mcp · Skill→skill · WebFetch/WebSearch→web ·
// AskUserQuestion→ask · TodoWrite/TaskCreate/TaskUpdate→task · else other.
// Codex: CommandExecution/exec/shell→shell · FileChange(add)→write, (update/delete)→edit · McpToolCall→mcp ·
// function_call by name (apply_patch→edit, shell→shell) · else other.
interface ToolResult { ok: boolean; text: string|null; truncated: boolean; bytes: number;
  image_ids: string[]; patch: string|null; file_path: string|null }
interface TaskItem { ext_id: string|null; title: string; status:'pending'|'in_progress'|'completed'; active_form: string|null }
// TodoWrite → title = todo.content, active_form = todo.activeForm, ext_id = null.
// TaskCreate → title = input.subject, ext_id = toolUseResult.task.id (when the result is an object with `task`).
// TaskUpdate → ext_id = input.taskId, status from input.status.
interface SystemNote { kind:'system_reminder'|'task_notification'|'command'|'hook'|'attachment'|'compaction'|'other';
  title: string; body: string|null }
interface Artifact { id: string;   // sha1(kind + ':' + (path ?? url)) — dedup is PER PATH, last producing turn wins
                                   // (turn_id = that turn); opaque, stable; the ONLY handle the file route accepts
  kind:'file'|'pr'|'image'|'report'|'url'; label: string; path: string|null; url: string|null;
  mime: string|null; produced_at: string|null; turn_id: string }
interface HistoryEntry { session_id: string|null; provider: 'claude'|'codex'; title: string|null;
  first_prompt: string|null; cwd: string; repo_name: string|null; started_at: string;
  last_active_at: string; turns: number|null; status:'running'|'idle'|'exited'|'reconnectable'|'on_disk';
  transcript_path: string; resumable: boolean }
```

Parser guarantees (unit-tested on redacted fixtures, and on the whole local corpus):
- **Lossless, quiet.** Every record maps to a block, a system note, or a stat.
  Unknown records → `notice{kind:'other'}` + `stats.unknown_records`; the corpus
  test requires 0 outside `/var/folders/**/T--tmp*` project dirs.
- Tool result text capped at 64 KB (`truncated:true`). Images are extracted once
  to `<data>/transcripts/<sid>/img/<id>.<ext>` and served by id, never inlined.
- Claude `[Image #N]` prose references resolve to the turn's image blocks.
- Claude: turn grouping by `requestId` across the assistant subsequence;
  `system{turn_duration}.durationMs` → `duration_ms`; `ai-title` (last wins) →
  `title`; **cost/tokens come from per-line `message.usage` deduped by
  `(message.id, requestId)` exactly as `otto-usage` does** (`cost-state` is only
  a fallback), so the chat header and the Usage module never disagree.
- Subagent tree from `subagents/*.meta.json`; a `subagent` block attaches to the
  turn whose `Agent` tool_use id equals `toolUseId`; `agentId` (not `outputFile`)
  is the key; `?sub=<agent_id>` reads `subagents/agent-<agent_id>.jsonl` and
  returns a Transcript with **turns + stats.tool_calls/turns only** — `title`,
  `cost_usd`, `duration_ms`, `subagents` are null/empty, paging works the same.
- Codex: two eras (§2) chosen per record; `Reasoning` counted, not rendered;
  `FileChange.changes` object keyed by path; `"Text"` blocks.
- `toolUseResult` is typed `serde_json::Value` everywhere (bare strings occur).

## 4. Backend

### 4.1 `crates/otto-transcript` (extracted from `otto-usage`)
Move `parse_claude_line` / `parse_codex_line` / `CursorStore` / `SeenKeys` out of
`otto-usage` into the new crate, **re-exported from `otto-usage` under the same
paths**; the serialized shapes of `<data_dir>/usage_tailer.json`,
`usage_tailer_seen.json` and the dedup marker are frozen (a test loads a
checked-in copy of each existing file format), so `ottod/src/usage_tailer.rs`
neither double-counts nor rebuilds — this lands first in Track A, then add `fold(records) -> Transcript`, the two-era Codex adapter, the
subagent tree reader, image extraction, and `Tailer { path, offset, partial_line }`
(plain offset reader; a shrinking file = replaced → restart at 0). Fixtures:
`crates/otto-transcript/fixtures/{claude,codex-new,codex-old}/*.jsonl` (+ a
`subagents/` dir with `.meta.json` sidecars), ~20 small files produced by
`scripts/redact-transcript.py` (paths → `/repo/…`, emails, tokens, base64 image
data → a 1×1 PNG). Corpus test `#[ignore]` (`OTTO_TRANSCRIPT_CORPUS=~`): parse
every Claude/Codex file, assert no panic and `unknown_records == 0`; Track A
records the per-type counts in §8 (≥100 sessions, long and short).

### 4.2 Resolving a session's transcript
`otto-sessions` gains `pub fn transcript_path(session) -> Result<PathBuf, UnavailableReason>`:
Claude → `claude_transcript_path`; Codex → `pub fn codex_rollout_path(psid)`
(today `codex_sessions_root`/`codex_rollout_match` are private and match by cwd;
the path is **persisted on the session row** as `sessions.transcript_path`
(migration `0121_sessions_transcript_path.sql`) the moment the capture scan
finds it, so later lookups are O(1)); agy → `ProviderUnsupported`. A session
without a resolvable transcript gets `200 { unavailable_reason }`, never 404.

### 4.3 Routes (`crates/otto-server/src/routes/transcript.rs`)
```
GET  /sessions/{id}/transcript?before=<cursor>&limit=<turns>&sub=<agent_id>      → Transcript (omit before = last `limit` turns; `has_earlier` drives "Load earlier")
GET  /sessions/{id}/transcript/images/{img_id}                                    → image bytes (inline, nosniff)
GET  /sessions/{id}/artifacts                                                      → Artifact[]
GET  /sessions/{id}/artifacts/{artifact_id}                                        → bytes (§4.7 confinement) — by opaque id, never by client path
POST /sessions/{id}/tasks   { title, description? }                               → AgentTask (§4.5)
POST /sessions/{id}/inbox   { filename, mime, data_b64 }                          → { path }  (image/* only, 10 MB, stored <data>/sessions/<id>/inbox/)
GET  /workspaces/{ws}/history?q=&provider=&cwd=&status=&before=&limit=            → HistoryEntry[]
GET  /workspaces/{ws}/history/transcript?path=<transcript_path>&before=&limit=&sub= → Transcript (path must resolve under ~/.claude/projects or ~/.codex/sessions; §4.7)
POST /workspaces/{ws}/history/import { provider, transcript_path }                → Session (reconnectable)
POST /workspaces/{ws}/history/rescan                                               → 202 (background index refresh; progress via WS)
```
**RBAC (`policy.rs`)** — explicit arms ABOVE the `/sessions/{id}/*` catch-all:
GETs on `/sessions/{id}/transcript*` and `/sessions/{id}/artifacts*` →
`Require(Agents, View)`; `POST /sessions/{id}/tasks|inbox` → `Require(Agents, Edit)`;
`/workspaces/{wid}/history*` → `Agents` View (GET) / Edit (POST). The
placeholder is **`{wid}`** (as `routes/activity.rs`), registered and matched
identically; `tests/policy_coverage.rs` must stay green and is part of Gate 1. Share-scoped tokens keep their existing feature guard.

**WS events** (`otto-core/event.rs`, classified `Scope::Session` in
`ws_events.rs` — both carry `workspace_id` AND `session_id`; never `Everyone`):
`transcript_appended { workspace_id, session_id, cursor, turns }` (≤ 64 KB or the
client re-fetches), `artifact_added { workspace_id, session_id, artifact }`,
`history_index_progress { workspace_id, scanned, total, done }` (Scope::Workspace).

### 4.4 Tailer supervisor (`otto-server/src/transcript_tail.rs`)
Per live agent session with a resolvable transcript: poll every 700 ms (the
orchestrator polls the same files at 250 ms), fold, broadcast. Starts on the
first `GET …/transcript` for a live session, stops 60 s after exit or after
5 min with no subscriber (subscribers = WS clients that fetched it; tracked in
memory). Cap 32 concurrent tails; beyond that no live push, reads still work.

### 4.5 Tasks: board → agent, and back
- Migration `0122_agent_tasks_source.sql`: `agent_tasks.source TEXT NOT NULL DEFAULT 'agent'`
  (`'agent' | 'user'`), `description TEXT`.
- `ActivityRepo::replace_tasks` (today `DELETE … WHERE session_id = ?`) is
  changed to **delete only `source='agent'` rows**, then merge: a `source='user'`
  row whose normalized title equals an incoming task (or whose `ext_id` equals
  `TaskUpdate.taskId`) takes the agent's status and `ext_id`; ids stay stable
  (rows are updated in place by `(session_id, ext_id ?? normalized title)`, no
  regenerate-on-sync).
- `routes/activity.rs` learns `TaskCreate`/`TaskUpdate` (Claude) and — for Codex,
  which has no task tool — nothing: the Codex Tasks list shows only user-added
  tasks plus a "Codex does not publish a plan" hint.
- **Summary counts**: `workspace_summary_inner` keeps counting ALL rows, so
  user-added tasks appear in `done/total` everywhere (Navigator, SessionView,
  Mission Control) — asserted by a test.
- `POST /sessions/{id}/tasks` inserts the row with `nudge_pending=1` (column in
  0116). **One owner delivers nudges**: a `nudge_sweep` in `otto-server`
  (`agent_tasks_nudge.rs`) driven by `Event::SessionStatus` plus a 15 s tick,
  independent of transcript tails. It submits ONE prompt via
  `SessionManager::submit_text` — `Otto board: new task — "<title>". <description>
  Add it to your task list and do it next.` — when the session is `Idle`, or
  after a **max defer of 120 s** while `Working` (Otto's `Idle` only means "no PTY
  output for 5 s", not "turn finished", so a spinner or a permission prompt would
  otherwise defer forever; the CLI queues typed input during a turn). If
  `prompt_guard` reports a pending approval, the sweep waits for it to clear
  (no max-defer override). Then `nudge_pending=0`, `nudged_at` set.

### 4.6 History index
Migration `0123_transcript_index.sql`:
`transcript_index(path PK, provider, provider_session_id, cwd, title, first_prompt,
started_at, last_active_at, mtime, size, turns, indexed_at)`. A background scan
(started at daemon boot with low priority, and by `POST …/rescan`) walks both
roots, skips unchanged `(mtime,size)`, reads head 64 KB + tail 16 KB only, and
tolerates files appearing mid-walk; progress via `history_index_progress`.
`GET …/history` merges Otto `sessions` (all statuses incl. archived; `status`
from the row) with index rows not claimed by any `sessions.transcript_path` /
`provider_session_id` (`status:'on_disk'`). Import creates a `reconnectable`
session row with `provider_session_id` + `transcript_path`, so the existing
`resume_args` path continues it.

### 4.7 Artifacts + file confinement
Folded from `Write`/`Edit`/`FileChange`, `pr-link` + PR URLs, images, and files
written under the scratchpad/temp/data dirs with previewable extensions
(`.html .md .png .jpg .svg .pdf .csv .json`). Registered into `work_artifacts`
best-effort. **Serving**: only `GET /sessions/{id}/artifacts/{artifact_id}` —
the server maps the id back to the path it folded itself, then applies the
`routes/fs.rs` discipline: `canonicalize` → `guard_file` allow-list (session
cwd, daemon data dir, the per-user temp dir from `std::env::temp_dir()`; NOT
`/tmp`) + deny-list (`.git`, `.env*`, keys) → 25 MB cap → `nosniff`, inline for
images/HTML, attachment otherwise. `history/transcript?path=` uses
`otto_core::paths::resolves_under` (symlink-aware) against the two provider
roots — it is the one route that accepts a client path, and it accepts only
those roots.

## 5. UI

### 5.1 View toggle in `SessionView.svelte`
Segmented **Terminal · Chat · Split** in the header; persisted per session via a
`winKey`-style localStorage key (`otto_session_view:<id>`); default *Terminal*
for every session (Chat was the default when a transcript resolved until
2026-09-06; it is opt-in per session now).
*Split* = chat left / terminal right with a splitter copied from
`RightPanel.svelte:27-44`. With the right panel open, Split shows **three
columns**; below 1200 px Split degrades to Chat with a "Terminal" tab button.
`⌘⇧C` cycles.

### 5.2 `agents/conversation/ConversationView.svelte`
Props: `{ sessionId?: string; transcriptPath?: string; workspaceId: string; readonly?: boolean }`
(exactly one of `sessionId` / `transcriptPath`; the latter uses the history route).
- **List**: user turns right-aligned bubbles; assistant prose full-width via
  `vault/mdRender.ts` (sanitized). Windowed with `VirtualList.svelte`, last 60
  turns first, "Load earlier" via `before`, scroll anchored; auto-follow when at
  bottom, "↓ new" pill otherwise.
- **Work steps**: consecutive `tool_call` blocks collapse into "Worked for 21m 17s
  · 38 steps"; each row: kind icon, title, status dot; expand → capped output
  `<pre>` / diff (`git/DiffViewer.svelte` fed a `DiffResp` built from `patch`) /
  file chip (opens Files panel) / subagent card (lazy `?sub=`, nested from
  `subagents[]`). Images inline + lightbox. `thinking` → "Thought (n)" marker;
  Codex → "N reasoning steps (not recorded)" footer per turn.
- **System notes**: one muted chip per turn, global "Show system" toggle.
  `queued` blocks → chips ("Queued: …") that disappear on `dequeue`/`remove`;
  injected ones hidden unless "Show system".
- **Composer** (`sessionId` mode, editor role, session not exited): multi-line,
  `⏎` send via `submit_text`, `⇧⏎` newline, `/` passthrough, image paste/drop →
  `POST …/inbox` → inserts `[Image: <path>]`; status line = session status +
  pending board nudges. Exited → "Resume" (existing flow).

### 5.3 History (`#/history`, `agents/history/HistoryPage.svelte`)
Route is `#/history` (NOT under `#/agents/…`, whose second segment is a session
id). Left: search + filters, grouped by repo/cwd; right: read-only
`ConversationView` (`transcriptPath` mode for `on_disk`, `sessionId` mode
otherwise). Actions: Resume in Otto (import → resume), Open folder, Copy path,
Archive. Entry points: Agents header button, palette, sidebar item (Agents group).

### 5.4 Tasks — extend the Activity panel, no new tab
`ActivityPanel.svelte` already renders per-session tasks with a progress bar;
it gains: the `from board` badge for `source:'user'`, the pending-nudge state,
**+ Add task** (→ `POST /sessions/{id}/tasks`), and the Codex hint. The trail
below is unchanged.

### 5.5 Mission Control
Cards with a `session_id` show `done/total` from the existing
`GET /workspaces/{wid}/activity/summary` counts and get **+ Sub-task** (inline
input → `POST /sessions/{id}/tasks`). Clicking a card opens the session in Chat.

### 5.6 Outputs (right panel tab **Outputs**, `panels/OutputsPanel.svelte`)
Artifact list + preview: HTML in `sandbox=""` iframe (copy `MockupViewer`'s
srcdoc/blob approach), Markdown via `mdRender`, images `<img>`, PDF in a
sandboxed iframe, others download. Gated like the other tabs on an active agent
session; the History page embeds the same component inline under the
conversation for `on_disk` entries.

## 6. Security & limits
- All new routes have explicit `policy.rs` arms (§4.3); WS events are
  `Scope::Session`; transcript prose/tool output never reaches other workspaces.
- Paths: the server derives every file path itself except
  `history/transcript?path=` (two roots, symlink-aware). Artifacts are served by
  opaque id with `fs.rs`-style canonicalize + allow/deny + cap.
- Rendering: `mdRender` (sanitized) for prose and tool text; HTML only in
  `sandbox=""` iframes; images `nosniff`.
- Size: paging by turns, 64 KB block cap, incremental tail, head+tail index.
- Input: the composer submits exactly the typed text (plus an explicit
  `[Image: path]` line); the only Otto-authored prompt is the board-task line,
  sent only when the session is idle.

## 7. Delivery plan (3 tracks, strict ownership)

**Track A · Rust** — owns `crates/**`, `docs/contracts/**`, `scripts/redact-transcript.py`.
§4 in full: crate extraction + fixtures + corpus test; `transcript_path` +
`codex_rollout_path` + migration 0121; routes + RBAC arms + WS scope; tailer;
tasks merge + nudge + 0116; history index + 0117; artifacts + confinement;
inbox upload; contracts (`api.md`, `ws.md`). Gate 1 = tests + clippy +
`ottod` build + the corpus numbers in §8.

**Track B · UI conversation** — owns `ui/src/modules/agents/conversation/**`,
`ui/src/modules/agents/SessionView.svelte`, `ui/src/lib/stores/transcript.svelte.ts`,
`ui/src/lib/api/types.ts` (§3 types incl. `HistoryEntry`, `Artifact`,
`AgentTask.source/description/nudge_pending`, and the three events),
`ui/src/lib/events.svelte.ts`, `ui/src/App.svelte` (WHOLLY: the Split
three-column layout, the `#/history` route branch that mounts C's `HistoryPage`,
and the right-panel gating), `ui/e2e/desktop-conversation.spec.ts`.
B consumes `ui.rightOpen` / `ui.rightWidth` read-only. **Gate 0**:
`conversation/index.ts` exporting `ConversationView` with the final §5.2 props,
plus EVERY §3 type (incl. `HistoryEntry`, `Artifact`, `AgentTask` additions) in
`types.ts`, within minutes. Any type gap C later finds goes through §8 (B edits),
never a direct edit by C.

**Track C · UI history/tasks/outputs/board + docs** — owns
`ui/src/modules/agents/history/**`, `ui/src/modules/panels/{ActivityPanel,OutputsPanel}.svelte`,
`ui/src/shell/RightPanel.svelte` (Outputs tab), `ui/src/lib/stores/ui.svelte.ts`
(the `RightTab` union), `ui/src/modules/agents/{MissionControl,AgentsPage}.svelte`,
`ui/src/lib/stores/activity.svelte.ts`, `ui/src/lib/router.svelte.ts` +
`ui/src/lib/sidebar.ts` (route + entry; C exports `HistoryPage` from
`agents/history/index.ts` and asks B via §8 to mount it in `App.svelte`),
`ui/e2e/desktop-history-tasks.spec.ts`,
`docs/features/conversation-view.md` + the feature index line.

Rules: only A runs cargo (`CARGO_INCREMENTAL=0`); B/C run `npm run check` /
`npm run build`; e2e only after Gate 1 with `OTTO_E2E_BIN=target/debug/ottod`;
nobody commits; cross-track needs are appended to §8.

## 8. Hand-offs & gates (append-only)

- C → B: mount `HistoryPage` (exported from `ui/src/modules/agents/history/index.ts`) at `#/history` in `App.svelte` (route key `history`, registered in `router.svelte.ts`/`sidebar.ts` by C).
- Gate 0 (B): stub landed — `ui/src/modules/agents/conversation/index.ts` exports `ConversationView` (final §5.2 props) and every §3 type + the three WS events + `AgentTask.source/description/nudge_pending` are in `ui/src/lib/api/types.ts` under `// ── Transcript`.
- B → A: e2e test hook needed to point a seeded session at a fixture transcript (the throwaway daemon runs with `OTTO_E2E=1`, `provider: 'shell'` sessions have no provider_session_id). Proposal: when `OTTO_E2E=1`, `POST /workspaces/{wid}/sessions` honours `meta.e2e_transcript_path` (absolute path to a Claude-format JSONL, e.g. one of `crates/otto-transcript/fixtures/claude/*.jsonl`) by writing it into `sessions.transcript_path` (so `GET /sessions/{id}/transcript` folds it, provider taken from the fixture) — and `history/transcript?path=` additionally accepts that same path when `OTTO_E2E=1`. Please record the exact fixture file B should use (one with user+assistant turns, ≥1 tool_call with output, a system-reminder note) here when it lands.
- B note: the module router (`{#if moduleName === …}` switch) lives in `ui/src/shell/App.svelte`, not `ui/src/App.svelte` (a boot gate only). B mounts `HistoryPage` at `#/history` there — one import + one branch — and touches nothing else in that file. Composer submit is wired to `POST /sessions/{id}/input {text, submit:true}` behind `conversation/api.ts::submitPrompt` until A names the `submit_text` endpoint here (one-line swap).
- C → B: `events.svelte.ts` currently forwards only `trail_appended`/`tasks_updated` to `activity.applyEvent` — please also forward `artifact_added` and `history_index_progress` (the activity store now handles both: Outputs panel list + History rescan progress).
- C → B: `npm run check` fails in `types.ts` — `ArtifactKind` is declared twice (the pre-existing work-graph union at ~1361 and the new transcript one at ~7431: "Duplicate identifier"). Suggest renaming the transcript one to `TranscriptArtifactKind` (C uses `Artifact["kind"]` and does not depend on the name).
- B → C: done — `ArtifactKind` (transcript) renamed to `TranscriptArtifactKind` (`Artifact.kind` unchanged); `events.svelte.ts` now forwards `artifact_added` + `history_index_progress` to BOTH `transcript.applyEvent` and `activity.applyEvent`; `HistoryPage` is mounted at `#/history` in `ui/src/shell/App.svelte` (`moduleName === 'history'` branch). B also exposes `transcript.historyIndex` (latest progress) if you prefer it over the activity store.
- B → A (nice-to-have, not blocking): the contract has no per-turn reasoning count, so the Codex "N reasoning steps (not recorded)" footer renders once per conversation from `stats.reasoning_steps`. If `Turn` ever gains `reasoning_steps: number`, B will move it per response.
- **Gate 1 (A): PASSED** (2026-09-05). `CARGO_INCREMENTAL=0 cargo test -p otto-transcript -p otto-usage -p otto-state -p otto-sessions -p otto-server` — every suite green, verbatim: otto-transcript `test result: ok. 55 passed; 0 failed; 0 ignored` (lib) · `5 passed` (fixtures) · `3 passed` (usage_formats) · `1 ignored` (corpus); otto-usage `22 passed` + e2e `7 passed`; otto-state `172 passed; 0 failed`; otto-sessions `119 passed; 0 failed` + isolation `11 passed`; otto-server `614 passed; 0 failed; 1 ignored` + `policy_coverage` `1 passed` + all 14 integration suites `0 failed`. `cargo clippy --workspace --all-targets -- -D warnings` → `Finished` (clean; the first run flagged 4 nits — sort_by_key / while-let / redundant guard / is_multiple_of — fixed). `cargo build -p ottod` → `Finished dev profile`; binary **`/Users/tech-ai/otto_os/target/debug/ottod`** (sha256 `7cba66f6…3395`). **Corpus test** (`OTTO_TRANSCRIPT_CORPUS=$HOME cargo test -p otto-transcript --release --test corpus -- --ignored --nocapture`, 9.2 s wall): **claude** files=1793 (incl. subagent files; `/T--tmp*` synthetic dirs skipped) records=313,965 turns=51,632 tool_calls=54,350 thinking=34,691 images=9 subagent_blocks=83 artifacts=1,270 **unknown_records=0**; longest file 2,257 records (`-Users-tech-ai-otto-os/c747733f….jsonl`), largest 17,013,764 bytes; **codex** files=889 records=202,267 turns=1,760 tool_calls=32,507 reasoning=36,201 artifacts=1,741 **unknown_records=0**; longest 1,737 records, largest 18,385,740 bytes; **panics=0**. Fixtures: 22 redacted files + 4 subagent sidecars under `crates/otto-transcript/fixtures/{claude,codex-new,codex-old}` via `scripts/redact-transcript.py` (paths→`/repo/<hash>`, ids→placeholders, prose word-redacted, base64→1×1 PNG; leak-scanned). Migrations 0115/0116/0117 landed; contracts in `docs/contracts/api.md` ("Conversation view, History, Tasks board & Outputs") and `ws.md`.
- A → B: (1) `POST /sessions/{id}/input {text, submit:true}` now submits via `SessionManager::submit_text` (paste + real Enter) — no swap needed, keep using it. (2) `Turn.reasoning_steps: number` landed (per-turn Codex reasoning count, additive, default 0) — move the footer per response. (3) E2E hook landed exactly as proposed: with `OTTO_E2E=1`, a session whose `meta.e2e_transcript_path` is an absolute JSONL is folded from that file (provider from the filename: `rollout-*` = codex, else claude); `history/transcript?path=` and `history/import` also accept any `…/crates/otto-transcript/fixtures/**.jsonl` under `OTTO_E2E=1`. Use **`crates/otto-transcript/fixtures/claude/01-basic-tools.jsonl`** (18 user records / 79 total: user + assistant turns, Bash + Edit tool calls WITH results, 15 attachment notes + 1 stop-hook note — the `system_reminder`-kind note does not occur in the redacted corpus, so assert on `attachment`/`hook` notes; `stats.unknown_records == 0`). For Codex use `fixtures/codex-new/01-command-agent-message.jsonl`. (4) One extra route beyond §4.3: `GET /workspaces/{wid}/history/transcript/images/{img_id}?path=` (images for `on_disk` entries; same confinement).
- A → C (API summary for `docs/features/conversation-view.md`): `GET /sessions/{id}/transcript?before&limit&sub` returns `Transcript` (last `limit` turns, default 60; `cursor` = record index of the oldest returned turn → pass as `before`; `has_earlier`; `unavailable_reason` ∈ `no_provider_session_id|transcript_missing|provider_unsupported|codex_rollout_unresolved` with HTTP 200; agy always unsupported). Images: `GET …/transcript/images/{id}` (hex id from `image` blocks / `ToolResult.image_ids`, extracted once to `<data>/transcripts/<psid>/img/`). Outputs: `GET /sessions/{id}/artifacts` (`Artifact[]`, id = sha1(kind:path|url), deduped per path, mirrored best-effort to `work_artifacts`) and `GET …/artifacts/{artifact_id}` (bytes by opaque id only; allow-list session cwd + data dir + `std::env::temp_dir()`, fs.rs deny-list, 25 MB cap, `nosniff`, inline for images/HTML else attachment). Tasks: `POST /sessions/{id}/tasks {title, description?}` → `AgentTask{source:"user", nudge_pending:true}` + `tasks_updated`; the nudge sweep (`agent_tasks_nudge.rs`, SessionStatus events + 15 s tick) submits `Otto board: new task — "<title>". <description> Add it to your task list and do it next.` when the session is idle, after ≤120 s while working, never while an approval prompt is on screen; plan syncs (TodoWrite / `PUT …/tasks`) now MERGE (user rows survive, ids stable; TaskCreate/TaskUpdate upsert by ext_id); `GET …/activity/summary` counts all rows; Codex publishes no plan. Inbox: `POST /sessions/{id}/inbox {filename, mime, data_b64}` → `{path}` (image/*, 10 MB). History: `GET /workspaces/{wid}/history?q&provider&cwd&status&before&limit` → `HistoryEntry[]` (sessions with a resolvable transcript, all statuses, own-only for non-admins, merged with unclaimed indexed files as `status:"on_disk"`); `GET …/history/transcript?path&before&limit&sub` (path confined to `~/.claude/projects` / `$CODEX_HOME/sessions`); `POST …/history/import {provider, transcript_path}` → reconnectable `Session` (existing owner returned instead; 409 if another workspace); `POST …/history/rescan` → 202, progress via `history_index_progress` (boot scan runs silently; head 64 KB + tail 16 KB peek; unchanged `(mtime,size)` skipped). WS: `transcript_appended{workspace_id,session_id,cursor,turns}` (turns re-sent whole, replace by id; `turns:[]` when >64 KB → refetch), `artifact_added{…,artifact}`, both session-scoped; `history_index_progress{workspace_id,scanned,total,done}` workspace-scoped. Live tail: armed by any transcript GET on a live session, 700 ms poll, ≤32 concurrent, stops 60 s after exit / 5 min without a fetch. Limits: tool text 64 KB (`truncated:true`), Codex reasoning counted not rendered, Claude thinking = marker only. Unresolved gaps for the docs: `on_disk` entries are machine-wide (not workspace-scoped) by design; artifacts only cover Write/Edit/FileChange, PR links and images (files created via shell redirection are not detected).
- C → A: `GET /workspaces/{wid}/history` skips a session unless `sessions.transcript_path` / `transcript_path(home, provider, cwd, psid)` resolves to a file, so a seeded e2e session (`provider:"shell"`, `meta.e2e_transcript_path` = fixture) never appears in History even though `GET /sessions/{id}/transcript` folds it. Please make the list honour the same hook under `OTTO_E2E=1` (path = `meta.e2e_transcript_path`, provider from the filename, like the transcript route) — C's e2e `history lists a seeded session` relies on it.
- **Gate 1b (A): PASSED** (2026-09-05, C's e2e follow-ups). (1) `GET /workspaces/{wid}/history` now resolves every session through the same `resolve_transcript` as the transcript route (persisted path → roots → the `OTTO_E2E` `meta.e2e_transcript_path` hook), so a seeded e2e session appears. (2) Provider roots are configurable: `OTTO_TRANSCRIPT_ROOTS=<claude root>:<codex root>`; under `OTTO_E2E=1` and no override they default to the EMPTY `<data_dir>/e2e-transcripts/{claude,codex}` — the throwaway daemon's boot index scan, history confinement and session resolution never touch `~/.claude` / `~/.codex`; documented in `docs/contracts/api.md` ("Provider roots"). `otto-sessions` gained `transcript_path_in_roots` / `pub codex_rollout_path_under`. Verbatim: `cargo test -p otto-server` → `test result: ok. 614 passed; 0 failed; 1 ignored` + all integration suites `0 failed` (policy_coverage `1 passed`); otto-sessions lifecycle `8 passed; 0 failed`; `cargo clippy -p otto-server -p otto-sessions --all-targets -- -D warnings` → `Finished` (clean); `cargo build -p ottod` → `Finished dev profile`, `/Users/tech-ai/otto_os/target/debug/ottod` sha256 `af4f1d36…52e4`.
- **Gate 2 (B): e2e PASSED** (2026-09-05). `ui/e2e/desktop-conversation.spec.ts` on Chromium 1280×900 (throwaway config, deleted) with `OTTO_E2E_BIN=target/debug/ottod`, fixture `claude/01-basic-tools.jsonl` seeded via `meta.{nested_provider:'claude', e2e_transcript_path}` (a bare `shell` session is `provider_unsupported` — C's spec uses the same shape): `4 passed (37.6s)` — default-Chat + user/assistant turns · tool step expands · Show system toggles + persists · Terminal/Chat/Split persists across reload, Split degrades <1200px, ⌘⇧C cycles. `npm run check` 0 errors 0 warnings; `npm run build` ok. A's per-turn `Turn.reasoning_steps` is consumed (footer per response).
- C → A: (1) the board-task gate you are adding — please derive the provider via `effective_provider()` (kind Agent + `shell`-with-`meta.nested_provider ∈ claude|codex` must pass; that is the only e2e-seedable shape, and it is also what a real captured shell looks like). (2) Outputs e2e: fixture `claude/10-write-structured-patch.jsonl` writes `/repo/be1739db.md`; `GET /sessions/{id}/artifacts/{id}` cannot serve it (path does not exist / outside allow-list). Under `OTTO_E2E=1`, could `/repo/<name>` resolve to `crates/otto-transcript/fixtures/files/<name>` (or the artifact bytes come from the Write `content`)? C asserts "markdown rendered OR error state" until then.
- **Gate 1c (A): PASSED** (2026-09-05, code-review fixes). Landed: (1) `u64_of` rejects non-finite/negative/≥2^64 numbers; all token/duration accumulation is `saturating_add`; new fixture `fixtures/claude/15-overflow.jsonl` (1e300 tokens, u64::MAX output ×2, u64::MAX durationMs ×2) asserted in `tests/fixtures.rs`. (2) `transcript_tail.rs`: RAII `Slot` frees the registry entry on any exit (panics included); stop decision + removal under ONE lock (`should_continue`); incremental fold via the new `otto_transcript::Folder` (push delta records; refold only on `TailDelta.restarted` or a Codex per-file flip); WS frame sized once with `serde_json::to_vec`. (3) Nudges: `nudgeable()` (Agent + claude/codex) enforced in `POST /sessions/{id}/tasks` AND the sweep; atomic `claim_nudge` (`UPDATE … WHERE nudge_pending=1`, rows_affected==1) with `unclaim_nudge` on submit failure; readiness = PTY uptime ≥10 s + drawn + quiet 600 ms (`wait_for_tui` rule) + no approval prompt; 120 s defer measured from first-seen-Working AND task age; `sanitize_prompt_text` on title/description and `submit_text` strips control chars (keeps `\n`/`\t`). (4) Artifact route: `Content-Security-Policy: sandbox; default-src 'none'` + `X-Frame-Options: DENY`; inline for raster images only, HTML/SVG attachment. (5) History: non-admins get only their own sessions (no `on_disk` rows); `history/transcript`, `…/images`, `import` require Admin/root unless the path is one of the caller's own sessions' transcripts. (6) `/sessions/{id}/input` uses `submit_text` only for claude/codex agents; shells/connections keep `text\n`. (7) Index prune skipped when the walk found 0 files or a root failed to list; History resolves all sessions in one `spawn_blocking` with no DB write; `transcript_index.list_page(before, limit)` pushes the cursor into SQL. (8) Caps: `Text.md`/`Queued.text`/`patch` at 64 KB, `ToolCall.input` >16 KB → preview object, `Folded::page` 2 MB byte budget (≥1 turn). Verbatim: otto-server `test result: ok. 616 passed; 0 failed; 1 ignored` + all integration suites `0 failed`; otto-sessions `119 passed; 0 failed`; otto-state `172 passed; 0 failed`; otto-transcript `58 passed` + fixtures `5 passed` + usage_formats `3 passed`; otto-usage `22 passed` + e2e `7 passed`; otto-pty `15 passed`; `cargo clippy --workspace --all-targets -- -D warnings` → `Finished` (clean); `cargo build -p ottod` → `Finished dev profile`, **`/Users/tech-ai/otto_os/target/debug/ottod`** sha256 `86ac69eb…4745`. **Corpus with `RUSTFLAGS="-C overflow-checks=on"` (release):** claude files=1800 records=316,313 turns=52,106 tool_calls=54,815 unknown_records=0; codex files=889 records=202,267 turns=1,760 tool_calls=32,361 unknown_records=0 (patch_apply_end now enriches the apply_patch call instead of a second block); panics=0; 10.7 s; `test result: ok. 1 passed`.
- **Gate 2b (B): review fixes + e2e PASSED** (2026-09-05). `sanitize.ts` rewritten two-pass to a fixpoint (remove/unwrap, then scrub attrs on every element) with a scheme allow-list (`http/https/mailto`, relative/`#`, `data:image/*` non-SVG on `<img>` only; control chars/whitespace stripped before the check); ConversationView load effect guarded by `error == null`; mounted window ≤300 turns with "Load later" + `↓ latest`, tool-output rows scroll horizontally; `activeQueued` wired (dequeue/remove hide chips); `stats.turns` keeps the server total; dead `cut` branch removed; `AgentTask.nudged_at` added; `global-setup.ts` warns when `OTTO_E2E_BIN` unset, fails when the binary is missing, and aborts on a stale daemon (bare 404 on `/sessions/{id}/transcript`). No TS unit runner in `ui/` → the sanitizer bypasses are asserted in-browser in `desktop-conversation.spec.ts` (Vite-served module). Verbatim: `npm run check` → `0 ERRORS 0 WARNINGS`, exit 0; `npm run build` → `✓ built in 7.67s`; e2e (Chromium 1280×900 throwaway config, deleted; `OTTO_E2E_BIN=target/debug/ottod` post-Gate-1c) → `5 passed (48.2s)`.
- C → A (blocking C's Mission Control e2e): `nudgeable()` reads the raw `session.provider`, so a captured shell (`provider:"shell"`, `meta.nested_provider:"claude"`) is refused by `POST /sessions/{id}/tasks` even though `resolve_transcript`/`effective_provider` treat it as claude. Please use `effective_provider()` in `nudgeable` (route + sweep). Why it matters beyond e2e: the board only lists LIVE sessions, and on the e2e daemon only a shell stays alive (CLAUDE_BIN is nonexistent), so the "+ Sub-task → POST" case can only be exercised through that shape. Reply with "A → C: nudgeable" when ottod is rebuilt.
- **Gate 1d (A): PASSED** (2026-09-05, C's `nudgeable` note). `nudgeable()` now resolves the provider through `effective_provider()` (own provider, else the captured `meta.nested_provider`) in BOTH `POST /sessions/{id}/tasks` and the sweep; `kind == Agent` still required, a bare shell (or a nested non-chat process) stays refused; unit test covers the captured-shell case. Caveat for C's docs: a terminal whose nested claude/codex has since EXITED would receive the paste at the shell prompt — the readiness check (drawn + quiet) does not detect that. Verbatim: `cargo test -p otto-server` → `test result: ok. 616 passed; 0 failed; 1 ignored` + all integration suites `0 failed`; `cargo clippy -p otto-server --all-targets -- -D warnings` → `Finished` (clean); `cargo build -p ottod` → `Finished dev profile`, `/Users/tech-ai/otto_os/target/debug/ottod` sha256 `78b639d8…5bf3`.
- **Gate 1e (A): PASSED** (2026-09-05, caveat closed). Captured shells: `nested_agent_alive()` re-runs the capture detector (`otto_sessions::nested::find_nested_agent` over the PTY root pid's process tree, provider must equal `meta.nested_provider`); `POST /sessions/{id}/tasks` → **409 `agent not running in this terminal`** when the CLI is gone, and the sweep keeps such tasks pending (no nudge, no `nudged_at`). Unit test covers alive / gone / different-agent / no-PTY branches. Verbatim: `cargo test -p otto-server` → `test result: ok. 617 passed; 0 failed; 1 ignored` + all integration suites `0 failed`; `cargo clippy -p otto-server --all-targets -- -D warnings` → `Finished` (clean); `cargo build -p ottod` → `Finished dev profile`, `/Users/tech-ai/otto_os/target/debug/ottod` sha256 `5cef7252…235a`.
