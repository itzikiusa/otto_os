# Otto Self-Improvement Engine — User & Operator Guide

Otto can watch how its agents actually performed in a workspace and quietly get
better at it: after a run it reflects on recent sessions, then proposes precise,
evidence-backed edits to that workspace's **handling skills** and **memory**.
Safe, additive edits can apply themselves; risky or out-of-scope edits queue for
your approval. It is **optional and off by default**, scoped per workspace, can
run the analysis on several agent CLIs at once for varied suggestions, and can
ping a Slack/Telegram channel the moment it learns something.

> Crate: `crates/otto-improve` (`ImprovementEngine`, `Scheduler`, `LiveEvolver`,
> `RealProposalProducer`). UI: `ui/src/modules/settings/SelfImprovement.svelte`.
> Contracts: `docs/contracts/api.md` (§ *Self-improvement engine*),
> `docs/contracts/ws.md` (§ *Self-improvement update*). Notifier:
> `crates/otto-channels/src/improve_notify.rs`.

---

## 1. What it does, in one minute

A self-reflection **run** does six things, in order:

1. Reads the workspace's `self_improvement` config.
2. Gathers the workspace's agent sessions active within a **look-back window**.
3. Turns each session's transcript into a compact **digest** (turns, skills it
   used, tool-error count, a capped text excerpt).
4. Assembles one analysis prompt (the bundled `workspace-self-reflection` skill +
   the in-scope skill files + the current memory files + the fenced digests) and
   runs it on every configured **provider**.
5. Parses each provider's JSON **proposal** — a list of proposed edits to a skill
   or memory file, each with a target, kind, risk, rationale, and evidence.
6. For each edit, decides **auto-apply vs. queue for approval** by the autonomy
   policy and a deterministic safety gate, writes/queues it, logs it in the
   version log, and emits events.

It only ever touches **skills and memory** — never your repository code, git
history, databases, or any other workspace state (see §9).

---

## 2. Overview & where it lives

| Concern | Location |
|---|---|
| Engine (run / apply / approve / reject / rollback) | `crates/otto-improve/src/engine.rs` (`ImprovementEngine`) |
| Cron scheduler (fires due runs) | `crates/otto-improve/src/scheduler.rs` (`Scheduler`, 60 s scan) |
| In-loop live evolver (improve after each interaction) | `crates/otto-improve/src/live.rs` (`LiveEvolver`, 30 s idle debounce) |
| Autonomy / safety decision | `crates/otto-improve/src/classify.rs` (`decide`, `memory_content_gate`) |
| Proposal schema + tolerant parser | `crates/otto-improve/src/proposal.rs` |
| Session → digest | `crates/otto-improve/src/digest.rs` |
| Prompt assembly + untrusted-text fencing | `crates/otto-improve/src/prompt.rs` |
| Bundled analyst skill | `crates/otto-improve/assets/workspace-self-reflection.md` |
| Path-safety (resolve target → guarded path) | `crates/otto-improve/src/pathsafe.rs` |
| Provider runner (claude PTY / codex / agy headless) | `crates/otto-improve/src/producer.rs` |
| HTTP routes | `crates/otto-improve/src/http.rs` |
| Config type | `otto-core::api::SelfImprovementConfig` |
| Domain enums (`Autonomy`, `ImprovementRisk`, …) | `otto-core::domain` |
| Channel notifier | `crates/otto-channels/src/improve_notify.rs` |
| Settings UI | `ui/src/modules/settings/SelfImprovement.svelte` |

Config is stored as a JSON block under `Workspace.settings.self_improvement` (no
dedicated table; runs and edits do have their own persisted tables). Two
timestamps in that block — `last_run_at` and `next_run_at` — are
**scheduler-managed** and not user-editable.

---

## 3. How reflection & proposals work

### 3.1 What it reflects on

A run gathers the workspace's sessions that were **active within the look-back
window** (`lookback_hours`, default 24 h). For each session with a transcript,
`build_digest` parses the claude JSONL transcript on disk into a `SessionDigest`:

- **`turns`** — count of user/assistant messages.
- **`skills_used`** — skills the session actually invoked, detected from:
  - `Skill` tool-use blocks (`input.skill`),
  - `/slash-command` mentions in user text,
  - `ToolSearch` `select:` queries (the deferred-tool selection list).
