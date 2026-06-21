# Otto — Feature Enhancement: Implementation Check & Must-Haves (Consolidated)

Date: 2026-06-20

Scope: verify whether the identified enhancement directions are actually implemented,
then enumerate the remaining must-have enhancements — **existing features only, no new
top-level modules**. Research/planning artifact, not an implementation change.

This document **consolidates two parallel research passes**: (A) an implementation-status
+ must-haves check run against the working checkout, and (B) an independent verification of
the `feat/fe-enh` implementation branch (6 read-only agents diffing the master plan against
the actual code + a clean gate re-run). Where they disagreed, the **code-verified** result
wins.

---

## 0. Reconciliation — read this first

**Critical correction:** the feature-enhancement master plan is **not** missing, and most
of it **is** implemented.

- The master plan (`docs/feature-enhancement-master-plan-2026-06-20.md`) and the
  implementation are **committed on branch `feat/fe-enh`** (worktree
  `/Users/itziklavon/otto_os-fe`), **28–33 commits beyond `main`@`19cf2398`**, migrations
  **0049–0053**. The plan now also carries **§9 verified status + §10 next-must-have-wave**.
- An earlier pass reported "`docs/feature-enhancement-master-plan-…md` … missing" and many
  items "still missing" — because it inspected a **checkout that does not contain
  `feat/fe-enh`** (the primary checkout is on `feat/kafka-ssh-tunnel`, which a concurrent
  session has been churning; it even temporarily wiped untracked copies). Those readings are
  the **pre-implementation** picture and are corrected below.
- **`feat/fe-enh` is not merged; `main` is untouched.** The deployed Otto.app was built from
  this branch. It still needs rebase/reconcile against current `main` (now `94668bee`) +
  migration renumber if `main` advanced past 0048.

**Independently verified release gates (worktree `feat/fe-enh`):**

| Gate | Result |
|---|---|
| `cargo build --workspace` | ✅ Finished |
| `cargo test --workspace` | ✅ **858 passed / 0 failed** (78 bins) |
| `cd ui && npm run check` | ✅ **516 files, 0 errors, 0 warnings** |

> The **primary** checkout currently does **not** compile (`otto-brokers` has a half-finished
> `BrokersService { group_denied }` WIP) — unrelated to this work.

**Verified coverage of the ~107 concrete plan items: ≈ 88 done · 19 partial · 5 missing.**

| Cluster | ✅ Done | 🟡 Partial | ❌ Missing |
|---|---|---|---|
| Sessions/Terminal + Git/Review | 16 | 7 | 0 |
| Product + Swarm | 24 | 0 | 1 |
| Connections/DB + API Client | 11 | 5 | 2 |
| Brokers + Usage/Insights | 21 | 3 | 0 |
| Skills/Improve/Channels + Workflows | 16 | 3 | 1 (Vault) |
| Settings/RBAC/Shell + Help | 8 | 1 | 0 |

**Two dead event paths (load-bearing):**

| Event | Emitted (prod) | Consumed (UI) | Verdict |
|---|---|---|---|
| `product_changed`, `improvement_updated`, `workflow_run_updated`, `skill_eval_updated`, `usage_metrics_tick`, `insight_ready` | ✅ | ✅ | **Live** |
| **`review_changed`** | ✅ ×8 | ❌ no UI handler | **DEAD** — review still polls |
| **`budget_exceeded`** | ❌ only in `#[cfg(test)]` | toggle exists | **DEAD** — undeliverable |

**The genuinely-open frontier (both passes agree):** the strategic workflow-integration
layer is mostly unbuilt — work-graph attribution (S3), first-party live agent tools (S5),
capability/health registry (S6), cross-module search (S7), the work-control surface (S1),
plus Product model controls (S4) and Vault.

---

## 1. Verified implementation status by existing feature

Each entry: **Implemented (verified in `feat/fe-enh`)** then **Still open** (genuinely
remaining). ✅/🟡/❌ reflect *code-verified* state, correcting the pre-impl readings.

