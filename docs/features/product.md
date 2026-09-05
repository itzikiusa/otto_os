# Otto Product — Jira / Confluence Product-Owner Workflows

Otto's **Product** module turns an imported Jira issue or Confluence page into a
living product-owner workspace: run a fan-out of analysis agents over it, collect
and post back clarifying questions, suggest a rewrite, generate PO-approved test
cases (published to a linked Confluence page), break it into an implementation
plan, hand that plan to an Agent Swarm, draft greenfield ideas in Discovery mode,
and inject the whole refined picture into a coding agent — all while a background
watcher folds in new source comments. This is the definitive end-user and
operator reference for the feature.

> Product is a **global library**: imported stories and the learnings knowledge
> base are visible from **every** workspace (like Connections and Brokers), not
> siloed per-workspace. See [Capabilities & limitations](#6-capabilities--limitations).

---

## 1. Summary

| You want to… | Product gives you |
|---|---|
| Pull a Jira/Confluence item into Otto | **Import** by project/space search (no key prefix needed) — or paste a key/URL |
| Understand & pressure-test a story | **Analysis** — a multi-lens, multi-provider fan-out + a summarizer, each lens a real openable session |
| Get the open questions answered | **Questions** — surfaced by analysis, editable, posted back as a single Jira/Confluence comment |
| Improve the wording | **Rewrite** — a `jira-story-writer` / `rfc-writer` suggestion you can diff and publish back |
| Define how it's tested | **Test Cases** — generated drafts you approve/edit, then publish to a linked Confluence page |
| Plan the work | **Plan** — a multi-agent task breakdown into `### Task N:` checklists, then **Send to Swarm** |
| Start from a blank page | **Discovery drafts** — drop ideas/transcripts, then publish as a Jira story or a Confluence RFC |
| Capture recurring wisdom | **Learnings** — a global patterns/avoids knowledge base, auto-suggested by agents |
| Stay in sync with the source | **Watcher** — periodic poll that reconciles new comments into answered questions |
| Feed a coding agent the full picture | **Inject** — a consolidated context bundle dropped into a new agent session |
| Design what it looks like — 2D and 3D | **Design** — one arena for HTML screens (device frames), Excalidraw boards, Mermaid diagrams and `scene3d` 3D scenes (viewport + hierarchy + inspector), each agent-assisted, file-backed and autosaved; GLB / Blender-script export, optional Blender render |
| Keep a swarm's output under one roof | **Epic tree** — a story can be an *epic* with children in *folders*; swarm agents publish INTO their project's epic instead of piling up top-level drafts |

---

## 2. Overview & where it lives

Architecturally, Product is split across a feature crate (CRUD, source fetch,
publishing, context assembly), the daemon server (the AI orchestration that needs
the session manager + improvement engine), and a Svelte UI module.

| Layer | Path | Responsibility |
|---|---|---|
| Feature crate | `crates/otto-product/` | `ProductService` (import/refresh/publish/draft/context), the `router()` for CRUD routes, the seven bundled skills, the memory façade |
| · types | `crates/otto-product/src/types.rs` | Request DTOs + response wrappers (`ImportStoryReq`, `AnalyzeReq`, `InjectBundle`, …) — **not** in `otto-core` |
| · skills | `crates/otto-product/src/skills.rs` + `crates/otto-product/assets/skills/` | The bundled agent skills, version-gated seed into the library |
| · memory | `crates/otto-product/src/memory_facade.rs`, `extract.rs` | Ingest structured artifacts into `otto-memory`; deterministic extractors |
| Persistence | `crates/otto-state/src/product.rs` | `ProductRepo` + the `product_*` tables |
| Orchestration | `crates/otto-server/src/product_run.rs` | `run_analysis` / `run_rewrite` / `run_generate_tests` / `run_generate_plan`, retries, summarizers |
| Watcher | `crates/otto-server/src/product_watcher.rs` | The background poll/reconcile supervisor |
| Swarm bridge | `crates/otto-server/src/product_swarm.rs` | `story_to_swarm` (Plan → Swarm) |
| Memory route | `crates/otto-server/src/routes/product_memory.rs` | `POST …/memory/ingest` |
| Issue trackers | `crates/otto-issues/` | The Jira/Confluence clients (`JiraClient`, `ConfluenceClient`) Product proxies through |
| Design formats | `crates/otto-server/src/design_format.rs`, `design_scene3d.rs`, `design_blender.rs` | The single `DesignFormat` enum (html · mermaid · excalidraw · scene3d), the `scene3d` validator + deterministic Blender-script generator, Blender detection / render jobs |
| Media | `crates/otto-server/src/product_media.rs`, `mockup_assist.rs` | Attachment upload / content PUT (mime allow-list, sniffing, size caps), the in-place file-backed assist agent |
| UI | `ui/src/modules/product/` | `ProductPage.svelte` + per-tab components; `types.ts` mirrors the DTOs |
| · Design Arena | `ui/src/modules/product/design/` | `DesignArena.svelte` (assets / viewport / inspector / assistant shell), `DesignBoard.svelte` (Excalidraw island), `DeviceFrame.svelte`, templates, exporters |
| · 3D | `ui/src/modules/product/design/scene3d/` | `Scene3DViewport.svelte` (lazy three.js), `Hierarchy.svelte`, `Inspector.svelte`, `types.ts` / `validate.ts` / `ops.ts` (pure doc mutations) / `exportGlb.ts` |
| Skills | `crates/otto-skills/assets/skills/development/otto-{mockup,design-2d,design-3d}/` | What the assistant / swarm agents know about the four formats and their authoring rules |
| Contracts (authoritative) | `docs/contracts/product.md`, `api.md` (§ Product), `ws.md` (Product events) | The source-of-truth API spec |

In the app, Product is reached from the left nav (the **Product** item) or the
command palette entry **"Go to Product"**; the route is `#/product`.

---

## 3. Prerequisites & setup

Product is a consumer of Otto's **Issue trackers** (Jira / Confluence)
integration. You need a working issue account before you can import anything.
See [`./jira-confluence.md`](./jira-confluence.md) for the full setup; the
essentials:

- **An issue account is per-user and owner-scoped.** Each user adds their own
  Jira account; the token is theirs and is never shared or shown back.
- **Add an account** via `POST /api/v1/issue/accounts`
  (`CreateIssueAccountReq`): `provider` (only `jira` is recognized today —
  Confluence is reached through the same Atlassian-cloud account/token),
  `label`, `email` (your Atlassian login email), `base_url` (the site root, e.g.
  `https://acme.atlassian.net`), and `token` (an Atlassian API token).
  Optionally set `token_expires_at` to drive expiry notifications.
- **The token is write-only on the wire and stored in the macOS Keychain** via
  `otto-keychain`; the SQLite row keeps only an opaque `token_ref`. The account
  JSON returned to the UI never contains the token.
- **RBAC:** to use Product a user needs the **`Issues`** capability (to reach the
  account/search endpoints) and the **`Product`** capability (to use the Product
  module). See [Security & permissions](#7-security--permissions).

Once an account exists, Otto can list its projects/spaces, search them, and read
issue/page content for import. No Atlassian app, webhook, or marketplace
install is required — everything is REST against the cloud API using your token.

---

## 4. Full feature walkthrough

The Product page is a two-pane layout: a left sidebar with the **story list**
(plus import controls and a tag filter) and a main area with a **Stories /
Learnings** toggle. With a story selected, the main area shows a tab strip:

> **Overview · Analysis · Questions · Notes · Rewrite · Test Cases · Plan · Design · History · Inject**

(For a draft, Overview becomes an editor; for a Jira-backed story, Overview adds
a live Jira metadata panel. The tab id `mockups` is kept as a deep-link alias for
**Design**; a `tree_kind:"doc"` child hides the analysis/plan tabs.) The story
list itself is a **tree** — epics with folders and children — see
[§4.13](#413-epic-tree--epics-folders-and-swarm-publishing).

### 4.1 Importing a story (search by project/space — no key prefix)

**UI** — Click **Import** in the sidebar. The **Import story** dialog:

1. **Account** — pick one of your issue accounts (shown as `label (base_url)`).
2. **Source** — toggle **Jira issue** or **Confluence page**.
3. **Search** (the no-prefix path):
   - *Jira:* pick a **Project** (or "All projects"), then type a number, key, or
     free text into **Search issues**. Results show `key · summary · status`.
   - *Confluence:* pick a **Space** (or "All spaces"), then search **pages** by
     title or id; results show `title` + the space key.
4. Click a result; it becomes a highlighted chip with a **Change** button.
5. *(Optional)* expand **"Enter issue key / page ID manually"** to paste a key,
   a `…/browse/PROJ-123` URL, or a Confluence page URL/`pageId` (auto-normalized).
6. *(Optional)* set a **Repo path** (the `cwd` the architecture lens and
   injected agents will run in) and tick **"Watch this story for changes."**
7. Click **Import**.

**Behind it** — `POST /api/v1/workspaces/{ws}/product/stories` with
`ImportStoryReq { source_kind, account_id, source_key, cwd?, watch_enabled? }`.
`ProductService::import_story` fetches the source:

- *Jira* (`source_kind: "jira"`): `JiraClient::get_issue(source_key)` →
  title (summary), URL, body markdown (ADF → markdown), and a `raw_json` capture
  (key, status, issue type, assignee, …).
- *Confluence* (`source_kind: "confluence"`): `ConfluenceClient::get_page(id)` →
  title, URL, body markdown (storage → markdown), `raw_json` (space key, version).

It creates the story row (`stage: "imported"`), writes **version 1** (`kind:
"source"`), and records an `imported` event. The search endpoints used by the
dialog are `GET /issue/projects`, `/issue/search`, `/issue/confluence/spaces`,
`/issue/confluence/search`.

**Refresh.** `POST /product/stories/{sid}/refresh` re-pulls the source; a new
`source` version is created **only if the body changed**, with a `refreshed`
event.

### 4.2 Analysis — multi-lens, multi-provider fan-out + summarizer

**UI (Analysis tab)** — In **Configure**, pick from three lenses, each with
toggleable provider chips (you can run one lens on claude *and* codex *and* agy):

| Lens label | Skill | What it does |
|---|---|---|
| PO Overview | `po-story-overview` | Value/clarity/scope/AC completeness, at product altitude |
| Architecture | `story-architecture-overview` | Maps the story to the codebase under the repo path — files, contracts, risks |
| Clarifying Questions | `story-clarifying-questions` | The fewest, sharpest, categorized open questions |

Add an optional **Focus** note, choose a **Summarizer** provider (shown when more
than one agent is selected), and click **Run analysis**. Each (lens × provider)
runs as a **real, openable session** with a live status pill (running / waiting /
done / error) and an **Open** terminal; failed lenses can be **Retried** and
running ones **Stopped**. When all finish, a **Synthesized Summary** appears,
followed by per-lens findings in collapsible sections: **Related repos**,
**Functionalities**, **Integration points**, **Risks**, **Open questions** (each
categorized), and **Suggested learnings** (which land in the Learnings base as
pending suggestions).

**Behind it** — `POST /workspaces/{id}/product/stories/{sid}/analyze`
(`AnalyzeReq`) returns **200** with the created `ProductAnalysis` row, then
`product_run::run_analysis` fans out in the background:

- **Defaults when `agents` is empty:** the three lenses above, all on the
  resolved default provider (workspace → global → `claude`).
- A shared, enriched context file is written once (see
  [§4.10](#410-how-product-builds-an-agents-context)) and every distinct provider
  is pre-trusted on the `cwd` so no session stalls on a "trust this folder?" prompt.
- Each lens runs concurrently via `run_lens_session` (a real `SessionManager`
  session, *not* the old claude-only `Orchestrator::run_agent`), with auto-retry
  up to 3 attempts. Failures are isolated per agent.
- The **summarizer** consolidates all per-lens `findings_json` into one result —
  merging questions/repos/risks/learnings, resolving conflicts, and recording
  `conflict_notes`. If it fails, Otto falls back to a deterministic Rust-side merge.
- **Open questions** are extracted (summarizer first, else per-lens), deduped by
  normalized text, and persisted as `ProductQuestion` rows (status `open`).
- On completion it emits the `product_changed` event (`section: "analysis"`,
  `status: "done"` or `"partial"`) and moves the story to `stage: "analyzed"`.

Per-agent control: `POST /product/analyses/{aid}/agents/{agent_id}/retry` and
`…/stop` (both **202**).

### 4.3 Questions — answer, edit, and post back as comments

**UI (Questions tab)** — Filter by status (Open / Posted / Answered / Discarded)
and category (Scope / Data / UX / Edge Case / Dependency / Other). For each card
you can **Edit**, **Answer** (inline), **Discard**, or **Delete**. Tick one or
more and click **"Post N to Jira / Confluence."**

**Behind it** — CRUD via `POST /product/stories/{sid}/questions`,
`PATCH /product/questions/{qid}`, `DELETE …`. Posting is
`POST /product/stories/{sid}/questions/post` (`PostQuestionsReq { ids, format? }`):
`ProductService::post_questions` builds **one** combined markdown comment
("Clarifying questions from Otto:" + a numbered list) and posts it via
`JiraClient::add_comment` or `ConfluenceClient::add_comment` (markdown is
converted to storage format for Confluence). Each posted question flips to status
`posted` with its `posted_ref` (the comment URL, or id). A single
`question_posted` event is recorded in the `questions` section.

> **Note:** the `format` field is accepted but currently a no-op — comments are
> always posted as markdown. ADF/plain selection is a future enhancement.

### 4.4 Rewrite — suggest a better story/RFC and publish it back

**UI (Rewrite tab)** — Pick a **Provider** and click **"Generate suggested
rewrite."** When ready you get a **word diff** (Source vs Suggested, with
"Source only / Suggested only" views) and **Publish to Jira/Confluence** (a
confirm step warns it overwrites the live ticket/page).

**Behind it** — `POST /workspaces/{id}/product/stories/{sid}/rewrite`
(`RewriteReq`, **202**). `run_rewrite` auto-selects the writer skill by source
kind — **`jira-story-writer`** for Jira, **`rfc-writer`** otherwise — enriches
the context with answered questions and the latest analysis summary, and writes a
new **`suggested`** version (`{title, body_markdown, change_notes}`), advancing
the story to `stage: "refined"` and emitting `product_changed`
(`section: "rewrite"`).

**Publishing a version** — `POST /product/versions/{vid}/publish` pushes a
version's body to the live source: Jira via `update_description`, Confluence via a
versioned `update_page`. It records a `published` version and a `published` event,
returning `{url, ref}`.

### 4.5 Test Cases — generate, approve/edit, and publish to Confluence

**UI (Test Cases tab)** — Pick a **Provider** and **Generate test cases.** A run
appears with its cases grouped by category (**Happy Path / Validation / Error
Cases / Edge Cases**), each with a priority badge and steps (Preconditions /
Steps / Expected). Per case you can **Approve**, **Request changes** (with a
note), or **Edit** (title, category, priority, the three step fields). You can
**Select all** and **Approve N** in bulk, drag to **reorder**, **Approve run**,
and **Publish to Confluence** (with optional space key / parent page id).

**Behind it:**

- Generate: `POST /workspaces/{id}/product/stories/{sid}/testcases/generate`
  (`GenerateTestsReq`, **202**). `run_generate_tests` uses the `story-test-cases`
  skill, creating a **draft** `ProductTestcaseRun` + `ProductTestcase` rows whose
  `steps_json` is `{preconditions[], steps[], expected}`; story → `stage:
  "tests_drafted"`; emits `product_changed` (`section: "testcases"`).
- Per-case edit / approve / request-changes: `PATCH /product/testcases/{tid}`.
- Bulk: `POST /product/testcase-runs/{rid}/testcases/bulk-approve` (`{ids}`).
- Reorder: `POST /product/testcase-runs/{rid}/testcases/reorder` (`{ordered_ids}`).
- Approve the run: `POST /product/testcase-runs/{rid}/approve` — flips all cases +
  the run to `approved` **and** spawns a `story-test-cases` skill
  self-improvement pass (`otto-improve::run_for_narrative`), folding any
  "request changes" feedback into the narrative so the skill evolves.
- Publish: `POST /product/testcase-runs/{rid}/publish` (`PublishTestsReq {
  space_key?, parent_id? }`). Only **approved** cases are rendered to markdown,
  converted to Confluence storage, and written to a page titled
  *"Test Cases — {story title}"* (created once, then updated in place). For a
  Confluence-backed story the space key is read from the source; for a Jira story
  a `space_key` is required and Otto best-effort comments the page URL back on the
  issue. Returns `{url}`.

### 4.6 Plan — multi-agent task breakdown, then Send to Swarm

**UI (Plan tab)** — Pick one or more **Planning agents** (provider chips); a
**Summarizer** dropdown appears when more than one is selected. The checkbox
**"Don't ask me questions — I'm not available; I'll review the plan at the end"**
is **checked by default** (autonomous mode). Click **Generate plan** (or
**"Generate plan · N agents"**). While it runs, the planning sessions **tile
side-by-side** and a **"Watching N planning agents"** banner appears; in
interactive mode you can answer questions in the tiles. The finished plan renders
as a checklist of **Task N** groups; ticking an item cycles todo → in-progress →
done and the progress bar updates. **⚡ Send to Swarm** hands the plan off.

**Behind it:**

- `POST /workspaces/{id}/product/stories/{sid}/plan/generate` (`GeneratePlanReq`,
  **202**). `run_generate_plan`: one **visible** session per provider (each its
  own openable session, `source: "product-plan"`), enriched with answered
  questions, the latest analysis summary, and **approved test cases**. When
  `interactive` is false (the default; `None` is treated as false) an autonomy
  directive is prepended instructing agents not to ask questions. With more than
  one planner a **summarizer** merges the candidate plans (falling back to the
  first plan if it fails). Each new session id is broadcast via the `plan_run` WS
  event so the UI tiles them live.
- The result is saved as a `kind: "plan"` version titled *"Implementation Plan,"*
  whose body uses `### Task N: <title>` headings with `- [ ]` checklist steps and
  a `**Verify:**` line; story → `stage: "planned"`; emits `product_changed`
  (`section: "plan"`).
- Checkbox state persists via `POST /workspaces/{id}/product/stories/{sid}/plan`
  (`SavePlanReq { body_md }`, **204**) — it overwrites the latest plan version's
  body in place (no new version).
- **Send to Swarm:** `POST /product/stories/{sid}/to-swarm` (`ToSwarmReq {
  swarm_id?, name? }`). `story_to_swarm` is **idempotent** — re-sending returns
  the linked project (`created: false`). It resolves a swarm (explicit id →
  workspace's first → an auto-created paused **Default Swarm**), sets the
  project's `goal_md` to the most-refined body (latest `suggested` → `source` →
  title), and seeds Kanban tasks by parsing the plan's `### Task N:` headings (or,
  if there is no plan, by running the swarm planner over the goal). Seeded tasks
  carry a `product` label. The story view then shows a **linked swarm project**
  badge (`SwarmLinkCard`) with task/run/PR/cost rollups; see
  [`./agent-swarm.md`](./agent-swarm.md).

### 4.7 Discovery drafts — start blank, publish as RFC or story

**UI** — Click **New draft** in the sidebar. The Overview tab becomes an editor
(**Title**, **Body (Markdown)**, **Save draft**) with a **Transcripts** panel on
the side: paste a conversation/call notes as a transcript (title + body, **Add
transcript**). Refine the draft using the Analysis / Questions / Rewrite tabs,
then **Publish as Jira Story** or **Publish as Confluence RFC**.

**Behind it:**

- Create: `POST /workspaces/{ws}/product/drafts` (`NewDraftReq`) — a story with
  `source_kind: "draft"`, `stage: "draft"`, and a blank `kind: "draft"` version.
- Edit: `PATCH /product/stories/{sid}/draft` (`UpdateDraftReq { title, body_md }`)
  — edits the draft version in place (no new version).
- Transcripts: `GET/POST /product/stories/{sid}/transcripts`,
  `DELETE /product/transcripts/{trid}`. Transcripts feed the agent context (they
  do **not** appear in the inject bundle).
- Publish as RFC: `POST /product/stories/{sid}/publish-as-rfc` (`PublishAsRfcReq {
  account_id, space_key, parent_id?, title? }`) — creates a Confluence page from
  the best content version, records a `published` version, and **rebinds** the
  draft to that page (`source_kind → "confluence"`, `stage → "refined"`).
- Publish as story: `POST /product/stories/{sid}/publish-as-story`
  (`PublishAsStoryReq { account_id, project_key, issue_type }`) — creates a Jira
  issue and rebinds the draft to it (`source_kind → "jira"`). (For a
  Confluence-backed RFC this instead spawns a *new* linked Jira story and
  cross-links the two.)

### 4.8 Learnings — a global recurring-patterns knowledge base

**UI (Learnings toggle)** — A **Knowledge Base** with two columns: **Patterns to
follow** and **Cases to avoid**. **+ Add learning** captures `kind` (pattern /
avoid), title, body, comma-separated tags, and a JSON `refs` array. Agent
**Suggested learnings** show a "⚡ AI-suggested · pending acceptance" banner with
an **Accept** button; each learning has an **Active / Inactive** toggle, Edit, and
Delete.

**Behind it** — `GET/POST /workspaces/{ws}/product/learnings`,
`PATCH /product/learnings/{lid}`, `DELETE …`, and
`POST /product/learnings/{lid}/accept` (adopt a suggestion → `active = true`).

> **Global, not workspace-scoped (verified in code).** As of the commit
> *"feat(product): make Stories + Learnings a global library (not
> workspace-scoped)"*, `ProductRepo::list_learnings(active_only)` and
> `list_stories()` take **no** workspace argument and run **no** `WHERE
> workspace_id = ?` filter. The `workspace_id` column is retained purely as the
> *creating* workspace for provenance. The same is true of imported **stories** —
> the story list shows every story from every workspace. No migration was needed;
> existing rows became visible everywhere immediately.

### 4.9 History — sectioned event timeline + versions + tags

**UI (History tab)** — A timeline filterable by **Section**: *Source, Analysis,
Questions, Notes, Rewrite, Tests, Publish, Inject, Watch*. Each entry shows a
colored section badge, the event kind, actor, time, and summary. Tags on a story
(set via the sidebar tag filter / `PATCH` story) drive the sidebar tag chips.

**Behind it** — `GET /product/stories/{sid}/events?section=…` returns
`ProductEvent` rows (immutable, append-only); versions are
`GET /product/stories/{sid}/versions` (no body) and `GET /product/versions/{vid}`
(with body). Version `kind` values across the codebase: `source`, `suggested`,
`published`, `draft`, `plan`.

### 4.10 How Product builds an agent's context

Every AI run (analysis, rewrite, tests, plan) writes a shared context file built
by `ProductService::build_agent_context(story_id, focus)`:

- Story header (title, source/key, URL, stage) and an optional **FOCUS** block.
- The **best content version** body (newest `suggested`, else `source`).
- For Jira stories, the **full live Jira context** (`get_issue_full`): status,
  assignee, reporter, priority, labels, custom fields, linked issues, the last 20
  comments, recent change history, and attachments — cached per-daemon by
  `(story id, updated_at)` with a 64-entry FIFO cap.
- The active **Learnings** (grouped patterns/avoids) and the story's **transcripts**.

The **Inject** tab (`GET /product/stories/{sid}/inject`) assembles a separate,
consolidated `InjectBundle` of up to six sections — **Story, Analysis Summary,
Answered Questions, Approved Test Cases, Relevant Learnings, Implementation
Plan** — as both a flat `markdown` blob and a `sections[]` list. **Build /
Preview** shows it; **Open in agent** (`POST
…/inject-session`, `InjectSessionReq { provider?, model?, cwd? }`) spawns a new
agent session preloaded with the bundle.

### 4.11 Recall from the memory layer

Product can push a story's structured artifacts into Otto's domain-agnostic
memory layer (`otto-memory` — keyword/FTS5 recall; distinct from the
file-backed docs [Vault](./vault.md)) so they become recallable instead of
re-fetched raw each turn:

- **Ingest (wired):** `POST /workspaces/{ws}/product/stories/{sid}/memory/ingest`
  runs `ProductMemory::ingest_story`, which uses the deterministic extractors in
  `extract.rs` to turn **answered questions** (→ `qa`), **active learnings** (→
  `learning`), the **latest analysis summary** (one classified memory per
  bullet — decision / constraint / requirement / fact), and the **newest version**
  (→ `summary`) into atomic memories saved through `MemoryService::save`
  (exact-duplicate dedup happens inside `save`).
- **Recall (implemented, not yet auto-used):** `ProductMemory::recall_brief`
  exists (a compact, token-budgeted brief grouped by kind) but the orchestrator
  does **not** yet call it — today the AI runs build context via
  `build_agent_context` (fresh DB reads + the live-Jira LRU cache described
  above), not via memory recall. Treat memory recall as opt-in ingestion now,
  with automatic recall-into-context as a planned enhancement.

### 4.12 The background watcher

Per story, set **watch** on import or via `PATCH /product/stories/{sid}`
(`watch_enabled`, `watch_cadence_min`). A daemon supervisor rescans watched
stories about every **60 seconds**; a story is polled when its cadence elapses
(cadence floor is **5 minutes** — smaller values are clamped). On a poll it:

1. refreshes the source (captures description edits),
2. fetches comments newer than `watch_cursor`, records each as a `watch`-section
   `comment` event, and advances the cursor,
3. runs a **reconcile** agent pass (timeout ~180s, up to 3 attempts) that maps new
   comments onto open questions — marking questions **answered** with the extracted
   answer, adding **new questions**, and proposing a recommended next step
   (recorded as a `reconciled` event),
4. triggers a `story-clarifying-questions` + `po-story-overview` skill
   self-improvement pass, and
5. emits a `Notice` ("Story {key}: N new comment(s)") surfaced in the
   notification center.

---

### 4.13 Epic tree — epics, folders and swarm publishing

A story can be the **parent** of other stories. There is exactly **one level**
of nesting (an epic → children), and each child sits in a **folder** inside the
epic (`Design`, `PO`, `QA`, … — free text, `''` = unfiled). Three columns on
`product_stories` carry it: `parent_id`, `tree_kind` (`story` | `epic` | `doc`)
and `folder`. `tree_kind` is Otto's *tree role* — deliberately independent of
`source_kind` (jira/confluence/draft) and of Jira's own issue type — so a Jira
story linked to a swarm project shows as the root of its tree **without Otto
ever rewriting the Jira row**: the UI treats a row as an epic when
`tree_kind == 'epic'` **or** it has children.

**UI (list pane)**

```
▾ 🗂 Loyalty programme (epic · 6)        ← click = epic overview; ▸ collapses
    Design/                              ← folder header (collapsible)
      ◻ Tier ladder screens      DRAFT
      ◻ Rewards 3D kiosk         DRAFT
    PO/
      ◻ Feature draft v2
  ○ GS-1204 Bonus wallet limits          ← top-level story, unchanged look
```

- The story header shows a breadcrumb `Epic › Folder › Title`.
- Right-click a story: **Move to epic…** (picker), **Set folder…**, **Mark as
  epic**, **Detach from epic**. Toolbar **New ▾** → *Draft* / *Epic*; inside an
  epic **Add child ▾** → *Story* / *Doc*. (Drag-and-drop is a follow-up.)
- An epic's **Overview** adds a *Children* board (folder → cards) with rolled-up
  counts, and its **Design** tab shows every child's design artifacts
  (filterable by child) — the epic is the single place to review a feature.
- The tag filter still applies (to the flattened list).
- **Deleting an epic re-parents its children to top level** — it never cascades
  into them. Deleting any story now also removes its attachments and annotation
  rows (and, best-effort, its files under `data_dir/product/attachments/<sid>/`).

**Swarm publishing.** Before, every `otto-product` call from a swarm agent
created a fresh top-level draft — a project with a PO, an analyst and a designer
(plus retries) produced a pile of pages. Now `POST /ingest/swarm/product`:

1. resolves the **epic** — the swarm project's `story_id` if set (that story is
   left untouched and simply gains children); otherwise ONE `tree_kind:'epic'`
   draft named after the project goal is created and stored on the project (a
   unique index + `Conflict` mapping make two racing agents converge on the same
   epic); a swarm without a project keeps the old top-level behaviour;
2. files the **child** with `otto-product --title T --folder F --kind doc|story`
   (defaults: folder = the agent's role title, kind = `doc`). A child with the
   same normalised title already under the epic is **updated** (a new `suggested`
   version) rather than duplicated;
3. broadcasts `product_changed { section:"tree" }` so the list refreshes.

`otto-mockup` follows the same target resolution (discovery run's story →
project's story → the resolved epic) and accepts `--format
html|mermaid|excalidraw|scene3d`. Both helpers now **fail loudly** (`curl -f`,
HTTP status printed, non-zero exit) instead of silently 204-ing.

### 4.14 Design Arena — screens, boards, diagrams and 3D scenes

The **Design** tab (formerly *Mockups*) is one arena for every visual artifact on
a story. Desktop layout; on mobile it collapses to a segmented top bar with a
single pane, like the Database Explorer:

```
┌──────────────┬─────────────────────────────────────────────┬────────────────┐
│ ASSETS       │  ⟵ toolbar: New ▾ · Import · Export ▾ · ⋯  │ INSPECTOR      │
│ Screens (3)  │                                             │ (per format)   │
│  ◻ Home      │            VIEWPORT / BOARD                 │  Transform     │
│  ◻ Checkout  │   html: sandboxed iframe (device frames)    │  Material  …   │
│ Boards (1)   │   excalidraw: full editor (React island)    │────────────────│
│ Diagrams (2) │   mermaid: existing renderer                │ ASSISTANT      │
│ 3D (1)       │   scene3d: three.js + orbit + gizmo         │  (live agent   │
│  ◈ Kiosk     │                                             │   shell +      │
│ Images (4)   │  status: 12 objects · 2 lights · saved 2s   │   composer)    │
└──────────────┴─────────────────────────────────────────────┴────────────────┘
```

**Artifacts are attachments.** Every design is a `product_attachments` row of
`kind:'design'` (legacy `kind:'mockup'` rows and `image`s are listed alongside)
whose `meta_json` carries `{ format, assist_session_id, group }` — `group` is the
asset-list heading (*Screens*, *Boards*, …), unrelated to the epic `folder`.
There is **one format enum**, `DesignFormat`, on both sides of the wire; anything
else is a **400**, never a silent fallback to HTML:

| format | file | mime | renders with |
|---|---|---|---|
| `html` | `design.html` | `text/html` | sandboxed iframe in a **device frame** (iPhone / iPad / desktop / none) with a light/dark toggle |
| `mermaid` | `design.mmd` | `text/vnd.mermaid` | the Mermaid renderer |
| `excalidraw` | `design.excalidraw` | `application/vnd.excalidraw+json` | the Excalidraw board (`DesignBoard.svelte`; the user edits by hand, autosaved) |
| `scene3d` | `scene.json` | `application/vnd.otto.scene3d+json` | the three.js viewport (lazy-loaded on first use) |
| `glb` / `gltf` | uploaded | `model/gltf-binary` / `model/gltf+json` | as a `gltf` object inside a scene, or standalone |
| images | uploaded | `image/*` | `<img>` |

**Common to every format**

- **New ▾**: Screen · Board · Diagram · 3D scene · **From template** (Canva-like
  starters: mobile app screen, dashboard, landing page, user flow, game level
  blockout, product shot — static files under `design/templates/`).
- **Import**: files (`.glb .gltf .png .svg .excalidraw`, …) through the existing
  upload; **Export ▾**: PNG (viewport/board snapshot), SVG (mermaid/excalidraw),
  the source file, **GLB** (scene3d, via three's `GLTFExporter`) and **Blender
  script** (scene3d, generated server-side).
- **Annotations**: the pin-comment layer stays on html/image/mermaid/excalidraw.
  Camera-anchored 3D pins are deferred; a scene object has a per-object **notes**
  field in the document instead (the assistant reads them as instructions).
- **Assistant**: the in-place agent (`MockupAssistPanel`, generalised) with a
  format-aware prompt, provider/model pick and a live preview of every format,
  resumable per artifact. It edits the artifact's file; each save is validated
  and broadcast as `mockup_updated`.
- **Autosave**: the UI's own edits (inspector, board, code view) go through
  `PUT /product/attachments/{aid}/content` with a 600 ms debounce; the status line
  shows *saved / saving / conflict*. If an agent edit arrives while you have
  unsaved local changes the panel asks before replacing — no silent clobber.

**2D — Figma/Canva side.** The Excalidraw board is the freeform tool (frames as
artboards, shapes, text, arrows, groups, align — Excalidraw provides them). HTML
screens render in a device frame with a scheme toggle. The **`otto-design-2d`**
skill teaches agents Excalidraw JSON authoring (frames as artboards, 8-pt grid,
palette, components, bound labels/arrows) and the script-free HTML screen rules.

**3D — game-studio side.** Opening a `scene3d` artifact swaps the asset list for
a **Hierarchy** and the right pane for the scene **Inspector**:

- **Viewport** (`Scene3DViewport.svelte`): orbit / pan / zoom, grid + axes,
  shadows, PBR lighting from the document, click-to-select (drag-select is off in
  v1), a **gizmo** (`TransformControls`) — `W` move · `E` rotate · `R` scale ·
  `F` frame selection · `Delete`/`⌫` delete · `⌘D`/`Ctrl+D` duplicate · `Esc`
  deselect (keys are ignored while an input has focus). **Play** hides the gizmo,
  grid and light markers and looks through the document camera — a presentation
  view; **View → camera** stores your current orbit view into the document so
  Play, the agent and the Blender export all see it.
- **Hierarchy**: objects and nested groups, lights in their own section; click to
  select, double-click / `F2` to rename inline, the eye to hide (hidden ancestors
  dim their subtree), a filter box, and a `+` menu with the seven primitives, the
  five light types, *Group* and **Import GLB / glTF…** (an upload that becomes a
  `gltf` object). Right-click / ⋯: rename, duplicate, move up/down, move to a
  group / top level, group this, delete (a group deletes its subtree).
- **Inspector**: with nothing selected — **Scene** (background, grid) and
  **Camera** (position, target, FOV, clip planes); for an object — **Transform**
  (position m, rotation °, scale; scrub-drag numeric fields, reset / snap-to-floor
  buttons), **Material** (color, metalness, roughness, opacity, emissive,
  wireframe), **Text** or **Model** (attachment id) panels; for a light — type,
  intensity, color, position/target, angle/distance, shadows; for a group — its
  members; every node has **Notes**. Each field patches the JSON document → autosave.
- **The document** (`otto-scene3d` v1) is small and declarative — the browser
  renders it, the agent edits it, the inspector round-trips it. Units are
  **metres, y-up, floor at y = 0**; rotation is in **degrees**; `type ∈ box |
  sphere | cylinder | cone | torus | plane | text | gltf`; a `gltf` object
  references an uploaded model by **`attachment_id`, never a URL** (the viewer
  resolves it through the authed attachment route). It is validated on both
  sides before render and before save: known types only, finite numbers, ≤ 2 000
  objects / 64 lights / 500 groups, unique ids, safe `attachment_id`s, `#rrggbb`
  colours. All UI mutations are pure functions in `scene3d/ops.ts` — no component
  mutates the document directly. The **`otto-design-3d`** skill carries the full
  schema, unit sizes, blockout conventions and lighting recipes.

**Blender bridge (optional, detected).** Everything above works without Blender.
When `GET /product/design/blender` reports it installed (`$OTTO_BLENDER`, then
`PATH`, then `/Applications/Blender.app`), a scene gains **Render in Blender**:
the daemon **generates** a Python script from the *validated* document (a fixed
template interpolating only validated numbers/enums/escaped names — never a user
or agent file), runs `blender -b --python … -- --out <dir>` under a sandbox
profile with a 120 s timeout, and attaches the resulting `render.png` (Eevee,
1280×720) and `scene.glb` as new design artifacts (`meta.derived_from = <aid>`),
each announced by `mockup_updated`. **Download Blender script** returns the same
`.py` to open in Blender by hand. For agents that have the **Blender MCP**
available (the MCP page offers a *Blender* template — `stdio`, `uvx
blender-mcp`, tool access *ask*, disabled until saved), `otto-design-3d` says to
prefer it for real modelling and export a GLB back into the scene; otherwise the
agent edits `scene.json`.

---

## 5. API / contract reference

`docs/contracts/product.md` is authoritative; `api.md` (§ Product) and `ws.md`
mirror it. Conventions: `/api/v1` base, JSON snake_case, RFC3339 timestamps, ULID
ids, `Authorization: Bearer <token>`; reads need **ws viewer**, mutations **ws
editor**. Routing is two-tier — **collection** routes are
`/workspaces/{ws}/product/…`; **item** routes are flat `/product/<entity>/{id}`
and resolve the workspace from the owning row.

### CRUD & content (served by `otto-product::router`)

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /workspaces/{ws}/product/stories` | viewer | list stories (global) |
| `POST /workspaces/{ws}/product/stories` | editor | import a story |
| `GET /product/stories/{sid}` | viewer | story detail (+ source version, counts, swarm link) |
| `PATCH /product/stories/{sid}` | editor | update cwd/stage/tags/watch |
| `DELETE /product/stories/{sid}` | editor | delete (cascades children) |
| `POST /product/stories/{sid}/refresh` | editor | re-pull source (new version iff changed) |
| `GET /product/stories/{sid}/versions` · `GET /product/versions/{vid}` | viewer | versions list / one |
| `POST /product/versions/{vid}/publish` | editor | push a version to the source → `{url, ref}` |
| `GET /product/stories/{sid}/analyses` · `GET /product/analyses/{aid}` | viewer | analyses; one (with per-agent state) |
| `GET/POST /product/stories/{sid}/questions` · `PATCH/DELETE /product/questions/{qid}` | viewer/editor | questions CRUD |
| `POST /product/stories/{sid}/questions/post` | editor | post selected questions as one comment |
| `GET/POST /product/stories/{sid}/notes` · `PATCH/DELETE /product/notes/{nid}` | viewer/editor | notes CRUD |
| `GET /product/stories/{sid}/events?section=` | viewer | sectioned history |
| `GET /product/stories/{sid}/testcases` | viewer | testcase runs (+ cases) |
| `PATCH /product/testcases/{tid}` | editor | edit / approve / request-changes a case |
| `POST /product/testcase-runs/{rid}/publish` | editor | publish approved cases to Confluence → `{url}` |
| `POST /product/testcase-runs/{rid}/testcases/bulk-approve` · `/reorder` | editor | bulk-approve / reorder |
| `GET/POST /product/stories/{sid}/transcripts` · `DELETE /product/transcripts/{trid}` | viewer/editor | transcripts |
| `PATCH /product/stories/{sid}/draft` | editor | update the working RFC draft body |
| `POST /product/stories/{sid}/publish-as-rfc` · `/publish-as-story` | editor | publish a draft |
| `GET/POST /workspaces/{ws}/product/learnings` · `PATCH/DELETE /product/learnings/{lid}` · `POST …/accept` | viewer/editor | learnings (global) |
| `GET /workspaces/{ws}/product/drafts` | viewer | list drafts |
| `GET /product/stories/{sid}/inject` | viewer | the consolidated inject bundle |
| `GET /product/stories/{sid}/swarm` | viewer | the linked-swarm closure (tasks/runs/PRs/cost) |
| `POST /product/stories/{sid}/children` | editor | `{ title, tree_kind, folder }` → `ProductStoryDetail` — add a child to an epic (one level only; a parent that itself has a parent is rejected) |

`ProductStory` carries the tree: `parent_id: Id | null`, `tree_kind: "story" |
"epic" | "doc"`, `folder: string`. `PATCH /product/stories/{sid}` accepts them
(`parent_id` may be set to `null` to detach); `DELETE` re-parents children to top
level and removes the story's attachments + annotations.

### Design Arena (attachments, assist, Blender)

> Written from §5 of `docs/design/product-design-arena.md`; `docs/contracts/api.md`
> and `ws.md` are authoritative once Track A lands — re-check them if a name below
> disagrees.

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /product/stories/{sid}/attachments` · `POST …/attachments` (multipart) | viewer / editor | list / upload. Allowed mimes now include `application/vnd.excalidraw+json`, `application/vnd.otto.scene3d+json`, `model/gltf-binary`, `model/gltf+json` (sniffed: glTF magic / JSON); their default `kind` is `design` |
| `GET /product/attachments/{aid}` | viewer | the bytes (the viewer's `authedBlobUrl` — also how `gltf` objects load) |
| `PUT /product/attachments/{aid}/content` | editor | `{ "data_b64": "<base64>" }` → `ProductAttachment`. Saves an edited artifact (inspector / board / code view). Same guards as upload (`allowed_mime`, `sniff_ok`, size cap, confined path, 40 MB body limit); the row's `mime`/`filename` never change; bumps `size_bytes` + `updated_at`; broadcasts `mockup_updated` |
| `POST /product/stories/{sid}/mockups/assist` | editor | start / resume the in-place assistant — `format ∈ html \| mermaid \| excalidraw \| scene3d`, anything else **400** |
| `GET /product/design/blender` | viewer | `{ installed, path, version }` |
| `POST /product/stories/{sid}/design/{aid}/blender-render` | editor | **202** `{ id }` — queue a headless render + GLB export of a `scene3d` artifact |
| `GET /product/design/jobs/{id}` | viewer | `{ id, attachment_id, status: queued\|running\|done\|error, error, outputs: [aid…], started_at, finished_at }` — in-memory, not persisted across daemon restarts |
| `GET /product/stories/{sid}/design/{aid}/blender-script` | viewer | the generated `.py` for the scene |
| `POST /ingest/swarm/product` · `POST /ingest/swarm/mockup` | session token | `ProductIngestReq { title?, body_md, tree_kind?, folder? }` (the `otto-product` helper) and the `otto-mockup` helper with `format` — resolve the project's epic as in §4.13 |

The `scene3d` document schema (v1) is in §4.14 and, in full, in the
`otto-design-3d` skill; the TS mirror lives in
`ui/src/modules/product/design/scene3d/types.ts` + `validate.ts`, the Rust
validator in `crates/otto-server/src/design_scene3d.rs`.

### AI orchestration (served by `otto-server`)

| Method & path | Response | Notes |
|---|---|---|
| `POST /workspaces/{id}/product/stories/{sid}/analyze` | **200** `ProductAnalysis` | row created now, fan-out spawned in background |
| `POST /workspaces/{id}/product/stories/{sid}/rewrite` | **202** | `suggested` version |
| `POST /workspaces/{id}/product/stories/{sid}/testcases/generate` | **202** | draft run + cases |
| `POST /workspaces/{id}/product/stories/{sid}/plan/generate` | **202** | tiled planners + summarizer; emits `plan_run` |
| `POST /workspaces/{id}/product/stories/{sid}/plan` | **204** | persist PO checkbox state |
| `POST /product/stories/{sid}/to-swarm` | `ToSwarmResp` | idempotent Plan → Swarm |
| `POST /workspaces/{id}/product/stories/{sid}/inject-session` | `Session` | spawn an agent preloaded with the bundle |
| `POST /product/testcase-runs/{rid}/approve` | `ProductTestcaseRun` | + skill self-improvement |
| `POST /product/analyses/{aid}/agents/{agent_id}/retry` · `/stop` | **202** | per-lens control |
| `POST /workspaces/{ws}/product/stories/{sid}/memory/ingest` | `{ingested}` | extract artifacts into `otto-memory` |

### WebSocket events (`/ws/events`)

- **`product_changed`** — `{ workspace_id, story_id, section:
  "analysis"|"rewrite"|"testcases"|"plan"|"tree", status: "done"|"error"|"changed" }`.
  Emitted at the end of every AI run (each Product tab polls once on its matching
  section) and, with `section:"tree"`, when swarm ingest adds or updates a child
  under an epic (the list pane refreshes).
- **`mockup_updated`** — `{ story_id, attachment_id, format, content: string | null }`.
  Every save of a design artifact — by the assistant, by `PUT …/content`, or a
  Blender job's outputs. Text formats carry the new source in `content`; binary /
  large payloads (GLB, images) carry an explicit `null` and clients re-fetch.
- **`mockup_session_started`** — the assist session for an artifact is live
  (the Assistant panel attaches its terminal).
- **`plan_run`** — `{ workspace_id, story_id, session_ids[], interactive }`.
  Re-emitted as each planning/summarizer session appears (later frames are
  supersets), so the Plan tab can tile and watch them live.

---

## 6. Capabilities & limitations

**You can:**

- Import any Jira issue or Confluence page by **searching a project/space** (no
  key prefix), or by pasting a key/URL.
- Run **multiple analysis lenses across multiple providers** (claude, codex, agy)
  simultaneously — each is a real, openable, retryable/stoppable session — and get
  a single consolidated summary with conflict notes.
- Post a batch of clarifying questions back as **one** Jira/Confluence comment.
- Get a **rewrite** suggestion, diff it, and publish it back over the live item.
- Generate, edit, approve/bulk-approve, reorder, and **publish test cases to a
  linked Confluence page**.
- Break a refined story into a multi-agent **plan** and **hand it to an Agent
  Swarm** (idempotently).
- Start from a **blank Discovery draft**, drop in transcripts, and publish it as a
  Jira story or Confluence RFC.
- Keep a **global learnings** base and **inject** a story's full context into a
  fresh coding agent.
- Have a **watcher** fold new comments into answered questions automatically.
- Organise stories as **epics with folders**, and have swarm agents publish into
  their project's epic instead of creating top-level drafts.
- Design in one arena: HTML **screens** in device frames, Excalidraw **boards**,
  Mermaid **diagrams** and **3D scenes** (viewport, hierarchy, inspector, gizmo,
  Play view), each agent-assisted, autosaved, exportable (PNG / SVG / source /
  GLB / Blender script) and, with a local Blender, rendered headlessly.

**You cannot (today):**

- Nest epics **recursively** — one level plus folders (Jira's epic → story shape).
  Drag-and-drop in the tree is a follow-up; use the context menu.
- Pin **3D annotations** — per-object `notes` in the scene document stand in
  until `product_mockup_annotations` grows a `meta_json` column.
- Import **Figma** files, sculpt / UV-edit meshes in the browser, or co-edit with
  live cursors — Blender (via the bridge) is the modelling tool; the arena is for
  layout, review and iteration.
- Upload **Python** / Blender scripts as attachments — the Blender export is always
  generated by the daemon from a validated scene; `text/x-python` is not an
  accepted mime.

- Use a **non-Atlassian** issue tracker. `IssueProviderKind` recognizes only
  `jira`; Confluence rides the same Atlassian account.
- Choose a comment **format** when posting questions — markdown only (the `format`
  field is a no-op).
- Rely on **automatic memory recall** in AI runs — ingestion is wired and
  `recall_brief` exists, but the orchestrator still builds context from fresh DB
  reads + a live-Jira cache, not from memory recall.
- Silo stories or learnings **per-workspace** — both are a single global library
  (the `workspace_id` is provenance only). If you need isolation, this is
  intentionally not provided.
- Expect **versioned plan history from PO checkbox edits** — toggling checkboxes
  overwrites the latest plan version in place.

---

## 7. Security & permissions

- **RBAC.** Product is gated by the **`Product`** feature capability (`None <
  View < Edit < Admin`); reads need a workspace **Viewer** role, mutations a
  workspace **Editor** role. The import/search/read-back flows additionally need
  the **`Issues`** capability. Default-deny: no grant ⇒ `403`. See
  [`./../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).
- **Credential ownership.** Issue accounts are per-user and owner-scoped. Routes
  that act through an account (import, refresh, post questions, publish, publish
  as RFC/story) enforce that the caller **owns** the bound account (or is root) —
  so you can never publish through, or leak, someone else's Atlassian token.
- **Keychain.** Atlassian API tokens live in the macOS Keychain via
  `otto-keychain`; the DB stores only an opaque `token_ref`, and account JSON
  never includes the token.
- **No new outward surface.** Product reaches Jira/Confluence over HTTPS using
  your token; the daemon itself stays loopback-only unless you explicitly enable a
  network listener.
- **Design artifacts are untrusted content.** HTML screens render only in the
  sandboxed iframe (scripts disabled); `scene3d` JSON is validated on both sides
  before it is rendered or saved; models load solely by `attachment_id` through
  the authed attachment route (no URL surface); the content `PUT` reuses the
  upload's mime allow-list, sniffing and size caps; Blender only ever runs a
  daemon-generated script, under a sandbox profile, a 120 s timeout and
  Editor-only RBAC. All `/product/` routes ride the existing `Feature::Product` gate.
- **Note: global visibility ≠ no auth.** Stories and learnings are visible across
  workspaces, but every read/write still passes the feature + workspace gates, and
  publishing still requires owning the credential.

---

## 8. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Import dialog has no accounts | Add a Jira account first (Settings → Issue trackers); you also need the `Issues` capability. |
| Search returns nothing | Wrong project/space selected, or the token lacks access. Try the manual key/URL fallback to confirm the item exists. |
| Analysis agent stuck "waiting" / times out | The provider may be prompting to trust the `cwd`. Otto pre-trusts each provider, but a misconfigured CLI can still stall — open the session to inspect, then **Retry** or **Stop**. |
| Architecture lens has no codebase findings | The story has no **Repo path** (`cwd`) set, or it points nowhere. Set it via Import or `PATCH` the story. |
| "space_key required" when publishing test cases | Jira-backed stories need an explicit **Space key** in the publish form; Confluence stories infer it from the source. |
| Posting questions does nothing visible | All selected questions were already `posted`/`answered`/`discarded`, or the comment failed — check the issue and the History → Questions section. |
| Plan agents ask questions when you wanted them unattended | Leave **"Don't ask me questions"** checked (the default = autonomous); unchecking it is interactive mode. |
| `to-swarm` "created another project" you didn't expect | It's idempotent per story — re-sending returns the existing project (`created:false`). A second project means a different story. |
| Watcher isn't picking up comments | `watch_enabled` must be on; cadence is clamped to a 5-minute floor and the supervisor rescans ~every 60s, so allow a few minutes. |
| Learning appears in another workspace | Expected — learnings and stories are a **global** library. |
| Injected agent missing test cases / plan | The inject bundle only includes **approved** test cases and the latest `plan` version — approve the run / generate a plan first, then **Rebuild**. |
| Swarm agents still create top-level drafts | The swarm has no project, or the project has no `story_id` and epic creation failed — check the helper's printed HTTP status (`otto-product` no longer swallows errors). A story linked via Plan → Swarm becomes the epic automatically. |
| Design tab shows "3D viewport unavailable" | WebGL is disabled/blocked in the webview, or the lazy `three` chunk failed to load (offline dev server). Reload; check the browser console for the import error. |
| A model shows as a red wireframe box | The `gltf` object's `attachment_id` did not resolve (deleted attachment, no viewer rights, or a non-glTF file). The status line lists the failing models; re-import the GLB. |
| "Save rejected" / conflict on a scene | The document failed validation (unknown `type`, non-finite number, > 2 000 objects, duplicate id) or an agent saved in between — the panel asks before replacing your local edits; **Reload** to take the agent's version or **Keep mine** to overwrite. |
| Assistant returns 400 on create | Unknown `format` — only `html`, `mermaid`, `excalidraw`, `scene3d`; there is no fallback to HTML any more. |
| Blender buttons missing | `GET /product/design/blender` reports `installed:false`: set `$OTTO_BLENDER` to the binary, put `blender` on `PATH`, or install to `/Applications/Blender.app`. Everything else in 3D works without it. |
| Blender render job stuck `running` / `error` | Jobs are in-memory (lost on daemon restart) and killed after 120 s; check `error` on `GET /product/design/jobs/{id}` — large scenes with many shadow-casting lights are the usual timeout. |

---

## 9. Related docs

- [`./jira-confluence.md`](./jira-confluence.md) — connecting and managing Jira /
  Confluence accounts (the prerequisite for Product).
- [`./agent-swarm.md`](./agent-swarm.md) — the Agent Swarm that **Send to Swarm**
  hands a plan off to.
- `docs/contracts/api.md` (§ Memory layer) — the `otto-memory` keyword store
  Product ingests into (and will recall from). The [Vault](./vault.md) is now
  the separate file-backed docs home.
- [`./channels-slack-telegram.md`](./channels-slack-telegram.md) — bridging agent
  sessions (including Product's) to Slack / Telegram.
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — the feature/workspace/ownership
  permission model that gates Product.
- Authoritative contracts: `docs/contracts/product.md`, `docs/contracts/api.md`
  (§ Product / § Issue trackers), `docs/contracts/ws.md` (Product events).
- `docs/design/product-design-arena.md` — the reviewed design for the Design
  Arena + epic tree (data model, `scene3d` schema, Blender bridge, delivery plan).
- [`./canvas.md`](./canvas.md) — the separate Canvas module (Excalidraw / D2 /
  Mermaid scenes not bound to a story); [`./mcp-control-plane.md`](./mcp-control-plane.md) — where the
  Blender MCP template lives.
