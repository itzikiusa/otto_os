# Otto — Feature, UX & Killer-Feature Roadmap (impact-ranked)

> **Date:** 2026-06-19
> **Lens:** Product / UX / killer-feature — *not* security or correctness. The
> bug/security/correctness work is already covered by
> `docs/deep-dive-improvements-2026-06-19.md` and verified in
> `docs/fixes-2026-06-19/VERIFICATION.md`; the broad MoSCoW pass is in
> `docs/research/2026-06-19-otto-product-improvement-audit.md`. This doc adds the
> missing angle those two don't: *what would make users love Otto more*, ranked
> by impact.
> **Method:** Six parallel deep-dive research agents, one per feature cluster
> (sessions/shell · git/PR/review · product/channels · DB/API · swarm/workflows ·
> intelligence/integration), each reading the **current** post-fix code and
> deliberately excluding the known security/bug list. Findings de-duplicated and
> re-ranked here by product impact.

---

## The throughline (read this first)

Three patterns showed up independently in nearly every agent's report:

1. **The backend already knows; the UI throws it away.** The daemon tracks rich
   live state — each session's current/in-progress task, "agent finished" /
   "needs attention" hooks, `last_active_at`, the full activity trail; swarm runs
   carry `tokens_input/output/cost_usd` columns and `swarm_id/task_id/project_id`
   in session meta; reviews carry per-agent findings. **Most of it never reaches
   the surfaces where it matters** (tiled panes, tabs, notification copy, run
   rows, a home screen). A large share of the highest-impact wins below are
   *surfacing existing data*, not new systems.

2. **The modules are federated by SQLite foreign keys, not by product surfaces.**
   Otto owns an unusually complete vertical (Sessions, Git/PR, Product, DB, Swarm,
   Usage, Channels, Workflows, Context/Skills) — but they barely talk to each
   other. Product has a planner *and* Swarm has a planner and they never meet;
   Usage records cost per session but not per repo/story/PR; the review loop
   dead-ends at "handoff" instead of re-verifying. **The biggest untapped value is
   integration, not new modules.**

3. **The agent is the moat, but it's bolted on as a one-shot paste.** "Ask the
   agent about this" exists in the DB grid and the query editor — but it pastes a
   frozen snapshot. The agent can't iterate (run a follow-up query, re-review a
   fix, drill into data). Turning the agent from a *reader of snapshots* into a
   *live, guardrailed participant* is the category-defining move no competitor
   (TablePlus, Postman, GitHub, Cursor) can match, because none of them own all
   the surfaces Otto does.

---

## Tier 0 — Killer features (highest leverage; do these first)

These are the cross-cutting bets. Each was independently flagged by 2+ research
agents.

### K1 · "Mission Control" — a home dashboard + live per-pane status
**Impact: Very High · Effort: M (mostly wiring existing data)**