### 1.1 Agents, Sessions & Shell — 🟡 partial
**Implemented:** needs-you status dot distinct from idle (`StatusDot.svelte`, set from the
`:waiting` notice), needs-you filter + count (`Navigator.svelte:157`), keep-alive + settings
idle-grace + idle countdown, tab drag-reorder, multi-pane broadcast, desktop terminal
toolbar + copy-on-select, PTY→WS frame coalescing, prompt-guard trail, xterm
no-teardown-on-switch. Palette indexes commands + workspaces + sessions + connections + help,
with frecency.
**Still open:** the **work-control surface (S1)** — grouped work queue (Needs-attention /
Working / Review-ready / Waiting / Done / Suspended), full pane-header attribution
(model/repo/branch/PR/cost/last-output), bulk "resume all suspended / pause noisy group",
and saved filtered views ("waiting on me", "high cost today", "failed runs"). **Deep
command-palette object search (S7)**. Two verified partials: server-side ring-search backend
done but **client never sends the `Search` frame**; grid size **saved to meta but never
restored** on spawn (still 80×24).

### 1.2 Git, PRs & Review — 🟡 partial (verified-loop incomplete)
**Implemented:** `FileDiff` stats/`too_large`/`language` (`parse.rs:457`), paginated PR/repo
lists (`client.rs:322` follows `Link`), GHE detection (`detect.rs:18`), provider ETag/TTL GET
cache (`client.rs`), poll backoff + visibility gate, viewed-files progress + keyboard nav,
per-config review `timeout_secs`/`max_attempts`, approved findings → **true inline** PR review
comments (`github.rs:315`), finding-identity DB schema (`0049_review_findings.sql`) +
`verdict`/`blocker_count`/`summary_md`.
**Still open / verified partials:** **`review_changed` emitted ×8 but never consumed** →
review still polls. **`ci_status` hardcoded `None`** in all three providers → **no CI/check
pill, no merge-readiness CI signal**. Finding identity is in the DB but the **UI keys findings
by array index** (no fingerprint/grouping) → cross-run resolved/regressed state not surfaced.
`DiffViewer` is **hunk-capped, not virtualized** (the shared `VirtualList` is unused;
`splitRows` not memoized); the `too_large` flag is plumbed but the UI never shows an "open on
host / load anyway" guard. **Merge-readiness panel** (unresolved findings + approvals +
mergeable + CI + branch freshness + conflicts) still not assembled.

### 1.3 Product (Jira/Confluence) — ✅ mostly done, 1 trust gap
**Implemented (corrects pre-impl "still missing"):** **N+1 testcase count fixed** →
`count_testcases` `COUNT(*)` on the served path (`http.rs:327`); **`product_changed` WS event
emitted ×7 and consumed by all 4 tabs** (no longer polling-only); shared `reqwest::Client` per
account; `build_agent_context` cache keyed by `(story_id, updated)`; word-level `RewriteTab`
diff via shared `DiffView`; **Confluence storage→markdown table/macro/image fidelity**
(`confluence.rs:661`, round-trip); search load-more + recency default; Jira skeleton;
issue-key autodetect; token-expiry badge; debounced plan save; testcase bulk-approve +
drag-reorder.
**Still open (must-fix correctness, S4):** **Product model selectors are accepted but
ignored** — `_model` is dropped in rewrite/generate-tests/generate-plan (`product_run.rs`).
Wire it through or remove the control. **Product↔Swarm closure** (story should reflect swarm
state/artifacts/PRs/review/accepted evidence/cost) still not a full loop.

### 1.4 Agent Swarm — ✅ strong, cross-feature traceability open
**Implemented:** budget/attempt meter + pause-with-reason + **one-click raise-&-resume**;
live graph via `applyEvent` (debounced `loadGraph`); Kanban drag-drop; OrgTree inline-add +
drag-reparent + skill autocomplete; Run Inspector (brief/cwd/artifacts/board/tokens/cost/raw);
**swarm runs backfill per-turn tokens/cost from usage** (`swarm_run.rs:362`); virtualized
RunsList; findings severity rollup; ticking timestamps; `runForAgent` id fix.
**Still open:** **bound the recruiter/planner skill list** (the one Product+Swarm item not
done — full library injected into the prompt, `swarm_runtime.rs:793`). Artifact↔Product/Git
cross-links first-class (PR/branch/files/run/review/story/task pointing to each other); task
acceptance/evidence state; per-role mandatory input/output contracts.

