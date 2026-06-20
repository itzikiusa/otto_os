# Otto Feature Enhancement Master Plan — 2026-06-20

> **Single source of truth** for the next product phase. Otto already has enough
> breadth — this plan **enhances existing features** (performance, legibility,
> safety, connectedness) instead of adding new top-level modules.
>
> This document consolidates two passes:
> 1. **Strategic roadmap** — product direction, competitive signals, sprint
>    sequencing, priority table (was `docs/research/2026-06-20-otto-existing-feature-enhancement-roadmap.md`).
> 2. **Engineering catalog** — ~107 code-grounded enhancement ideas, each tagged
>    `PERF`/`VISUAL`/`EASE`/`SUBFEAT`, rated impact/effort, and pinned to the exact
>    `file:line` to change (produced by 9 parallel read-only code-audit agents).
>
> Every feature section below carries **both** altitudes: *Direction* (the strategic
> "why / in what order") and *Concrete enhancements* (the engineering "what and where").

---

## 0. The thesis

Otto's strongest position is **not** "we have a terminal, a DB client, a PR tool, a
Kafka UI, a Product tool, and workflows." It is that all of those exist in the **same
local agent operating environment**. The next phase should make that integration
explicit and turn the current modules into one coherent workflow:

```
story → swarm → session → branch → PR → review → fix → re-review → merge
```

…with **cost, evidence, approvals, and provenance visible the whole way**.

**Product-direction rules for this phase:**
1. Prefer subfeatures inside existing modules.
2. Prefer state visibility over hidden automation.
3. Prefer verified loops over one-shot agent actions.
4. Prefer reusable context packets over one-off prompt pastes.
5. Prefer point-of-action guardrails over Settings-only safety.
6. Prefer proof of outcome over "agent said it worked."

The recurring issue is not missing modules — it's that Otto already knows a lot of
useful state, but users need it surfaced **in the current workflow surface**.

**Bottom line — what "done" looks like for this phase:**
- Every agent run has a visible owner, goal, context, cost, and result.
- Every AI finding has evidence, status, and a way to verify resolution.
- Every external action has a policy, audit trail, and approval mode.
- Every story/swarm/review/workflow leaves traceable artifacts.
- Every expensive or risky action shows the budget and guardrail at the exact point
  where the user acts.

---

## 1. What changed since the June 19 audits (don't re-plan these)

These already shipped — the *next* work is the follow-on, not the original:

- **Product → Swarm handoff** exists (`product/PlanTab.svelte`). Next: traceability,
  execution status, artifact/PR feedback **back** into Product.
- **Usage by feature + budgets** exist (`UsagePage.svelte`, `routes/usage.rs`). Next:
  deeper work-graph attribution (repo/branch/PR/story/swarm/workflow) and budget
  status **at the point of action**.
- **Swarm run/time/cost budgets + max attempts** exist (`otto-swarm`, `swarm_runtime.rs`).
  Next: budget UX, run inspection, artifact handling, recovery confidence.
- **DB server-side cancel** exists. Next: explain-plan, schema diff/ERD, parameterized
  saved queries, import/export, safer prod workflows.
- **Terminal scrollback restore + TiledView lazy attach** exist. Next: richer terminal
  affordances, clearer lifecycle states, easier reuse.
- **Shell UX**: focused-session palette commands, shortcuts overlay, mobile action bar,
  right panel in tiled/split — present.
- **MCP server management** exists. Next: governance, health, tool discovery, secrets,
  Otto-owned MCP tools.

---

## 2. Cross-cutting themes (two lenses, highest leverage)

The biggest wins aren't per-feature — they're patterns that repeat across the app.
Two complementary lenses: **engineering** (recurring code weaknesses) and
**strategic** (recurring product gaps). They reinforce each other.

### 2A. Engineering themes (recurring code patterns — fix once, land everywhere)