Today the app drops you into a grid of terminals. There is no Home route, and the
Activity panel (the *only* place an agent's current task + trail is shown) is
**hidden the moment you go tiled or split** (`App.svelte:375` gates `RightPanel`
on `singleSessionView`) — i.e. it vanishes exactly when you're running many agents
and most need a per-agent readout. Tiled panes show only a status *dot*.

Build the cockpit:
- A **Home/Mission-Control board**: every agent as a card grouped into
  **Needs you / Working / Idle / Done**, each showing current task, idle time,
  repo/branch, and (via K3) spend. Add open PRs awaiting review/merge, swarm runs,
  pending self-improvement edits, stories with new comments, today's spend.
- A **one-line "now: «in-progress task»"** + done/total chip in **every** tiled/
  split pane header (data already in `activity.summary(id)`).
- A **sticky "Needs you" state** distinct from "idle." Right now "idle" is
  inferred from "no PTY output for 5s," so an agent *thinking* and an agent
  *blocked on a prompt* look identical. The "agent needs attention" hook already
  exists — make it a persistent badge + a filter, not a transient toast.
- **Name the notifications.** `monitor.rs:527` emits literally *"An agent session
  is idle"* with no name/task — useless with 5 agents running. The code already
  has `session_id` and reads `s.meta` two lines later.

> This converts Otto from "a nice terminal multiplexer" into "the cockpit for a
> team of agents" — its actual differentiated promise — and it's almost entirely
> surfacing data the daemon already produces.
> *Connects:* Sessions · Notifications · Activity · Swarm · Usage · Git · Product.

### K2 · The verified review loop: review → fix → re-review → merge-gate
**Impact: Very High · Effort: M–L**

Otto already has multi-agent parallel review, openable review sessions, a
summarizer, and a `handoff` that spawns a fix agent. But the workflow stops one
step short of being a *loop*: the fix agent runs in a separate view, and nothing
re-runs review or marks findings resolved. The PR-review panel has *no* handoff at
all. This is the difference between a linter and an agentic review tool — and it's
the race CodeRabbit/Graphite/Cursor are running.

Close it into one tracked cycle:
- **Persistent finding identity** across runs (fingerprint = path + normalized
  body hash + nearby context). This is the data-model prerequisite for everything
  else; today every run is an unrelated list and declining a finding only hides it
  for that run. Add `confidence`, `fingerprint`, `status` to the finding model
  (`domain.rs:748-830`).
- **"Fix with agent" → auto re-review the affected lenses on the new diff →** each
  finding flips to `resolved` / `still-open` / `regressed`, with a single "N of M
  resolved, 0 regressed" badge.
- **An AI-review trust layer:** confidence score + cross-agent consensus
  ("flagged by Correctness + Security") + a required quoted-evidence line tying the
  claim to the exact diff hunk. False-positive fatigue is the #1 reason teams
  abandon AI review; the multi-lens design already produces consensus data that's
  thrown away at summarization.
- **A merge-readiness gate** that fuses (a) unresolved bug-severity findings,
  (b) CI/check-run status (add `checks()` to the provider trait — `get_pr` never
  fetches check runs today), (c) required approvals, (d) conflicts → one confident
  green/red. "Is this safe to merge?" is the highest-value question in the
  workflow and Otto currently can't answer it.

> *"Otto knows whether your PR is safe to merge, and its agents close the gap when
> it isn't."* No competitor fuses multi-agent review + an agentic fixer + a merge
> gate into one verified loop. Otto owns every component already.
> *Connects:* Review · Sessions · Git.

### K3 · Work-graph cost attribution (one cheap primitive unlocks several features)
**Impact: Very High · Effort: M**

`UsageEvent` records only `{workspace_id, session_id, provider, model, tokens,
cost_usd}` — no `repo_id`, `branch`, `story_id`, `swarm_task_id`, or `origin`.
So Otto, which uniquely owns *both* an embedded ClickHouse usage engine *and* the
work graph, **cannot answer "what did this Jira story / this PR review / this swarm
project actually cost me?"** Worse: the **swarm run rows have `tokens_*`/`cost_usd`
columns and `RunsList.svelte` renders a "Tokens" column that is permanently `—`**
because the runtime never writes them — a promised number that's always blank is
more corrosive to trust than no column at all.

**Cheapest immediate win:** `UsageEvent.kind` *already* distinguishes
`review | product | channel | agent | shell`, so a **by-kind rollup** ("review cost
$X, Product AI $Y, Swarm $Z this week") is ~90% built — it just needs a group-by in
`/usage/summary` + a UI slice. That alone is High-impact / S-effort.

The deeper version is one cheap primitive: **stamp work-graph identity (`repo_id` /
`story_id` / `swarm_task_id` / `origin`) onto session meta → propagate to
`/ingest/usage` → add nullable columns to the ClickHouse `usage_events` MergeTree
(additive) → add group-by to `/usage/summary`.** That single seam unlocks:
- **Per-feature / per-PR / per-story / per-swarm cost** (no competitor can do this).
- A **swarm spend leash**: "swarm spend today $X · this run ~$Y" in the header →
  turns "I'm scared to leave it running overnight" into "I gave it a $20 budget."
- **Per-session token/cost** in the InfoPanel/pane header (Claude Code shows this
  inline; Otto hides it in a root-only aggregate module).
- Cost-aware **Insights** and **workspace-scoped usage** for editors (today usage
  is root-only — the people spending tokens can't self-serve).

> *Connects:* Usage · Sessions · Git · Product · Swarm · Review · Insights.

### K4 · Connect the planners: Product story → Swarm project / Workflow
**Impact: Very High · Effort: M**

Product already does an AI **plan/tasks breakdown** that produces `SwarmTask`-shaped
tasks. Swarm has `POST /swarm/projects/{pid}/plan` that decomposes a project into
`SwarmTask[]`. **These are two parallel planners that never meet** — grep confirms
zero product↔swarm code. A PO who refines a Jira story in Otto (analysis, rewrite,
open questions, plan) then has to re-plan it from scratch in Swarm to build it. And
the most natural input to a swarm — a story you already wrote in Otto — can't feed
it.

- **"Send to Swarm"** on a Product story → instantiates a SwarmProject with
  `goal_md` pre-filled and the plan tasks seeded, back-linking `story_id` into
  swarm meta (which also feeds K3).
- A **"Goal → Swarm" express path** in NewSwarm: type a goal → recruiter forms a
  fitting org *and* plans tasks *and* offers Start (today goal→running is ~6 manual
  hops).
- Longer term: a **traceability spine** — story → tasks → sessions → branch/PR →
  done — so status flows back to the story instead of `inject-session` being a
  one-way prompt paste.

> The marquee "idea → a team of agents builds it" demo. The data shapes already
> align.
> *Connects:* Product · Swarm · Sessions · Git.

### K5 · The agent as a live, guardrailed participant in DB & API
**Impact: High · Effort: M–L · Prerequisite: K6 guardrails**

`otto-sessions/src/mcp.rs` already writes `.mcp.json` so claude/codex load an MCP
server on launch — but only for a Playwright browser. The DB→agent path only pastes
a **frozen, 50-row snapshot** into the prompt; the agent can't run a follow-up
query, EXPLAIN, sample distributions, or verify a hypothesis.

- **A live read-only DB MCP tool** wired to the connection (reuse
  `DbViewerService::resolve()` with its existing tunnels/row-caps). "Investigate
  why orders dropped 12% on Tuesday" becomes a real agentic loop.
- **Natural-language → SQL** streamed *into the editor* (not a chat bubble), where
  the existing review-modal already gates execution. Schema/FK introspection +
  resident agent already exist; this is the headline feature users now expect.
- **An agent affordance in the API client** (which today has *none*): "→ Agent" on
  the response to "write a test for this," "explain this 500," or "update my
  client code to match this new field." The DB grid's `sendToRunningAgent` is a
  25-line precedent to copy. *API response → code change in the repo* is something
  Postman structurally cannot do.
- **MCP host + server** more broadly: let users manage MCP servers per workspace
  (the `.mcp.json` writer is half the plumbing), and — the strategic play — expose
  *Otto's own* assets (DB connections, Jira/Confluence, git, API collections) as an
  **MCP server** so any agent can use them. Otto becomes the context/tool hub.

> *Connects:* DB · API · Sessions · Context · Connections · Product · Git.

### K6 · Connection environment labels + production guardrails
**Impact: High · Effort: S–M · Ship with/before K5**

`Connection` has no environment/role/danger field; "prod vs staging" is implied
only by folder naming. There's no read-only flag, no confirm-before-write tied to a
connection, no danger styling. An agent with a DB tool (K5) on a prod connection is
a real risk. Add `environment: dev|staging|prod` + `read_only: bool`; render prod
tabs with a red rail; gate writes behind a typed confirm; **default the agent's DB
tool to read-only on prod.** This is the guardrail that makes K5 deployable, and it
makes Otto trustworthy on real infrastructure on its own.

### K7 · Unified global search / live command palette
**Impact: High · Effort: M**

The palette indexes static nav actions + live sessions/workspaces/connections only.
It does **not** search repos, PRs, commits, stories, Jira issues, swarm tasks,
saved DB queries, or API requests — and there is **no cross-session search** over
trails / commands / files-touched (the trail records Command/File/Web/Prompt kinds
but nothing searches them). In a ~14-module app this is the biggest discoverability
tax. A real "find anything" `⌘K` that jumps to a PR / a story / a swarm task / a
saved query — plus search across all sessions' work — is what makes Otto feel like
one product instead of fourteen tabs. (Cross-session search is also the missing
*memory* layer for parallel long-running agents.)

### K8 · Workflows become the connective tissue (module-aware nodes + triggers)
**Impact: High (strategic) · Effort: M–L**

The DAG executor is solid (topo sort, re-run-from-here, wall-clock budget, orphan
reaping) but **starved of useful nodes**: the catalog is `trigger, agent, http,
transform, delay, log, game_engine, verifier` — the last two are explicit scaffolds
("wire a real game engine here"), and templates are *three slot-machine games*.
There is no node for git/PR, DB query, Jira, Slack, swarm, file, or shell — despite
Otto shipping a crate for each. And the **only trigger is a human clicking Run** (no
schedule, no webhook), even though Swarm already has a generic `is_due` scheduler to
reuse.

- Add **module-backed nodes** (`git_pr`, `db_query`, `jira_issue`, `slack_post`,
  `swarm_run`, `shell`/`file_write`, `condition`) — each a thin arm over a crate
  Otto already has.
- Add a **`schedule_trigger`** (reuse `swarm_scheduler::is_due`) and ideally a
  **`webhook_trigger`**. "Every morning, summarize overnight commits → post to
  Slack" is the canonical use case and is impossible today.

> This is what turns Workflows from a slot-machine sandbox into the automation glue
> of the whole app. *Connects:* Workflows · Git · DB · Product · Channels · Swarm.

---

## Tier 1 — High-impact per-module UX & feature gaps

| Finding | Where | Impact | Effort |
|---|---|---|---|
| **Swarm Run Inspector** — what did each agent *do*? The brief sent, cwd/worktree (reveal-in-Finder/diff), parsed `artifacts[]` as clickable rows, board posts for that run, tokens/cost, raw result. Workflows already nailed this (`RunSteps.svelte`); the higher-stakes Swarm has nothing. Most data is already persisted. | `swarm_run.rs`, `RunsList.svelte`; copy `workflows/RunSteps.svelte` | High | M |
| **Swarm artifacts are orphaned** — agents run in a scratch cwd; their file output is never gathered, diffed, PR'd, or even listed; `result_ref` never shown on the Done card. A coding swarm whose code you can't find isn't a tool. | `swarm_workspace.rs`, `KanbanBoard.svelte` | High | M–L |
| **AI-assisted conflict resolution** — `ConflictHunk` offers only manual ours/theirs/edit, yet the backend already has ours+theirs+base via diff3. A "Resolve with agent" per-hunk is a standout few do well, and Otto uniquely has the agent + structured context in hand. | `ConflictHunk.svelte`, `local.rs:546/706` | High | M |
| **"What the team caught"** — surface swarm concerns/reviews/decisions (the disagreements that changed direction) as a pinned per-project rollup, not a scrolling feed. This is the single best *proof* that a team beats a soloist — and the data already exists. | `BoardFeed.svelte`, `swarm_runtime.rs:245` | High | S–M |
| **PO dashboard across stories** — today just a flat list + per-story counts (N+1), tags stored but not filterable. POs need stale / unanswered-questions / ready-for-dev / ready-for-QA at a glance. | `ProductPage.svelte` | High | M |
| **Slack/Telegram draft-vs-send approval** — outbound replies to customers/providers should default to *draft*, sent only when configured; plus per-channel audit (source msg → session → reply). Trust gate for support-via-chat. | `otto-channels/mirror.rs`, channel settings UI | High | M |
| **AI commit messages in the staging flow** — PRs get "Draft with AI" but the commit box is a bare textarea. Asymmetric and surprising; reuse the `draft_pr` agent against the *staged* diff. Quick daily-delight win. | `ChangesView.svelte`, `draft_pr` (`modules.rs:1526`) | Med | S |
| **Terminal clickable links + `file:line`** — xterm loads fit/search/webgl but not web-links; URLs and `src/foo.rs:42` are dead text. URLs → `openExternal`; `path:line` → open in the Files panel. Editor-grade affordance. | `Terminal.svelte`, `FileTree.svelte` | High | S / M |
| **ERD / diagram view** — absent, yet FK metadata is fully introspected and the JOIN canvas already does draggable cards + drawn edges. Repurpose into a read-only "Diagram" tab. Closes a glaring DB-GUI parity gap, mostly reuse. | `joinCanvas.ts`, `foreign_keys_of()` | High | M |
| **Agent-assisted query plans** — EXPLAIN output dumps into the flat grid; no plan tree, no "full scan on orders (12M rows) — add an index" reading that pipes into the existing index builder. Leapfrogs DBeaver once K5 lands. | `database.svelte.ts:996`, `StructureView.svelte` | High | M |
| **Editable/saveable org templates + whole-team recruiting** — recruiter hires one agent per call (6 trips for a 6-person org); presets are read-only and a user's tuned team is unsaveable. Add "recruit a team" + "save swarm as template." | `recruiter.rs`, `presets.rs`, `RecruiterWizard.svelte` | Med–High | M |
| **Outcome-aware self-improvement** — improve learns only from session *transcripts*, never from whether the work *succeeded* (PR merged? findings recurred? tests passed?). It optimizes for plausible edits, not effective ones. Feed in git/review/test outcomes. | `otto-improve/{engine,digest,proposal}.rs` | Med–High | M |
| **Context / AGENTS.md governance** — `otto-context` materializes skills/souls into the CLIs (genuinely differentiated vs `.cursorrules`/`CLAUDE.md`/`AGENTS.md` soup) but it's root-only, thin on versioning/drift/provider-specific output. A governed, versioned, drift-detecting context layer is a real wedge. | `otto-context/{merge,materialize}.rs` | Med–High | M |
| **Live swarm surfaces are stale-by-default** — RunsList/BoardFeed/RunGraph/OrgTree need manual refresh though the events (`SwarmTaskUpdated` etc.) are already emitted. "The user is watching" is in the agent's prompt; the board shouldn't be frozen. | swarm UI subscribe to existing events | Med | S–M |
| **Findings triage + dispute-in-session** — review panel is a flat list; no severity filter, no "bugs only," no bulk action, no "ask the reviewer why" that re-prompts the still-open review session into a conversation. | `ReviewPanel.svelte` | Med | S–M |
| **Chain the two self-improvement engines** — Otto runs *two disjoint* improvement loops: `otto-improve` (transcript → propose edit → autonomy gate / version log / rollback) and `skill_eval` (worktree A/B: impl agent → validator fleet → improver → promote). Same purpose, zero shared code. Killer chain: **improve proposes → skill_eval validates the edit on a real task → promote, with improve's rollback as the net.** Today improve applies with no empirical check. | `otto-improve/engine.rs`, `skill_eval.rs` | High | M |
| **Channels as an output bus** — Slack/Telegram only *mirror* sessions; background intelligence (insight ready, approval pending, swarm done, **budget exceeded**, review finished) pushes nowhere. Route Notifications → channels to close the "Otto works while I'm away" loop (ties to the remote/mobile goal). | `otto-channels`, `monitor.rs` notices | Med | S–M |

---

## Existing-feature issues worth fixing (not just new features)

The user asked specifically about *issues in current features*. Beyond the prior
security/bug audit, these are **accepted-but-ignored controls** — the UI offers a
knob that does nothing, which is worse than not offering it:

- **Per-agent *model* selection is a silent no-op** in Product. The UI sends a
  model; rewrite/tests/plan accept `_model` and **explicitly discard it**
  (`product_run.rs:1566,2156`), and the analyze lens carries `model` but
  `run_lens_session`/`CreateSessionReq` never thread it through. Provider *is*
  honored; model is not. Either wire it or remove the control. · *S*
- **`PostQuestionsReq.format`** (`service.rs:408`) and **`PublishVersionReq`**
  (`types.rs:84`) are accepted-but-ignored stubs. · *S*
- **Swarm "Tokens" column always renders `—`** — the run rows have the columns but
  the runtime never writes them (see K3). A blank promised number reads as broken. · *S*
- **Story traceability needs a migration, not just UI** — `product_stories` has no
  PR/branch/commit/done columns and `product_events` has no code/PR/merge sections
  (`0024_product.sql`). So K4's back-link spine is partly an `otto-state` migration
  (add first-class task rows + code/PR event kinds), which is why it's the
  foundational bet rather than a quick wire-up.

---

## Tier 2 — Polish that compounds (lower effort, real friction)

- **New-Session templates / recents / native folder picker** — cwd is a raw text
  input; operators re-type the same session shape daily. (`NewSession.svelte`) · *M*
- **Palette commands for the focused session** — restart/archive/handover/rename/
  attach are only in a hover `⋯` menu, undiscoverable and unreachable from `⌘K`.
  (`App.svelte:206`) · *S*
- **`?` keyboard-shortcut overlay** — a rich keymap exists (⌃1–9 jump, ⌘F-in-term)
  but is completely undiscoverable. (`keys.ts`) · *S*
- **Confirm + undo for Delete session** — `killSession` destroys the row + history
  with no confirm/undo, and there's no transcript persistence to recover from. · *S*
- **Saved-query parameters / bind variables** — `QueryRequest.params` is defined but
  ignored by every driver; the API client already has `{{var}}` UI to mirror. · *M*
- **Passive connection-health dots** — test-connect is on-demand only; a live
  green/red/grey dot per connection is a small touch with high daily value. · *S–M*
- **Data import (CSV/JSON → table) + SQL-dump export** — everyday DBeaver actions,
  absent; forces users back to the CLI. · *M*
- **FK-click navigation in results** — both halves exist (FK graph + result→table
  mapping); connect them for relational browsing without writing JOINs. · *M*
- **Hunk/line-level staging** — file-level only; agents produce big mixed diffs that
  collapse into one unreviewable commit (also the foundation for clean stacked
  PRs). · *L*
- **Blame + commit search + per-file history** — absent; "who/why did this line
  change" is core to understanding a diff during review. · *M*
- **Word-level diff + ignore-whitespace + expand-context** — DiffViewer is
  line-level only; whitespace churn buries real changes and inflates review cost. · *M*
- **PR creation depth** — only title/desc/branches; no draft flag, reviewers,
  labels, PR-template fetch, or linked-Jira (which Otto already integrates). Users
  bounce to the web UI. · *M*
- **Workflow/swarm import-export-duplicate** — graphs/orgs are trapped in one
  machine's SQLite; the `WorkflowGraph` already serializes. · *S*
- **Workflow agent nodes are stateless one-shots** — no per-node cwd/model/skills;
  upstream data truncated to 4 KB into the prompt. It's a chat box, not a worker. · *M*
- **Self-curating skill library** — bundled catalog, skill-eval, and self-improvement
  all act on the same library with no shared quality signal (used N times, accepted
  X%, drove Y findings, cost Z). · *M*
- **Skill versioning/changelog in the UI** before install/update; **stacked PRs**
  (Graphite's wedge, increasingly how agent-driven small-PR work flows — Swarm could
  *generate* stacks). · *M / L*
- **Onboarding & teaching empty states** — best features (plain-English
  orchestration, tiled cockpit, handover, broadcast) are invisible on first run;
  empty states are dead ends that teach nothing. · *S–M*
- **Explain suspended/resumable** — clever idle-suspend reads as "my sessions
  crashed" without a first-time explanation + "N suspended / resume all." · *S*

---

## If you only do five things

Ordered by impact-per-effort, and chosen so each unblocks others:

1. **K3 — work-graph cost attribution** (stamp `repo_id/story_id/swarm_task_id/
   origin` onto session+usage rows). One cheap primitive; unblocks per-feature cost,
   the swarm spend-leash, per-session cost, and outcome-aware improvement. Also fills
   the swarm "Tokens" column that's currently a broken promise.
2. **K1 — Mission Control** (home board + live per-pane task + named, sticky
   "needs you"). Almost entirely surfacing data the daemon already produces; turns
   the multi-agent promise from implicit to visible.
3. **K2 — the verified review loop** (finding identity → fix → auto re-review →
   merge gate). The must-use differentiator for the core dev loop; Otto already owns
   every component.
4. **K4 — Product story → Swarm** (one-click). The marquee "idea → a team builds
   it" demo; the planners' data shapes already align.
5. **K5 + K6 — agent as a live, guardrailed DB/API participant** (read-only DB MCP
   tool + NL→SQL + env labels/prod read-only). The agent is the moat; stop pasting
   frozen snapshots — and gate it on prod before shipping.

**The meta-point:** Otto's next leap is not breadth — the surface area is already
ahead of most competitors. It's **making the value legible (surface the state you
already track), making the agent a live participant (not a snapshot reader), and
wiring the modules into each other (cost ↔ work, story ↔ swarm, review ↔ fix ↔
merge).** The data and the crates are mostly already there; the missing 20% is the
connective UX.

---

### Appendix — source reports

Six parallel research agents (2026-06-19), each excluding the prior security/bug
audit and reading current code:
- Sessions + UI shell · Git/PR/Review · Product + Channels · DB Explorer + API
  Client · Swarm + Workflows · Intelligence layer + cross-module integration.

Confidence: product/UX judgements from code reading, not user testing. Line numbers
reflect the working tree at audit time and may drift. Validate effort estimates
against the actual code before committing to a sprint.