### 1.5 Connections & Database Explorer — 🟡 many done, AI loop open
**Implemented:** parallel `schema_graph` (`buffer_unordered(8)`); FK jump-to-table; write-gate
typed-confirm modal + prod/read-only badges; DSN/URI paste import; recency/pins (`0050`);
server-side **export past the preview cap**; per-statement timeout (MySQL); result-grid
virtualization; tunnel-cache reuse for **test** opens; guarded writes audited; DB context →
agent.
**Still open / verified partials:** **DB explain-plan UI + AI plan explanation + index hints**
(DBeaver/CloudBeaver parity) — not built. **Failed-query explain/fix loop** (query + params +
error + schema → agent). **Read-only DB MCP tool** for agents (RBAC, row caps, timeout, audit)
— part of S5. Schema-tree filter is **client-side root-only** (server `_filter` still unused).
Row-count estimate is **backend-only / UI-dead** (no "~"). Result **streaming / server-side
sort+filter** not done (only UI virtualization + export-past-cap). Per-statement timeout
**MySQL-only**. Tunnel-cache reuse covers **test, not terminal**. Masking/redaction presets for
PII/prod views. No labeled "Recent" group in the UI.

### 1.6 API Client — ✅ baseline solid, agent-native layer open
**Implemented:** debounced/memoized JSONPath + pretty-print (size-gated); virtualized
body/history + stream ring-buffer cap; ⌘↵/⌘S/⌘T shortcuts + AbortController stop. (cURL import,
OpenAPI export, scripts/tests/traces, automations, gRPC pre-existed.)
**Still open:** **send request/response/error/history packet to a focused agent** (S5).
**First-party API-client MCP tools** (list/read/run-in-safe-mode/inspect-failures).
AI-generated assertions from real responses/failures. **Collection-run comparison** (prev vs
current: latency/status/schema/assertion drift). **Environment drift view** (local/stage/prod
variable sets). OpenAPI *import*; secret-redacted variables.

### 1.7 Message Brokers — ✅ operator basics, deeper workflows open
**Implemented:** **consumer-group offset reset** (guarded + typed-confirm + audit →
`broker_write_audit`, `0052`); batch topic-stats; produce headers/tombstone/base64; URP +
leader-skew on Overview; live-tail (incremental) + per-partition offset bar; server-side
key-filter / find-from-beginning; lag sort + per-row bar; metrics-sweep cache; tunnel warm-up;
copy/export peeked messages.
**Still open:** **DLQ/replay helper** (select → preserve/transform headers+key → produce to
target → record replay evidence). **Schema compatibility check + diff + version graph +
metadata + safe registration**. **Topic-level lag alerts + saved investigations**. **Broker
message packet → agent + broker read-only MCP tools** (S5). Replace any remaining browser
`confirm()` in broker/MCP paths with the shared confirmer. Drop the dead `message_count:-1`
sentinel.

### 1.8 Workflows — ✅ engine + events, real nodes open
**Implemented (corrects pre-impl "scaffold/needs events"):** **per-kind node param editor**
(http_request/transform/delay/game_engine); **`workflow_run_updated` + `skill_eval_updated` WS
events emitted and consumed**; node-result cache / skip-unchanged (`0051`).
**Still open:** `game_engine` + some verifier behavior still **scaffolded**. **Module-native
nodes** (Product analyze/rewrite/plan, Swarm task, PR review, API run, DB query, broker peek,
channel notify, **budget-gate**, human-approval). Schedule/webhook/event triggers. Typed
input/output contracts + validation. Retry/backoff/error-branch primitives. Visual run replay.

### 1.9 Usage, Insights & Metrics — 🟡 improved, attribution open
**Implemented:** `priced_as_of` + estimated/fallback flag; CSV/JSON export; per-session
drill-down (open session); opt-in auto-refresh + `usage_metrics_tick`; live insights run
status (`run_id` polling) + offset picker + report download/new-tab + skill-missing empty
state; batched top-session enrichment; SVG chart axes/tooltips; version/retention card.
**Still open (S3):** usage events lack **repo/branch/PR/story/swarm-task/workflow/channel/
review/artifact** dimensions (only swarm-id meta + swarm cost backfill exist). **Budget checks
at action start** (not only after spend) — and the **`budget_exceeded` emitter is missing**
(toggle is dead). Per-feature budgets visible **where work is launched**. **Cost forecast**
before launching a run. High-cost session → exact prompt/run/artifact/PR drilldown. Residual
perf: `by_kind_rollup` N+1 + 4th-query not batched.