- **T1 — Polling → WebSocket events** *(top priority).* Review, Product/Jira,
  Self-improvement, Workflows, Skill-eval, Brokers, Usage all re-fetch whole
  collections on a timer — and in most cases **the backend already broadcasts the
  event** (swarm's `swarm.svelte.ts:336` `applyEvent` is the reference). Same class of
  idle-loop waste as the documented "Otto Networking" CPU peg.
- **T2 — Virtualize** big diffs/lists/results/streams (`DiffViewer`, `ResultsGrid`,
  API `ResponseViewer`, swarm `RunDetail`, Usage top-sessions, history lists).
- **T3 — One shared diff component.** Git review, Jira rewrite, self-improve approvals,
  skill-eval all hand-roll or skip diffing — extract the git `DiffViewer` line/word diff
  and reuse it everywhere.
- **T4 — HTTP/connection reuse + caching.** `otto-issues` builds a fresh
  `reqwest::Client` per request (`jira.rs:166`); git providers have no ETag cache;
  dbviewer's warm tunnel cache isn't shared with connection `test`/open.
- **T5 — "Get the data out": copy / export.** Missing on Usage, DB Explorer (capped at
  the preview row limit), Brokers peek. A consistent Copy-as-JSON / Export-CSV affordance
  (server-streamed past the UI cap where the driver already streams).
- **T6 — Surface features that are built but hidden.** `channels.notify_self_improvement`
  toggle, Kafka produce headers/tombstone/base64, Kafka `key_filter`, API-token mgmt
  (`/auth/tokens` #87-89), `extra_context_md`/`include_memory` — backend exists, no UI.
- **T7 — Recency / frecency ordering.** Connections (`ORDER BY name`), command palette
  (no frecency), Jira import (no "my recent issues" default).
- **T8 — Consistent confirmation + write-gate legibility.** Mixed `window.confirm`,
  missing confirms on irreversible actions (revoke-all shares, terminate session), and a
  near-invisible DB prod write-gate — while a styled `confirmer.ask` modal already exists
  (`Channels.svelte:165`).
- **T9 — Parallelize serial tunneled round-trips.** dbviewer `schema_graph` (60 serial
  RTTs), Brokers lazy topic counts, Product N+1 testcase counts → `buffer_unordered`/batched.

### 2B. Strategic leverage themes (recurring product gaps)

- **S1 — Work-control surface inside the existing Agents shell.** A grouped work queue
  (Needs attention / Working / Review ready / Waiting / Done / Suspended); task title,
  provider/model, repo/branch, cost, last output in pane headers; named notices instead
  of generic ones; a true "needs user" state (not PTY silence); "resume all suspended" /
  "pause noisy group". *(Mostly surfacing state that already exists — pairs with T1.)*
- **S2 — Verified review loop.** Persistent **finding identity** (path + normalized body
  + nearby diff context), status across runs (open/fixing/resolved/regressed/declined),
  confidence + consensus, required evidence, "fix selected → re-review affected → mark
  resolution", and a merge-readiness gate. *(Pairs with T3.)*
- **S3 — Work-graph attribution everywhere.** Stamp `repo_id`/`branch`/`pr_number`/
  `story_id`/`swarm_id`/`swarm_project_id`/`swarm_task_id`/`workflow_id`/`workflow_node_id`/
  `origin` onto session meta + usage ingest; show cost on the surface where spend happens;
  fill swarm run tokens/cost from usage; "why did this cost so much?" drilldown.
- **S4 — Accepted-but-ignored controls cleanup (trust killers).** Product model selection
  is reserved-and-discarded (`product_run.rs` `_model`); audit `PostQuestionsReq.format`,
  `PublishVersionReq`, and any handler-accepted-but-unused fields; add an "unused field"
  test. *Make existing controls truthful before adding more.*
- **S5 — Live agent tools for existing DB / API / Brokers** (via the present MCP/settings
  infra, **not** a new module): read-only DB MCP tool (row caps, timeout, env guardrails,
  write-prohibited by default); API "send request/response/history to focused agent" +
  "generate tests from this request"; Brokers "investigate lag/topic/schema" context
  packet — each showing the exact context packet (size + secrets redaction) before sending.
- **S6 — Capability & health registry.** Shared registry (provider auth, Keychain,
  ClickHouse, issue account, Slack socket, Kafka reachability, MCP health); each module
  renders ready/degraded/missing-setup with one-click fixes; a support-bundle export;
  point-of-failure diagnostics distinguishing auth/permissions/rate-limit/outage/network.
- **S7 — Cross-module command palette & search.** Index stories/versions/questions/
  learnings/PRs/commits/branches/saved queries/API requests/workflows/swarm tasks/broker
  topics/schemas/Vault memories; action rows (open/send-to-agent/attach/copy-context/
  run/reveal); search activity trails + transcripts; recents + "most likely next action".
  *(Pairs with T7.)*

> **How the lenses connect:** T1 (polling→events) is the technical substrate for S1
> (live work-control surface). T3 (shared diff) underpins S2 (verified review). S3/S6/S7
> are mostly *surfacing state Otto already holds*. S4 is pure trust hygiene and should go
> first.

---

## 3. Competitive signals (external benchmarks)

- **GitHub Copilot cloud agent** — background task execution, branch/commit automation,
  custom agents, many entrypoints, session logs, automations.
  ([cloud agent](https://docs.github.com/en/copilot/concepts/agents/cloud-agent/about-cloud-agent),
  [custom agents](https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/create-custom-agents),
  [sessions](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/cloud-agent/start-copilot-sessions))
- **Cursor** — PR review (Bugbot), background agents, memories, MCP setup, "fix in editor"
  loops. ([Bugbot](https://cursor.com/bugbot), [1.0 changelog](https://cursor.com/changelog/1-0))
- **Postman** — APIs as agent tools, MCP. ([AI Agent block](https://learning.postman.com/docs/postman-flows/reference/blocks/ai-agent),
  [Postman MCP](https://learning.postman.com/docs/reference/postman-api/postman-mcp-server/overview))
- **DBeaver** — AI query suggestion/explanation, plan explanation, SQL error fixing,
  object descriptions, import mapping. ([AI Assistant](https://dbeaver.com/docs/dbeaver/AI-Smart-Assistance/))
- **Kafka consoles** (Conduktor / Redpanda) — multi-cluster ops, topics, schemas, consumer
  groups, offsets, audit logs, governance, chargeback.
  ([Conduktor](https://docs.conduktor.io/guide/release-notes),
  [Redpanda Console](https://www.redpanda.com/data-streaming/redpanda-console-kafka-ui),
  [Schema Registry UI](https://docs.redpanda.com/streaming/current/console/ui/schema-reg/))

**Otto's edge:** the *same coding agent* can inspect data (DB/API/Kafka) and then change
code in the repo, review the change, and merge it — in one local environment.

---

## 4. Per-feature plan

Each section: **Direction** (strategic) then **Concrete enhancements** (code-grounded,
tagged `[CAT]` · Impact H/M/L · Effort S/M/L, with `file:line`).

---

### 4.1 Agents, Sessions & Terminal
**Backend:** `otto-pty`, `otto-sessions` (manager/ws/providers/trust/prompt_guard/lifecycle), `otto-core` · **UI:** `modules/agents/`, `modules/panels/`, `shell/TabBar`, `lib/components/Terminal.svelte`, `lib/stores/workspace.svelte.ts`

**Direction**
- Clear lifecycle states: running, waiting-for-user, auth-required, suspended, resumable,
  failed-to-resume, gone-transcript. Suspended should look intentionally paused, not broken.
- Session templates (bugfix / PR review / Product story / DB investigation / support reply /
  swarm worker / shell); clone-fork-with-context (cwd, provider, attached story, extra dirs,
  skills, new-branch option); command preview before spawn/restart (binary, args, cwd, env,
  redacted secrets).
- Terminal linkifier (URL external; `path:line` → Files/repo; ⌘-click copy path); recording/
  export for debugging and share links; per-session "definition of done" checklist.
- Quick filters in Agents: mine / working / needs-me / attached-to-story / review / swarm / channel.
- Reliability: prompt-scanner lag-proofing (re-scan screen if the output channel lags);
  smarter idle classification via screen diffs + known spinners; reconnect dropped sessions
  in place (not new tabs); readiness-based first command for SSH/DB (not fixed sleep).

**Concrete enhancements**
- **[PERF] Align client scrollback with retained history** — H·S — emulator keeps 1000 (`lib.rs:116`), ring keeps 10 000 (`ring.rs:9`), but client asks 2000 and clamps to `min(total_history)` (`lib.rs:257`) → reconnect silently loses 1k-10k lines. Feed `snapshot_with_history` from the ring; tie the request to `term.options.scrollback`.
- **[PERF] Coalesce PTY output into fewer WS frames** — M·M — reader broadcasts every ≤8192B chunk (`lib.rs:130`), each its own `Message::Binary` (`ws.rs:418`). Add a time/size-bounded aggregator (`try_recv`+concat); `Lagged` already tolerates batching.
- **[SUBFEAT] Server-side terminal search** — M·M — find is xterm `SearchAddon` over the client buffer only (`Terminal.svelte:264`), lost on reconnect. Add `Search`/`search_result` frames (`ws.rs:57`) grepping the `RingBuffer`.
- **[EASE] Persist & restore grid size per session** — M·S — every PTY spawns 80×24 (`lib.rs:20`); store last cols/rows in `session.meta` on resize (`manager.rs:672`) → no reflow flash on resume; correct headless wrapping.
- **[SUBFEAT] Configurable idle-suspend grace + per-session keep-alive** — M·S — `SUSPEND_GRACE` hardcoded `5*60` (`manager.rs:63`). Read from settings; honor `meta.keep_alive` in `suspend_idle_unattached` (`manager.rs:774`); pin in `SessionView` menu. *(implements S1 "pause/keep" controls.)*
- **[VISUAL] Encode "needs you" vs idle in the status dot** — M·S — status is output-recency only (`manager.rs:1132`); thread `needsYou[id]` into `StatusDot` (amber pulse). Data already in the store. *(core of S1.)*
- **[EASE] Drag-to-reorder session tabs** — M·M — `openTabs` is a plain array (`workspace.svelte.ts:44`); add HTML5 drag + `reorderTab` + `persistTabs()`.
- **[SUBFEAT] One-click broadcast input to visible panes** — M·S — backend `broadcast_message` exists (`manager.rs:296`) but no UI; add a toggle mirroring input via `ws.sendInput`.
- **[PERF] Stop tearing down xterm/WebGL on every session switch** — M·M — `{#key sessionId}` (`SessionView.svelte:327`) rebuilds the whole xterm/WebGL per switch; reconnect the WS reactively instead.
- **[EASE] Desktop terminal toolbar (font zoom, copy-on-select)** — L·S — zoom/keys bar are phone-only (`Terminal.svelte:691`); add an optional desktop toolbar.
- **[SUBFEAT] Expose prompt-guard auto-approvals in the activity trail** — L·S — `PromptGuard` only `tracing::info!`s injected approvals (`prompt_guard.rs:188`); `record_trail` them → `ActivityPanel`.
- **[VISUAL] "X min idle / suspends in N" hint on idle panes** — L·S — suspension is silent; use `last_output_at()` (`lib.rs:289`) + grace for a header countdown.

---

### 4.2 Git, PRs & Code Review
**Backend:** `otto-git` (local.rs, providers/*, parse.rs, http.rs), `otto-server` (review_session.rs, modules.rs), `otto-orchestrator` · **UI:** `modules/git/`, `lib/stores/git.svelte.ts`

**Direction** *(S2 verified review loop is the headline)*
- **Finding identity:** fingerprint = path + normalized body + nearby diff context; track
  status across runs (open/fixing/resolved/still-open/regressed/declined); confidence +
  consensus (which agents found the same issue); require evidence (quoted hunk / file:line).
- **Verified fix:** "fix selected → re-review affected files → mark resolution"; PR-review
  handoff parity with local review; review-quality eval via Skills Evaluator on seeded PRs.
- **Merge readiness panel:** unresolved bug findings + CI/check status + approval + branch
  freshness + conflicts.
- **PR flow:** AI commit-message from staged diff; hunk/line staging; word-level diff /
  ignore-whitespace / expand-context / inline blame; PR-creation fields (draft, reviewers,
  labels, template, linked Jira); stacked PRs for agent-sized changes.
- **Safety:** destructive git previews exact files + offers undo; worktree-cleanup assistant
  for stale branches/swarms/review sessions.

**Concrete enhancements**
- **[PERF] Virtualize `DiffViewer` for large diffs** — H·M — renders every line into one `<table>` (`DiffViewer.svelte:420`); window rows, early-return `fileMatchesSearch`, memoize `splitRows`. *(T2.)*
- **[SUBFEAT] Carry stats + `too_large`/`language` on `FileDiff`** — H·M — DTO has only path/hunks (`api.rs:915`); add +/− counts, `too_large` cap in `parse.rs`, language. *(enables merge-readiness + "too large" guard.)*
- **[PERF] Paginate provider PR/repo lists** — H·M — each provider fetches one page (`github.rs:121`) and stops; follow `Link`/`next` cursor capped, or thread `page`/`cursor` through `GitProvider`.
- **[SUBFEAT] PR checks/CI + draft flag in `PrSummary`/`PrDetail`** — H·M — DTOs carry no CI/draft/labels (`api.rs:1060`); map GitHub check-runs / GitLab pipelines / Bitbucket statuses → green/red/draft pill. *(feeds merge readiness.)*
- **[SUBFEAT] Approved findings → true inline review comments** — H·M — approving posts a general comment (`modules.rs:2030`); add `comment_inline(remote,number,head_sha,path,line,body)`.
- **[SUBFEAT] Verdict + grouping + stable finding keys** — M·M — flat ≤20 comments (`modules.rs:1090`); `ReviewAgents.svelte:117` keys by `f.body` so duplicates collapse. Emit `{verdict,blocker_count,summary_md}`; group by file/severity; key by index. *(core of S2 consensus/identity.)*
- **[EASE] Poll only when visible + backoff** — M·S — `ReviewPanel` polls 2s×600, no backoff/visibility gate; gate on `visibilityState`, grow 2→5→10s. *(T1.)*
- **[SUBFEAT] Self-hosted GitHub Enterprise detection** — M·S — `detect.rs:17` handles GitLab but not GHE; add a `host.contains("github")` heuristic + honor `api_base_url`.
- **[VISUAL] Viewed-files progress + keyboard nav** — M·S — checkboxes exist (`DiffViewer.svelte:260`) but no progress / `]`/`[`/`n`/`N`.
- **[EASE] Per-config review timeout & attempt override** — M·S — grace hardcoded by diff size (`modules.rs:1133`), `MAX_REVIEW_ATTEMPTS=3` fixed; add to `ReviewConfig`.
- **[SUBFEAT] Surface truncation + "diff too large" guard** — M·S — review renders with `usize::MAX` (`modules.rs:1496`); combine with `too_large` → "open on host / load anyway".
- **[PERF] Cache provider GETs (ETag / short TTL)** — M·M — fresh request every call (`client.rs`); add `If-None-Match` cache returning cached body on 304. *(T4.)*

---

### 4.3 Product: Jira, Confluence, Discovery, Planning
**Backend:** `otto-issues` (jira.rs, confluence.rs, adf.rs), `otto-product` (service.rs, product_run.rs, product_watcher.rs) · **UI:** `modules/product/*`, `lib/stores/product.svelte.ts`

**Direction**
- **Source freshness:** last sync, changed fields, new comments, stale warning, failed-sync
  reason. **Story diff:** imported vs rewrite vs approved vs current Jira.
- **Traceability matrix:** requirements → questions → answers → test cases → plan tasks →
  swarm tasks → sessions → branches → PRs → decisions → learnings.
- **Model selection must be honored or hidden** (S4 trust killer — `_model` in `product_run.rs`).
- Per-story run presets (provider/model/lenses); learnings governance
  (suggested/accepted/disabled + evidence/confidence/last-used/feedback).
- **Product ↔ Swarm feedback loop:** show swarm project status / active+blocked tasks /
  artifacts / branch+PR / cost back in the story. PO dashboard (stale / unanswered /
  ready-for-dev / in-swarm / ready-for-QA / blocked / done).
- Plan task editor: merge/split/reorder, not only checkbox toggles. One-click "attach story
  to focused session" from every tab.

**Concrete enhancements**
- **[PERF] Reuse one HTTP client per Atlassian account** — M·M — every handler builds a fresh `reqwest::Client` (`jira.rs:166`, `confluence.rs:86`); share a `OnceLock` / cache by `account_id`. *(T4.)*
- **[PERF] Kill the N+1 testcase count** — M·S — `story_detail` (`service.rs:196`) + `get_story` (`http.rs:317`) loop `list_testcases` per run; add `count_testcases` `COUNT(*)`. *(T9.)*
- **[SUBFEAT] Push AI-run completion over `/ws/events`** — H·M — runs emit `Notice` (`product_run.rs:892`) but UI never subscribes; 4 tabs `setInterval(…,3000)`. Broadcast `ProductChanged{story_id,section,status}`. *(T1.)*
- **[PERF] `AnalysisTab` poll has no timeout** — M·S — polls 3s forever (`AnalysisTab.svelte:107`); add the `POLL_MAX_MS` cap the other tabs use.
- **[SUBFEAT] Real word-level diff in `RewriteTab`** — H·M — two raw rendered panes (`RewriteTab.svelte:254`); add `diffMarkdown` + reuse for version compare. *(T3 — also serves the "story diff" direction.)*
- **[SUBFEAT] Confluence `storage_to_markdown` fidelity** — M·M — drops tables/`ac:*` macros/images (`confluence.rs:631`); add GFM tables + macro mapping + round-trip.
- **[EASE] Debounce + persist Plan checkbox toggles** — M·S — POSTs every click overwriting the body (`PlanTab.svelte:177`); debounce `savePlan` ~500ms + "saved" tick.
- **[SUBFEAT] Surface `token_expires_at` warnings** — M·S — stored but nothing warns → 502 mid-publish; badge accounts expiring within N days.
- **[PERF] Cache `build_agent_context` issue fetch** — M·M — re-pulls `get_issue_full` per run (`service.rs:1066`); cache rendered context/JSON keyed by `updated`. *(T4 + S3.)*
- **[EASE] Search: load-more + recency default** — M·M — `maxResults=25` hardcoded (`jira.rs:276`), requires a query; add `startAt` + empty-query `assignee=currentUser() ORDER BY updated DESC`. *(T7.)*
- **[VISUAL] Loading skeleton while the Jira panel loads** — M·S — `loadIssueFull` fetches silently (`OverviewTab.svelte:205`); render the existing Skeleton.
- **[SUBFEAT] Bulk-select & reorder test cases** — M·M — one-at-a-time (`TestCasesTab.svelte`); add bulk approve + drag persisting `order_idx`.
- **[EASE] Issue-key autodetect on paste** — L·S — extend `normaliseSourceKey` to extract `PROJ-123` from a Jira URL.

---

### 4.4 Agent Swarm
**Backend:** `otto-swarm`, `otto-server/src/swarm_*.rs` · **UI:** `modules/swarm/`, `lib/stores/swarm.svelte.ts`

**Direction**
- **Run Inspector:** prompt, cwd/worktree, branch, skills, injected context, session id, raw
  output, parsed artifacts, board posts, retry count, cost. **Project artifact registry**
  (files/reports/screenshots/commits/PRs/docs/links). "What changed direction" rollup.
- **Capacity view** (parallel cap, active/queued, attempts, remaining budget, projected cost);
  **budget reason banner** when auto-paused.
- **Control:** per-agent pause/resume, per-task cancel/unblock, dependency editor w/ cycle
  prevention, drag/drop Kanban + bulk changes, save-as-template, recruit-team-from-goal,
  prefer isolated worktree (mark direct-repo mode high-risk).
- **Integration:** Product ↔ swarm status both ways; swarm tasks link to sessions/commits/PRs/
  findings/usage; "Create PR from project artifacts" guided action. *(S3.)*

**Concrete enhancements**
- **[EASE] Surface budget/attempt state (meter)** — M·S — auto-pause posts only a transient board line (`swarm_runtime.rs:263`); render `counts.total_runs`/`cost_usd`/runtime vs max as bars + inline pause reason. *(direction "capacity/banner".)*
- **[SUBFEAT] One-click "raise budget & resume"** — M·S — pause message says do it manually; add a "+N runs/+$X/+Nh and resume" PATCH+resume.
- **[SUBFEAT] Live run/task updates (auto-refresh graph)** — M·S — BoardFeed/RunsList/RunGraph have manual refresh; debounce `loadGraph(detail.id)` in `applyEvent` (`swarm.svelte.ts:336`) on task/run events. *(T1.)*
- **[SUBFEAT] Drag-and-drop Kanban** — M·M — moving a task is a right-click menu (`KanbanBoard.svelte:51`); add HTML5 drag → `updateTask({status})` + `order_idx` reorder.
- **[EASE] Inline agent add/move + skill autocomplete in OrgTree** — M·M — read-only nav; skills are free-text (`AgentEditor.svelte:49`) yet recruiter drops unknowns (`swarm_runtime.rs:810`). Add "add report here", drag-reparent via `update_agent({reports_to})`, autocomplete vs `/library/bundled`. *(supports Run Inspector + control.)*
- **[PERF] Bound the recruiter/planner skill list** — M·S — injects every library skill name (`swarm_runtime.rs:793`); cap/categorize + cache `list_skills()`.
- **[VISUAL] Findings density + severity rollup (skill-eval RunDetail)** — M·S — per-iteration fail/warn/info chips from `all_findings`; color score by pass/fail.
- **Confirmed cross-cutting:** unvirtualized RunsList/BoardFeed + per-line `<span>` diffs + raw-JSON dumps in RunInspector/RunSteps *(T2)*; static relative timestamps never tick (`ago()`/`rel()` need a 1s interval) *(VISUAL·S)*; `runForAgent` re-finds the task by title+assignee (`SwarmPage.svelte:104`) when the create call already returns the id *(EASE·S correctness)*.

---

### 4.5 Connections & Database Explorer
**Backend:** `otto-connections`, `otto-dbviewer`, `otto-ssh` · **UI:** `modules/connections/`, `modules/database/`

**Direction**
- Universal environment labels (dev/staging/prod, read-only, requires-typed-confirm).
- **Query plan UI** (EXPLAIN tree, flag full scans/missing indexes, send plan+schema to agent);
  NL-to-SQL **into the editor, not auto-execute**; saved-query **params/bind variables**
  (`QueryRequest.params` exists — make it real per engine); schema snapshot/diff between envs;
  **ERD/diagram** from FK metadata; FK-click navigation; CSV/JSON import + SQL dump/export;
  row-edit diff preview; data-masking profiles; connection health dots + last-error in sidebar.
- Performance: cache schema introspection per connection (invalidation); stream large exports;
  index query history by connection/table/error.

**Concrete enhancements**
- **[PERF] Parallelize `schema_graph` introspection** — H·M — sequential `object_detail` loop up to 60 = 60 serial tunneled RTTs (`service.rs:404`); `buffer_unordered(8)`. *(T9 — powers ERD.)*
- **[SUBFEAT] Server-side full-result export (CSV/JSON)** — H·M — export copies in-DOM rows only, capped at `DEFAULT_MAX_ROWS=1000` (`mysql.rs:264`); stream `POST …/db/export` past the cap. *(T5.)*
- **[PERF] Reuse the tunnel cache for `test`/terminal opens** — M·M — `test` spawns a fresh child per probe (`service.rs:276`); `build_command` re-handshakes `ssh -J`; route through the cached tunnel layer. *(T4.)*
- **[SUBFEAT] FK-aware "jump to referenced table"** — M·S — FK data + target `NodePath` ids already exist; make FK rows clickable in `StructureView`.
- **[SUBFEAT] Schema-tree search/filter for SQL & Mongo** — M·M — `filter` honored only by Redis (`mysql.rs:128`); thread into `objects_in_folder` + a filter input. *(T7.)*
- **[PERF] Cap dashboard widget refresh fan-out** — M·S — 20 tiles refresh in lockstep (`Dashboards.svelte:159`); jitter intervals / concurrency-limit; skip history for widget runs.
- **[SUBFEAT] Recency/favorites ordering** — M·S — `list_visible` is `ORDER BY name` (`connections.rs:93`); add `last_opened_at`/`pinned` + a "Recent" group. *(T7.)*
- **[EASE] Real write-gate modal + visible prod/read-only badge** — M·S — enforced server-side but cue is a 3px border; detect `write_blocked:` → typed-confirm modal. *(T8 + direction env labels.)*
- **[SUBFEAT] Opt-in MySQL row-count estimates** — L·S — `row_count=None` by design (`mysql.rs:224`); `approx:true` from `table_rows`, shown "~".
- **[PERF] Stream/virtualize huge result sets** — M·L — `run_read` buffers the whole set (`mysql.rs:739`); `fetch` streaming + early break; server-side sort/filter. *(T2.)*
- **[EASE] DSN/URI paste import** — M·S — only Mongo accepts a full string; add a "Paste connection URL" parser into `params`.
- **[SUBFEAT] Per-statement timeout** — M·M — no statement timeout, only cooperative cancel; add `timeout_ms` per engine.

---

### 4.6 API Client & Automations
**Backend:** REST client engine · **UI:** `modules/api/`

**Direction**
- Collection-level auth inheritance + env-override indicators; **secret variables** (hidden,
  redacted from history/exports); per-environment cookie jars; request diff + history restore.
- **OpenAPI import** (not only export); test/pre-request scripts in a sandbox; GraphQL
  introspection + query explorer; gRPC reflection browser; "generate tests from request/
  collection" into Automations/Workflows; **send request/response/error to focused agent**
  with a context preview *(S5)*.
- Performance: large/binary response streaming + saved file refs; assertion/test summary for
  automation runs; schedule collection smoke runs **via Workflows** (not a new scheduler).

**Concrete enhancements**
- **[PERF] Debounce + fast-path the JSONPath/pretty-print** — M·S — filter re-parses the whole body every keystroke (`ResponseViewer.svelte:50`); pretty-print on the main thread; debounce ~150ms + memoize + size-gate.
- **[PERF] Virtualize bodies, history, and the stream console** — M·M — full DOM render + unbounded stream append (`HistoryList.svelte:50`, `ResponseViewer.svelte:180`); cap the stream ring-buffer + virtualize. *(T2.)*
- **[EASE] Real keyboard shortcuts (send/save/cancel)** — M·S — only `Enter` sends (`RequestBuilder.svelte:529`); add ⌘↵/⌘S/⌘T + an AbortController Stop.

---

### 4.7 Message Brokers (Kafka)
**Backend:** `otto-brokers` (http/service/kafka/proxy/metrics/decode/schema_registry), `otto-ssh`, `otto-netguard` · **UI:** `modules/brokers/`, `lib/stores/brokers.svelte.ts`

**Direction**
- **Operational depth:** consumer-group offset preview/reset (typed confirm + audit); topic
  config diff + rollback; schema compatibility check before publish; dead-letter/retry helper
  (inspect + replay selected messages); lag investigation (top groups/partitions/assignment/
  stale consumers); time-travel browse by timestamp+partition; CSV/JSON export of messages +
  lag; Kafka Connect lifecycle if exposed; agent context packet (topic + samples + schema +
  lag + metadata) *(S5)*.
- **Guardrails:** stronger prod rails for produce/delete/config/schema/offset-reset; audit log
  for writes; health badge + explicit tunnel/proxy state per cluster.

**Concrete enhancements**
- **[SUBFEAT] Consumer-group offset reset / seek** — H·M — `describe_group` is read-only (`kafka.rs:501`); add `POST /groups/{group}/reset` (earliest/latest/offset/timestamp) via `commit_offsets`, gated by `guard()`+`confirm`. *(top operational gap.)*
- **[PERF] Batch lazy topic-count fanout server-side** — M·M — one HTTP call per row at `STATS_CONCURRENCY=4` (`TopicsTab.svelte:19`); add `POST /topics/stats {names:[]}` → `topic_message_counts` reusing `WATERMARK_WORKERS`. *(T9.)*
- **[SUBFEAT] Tombstone + headers + base64 in produce** — M·M — `ProduceReq` supports them (`types.rs:470`) but UI exposes only key/partition/value; add headers editor + tombstone checkbox + toggles. *(T6.)*
- **[PERF] Warm the tunnel/pool on tab open** — M·S — `client_for` opens lazily on first op (`service.rs:378`); on `brokers.select` fire the existing `/test` (`service.rs:406`) + a tunnel pill.
- **[VISUAL] Per-message timeline + richer peek table** — M·M — `PartitionRange[]` watermarks in `ConsumeResp` unused; add offset-position bar + value-preview + header badge.
- **[SUBFEAT] Live-tail (incremental)** — M·M — re-peeks the whole window every 60s from Latest (`TopicDetail.svelte:82`); track max offset, request just-past, append. *(direction time-travel/tail.)*
- **[PERF] Cap/share the metrics scrape + throughput cost** — M·S — `/metrics` every 4s runs a full watermark sweep + 8s Prometheus scrape; cache the sweep in `ClusterMetricState`, back off the UI poll. *(T1.)*
- **[EASE] Partition-skew + under-replication on Overview** — M·S — compute leadership imbalance + cluster URP (`isr.len()` vs `replicas.len()`) in the existing `overview()` pass. *(direction lag/health.)*
- **[SUBFEAT] Copy / export peeked messages** — M·S — detail shows `<pre>` only; add Copy-as-JSON + Export N → JSON/CSV. *(T5.)*
- **[EASE] Search a key across partitions / jump-to-offset** — M·S — filters apply after fetch (`service.rs:577`), key filter unwired; expose it + a "find from beginning" mode.
- **[VISUAL] Lag heatmap / sorting in GroupsTab** — L·S — add sort-by-lag + per-row bar + per-topic subtotals (data present).
- **[EASE] "Not computed" vs real zero in topic counts** — L·S — `message_count:-1` sentinel (`kafka.rs:217`) renders ambiguous `—`; add retry affordance + tooltip; drop the dead field.

---

### 4.8 Skills, Context, Self-Improvement, Skill-Eval & Vault
**Backend:** `otto-skills`, `otto-improve` (engine/scheduler/live/classify/digest/prompt), `otto-context` (materialize/library/merge/config) · **UI:** `settings/SelfImprovement.svelte`, `settings/ContextLibrary.svelte`/`ContextSoul.svelte`, `skills-eval/`, `agents/ContextPreview.svelte`, Vault

**Direction**
- **Vault:** force-directed graph layout + filters/edge-labels/focus/search-highlight; memory
  lifecycle (suggested/accepted/stale/contradicted/private-public); provenance diff (exact
  source session/story/message + confidence); "use in session" picker with token estimate;
  forget/merge/split with undo; import `AGENTS.md`/`CLAUDE.md`/`.cursorrules`/`.windsurfrules`
  as governed assets. *(Note: no code-grounded Vault items below — Vault wasn't in the audit
  pass; this is a coverage gap to close.)*
- **Skills/context:** versioning/changelog in install/update UI; skill linting (frontmatter/
  triggers/safety/examples/broadness); path-scoped rules so big repos don't load irrelevant
  guidance; context preview before spawn (files/skills/souls/MCP tools/config); tool-budget
  display.
- **Self-improvement:** outcome-aware (connect proposals → tests passed / PR merged / findings
  resolved / user accepted-rejected); chain to Skill-Eval (propose → evaluate on corpus →
  promote/queue); visible rollback preview + backup.

**Concrete enhancements**
- **[SUBFEAT] Surface the `notify_self_improvement` toggle** — M·S — gated on `channels.notify_self_improvement` (`improve_notify.rs:40`) but zero UI; add a checkbox in Channels/Notifications. *(T6.)*
- **[EASE] Live-refresh Self-Improvement on run events** — M·S — `runNow()` immediately `load()`s while the run finishes later (`SelfImprovement.svelte:112`); subscribe to `ImprovementRunFinished`/`ApprovalPending` (`engine.rs:494`). *(T1.)*
- **[SUBFEAT] Detect skills invoked via SlashCommand/deferred tools in the digest** — M·M — `digest.rs:75` counts only literal `"Skill"` tool_use; extend to slash-command/`ToolSearch` calls.
- **[PERF] Bound + async memory/skill reads** — M·M — blocking `std::fs` in async, no per-file cap (`engine.rs:653`/`:631`); `spawn_blocking` + per-file cap.
- **[SUBFEAT] Real diff in the pending-approval card** — M·M — before/after raw `<pre>` (`SelfImprovement.svelte:279`); reuse the git line-diff. *(T3.)*
- **[EASE] Per-channel "Send test message"** — M·S — status from `has_bot_token` only; post "Otto is connected ✅" via the adapter `send` (`improve_notify.rs:185`).
- **[PERF] Exponential backoff for Slack/Telegram reconnects** — L·S — fixed 3s retries (`slack.rs:21`, `telegram.rs:19`); cap-exponential, reset on success.
- **[SUBFEAT] `/restart` and `/who` bridge commands** — L·S — `bridge.rs:469` handles only 4; add restart (drop `ConvKey`) + who.
- **[EASE] Expose `extra_context_md` + `include_memory` in the Context UI** — M·M — carried (`api.rs:1514`) + injected (`materialize.rs:196`) but not editable; add a textarea + toggle persisting via `write_into_settings`. *(T6 + direction context preview.)*
- **[VISUAL] Render Slack mrkdwn / Telegram entities in relayed replies** — L·M — `mirror.rs:488` sends raw text; set Telegram `parse_mode=MarkdownV2` / Slack mrkdwn.
- **[SUBFEAT] Per-session "Evolve now" + which-skills-changed badge** — L·M — live evolve only after 30s idle (`live.rs:30`); add a manual `evolve_session` trigger.
- **[PERF] Cache `Library::list_skills`/frontmatter per materialize** — L·M — re-reads + re-parses every `SKILL.md` per spawn, then again in `describe_copy_dir`; read once and thread the body.
- **[SUBFEAT] Promote skill-eval diff to a real unified diff + apply** — M·S — hand-rolled LCS `simple_diff` (`skill_eval.rs:581`); reuse git diff; let promote target a bundled-skill name (drift→UpToDate). *(direction: chain improve→eval→promote.)*
- **[PERF] Pre-compute the skill-eval validation diff** — L·S — each validator runs `git diff` itself (`skill_eval.rs:1350`); compute once after impl, inject capped.

---

### 4.9 Workflows
**Backend:** `otto-core/src/workflows.rs`, `otto-server/src/workflow_engine.rs`, `routes/workflows.rs` · **UI:** `modules/workflows/`

**Direction** — *make Workflows the glue for existing modules, not a generic n8n clone.*
- Mark scaffold node kinds clearly (`game_engine`, `verifier`); **node schemas with generated
  forms + validation** (no raw JSON for common nodes); **module-aware nodes** (git PR, db
  query, api collection, product story, slack draft, swarm task, file write, shell); schedule +
  webhook triggers (reuse existing scheduler); import/export/duplicate as JSON; **run event
  streaming instead of polling**; per-node retry/timeout/error policy; secrets/env model;
  visual run replay on the canvas.

**Concrete enhancements**
- **[EASE] Node param editor for all node kinds** — H·M — inspector renders a field only for `agent_prompt` (`WorkflowsPage.svelte:382`); drive a form off `NodeTypeSpec` + per-kind schema (url/method/body, ms, json, game). *(direction "node schemas".)*
- **[PERF] Push workflow + skill-eval progress over WS** — H·M — `getRun` every 700ms; `RunDetail` 2s×600 (`WorkflowsPage.svelte:167`); emit `WorkflowRunUpdated`/`SkillEvalUpdated` + `applyEvent`. *(T1 + direction "event streaming".)*
- **[SUBFEAT] Node-result caching / "skip unchanged" on re-run** — M·M — full re-run re-executes every node; persist outputs keyed by `(node_id, params_hash, input_hash)`, mark "Success (cached)".

---

### 4.10 Channels: Slack & Telegram
**Backend:** `otto-channels` (manager/bridge/mirror/improve_notify/slack/telegram) · **UI:** `settings/Channels.svelte`, `settings/Notifications.svelte`

**Direction** — *support/chat is only trustworthy if sending is visible and reviewable.*
- Default customer/provider outbound to **draft/approval** mode; per-channel policy (auto-send
  internal / draft external); **audit every message** (source thread, session, response,
  outbound status, files, approver); retry/dead-letter for failed bridge events; redaction
  before injecting files/transcripts; file size/type policy + malware-scan hook placeholder;
  channel-specific prompt/context templates; **Notifications → Channels routing** (insight
  ready / swarm done / review done / budget exceeded / approval required).

**Concrete enhancements** *(the engineering items live in §4.8 — Channels/Improve/Context were one audit cluster):* surface the `notify_self_improvement` toggle, per-channel "Send test message", `/restart`+`/who` commands, exponential reconnect backoff, and mrkdwn/MarkdownV2 rendering. **New from direction:** Notifications→Channels routing is the highest-value addition — wire `improve_notify::deliver`'s path to also carry insight/swarm/review/budget/approval events.

---

### 4.11 Usage, Insights & Metrics
**Backend:** `otto-usage` (engine/clickhouse/tailer/pricing/metrics/schema), `otto-server` (routes/usage.rs, insights.rs, monitor.rs) · **UI:** `modules/usage/`, `modules/insights/`

**Direction**
- **Work-graph identity columns** (S3) + session-level cost in pane headers/InfoPanel + budget
  status at the action surface; usage export CSV/JSON; parser regression corpus (anonymized
  transcripts); unknown-model warning + fallback-rate explanation; **cost anomaly alerts**
  (retry loop / fallback / cache-miss spike); insights provenance (sources/range/providers/
  cache); "regenerate insight" with diff; quality/cost scatterplots (findings-per-$, etc.).

**Concrete enhancements**
- **[SUBFEAT] Surface `priced_as_of` + an "estimated" flag** — M·S — returned (`engine.rs:490`) but UI omits it; `is_priced` dead, unknown models silently get Opus-tier `FALLBACK` (`pricing.rs:59`); tag fallback-priced spend. *(direction "unknown-model handling".)*
- **[PERF] Single-pass summary, not 4 sequential spawns** — M·M — provider/daily/session sequential (`engine.rs:419`) + `session_totals` re-run for by-kind (`routes/usage.rs:82`) + budgets a third time; batch `--multiquery` / reuse one scan.
- **[SUBFEAT] Per-session drill-down → open the session** — M·S — rows static though enriched with title/kind/workspace (`routes/usage.rs:140`); make rows click-through. *(S3 traceability.)*
- **[EASE] Auto-refresh dashboard (opt-in)** — M·S — only on mount/manual; add interval refresh of `/usage/metrics` (Brokers pattern). *(T1.)*
- **[SUBFEAT] CSV/JSON export of rollups** — M·S — no export anywhere; serialize providers/daily/sessions/by_kind. *(T5.)*
- **[VISUAL] Real chart axes + hover tooltips** — M·M — CSS bars + axis-less sparklines (`UsagePage.svelte:381,574`); borrow `DbViz` rendering.
- **[PERF] Bound/virtualize top-sessions enrichment + table** — M·M — sequential `get()` per row + un-batched `by_kind` (`routes/usage.rs:119,140`); batch `WHERE id IN (...)` + virtualize. *(T2.)*
- **[EASE] Live insights run status (not a 2.5s guess)** — M·M — `runNow()` waits a blind 2500ms; return `run_id`, poll `/insights/reports` / subscribe.
- **[SUBFEAT] Open-in-new-tab / download report HTML** — L·S — modal iframe only; add download (blob URL in hand) + new-tab.
- **[SUBFEAT] Insights offset picker** — L·S — backend accepts `offset`; add "N periods back".
- **[VISUAL] Engine version + priced-as-of + retention gauge in install card** — L·S — under-surfaced; add version + "X days / Y MB retained".
- **[EASE] Empty state when the `insights` skill isn't installed** — L·S — `Ok(None)` with a reason silently toasts; render the reason + deep-link to Settings→Skills.

---

### 4.12 Settings, RBAC, Remote & Trust/Safety
**Backend:** `otto-rbac` (tokens/passwords/lib), `otto-keychain`, `otto-server` (feature_guard/policy/auth/routes/share/email_sender), `otto-state/grants.rs` · **UI:** `settings/`, `agents/ShareModal.svelte`, `lib/stores/auth.svelte.ts`

**Direction**
- Settings export/import (secrets excluded); backup/restore for SQLite state + context + skills
  + usage metadata; role-matrix docs+tests per route; **audit log** (settings/auth/token/destructive
  DB-git-broker/network-listener/external-sends); session/token revocation UI; security-posture
  score in Trust & Safety linked to fixes; remote/mobile hardening checklist (bind/TLS/origins/
  tunnel/rate-limits/listener status); **MCP governance** (enabled/health/tool-count/secrets/
  per-workspace trust/stale `.mcp.json` cleanup). *(S4 + S6.)*

**Concrete enhancements**
- **[PERF] Cache per-request grant/role/auth lookups** — H·M — 3 uncached SQLite hits per request (`tokens.rs:122`, `grants.rs:29`, `lib.rs:60`); short-TTL per-token `AuthContext`+grant-map invalidated by `set_grants`/`revoke*`. *(T1-adjacent: removes hot-path overhead under WS/poll.)*
- **[SUBFEAT] API-token management UI** — H·M — routes `#87-89` + `ApiTokenInfo` exist, no pane; add "Personal Access Tokens" wired to `POST/GET/DELETE /auth/tokens` (prefix + `last_seen_at` already returned). *(T6 + direction token revocation UI.)*
- **[VISUAL] "Acting as <user>" impersonation banner** — H·S — token swap persists across reloads (`auth.svelte.ts:142`), no indicator; sticky banner when `realUser.id !== me.id` + Stop + 30-min countdown.
- **[EASE] Replace `window.confirm` / add missing confirms** — M·S — native confirm (`AdminSessions.svelte:29`) + immediate revoke-all/impersonate/terminate; route through `confirmer.ask`. *(T8.)*
- **[SUBFEAT] Frecency ranking for the command palette** — M·S — score-only, no recency (`Palette.svelte:31`); persist `{commandId:{count,lastUsed}}`, blend into `fuzzyMatch`. *(T7 + S7.)*
- **[EASE] App-password presence + actionable SMTP errors** — M·S — field clears with no affordance; add a `●●●●` placeholder + Re-verify + 16-char hint.
- **[SUBFEAT] DB index for share-listing/extend** — L·S — only `idx_auth_sessions_scope`; add `(session_scope, revoked, expires_at)` migration.
- **[EASE] Search/filter + copy on Users and Audit lists** — M·S — no filter/copy/export; add filter box + copy-`@username` + copy-JSON + "Last 24h/7d" presets.

---

### 4.13 UI Shell, Navigation & Onboarding
**UI:** `shell/Rail.svelte`, `shell/App.svelte`, `shell/Palette.svelte`, `modules/help/`

**Direction** *(S1 + S7 land here)*
- First-run health checklist (workspace / provider CLIs / Keychain / ClickHouse / issue
  accounts / channels / MCP / first useful session); module empty states show prerequisites +
  one next action; recent/recommended actions from workspace state; customizable rail order +
  hidden modules; **cross-module command palette/search** (S7); better responsive/iPad baseline
  across Database/Swarm/Git/Product/Workflows; walkthroughs tied to **live app actions**.
- **Visual polish (ops app):** compact tables + clear state labels over decorative cards;
  consistent status colors across sessions/swarms/workflows/brokers/budgets/stories; empty
  states that **teach the workflow**.

**Concrete enhancements**
- **[VISUAL] Searchable, indexed in-app Help (not video-only)** — M·M — 15 hardcoded MP4s (`Walkthroughs.svelte:11`), no text/search/keyboard-nav, unreachable from ⌘K; pair each with a searchable blurb + register as palette commands + arrow-key rail. *(direction "walkthroughs + palette".)*
- *(Palette frecency + impersonation banner live in §4.12; status-dot needs-you + quick filters live in §4.1 — these are the shell-level wins.)*

---

### 4.14 Packaging, Docs & Release
*(Roadmap-only — no code-audit pass; included for completeness.)*

- Deterministic local release script (checks → UI build → daemon → sidecar copy → Tauri build
  → sign → DMG); keep expanding the contract-drift gate as routes change; migration
  rollback/recovery docs; troubleshooting docs (provider auth, Keychain, launchd, port
  conflicts, corrupted DB, broken ClickHouse, MCP spawn failures, Kafka tunnel); known
  limitations per major feature; changelog grouped by module; demo workspace seed data for
  screenshots/onboarding/tests.

---

## 5. Quick wins (engineering, Wave 1 — small effort, high daily payoff)

1. **"Acting as …" impersonation banner** (`shell/App.svelte`; data in `auth.realUser`/`me`).
2. **Command-palette frecency** (`Palette.svelte:31` + `fuzzy.ts`).
3. **Align terminal scrollback** (`otto-pty/src/lib.rs:116` vs `Terminal.svelte:173`).
4. **needs-you vs idle in the status dot** (`StatusDot`).
5. **`AnalysisTab` poll cap** (one-line guard).
6. **Usage `priced_as_of` + "estimated" flag** (`usage.svelte.ts:96`).
7. **Surface `notify_self_improvement` toggle** + **per-channel "Send test message"**.
8. **Warm broker tunnel on tab select** (`/test`) + **wire Kafka produce extras**.
9. **Clickable FK → referenced table** (DB `StructureView`).
10. **Debounce + memoize API-client JSONPath/pretty-print** (`ResponseViewer.svelte:50`).
11. **Fix `swarm` `runForAgent` to use the returned task id** (`SwarmPage.svelte:104`).

**Wave 2 (structural):** T1 polling→events rollout · T2 virtualization · T3 shared diff
component · T4 HTTP reuse · T9 parallelism · API-token UI · DSN/URI import.
**Wave 3 (sub-features):** consumer-group offset reset · server-side DB export + Usage CSV ·
PR checks/draft + inline review comments · Kafka live-tail · SQL/Mongo schema search ·
Confluence table/macro fidelity.

---

## 6. Suggested roadmap (strategic sequencing)

**Sprint 1 — Trust & legibility**
1. Fix Product model selection + other accepted-but-ignored controls (S4).
2. Work-control surface in Agents shell (needs-attention / task-cost-status in pane headers /
   named notifications) (S1).
3. Session lifecycle labels + clearer suspended/resumable presentation.
4. Capability/health registry MVP (providers, ClickHouse, issue accounts, channels, MCP, brokers) (S6).
5. Support-bundle export.

**Sprint 2 — Verified dev loop**
1. Review finding fingerprints + resolution state (S2).
2. Fix-selected → re-review for PR and local review.
3. Merge-readiness panel.
4. AI commit-message drafting from staged diff.
5. Hunk/line staging (if bandwidth allows).

**Sprint 3 — Traceability & cost**
1. Propagate work-graph IDs into sessions + usage (S3).
2. Cost at review/product/swarm/workflow action surfaces.
3. Fill swarm run cost/tokens + Run Inspector.
4. Reflect swarm project status back into Product.
5. Story traceability matrix.

**Sprint 4 — Agent as live participant**
1. DB read-only MCP tool with prod/read-only guardrails (S5).
2. API request/response "send to agent" context packet.
3. Broker investigation context packet.
4. NL-to-SQL into editor + explain-plan helper.
5. MCP server health/tool-count/governance UI.

**Sprint 5 — Automation glue**
1. Module-aware workflow nodes.
2. Schedule/webhook triggers.
3. Workflow import/export/duplicate + event streaming.
4. API collection smoke runs through Workflows.
5. Notifications → Channels routing.

> **How quick-wins relate to sprints:** the §5 Wave-1 items are mostly Sprint-1 fuel
> (legibility/trust) and can ship in parallel with the structural Sprint-1 work; they are not
> a separate track.

---

## 7. Priority table (strategic)

| Priority | Enhancement | Primary Modules | Effort | Why |
|---|---|---|---|---|
| P0 | Fix ignored Product model controls (S4) | Product, Sessions | S-M | Removes misleading UX |
| P0 | Work-control surface in Agents shell (S1) | Agents, Activity, Usage, Notifications | M | Makes current work visible |
| P0 | Review finding identity + verified fix loop (S2) | Git, Review, Sessions | M-L | Highest-value dev workflow |
| P0 | Work-graph usage attribution (S3) | Usage, Sessions, Product, Swarm, Git | M | Enables cost + traceability |
| P1 | Swarm Run Inspector + artifacts | Swarm, Sessions, Git | M | Makes swarm output usable |
| P1 | Capability/health registry (S6) | Settings, all modules | M | Reduces setup/debug friction |
| P1 | Cross-module command/search (S7) | Shell, all stores | M | Solves discoverability |
| P1 | DB explain/ERD/params/import-export | Database | M | Reaches DB-GUI parity |
| P1 | API secrets/auth inheritance/scripts/import | API | M | Reaches API-client parity |
| P1 | Broker lag/schema/offset/dead-letter flows | Brokers | M | Reaches Kafka-console parity |
| P2 | Outcome-aware self-improvement | Improve, Skill-Eval, Git, Product | M | Improves agent quality with evidence |
| P2 | Module-aware workflow nodes | Workflows, all modules | M-L | Turns workflows into real glue |
| P2 | Notifications → Channels routing | Notifications, Channels | S-M | Makes unattended work actionable |

---

## 8. Appendix

**Idea distribution (engineering catalog):** ~107 ideas — ~30 PERF · ~10 VISUAL · ~35 EASE ·
~32 SUBFEAT. Highest-leverage cross-cutting work: **T1 (polling→events)** and **T2
(virtualization)** each touch 5-6 clusters and align with the documented "Otto Networking"
idle-CPU lesson.

**Coverage gap:** the code-audit pass did not include a dedicated **Vault** cluster — §4.8's
Vault items are direction-only and need a follow-up grounded pass.

**Method:** strategic roadmap from a repo-wide research pass (evidence: `README.md`,
`shell/*`, `routes/mod.rs`, `modules.rs`, `lib/stores/*`, and the June-19 audits under
`docs/`); engineering catalog from 9 parallel read-only agents, one per cluster, every idea
citing the `file:line` it would change. **Nothing here is implemented — this is the menu.**

**Supersedes:** `docs/research/2026-06-20-otto-existing-feature-enhancement-roadmap.md` and the
interim `docs/feature-enhancements-2026-06-20.md` (both folded into this document).

---

## 9. Implementation status (verified 2026-06-20)

Most of this plan was implemented on branch **`feat/fe-enh`** (this worktree;
28–33 commits beyond `main`@`19cf2398`, migrations **0049–0053**, **not merged**). The
status below was **independently verified** by 6 read-only agents diffing the plan against
the actual code, plus a clean re-run of the release gates.

### 9.1 Release gates — independently GREEN (this worktree)
| Gate | Result |
|---|---|
| `cargo build --workspace` | ✅ Finished |
| `cargo test --workspace` | ✅ **858 passed / 0 failed** (78 bins) |
| `cd ui && npm run check` | ✅ **516 files, 0 errors, 0 warnings** |

> Note: the **primary** checkout (`/Users/itziklavon/otto_os`, `feat/kafka-ssh-tunnel`)
> currently does **not** compile — `otto-brokers` has an in-progress
> `BrokersService { group_denied }` change that's half-finished (unrelated WIP).

### 9.2 Coverage tally (concrete enhancement bullets, ~107)
| Cluster | ✅ Done | 🟡 Partial | ❌ Missing |
|---|---|---|---|
| Sessions/Terminal + Git/Review | 16 | 7 | 0 |
| Product + Swarm | 24 | 0 | 1 |
| Connections/DB + API Client | 11 | 5 | 2 |
| Brokers + Usage/Insights | 21 | 3 | 0 |
| Skills/Improve/Channels + Workflows | 16 | 3 | 1 (Vault) |
| Settings/RBAC/Shell + Help | 8 | 1 | 0 |
| **Total (approx.)** | **~88** | **~19** | **~5** |

**Net:** "~87/107 done" holds for *fully* done; ~19 are partial (usually the UI half, or
scope narrowed from "per engine" to one engine); ~5 genuinely missing.

### 9.3 T1 polling→events — wiring status (the load-bearing theme)
| Event | Emitted (prod) | Consumed (UI) | Verdict |
|---|---|---|---|
| `product_changed` | ✅ ×7 | ✅ all 4 tabs | **Live** |
| `improvement_updated` | ✅ | ✅ | **Live** |
| `workflow_run_updated` | ✅ ×10 | ✅ | **Live** |
| `skill_eval_updated` | ✅ | ✅ | **Live** |
| `usage_metrics_tick` | ✅ | ✅ | **Live** |
| `insight_ready` | ✅ | ✅ | **Live** |
| **`review_changed`** | ✅ ×8 | ❌ **no UI handler** | **DEAD** — review still polls |
| **`budget_exceeded`** | ❌ **only in `#[cfg(test)]`** | toggle exists | **DEAD** — promises an undeliverable notification |

### 9.4 Strategic themes (S1–S7) — mostly the next frontier
| Theme | Status | What landed / what's missing |
|---|---|---|
| S1 Work-control surface | 🟡 Partial | needs-you state + filter + status dot done; **grouped work-queue, pane-header attribution (model/repo/branch/PR/cost), bulk resume/pause = unbuilt** |
| S2 Verified review loop | 🟡 Partial | finding-identity DB schema (0049) + verdict/blocker_count exist; **UI keys findings by array index, no fingerprint/grouping → cross-run identity not surfaced** |
| S3 Work-graph attribution | 🟡 Mostly unbuilt | swarm-id meta + swarm per-turn cost backfill done; **no repo/branch/pr/story/workflow/origin stamping on session meta or usage ingest; no cost drilldown** |
| S4 Accepted-but-ignored controls | ⚪ Not addressed | Product `_model` etc. not audited this pass |
| S5 Live agent tools (DB/API/Brokers MCP) | ❌ Unbuilt | only passive MCP-server config exists |
| S6 Capability/health registry | 🟡 Thin | only a provider-outage banner (`serviceHealth.svelte.ts`); no cross-capability registry, one-click fixes, or support bundle |
| S7 Cross-module palette/search | 🟡 Partial | indexes commands+workspaces+sessions+connections+help; **no entity indexing (stories/PRs/queries/topics/memories), action rows, or transcript search** |

### 9.5 Notable gaps, dead paths & one latent bug
- **`review_changed` emitted but never consumed** — `events.svelte.ts` has no branch; review refresh still relies on the timer. (T1 not realized for review.)
- **`budget_exceeded` never emitted in production** — defined, scoped, routed, toggled, documented; only constructed in `#[cfg(test)]`. The toggle is a promise the daemon can't keep.
- **`ci_status` hardcoded `None`** in all three git providers — only `draft`+`labels` landed; no green/red CI pill (merge-readiness incomplete).
- **`DiffViewer` not truly virtualized** — hunk-line capping only; the shared `VirtualList` exists but isn't applied; `splitRows()` not memoized.
- **Terminal grid size saved to meta but never restored** on spawn/restart (always 80×24).
- **Server-side terminal ring-search backend complete but client never sends `Search`** (still local xterm addon).
- **MySQL row-count estimate backend exists but UI never requests it / no "~"** → unreachable.
- **DB result streaming / server-side sort+filter not done** (UI virtualization + server-side export-past-cap landed instead); per-statement timeout is **MySQL-only**; tunnel-cache reuse covers **test, not terminal**.
- **Vault**: only a cosmetic token badge + Copy-JSON; none of the substantial enhancements.
- **Latent panic:** `otto-improve/src/engine.rs:763,782` slices `content[..8000]` by **byte** index → panics on a multibyte boundary. **Must-fix.**

---

## 10. The next must-have wave (post-implementation)

Two tiers: **10A** finishes partials that currently *defeat their own goal* (small, high
value); **10B** is the strategic workflow-integration layer that's still mostly unbuilt —
the genuine "more must-have features."

### 10A — Finish-the-partials (must-have completions, mostly S/M)
1. **Consume `review_changed`** in `events.svelte.ts` (+ a review bus) so review goes
   event-driven like the other 5 — and **surface finding identity** (key by fingerprint,
   group by file/severity) to realize S2.
2. **Emit `budget_exceeded`** from the usage sampler when enforcement trips, and add a
   **point-of-action budget check** before review/product/swarm/workflow runs (makes the
   existing toggle + budgets real).
3. **Populate `ci_status`** (GitHub check-runs / GitLab pipelines / Bitbucket statuses) +
   a green/red/draft pill → completes the merge-readiness gate.
4. **True `DiffViewer` virtualization** (apply the existing `VirtualList`) + memoize
   `splitRows` + read the `too_large` flag for an "open on host / load anyway" guard.
5. **Restore terminal grid size** from `meta` on spawn/restart (kills the 80×24 reflow).
6. **Wire the remaining UI halves:** ring-search client frame; row-count "~" estimate
   opt-in; per-session "evolve" which-skills-changed badge; connections "Recent" group;
   audit-log quick presets + copy-JSON.
7. **Close the residual perf gaps:** `by_kind_rollup` N+1 + 4th-query batching (usage);
   bound the recruiter skill-list; per-statement timeout for ClickHouse/Mongo/Redis.
8. **Fix the latent panic** (`engine.rs` byte-slice → char-boundary-safe truncation).

### 10B — Strategic must-haves (the workflow-integration layer — mostly unbuilt)
The highest-value *new* must-haves; they turn the modules into one workflow and are what
competitors (Copilot/Cursor/Postman/DBeaver) are racing on.

- **S5 — Live agent tools for DB / API / Brokers** *(Otto's differentiator; entirely
  unbuilt).* Read-only DB MCP tool (row caps, timeout, env guardrails, write-prohibited by
  default); API "send request/response/history to focused agent" + "generate tests";
  Brokers "investigate lag/topic/schema" context packet — each showing the exact packet
  before sending. **Prerequisite: a secret/PII redaction layer for context packets** (new,
  must-have for safety).
- **S6 — Capability & health registry + support-bundle export.** One shared registry
  (provider-auth / Keychain / ClickHouse / issue-account / Slack-socket / Kafka-reach /
  MCP-health) with per-module ready/degraded/missing-setup + one-click fixes; plus a
  first-run health checklist (onboarding). Reduces support/debug friction across *every*
  module.
- **S7 — Cross-module search.** Index stories/versions/questions/learnings/PRs/commits/
  branches/saved queries/API requests/workflows/swarm tasks/broker topics/schemas/Vault
  memories; action rows (open / send-to-agent / attach / copy-context / run / reveal);
  search activity trails + transcripts; "most likely next action."
- **S3 — Work-graph attribution + cost drilldown.** Stamp `repo_id`/`branch`/`pr_number`/
  `story_id`/`workflow_id`/`origin` on session meta **and** usage ingest (new ClickHouse
  columns); "why did this cost so much?" drilldown. (Swarm-id + swarm cost backfill already
  exist — extend the pattern.)
- **S1 — Work-control surface.** The grouped work queue (Needs-attention / Working /
  Review-ready / Waiting / Done / Suspended), full pane-header attribution
  (model/repo/branch/PR/cost/last-output), and bulk "resume all suspended / pause noisy
  group". (needs-you substrate already exists — build the queue on top.)
- **Unified action audit + outcome capture.** Extend the existing `broker_write_audit` +
  impersonation audit into **one** audit surface for every external/destructive action
  (git push, PR post, channel send, DB write, settings/auth change); add per-session
  "definition of done" + outcome capture feeding outcome-aware self-improvement.
- **Vault grounded enhancements** *(coverage gap — no audit pass yet).* Force-directed
  graph + filters; memory lifecycle (suggested/accepted/stale/contradicted); provenance
  diff; forget/merge/split with undo; import `AGENTS.md`/`CLAUDE.md`/`.cursorrules` as
  governed assets.
- **Settings export/import + state backup/restore** (secrets excluded) — operational
  safety net that doesn't exist today.

**Sequencing:** 10A is a single cleanup sprint (small, finishes what's 80% there). 10B
maps onto Sprints 3–5 in §6 (S3 → Sprint 3, S5 → Sprint 4, S7/S6/S1 → ongoing), with the
**secret-redaction layer gating S5** and **`review_changed` consumption + `budget_exceeded`
emitter** the two highest-ratio fixes to do first.