- **`tool_errors`** — count of failed tool calls (`tool_result.is_error == true`,
  plus the older `toolUseResult.is_error` shape).
- **`text`** — a truncated concatenation of user + assistant text turns (capped at
  4000 chars/session to keep the prompt bounded).

The analyst skill (`workspace-self-reflection`) tells the model what to look for:
**failures to fix** (the user corrected the agent, it repeated a failing action,
a routing decision was wrong, repeated tool errors) and **successes worth
codifying** (a resolution pattern that should become a default rule; a recurring
answer that belongs in memory).

### 3.2 What it proposes

Each provider returns one JSON `ImprovementProposal`: a `run_summary` plus a list
of `ProposedEdit`s. An edit names:

| Field | Meaning |
|---|---|
| `target_type` | `skill` or `memory` |
| `target_ref` | a skill name (e.g. `support-triage-router`) or memory filename (e.g. `MEMORY.md`, `triage-patterns.md`) |
| `kind` | `add`, `modify`, or `remove` |
| `risk` | `low` or `structural` (see §4) |
| `rationale` | why the change helps (prefixed `[via <provider>]` after merge) |
| `evidence` | the `session_id`s that justify it (**no evidence → the analyst is told not to propose it**) |
| `dedup_checked` / `dedup_quote` | the analyst must confirm it isn't already present |
| `patch.after` | the **full new file content** (not a fragment); `patch.before` is the analyst's informational view — the engine snapshots the *real* on-disk content at apply time |

The analyst is instructed to **dedup** ("if not already there"), **cite
evidence**, **stay in scope** (prefer allow-listed skills; otherwise propose
memory), and to write the **entire** new file in `after`.

### 3.3 Where edits land on disk

`pathsafe::resolve_target` maps a `target_ref` to a single guarded path and
rejects anything that could escape (no `..`, no slashes, no absolute paths, single
safe segments only; memory must be a `*.md` file):

- **Skill** → the Otto **library** entry `<data_dir>/library/skills/<ref>/SKILL.md`
  when that file exists (the library is the source of truth); otherwise the
  workspace copy `<root>/.claude/skills/<ref>/SKILL.md`.
- **Memory** → `<project_dir(root)>/memory/<ref>` (claude's per-project memory dir,
  e.g. `MEMORY.md` and sibling `*.md` notes).

### 3.4 Multi-provider runs

`providers` (default `["claude"]`) lists the agent CLIs to run the analysis on.
Each runs **independently with its own default model**, and the engine **merges
whatever succeeds** — every edit's rationale is prefixed `[via <provider>]` so
merged suggestions stay attributable, and each provider's summary is tagged
`[<provider>]`. A failing provider is logged, skipped, and surfaced in the run
summary as `(skipped: <provider>: <error>)`; it never aborts the others. Only if
**every** provider fails does the run end `failed`.

Under the hood (`producer.rs`): `claude` is driven through the orchestrator's
interactive PTY path (default model, 180 s timeout); `codex` runs headless via
`codex exec`; `agy` runs headless via claude's `-p` print mode. A malformed
proposal is retried once with a stricter "reply with ONLY the JSON object"
reminder before the provider is treated as failed.

### 3.5 Run outcomes

A run finishes in one of these states (`ImprovementRunStatus`):

- **`skipped`** — no sessions in the window, or (for live/evolve) no transcript yet.
- **`done`** — at least one provider produced a proposal; `applied`/`pending`
  counts recorded.
- **`failed`** — every provider failed; the error is stored on the run.

(`running` is the in-progress state.) Scheduled runs **always advance the
schedule** — even on skip/fail — so a broken run can't busy-loop the scheduler.

---

## 4. Tiered autonomy: auto-apply vs. approval queue

Every proposed edit gets exactly one disposition — **Apply** (written now) or
**Queue** (`pending`, awaiting human approval) — computed by `classify::decide`.
Two guardrails run **before** the autonomy policy and can only ever force an edit
*toward* the queue; they can never widen what auto-applies.

### 4.1 Guardrail 1 — the skill allow-list (blast-radius guard)

A **skill** edit may auto-apply only if its `target_ref` is in the workspace's
`skill_allowlist`. A skill edit to a name **not** on the list **always queues**,
on every autonomy setting. An empty allow-list is therefore a hard "propose-only"
guarantee for skills — Otto can suggest skill changes but will never write one
without you adding that skill to the list. (Memory is *not* gated by the
allow-list; it is workspace-local — but see the next guard.)