### 1.10 Skills, Context, Self-Improvement & Vault — 🟡 improve done, Vault open
**Implemented:** `notify_self_improvement` toggle; live-refresh via `improvement_updated`;
DiffView approval card; digest slash/ToolSearch skill detection; memory-read caps; skill-cache;
skill-eval unified diff + pre-computed validation diff; per-session evolve endpoint;
per-channel "Send test message", `/restart`+`/who`, exp-backoff reconnects, relayed-reply
formatting.
**Still open:** **Vault** got only a cosmetic token badge + Copy-JSON — none of the substantive
items (force-directed graph + filters; memory lifecycle suggested/accepted/stale/contradicted;
provenance diff; forget/merge/split with undo; import `AGENTS.md`/`CLAUDE.md`/`.cursorrules` as
governed assets). Outcome-aware self-improvement (proposals ↔ tests-passed/PR-merged/findings-
resolved). Per-session "evolve" **which-skills-changed badge** (endpoint exists, UI absent).
**Latent panic:** `otto-improve/src/engine.rs:763,782` byte-slices `content[..8000]` → panics
on a multibyte boundary. **Must-fix.**

### 1.11 Settings, RBAC, Remote & Trust/Safety — ✅ mostly done
**Implemented:** short-TTL auth-lookup cache (login/api only; **never** share/impersonation;
synchronous invalidation on revoke/set_grants — verified safe); **API-token management UI**;
**impersonation banner + 30-min countdown**; `confirmer` replacing `window.confirm`; palette
frecency; app-password affordance + actionable SMTP errors; users filter/copy; share-listing
index (`0053`).
**Still open:** audit list lacks "Last 24h/7d" presets + per-entry copy-JSON. **Settings
export/import + state backup/restore** (secrets excluded) not built. **Unified action audit**
across all external/destructive actions (extend `broker_write_audit` + impersonation audit to
git push / PR post / channel send / DB write / settings/auth change).

### 1.12 MCP, Capabilities & Health — 🟡 thin
**Implemented:** per-workspace MCP server CRUD merged into `.mcp.json` on spawn; provider-
outage banner (`serviceHealth.svelte.ts`); onboarding + language-server surfaces.
**Still open (S6):** **capability/health registry** — one page: what Otto can do now, why
something is disabled, how to fix it, which features depend on which providers/tools/accounts;
per-module ready/degraded/missing-setup with one-click fixes; **support-bundle export**;
point-of-failure diagnostics. **First-party Otto MCP server** exposing safe read-only tools
(DB query/schema, API read/run, Git PR/review lookup, Product story/context, Swarm task/run,
Broker topic/group/schema) with RBAC/row/timeout/audit + non-plaintext secret handling (S5).

---

## 2. Consolidated must-have wave

Three tiers. **Tier A** finishes partials that defeat their own goal (small, high value).
**Tier B** is the strategic workflow-integration layer (mostly unbuilt — the real "more
must-haves"). **Tier C** is polish/perf.

### Tier A — Correctness & finish-the-partials (P0)
1. **Fix the misleading Product model controls (S4)** — wire `_model` through rewrite/
   generate-tests/generate-plan, or remove/disable the selector. *(Accepted-but-ignored
   controls are trust-breaking — the one item neither pass found addressed.)*
2. **Consume `review_changed`** in `events.svelte.ts` (+ a review bus) and **emit
   `budget_exceeded`** from the usage sampler — the two dead WS paths. Add a **point-of-action
   budget check** before review/product/swarm/workflow runs.
3. **Durable review finding identity + lifecycle in the UI** — fingerprint (repo+PR+path+
   normalized context+text/category), states (open/fixing/resolved/regressed/declined), attach
   fix-session/commit/re-review; key the UI by fingerprint (DB schema `0049` already exists).
4. **Merge-readiness panel** — populate `ci_status` (check-runs/pipelines/statuses) + a
   green/red/draft pill, then combine unresolved findings + approvals + mergeable + CI +
   branch freshness + conflicts + unpushed commits.
5. **True `DiffViewer` virtualization** (apply `VirtualList`) + memoize `splitRows` + the
   `too_large` "open on host / load anyway" guard.
6. **Finish the orphaned UI halves** — restore terminal grid size on spawn; ring-search client
   frame; row-count "~" estimate opt-in; per-session evolve "which-skills-changed" badge;
   connections "Recent" group; audit-log presets + copy-JSON.
7. **Close residual perf gaps** — usage `by_kind_rollup` N+1 + 4th-query batching; bound the
   recruiter skill-list; per-statement timeout for ClickHouse/Mongo/Redis.
8. **Fix the latent panic** (`engine.rs` byte-slice → char-boundary-safe truncation).

### Tier B — Strategic must-haves (P0–P1, mostly unbuilt)
9. **Work-graph attribution + cost drilldown (S3)** — stamp `repo_id`/`branch`/`pr_number`/
   `story_id`/`swarm_task_id`/`workflow_id`/`channel`/`review`/`artifact`/`origin` on session
   meta **and** usage ingest (new ClickHouse dimensions); "why did this cost so much?"
   drilldown; **cost forecast** before launching a run. *(Swarm-id + swarm cost backfill exist
   — extend the pattern.)*
10. **First-party Otto MCP tools + live agent layer (S5)** — read-only, RBAC-scoped, row/
    timeout/size-limited, audited tools for DB (query/schema), API Client (read/run
    collections), Git (PR/review lookup), Product (story/context bundle), Swarm (task/run),
    Brokers (topic/group/schema). Plus **send-to-agent context packets** for API/DB/Broker that
    **show the exact packet (size + secret/PII redaction) before sending**. *(Redaction layer is
    a hard prerequisite.)*
11. **Capability & health registry + support bundle (S6)** — one page of provider accounts/
    CLIs/language-servers/MCP servers/channels/Git+issue accounts/DB+broker deps + feature
    availability, with why-disabled + one-click fixes; per-module ready/degraded/missing-setup;
    support-bundle export; point-of-failure diagnostics.
12. **Work-control / Mission Control surface (S1)** — make it the default Agents/Workspace view
    from existing data: active sessions, needs-you, blocked prompts, open reviews, Product runs,
    Swarm runs, budget warnings, failed workflow runs, channel-delivery failures; saved filtered
    views.
13. **Deep cross-module palette/search (S7)** — index stories/versions/questions/learnings/PRs/
    commits/branches/saved queries/API requests/workflows/swarm tasks/broker topics/schemas/
    Vault memories; contextual action rows (open / send-to-agent / copy-context / rerun /
    review / export); search activity trails + transcripts; "most likely next action."
14. **Database explain/fix loop** — EXPLAIN plan UI for MySQL/ClickHouse + AI plan explanation +
    index hints; failed-query explain/fix tied to query+params+error+schema.
15. **Broker operator workflows** — offset reset *dry-run preview* (reset itself shipped);
    DLQ/replay with header/key preservation + evidence; schema compatibility/diff/version graph;
    topic-level lag alerts.
16. **Product ↔ Swarm closure** — story shows linked swarm tasks/active runs/artifacts/PRs/
    reviews/accepted test cases/budget+cost; swarm task shows source story + acceptance criteria.
17. **Workflow real nodes + triggers** — Product / Review / Swarm / API / DB / Broker / Channel /
    Budget-gate / Human-approval nodes; schedule/webhook/event triggers; typed I/O contracts;
    retry/backoff/error branches; visual run replay.
18. **Vault grounded enhancements** — force-directed graph + filters; memory lifecycle;
    provenance diff; forget/merge/split with undo; import `AGENTS.md`/`CLAUDE.md`/`.cursorrules`.

### Tier C — Polish & performance (P2)
19. Replace any remaining native `confirm()` in broker/MCP paths with the shared confirmer.
20. Make remaining polling event-driven (additive WS variants) wherever the backend can emit.
21. Server-side export endpoints wherever UI export is still preview-capped.
22. **Saved investigations** — "why did this API run fail?", "why is this consumer group
    lagging?", "why is this query slow?", "what did this swarm produce?".
23. **Data masking presets** for production DB and broker payload views.
24. **"What changed and why"** run summaries for Product/Swarm/Review/Workflow/API, tied to
    logs/artifacts.
25. **Settings export/import + state backup/restore** (secrets excluded); **unified action
    audit** across external/destructive actions.

---

## 3. Recommended implementation order

1. **Correctness pass:** fix Product model controls (S4); consume `review_changed`; emit
   `budget_exceeded`; fix the `engine.rs` panic. *(N+1 testcase count, polling→events for
   product/workflow/skill-eval, and the docs source-of-truth are already done on `feat/fe-enh`.)*
2. **Verified review loop:** finding-identity UI (fingerprint/status) on the existing `0049`
   schema; `ci_status` + merge-readiness panel; re-review-affected flow.
3. **Work graph (S3):** shared work-reference type; usage dimensions; artifact links across
   Product/Swarm/Git/Review/API/DB/Broker/Workflow; cost forecast.
4. **Agent-native context (S5):** first-party Otto MCP tools + API/DB/Broker send-to-agent
   packets with redaction; capability registry (S6).
5. **Operator workflows:** broker DLQ/replay + schema compat; DB explain/fix; workflow real
   nodes/triggers; Mission Control (S1) + deep palette (S7) as ongoing surfaces.

---

## 4. Do not rebuild these from scratch (already present — extend, don't replace)

Pre-existing on `main`: Product→Swarm handoff; Swarm budgets + Run Inspector; Usage by-kind +
budgets; DB write guards + cancel; DB context→agent; API request engine/history/import/export/
automation; Broker topics/groups/schema/SSH/read-only; session needs-you/activity surfaces;
user-managed MCP settings.

**Added on `feat/fe-enh` (verified — do not re-plan):** Product N+1 fix + `product_changed`
live updates + Confluence md fidelity; review finding-identity DB schema (`0049`) + inline PR
comments + paginated PRs + GHE + ETag cache; swarm budget meter + raise-&-resume + live graph +
Kanban DnD + cost backfill; DB parallel schema-graph + FK-jump + DSN import + recency + server-
side export + write-gate modal; brokers offset-reset (audited) + live-tail + key-filter +
batch-stats + URP; usage priced-as-of + CSV export + drill-down + live insights + charts;
self-improve event-refresh + DiffView approvals + notify toggle; workflows node param editor +
WS events + node cache; RBAC auth-cache + PAT UI + impersonation banner + confirmer everywhere +
searchable help.

---

## 5. External benchmarks

- **GitHub Copilot cloud agent** — background tasks, branch/commit automation, custom agents,
  many entrypoints (issues/CLI/MCP/Jira/Slack/Teams/Linear/schedules), session logs.
  ([about](https://docs.github.com/en/copilot/concepts/agents/cloud-agent/about-cloud-agent),
  [sessions](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/cloud-agent/start-copilot-sessions),
  [code review](https://docs.github.com/en/copilot/concepts/agents/code-review))
- **Cursor** — Cloud Agents, Bugbot (PR review), MCP. ([cloud-agent](https://cursor.com/docs/cloud-agent),
  [bugbot](https://cursor.com/docs/bugbot), [mcp](https://cursor.com/docs/mcp))
- **Postman** — MCP Server (workspace/collection/spec/mock/monitor/env/workflow tools), AI Agent
  block in Flows, Collection Runner routing failures into Agent Mode.
  ([MCP](https://learning.postman.com/docs/reference/postman-api/postman-mcp-server/overview),
  [AI block](https://learning.postman.com/docs/postman-flows/reference/blocks/ai-agent),
  [runner](https://learning.postman.com/docs/tests-and-scripts/running-collections/intro-to-collection-runs))
- **DBeaver / CloudBeaver** — AI create/edit queries, explain query, explain execution plan, fix
  SQL errors, describe objects. ([DBeaver](https://dbeaver.com/docs/dbeaver/AI-Smart-Assistance/),
  [CloudBeaver](https://dbeaver.com/docs/cloudbeaver/AI-Smart-Assistance/))
- **Kafka consoles** — Conduktor topic-scoped lag alerts; Redpanda Console consumer-group lag +
  offset reset/replay; Redpanda Schema Registry compatibility/versioning/metadata/normalization.
  ([Conduktor](https://docs.conduktor.io/guide/release-notes),
  [Redpanda Console](https://docs.redpanda.com/streaming/24.2/console/quickstart/),
  [Schema Registry](https://docs.redpanda.com/cloud-data-platform/manage/schema-reg/schema-reg-overview/))

---

## 6. Provenance

- **Master plan + implementation:** `feat/fe-enh` worktree
  `/Users/itziklavon/otto_os-fe/docs/feature-enhancement-master-plan-2026-06-20.md` (§9 verified
  status + §10 must-have wave appended there too).
- **Verification:** 6 read-only agents diffing the plan vs the actual `feat/fe-enh` code, plus a
  clean re-run of `cargo build` / `cargo test` (858 passed) / `npm run check` (516/0/0).
- This consolidated check supersedes the pre-implementation reading that reported the master
  plan and many items as "missing" (it had examined a checkout without `feat/fe-enh`).