### 4.2 Guardrail 2 — the deterministic memory content gate

Memory steers every future agent in the workspace, so a memory edit must pass
`memory_content_gate` before it can ever auto-apply — independent of the
model-reported risk/target. The gate **queues** (does not auto-apply) any memory
edit that:

- exceeds **8 KiB** of new content (`MEMORY_AUTO_APPLY_MAX_BYTES`), or
- contains a chat/tool **role marker or prompt-escape sequence** — e.g.
  `<|im_start|>`, `[system]`, `system:`, `assistant:`, `ignore all previous
  instructions`, the untrusted-content sentinels, etc.

A memory **removal** carries no new attacker content, so it bypasses the content
gate (it's still recoverable via the backup + version log). This gate is the
floor under the autonomy policy: a self-reported "low risk" memory edit that
smuggles an injection marker is queued even under `auto`.

### 4.3 The autonomy policy

After both guardrails pass, `autonomy` (an `Autonomy` enum) decides:

| Autonomy | Behavior |
|---|---|
| **`tiered`** *(default)* | Auto-apply iff `risk == low`; `structural` edits queue. |
| **`propose`** | **Everything** queues for approval (nothing auto-applies). |
| **`auto`** | Auto-apply every edit that passed the guardrails. |

**Risk** (`ImprovementRisk`) is what the analyst assigns per edit:

- **`low`** — purely additive or a clarification that removes no existing meaning
  (append a rule, add a memory note, tighten wording).
- **`structural`** — deleting/rewriting existing instructions, removing a memory
  entry, or reorganizing a section.

So the common, recommended `tiered` setup means: a *low-risk* edit to an
*allow-listed* skill, or a *low-risk* memory note that passes the content gate,
applies on its own; **everything else waits for you**.

### 4.4 Conflict guard on auto-apply

Auto-apply is a defense-in-depth atomic write (`safe_auto_apply`): it
canonicalizes the path (symlink defense), re-reads the file and **bails to the
queue if it changed since the proposal was snapshotted** (TOCTOU/concurrent-edit
guard), writes a timestamped `.bak-…` backup of the prior content, then renames a
temp file into place atomically. A conflicting edit becomes `pending` rather than
clobbering your file.

---

## 5. Reviewing, approving & rolling back

Queued and applied edits live in a per-workspace **version log**
(`ImprovementEdit` rows) and surface in **Settings → Self-Improvement**:

- **Pending approvals** — each card shows the target ref, target type, kind, risk
  chip (`structural` is highlighted), rationale, evidence session ids, and a
  collapsible **before/after diff**. **Approve** applies the edit (with the same
  conflict check); **Reject** marks it rejected and changes nothing on disk.
- **Recent runs** — a log of the last 50 runs with status, trigger, sessions
  reviewed, applied/pending counts, summary, and any error.

Edit lifecycle (`ImprovementEditStatus`):

| Status | Meaning |
|---|---|
| `pending` | queued, awaiting approval |
| `applied` | written to disk (by the system on auto-apply, or by you on approve) |
| `rejected` | you declined it; file untouched |
| `rolled_back` | a previously-applied edit was reverted |
| `conflict` | the file changed under the engine; the edit was not (re)written |

**Approve** (`POST /improvement/edits/{eid}/approve`) re-checks that the file
still matches the snapshot; on a mismatch it records `conflict` instead of
clobbering. **Reject** (`/reject`) just marks the edit `rejected`. **Rollback**
(`/rollback`) reverts an *applied* edit: it restores `before_content`, or deletes
the file if the edit had created it — again conflict-checked against the content
it wrote. The actor (the user id) is recorded on every action.

> The UI also offers **Evolve now** (per-session live evolve) and **Run now**
> (manual workspace run) buttons — see §6.4 and §8.

---

## 6. Configuration

All settings are per workspace, edited in **Settings → Self-Improvement** (the
workspace picker scopes the page) or via `PUT /workspaces/{id}/self-improvement`.

### 6.1 Fields

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `false` | Master switch for **scheduled** runs. Off by default. |
| `cadence_minutes` | u32 | `60` | How often a scheduled run fires (min 1). |
| `lookback_hours` | u32 | `24` | How far back to gather sessions (min 1). |
| `skill_allowlist` | string[] | `[]` | Skills that may be auto-edited; comma-separated in the UI. Anything outside always queues. |
| `autonomy` | `tiered` \| `propose` \| `auto` | `tiered` | See §4.3. |
| `providers` | string[] | `["claude"]` | Agent CLIs to run analysis on. Empty resolves back to `["claude"]`; at least one is always kept. |
| `live_evolve` | bool | `false` | Watch live sessions and evolve their skills after each interaction (§6.4). |
| `last_run_at` / `next_run_at` | timestamp | — | **Scheduler-managed**, read-only. UI shows "Next run". |

### 6.2 Enabling

1. Pick the workspace.
2. Set **Enabled** on (and choose **Autonomy** — `tiered` is recommended).
3. Add the skills you trust to auto-edit to the **Skill allow-list** (leave empty
   to keep all skill edits in the approval queue).
4. Choose **Providers** (chips of installed agent CLIs; `claude` always present).
5. Optionally set **Run every (minutes)** and **Look back (hours)**.
6. **Save.** Changing config recomputes the next run lazily, so an enabled
   workspace becomes due on the next scheduler scan.

### 6.3 Cadence & scheduling

The `Scheduler` supervisor scans **every 60 s**: for each non-archived workspace
it reads the config and, if `enabled` and **due**, spawns a run. A workspace is
*due* when enabled and either it has never run (`next_run_at` is unset) or
`next_run_at <= now`. Exactly one run per workspace at a time is guaranteed by an
in-memory in-flight set **plus** a DB `has_running` check. After each run the
schedule advances to `now + cadence_minutes`.

### 6.4 Live evolve (in-loop)

With `live_evolve` on (or a session carrying `meta.evolve == true`), the
`LiveEvolver` watches that workspace's live **agent** sessions on the event bus.
When a watched session goes **Idle**, it arms a **30 s debounce**; if the session
stays idle (the interaction concluded) and the transcript has **grown** since the
last evolve, it runs a single-session evolve pass focused narrowly on the
skill(s) that one session used. It re-arms when the session next goes Working.
Live evolve uses **only the first configured provider** (not the full fan-out) to
stay cheap per turn, and **does not touch the cron schedule**. It reuses the same
gate, version log, and approval flow as a scheduled run.

You can also trigger a single-session evolve on demand from the **Evolve now**
button (the focused session) → `POST /sessions/{id}/evolve` (see §8).

---

## 7. Channel notifications (opt-in)

The self-improvement notifier (`improve_notify.rs`) can push a concise one-line
message to a Slack/Telegram channel whenever something self-improvement-related
happens, so a user watching only chat sees what Otto learned without opening the
UI.

- **Opt-in, default OFF.** Gated on the boolean settings key
  **`channels.notify_self_improvement`** in the daemon's key/value settings store.
  The flag is re-read **per event**, so toggling it takes effect live without a
  restart.
- **Target chat.** The integration's configured *default* chat
  (`Integration.channel_id`) — the same chat the bot already operates in — for the
  **event's workspace**. If no enabled integration for that workspace has a
  default chat, the event is skipped silently.
- **No secrets, no diff bodies.** Only names and counts are posted.
- **Best-effort.** A slow/failed send is logged and swallowed; broadcast lag is
  skipped. Nothing here can stall or crash the engine.

The three events it forwards and their messages:

| Event | Message (example) |
|---|---|
| `ImprovementEditApplied` | `Self-improvement: skill \`support-triage-router\` — applied` |
| `ImprovementApprovalPending` | `Self-improvement: proposed edit to memory \`MEMORY.md\` — needs approval` |
| `ImprovementRunFinished` | `Self-improvement run: 2 applied, 1 queued` |

A finished run with **0 applied and 0 queued** produces no ping. Memory targets
are described as `memory \`<file>.md\``; skill targets as `skill \`<name>\``.

> The same notifier also gates other event families behind their own opt-in keys
> (`channels.notify_insight_ready`, `notify_swarm_done`, `notify_review_done`,
> `notify_budget_exceeded`). Those are out of scope for this feature.

See **[Slack & Telegram channels](./channels-slack-telegram.md)** to set up the
integration and bot token that this notifier delivers through.

---

## 8. API & contract reference

`docs/contracts/api.md` is authoritative. Paths are relative to `/api/v1`. Reads
require workspace **Viewer**; config write requires workspace **Admin**; run/edit
mutations require workspace **Editor**. (Plus per-feature RBAC: the
`SelfImprovement` capability — see `docs/MULTI-USER-RBAC.md`.)

| Method & path | Auth | Body | Response |
|---|---|---|---|
| `GET /workspaces/{id}/self-improvement` | ws viewer | — | `SelfImprovementConfig` |
| `PUT /workspaces/{id}/self-improvement` | ws admin | `UpdateSelfImprovementReq` | updated config |
| `POST /workspaces/{id}/self-improvement/run` | ws editor | — | `{ run_id }` (manual run; `409` if one is already running) |
| `GET /workspaces/{id}/improvement/runs` | ws viewer | — | `ImprovementRun[]` (≤ 50) |
| `GET /improvement/runs/{run_id}` | ws viewer | — | `{ run, edits }` |
| `GET /workspaces/{id}/improvement/edits?status=` | ws viewer | — | `ImprovementEdit[]` (default `status=pending`) |
| `POST /improvement/edits/{eid}/approve` | ws editor | — | applied `ImprovementEdit` (or `conflict`) |
| `POST /improvement/edits/{eid}/reject` | ws editor | — | rejected `ImprovementEdit` |
| `POST /improvement/edits/{eid}/rollback` | ws editor | — | rolled-back `ImprovementEdit` (or `conflict`) |
| `POST /sessions/{id}/evolve` | ws editor (`SelfImprovement:Edit`) | — | `{ run_id }` |

Notes:

- **`PUT`** sets the user-editable fields only (`enabled`, `cadence_minutes`,
  `lookback_hours`, `skill_allowlist`, `autonomy`, `providers`, `live_evolve`); an
  empty `providers` is normalized to `["claude"]`. The scheduler-managed
  timestamps are preserved.
- **`run now`** and **`evolve`** create the run row synchronously (so the id
  returns immediately) and run the heavy analysis in the background; both return
  `409 Conflict` if a run is already in progress for the workspace.
- **`evolve`** additionally requires the session to be **live**
  (`running`/`working`/`idle`); archived or exited sessions are rejected so it
  never evolves on a stale transcript.
- **`approve`/`reject`/`rollback`** authorize against the *edit's* workspace.

### WebSocket events

Workspace-scoped run/edit events (`docs/contracts/ws.md`):

```json
{"type":"improvement_run_started","workspace_id":"…","run_id":"…"}
{"type":"improvement_run_finished","workspace_id":"…","run_id":"…","status":"done|skipped|failed","applied":0,"pending":0}
{"type":"improvement_edit_applied","workspace_id":"…","run_id":"…","edit_id":"…","target_ref":"…"}
{"type":"improvement_approval_pending","workspace_id":"…","run_id":"…","edit_id":"…","target_ref":"…"}
```

And one **global** (everyone-scoped) refresh event the Settings pane subscribes to:

```json
{ "type": "improvement_updated", "kind": "run_finished|approval_pending", "id": "<run_or_approval_id | null>" }
```

`kind` is `run_finished` after a run/evolve completes, `approval_pending` when a
new edit awaits approval. The UI routes it to `improvementBus` and refreshes,
keeping a capped 30 s poll as a fallback. The TypeScript mirror in
`ui/src/lib/api/types.ts` is `{ type: 'improvement_updated'; kind: string; id?:
string | null }`.

---

## 9. Capabilities & limitations — what it can and cannot edit

**It can edit, and only edit:**

- **Skill files** — `SKILL.md` under the Otto **library** entry for an allow-listed
  skill (or, if no library copy exists, the workspace's `.claude/skills/<ref>/`).
- **Memory files** — `MEMORY.md` and sibling `*.md` notes in the workspace's
  claude per-project memory dir.

**It cannot touch anything else.** It does **not** modify your repository's source
code, run git operations, change databases, edit settings other than its own
scheduler timestamps, or write outside the resolved skill/memory paths.
`pathsafe::resolve_target` plus the canonicalize-within check reject any
`target_ref` that would escape those two guarded directories (traversal, slashes,
absolute paths, symlinked leaves).

**Bounds & merges:**

- Prompt inputs are bounded: per-session transcript ≤ 4000 chars; each skill/memory
  file read ≤ 8 KiB; at most 20 memory files.
- Candidate skills surfaced to the analyst are scoped to *allow-listed skills
  actually used in the window* — keeping the prompt focused and bounding blast
  radius.
- Multi-provider suggestions are **merged** (not voted/deduped across providers),
  each labeled by provider; the analyst's own dedup rule prevents re-proposing
  existing content within a provider's pass.

---

## 10. Security & gating — why it's off by default

This engine can write files that steer **every future agent** in the workspace,
and it learns from **untrusted** session transcripts (which may include text from
Jira/Confluence comments, external chat, etc.). It is therefore conservative by
construction:

- **Default off.** `enabled` defaults to `false`; nothing runs until you opt a
  workspace in.
- **Propose-by-default for skills.** With an empty allow-list, *no* skill edit can
  auto-apply — they all queue.
- **Deterministic memory gate.** Oversized or injection-flavored memory content is
  queued regardless of autonomy or the model's self-reported risk (§4.2).
- **Prompt-injection defense.** All untrusted session/title text is wrapped in
  unforgeable sentinel fences, with role markers defanged, code fences
  neutralized, and a standing instruction that everything inside the fence is
  *data, never instructions* (`prompt.rs`). A payload that tries to "rewrite a
  skill" or "weaken the allow-list" is to be reported as a finding, not obeyed.
- **Externally-triggered runs can't self-authorize.** A product-narrative run
  (`run_for_narrative`, triggered by Jira/Confluence data) may *name* target
  skills, but the auto-apply allow-list it uses is the workspace's **configured**
  allow-list only — a narrative can never authorize an edit to a skill you didn't
  allow-list.
- **Atomic, reversible writes.** Every auto-apply backs up the prior content and
  is conflict-checked; every applied edit is rollback-able from the version log.
- **No data exfiltration in notifications.** Channel pings carry names and counts
  only, never diff bodies or secrets, and the channel feature itself is a separate
  opt-in (§7).

For the team-deployment view (who can see/configure/run this via the
`SelfImprovement` feature capability and workspace roles), see
`docs/MULTI-USER-RBAC.md`.

---

## 11. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Run ends **skipped** with "no sessions in window" | No sessions were active within `lookback_hours`. Widen the window or run after some activity. |
| Run ends **failed** | Every provider failed; the run `error` (and `(skipped: …)` summary) names which. Check the provider CLI is installed and the prompt isn't timing out (180 s). |
| Nothing ever auto-applies, all edits queue | `autonomy = propose`, or (for skills) the `target_ref` isn't in the allow-list, or (for memory) the content gate rejected it. Check the risk chip and rationale. |
| A skill edit always queues even on `auto` | The skill isn't in `skill_allowlist` — that guard runs before autonomy. Add it. |
| A small memory note still queues | It contained a role/injection marker or exceeded 8 KiB. Inspect the diff; approve manually if it's genuinely safe. |
| Approve/rollback returns **conflict** | The file changed under the engine since the snapshot; the edit was not (re)written. Re-run to regenerate against current content. |
| No scheduled runs fire | Workspace not `enabled`, workspace archived, or daemon scheduler not running. Check **Next run** in the UI; it scans every 60 s. |
| **Run now** / **Evolve** returns `409` | A run is already in progress for that workspace; wait for it to finish. |
| **Evolve now** disabled / rejected | No focused live session — open and focus one. Archived/exited sessions are rejected. |
| Live evolve never fires | `live_evolve` off and the session lacks `meta.evolve == true`, the session isn't a live agent session, or the transcript hasn't grown since the last pass. |
| No channel pings | `channels.notify_self_improvement` is off (default), or the workspace's integration has no default chat / bot token, or the run produced 0 applied + 0 queued. See §7. |
| Settings pane not refreshing live | It relies on the `improvement_updated` WS event with a 30 s poll fallback; check the WS connection. |

---

## 12. Related docs

- [Skills library](./skills-library.md) — the bundled, versioned skills this
  engine refines; how skills resolve from the library vs. the workspace copy.
- [Vault / secrets](./vault.md) — where the bot tokens used by channel
  notifications are stored.
- [Slack & Telegram channels](./channels-slack-telegram.md) — set up the
  integration this engine's notifier delivers through.
- `docs/contracts/api.md`, `docs/contracts/ws.md` — authoritative API + event
  contracts.
- `docs/MULTI-USER-RBAC.md` — the `SelfImprovement` feature capability and
  workspace-role gating.
