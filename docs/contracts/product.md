# Otto API Contract — Product Story Analysis (`/api/v1`)

The `product` section: import a Jira issue / Confluence page, analyze it with
selectable agents, run a clarifying-questions loop, version content, keep notes +
sectioned history, suggest a rewrite, generate PO-approved test cases (published to a
linked Confluence page), maintain a learnings knowledge base, watch stories for
updates, and inject the refined picture into a developer agent.

Conventions match `api.md`: JSON snake_case, RFC3339 timestamps, ULID ids,
`Authorization: Bearer <token>`. Errors: HTTP status per `otto_core::Error` + body
`Problem{code,message}`. Roles: `viewer` for reads, `editor` for mutations.

DTO names: persisted row structs live in `crates/otto-state/src/product.rs`; request
DTOs + response wrappers in `crates/otto-product/src/types.rs` (NOT `otto-core`). The
TypeScript mirror is `ui/src/modules/product/types.ts` (not the shared
`ui/src/lib/api/types.ts`).

Routing is two-tier: **workspace-collection** routes are `/workspaces/{id}/product/...`;
**item** routes are flat `/product/<entity>/{id}` and resolve+role-check the workspace
from the owning row. The agent-orchestration endpoints (analyze / rewrite /
generate-tests / inject-session) and the testcase-run **approve** live in
`otto-server` (they need the orchestrator / session manager / improvement engine);
all other routes are served by the `otto-product` crate router.

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| P1 | GET /api/v1/workspaces/{id}/product/stories | ws viewer | — | `ProductStory[]` |
| P2 | POST /api/v1/workspaces/{id}/product/stories | ws editor | ImportStoryReq | ProductStoryDetail (fetches Jira/Confluence, creates story + v1 source version) |
| P3 | GET /api/v1/product/stories/{sid} | ws viewer | — | ProductStoryDetail (story + latest source version + counts) |
| P4 | PATCH /api/v1/product/stories/{sid} | ws editor | UpdateStoryReq | ProductStory |
| P5 | DELETE /api/v1/product/stories/{sid} | ws editor | — | 204 (cascades children) |
| P6 | POST /api/v1/product/stories/{sid}/refresh | ws editor | — | ProductStoryDetail (new source version iff content changed) |
| P7 | GET /api/v1/product/stories/{sid}/versions | ws viewer | — | `ProductStoryVersion[]` (no body) |
| P8 | GET /api/v1/product/versions/{vid} | ws viewer | — | ProductStoryVersion (with body) |
| P9 | POST /api/v1/product/versions/{vid}/publish | ws editor | — | `{url,ref}` (pushes a suggested version → Jira description / Confluence page; records a `published` version) |
| P10 | POST /api/v1/workspaces/{id}/product/stories/{sid}/analyze | ws editor | AnalyzeReq | ProductAnalysis (async fan-out; default agents if `agents` empty) |
| P11 | GET /api/v1/product/stories/{sid}/analyses | ws viewer | — | `ProductAnalysis[]` |
| P12 | GET /api/v1/product/analyses/{aid} | ws viewer | — | ProductAnalysisDetail (run + per-agent states; poll for progress) |
| P13 | GET /api/v1/product/stories/{sid}/questions | ws viewer | — | `ProductQuestion[]` |
| P14 | POST /api/v1/product/stories/{sid}/questions | ws editor | NewQuestionReq | ProductQuestion |
| P15 | PATCH /api/v1/product/questions/{qid} | ws editor | UpdateQuestionReq | ProductQuestion |
| P16 | DELETE /api/v1/product/questions/{qid} | ws editor | — | 204 |
| P17 | POST /api/v1/product/stories/{sid}/questions/post | ws editor | PostQuestionsReq | `{posted:[{id,ref,url}]}` (posts selected questions as one Jira/Confluence comment; marks `posted`) |
| P18 | GET /api/v1/product/stories/{sid}/notes | ws viewer | — | `ProductNote[]` |
| P19 | POST /api/v1/product/stories/{sid}/notes | ws editor | NewNoteReq | ProductNote |
| P20 | PATCH /api/v1/product/notes/{nid} | ws editor | UpdateNoteReq | ProductNote |
| P21 | DELETE /api/v1/product/notes/{nid} | ws editor | — | 204 |
| P22 | GET /api/v1/product/stories/{sid}/events?section=… | ws viewer | — | `ProductEvent[]` (sectioned history; optional `section` filter) |
| P23 | POST /api/v1/workspaces/{id}/product/stories/{sid}/rewrite | ws editor | RewriteReq | 202 (async; creates a `suggested` version; auto-selects jira-story-writer vs rfc-writer by source kind) |
| P24 | POST /api/v1/workspaces/{id}/product/stories/{sid}/testcases/generate | ws editor | GenerateTestsReq | 202 (async; creates a draft testcase run + cases) |
| P25 | GET /api/v1/product/stories/{sid}/testcases | ws viewer | — | `ProductTestcaseRunDetail[]` (each = run + cases[]) |
| P26 | PATCH /api/v1/product/testcases/{tid} | ws editor | UpdateTestcaseReq | ProductTestcase (PO approve / request-changes / edit) |
| P27 | POST /api/v1/product/testcase-runs/{rid}/approve | ws editor | — | ProductTestcaseRun (marks run approved; triggers `story-test-cases` skill self-improvement) |
| P28 | POST /api/v1/product/testcase-runs/{rid}/publish | ws editor | PublishTestsReq | `{url}` (creates/updates a linked Confluence page; comments the URL on a Jira story) |
| P29 | GET /api/v1/product/stories/{sid}/inject | ws viewer | — | InjectBundle (consolidated refined-story context: story + analysis + answers + approved tests + learnings) |
| P30 | POST /api/v1/workspaces/{id}/product/stories/{sid}/inject-session | ws editor | InjectSessionReq | Session (spawns an agent session preloaded with the bundle) |
| P31 | GET /api/v1/workspaces/{id}/product/learnings?active= | ws viewer | — | `ProductLearning[]` |
| P32 | POST /api/v1/workspaces/{id}/product/learnings | ws editor | NewLearningReq | ProductLearning |
| P33 | PATCH /api/v1/product/learnings/{lid} | ws editor | UpdateLearningReq | ProductLearning |
| P34 | DELETE /api/v1/product/learnings/{lid} | ws editor | — | 204 |
| P35 | POST /api/v1/product/learnings/{lid}/accept | ws editor | — | ProductLearning (adopt an agent-suggested learning: active=true) |
| P36 | POST /api/v1/product/stories/{sid}/to-swarm | ws editor | ToSwarmReq | ToSwarmResp (Plan → Swarm: create a swarm project from the story + seed tasks) |

Notes:
- **Agents are claude-backed (v1).** Analysis/rewrite/test/reconcile agents run as
  headless `Orchestrator::run_agent` (claude) one-shots; "multiple agents" = a fan-out
  over multiple lenses/skills (po-story-overview, story-architecture-overview,
  story-clarifying-questions) and the optional `model` hint. A non-claude `provider`
  in a request is recorded but still executes claude. Multi-provider execution
  (codex/agy) is a future enhancement.
- **Six library skills** (seeded write-if-absent into `<data_dir>/library/skills/`):
  `po-story-overview`, `story-clarifying-questions`, `story-architecture-overview`,
  `story-test-cases`, `jira-story-writer`, `rfc-writer`. They are editable in the
  Library UI and evolvable by `otto-improve` via `run_for_narrative` (triggered on
  test-case-run approval and by the watcher folding in new comments). To enable
  auto-apply, add the skill names to a workspace's `self_improvement.skill_allowlist`.
- **Story watcher**: per-story `watch_enabled` + `watch_cadence_min` (floor 5 min). A
  daemon background supervisor polls Jira/Confluence for new comments, records them,
  advances `watch_cursor`, runs a reconcile agent pass (maps comments → open
  questions, proposes next steps), triggers clarification skill self-improvement, and
  emits a `Notice` event.
- JSON-as-TEXT columns on the wire: `findings_json`, `steps_json`
  (`{preconditions[],steps[],expected}`), `refs_json`
  (`[{type:'jira'|'confluence'|'memory'|'url',ref,label}]`), `meta_json`.
- **Plan → Swarm bridge (P36).** `to-swarm` turns a refined story into a runnable
  Agent-Swarm project — the cross-feature path Product alone can't run. `ToSwarmReq
  {swarm_id?, name?}`: an explicit `swarm_id` (verified same-workspace), else the
  workspace's first swarm, else an auto-created paused **Default Swarm**. The project's
  `goal_md` is the story's most-refined body (latest `suggested` version → `source` →
  title). Tasks are seeded by reusing the story's `kind="plan"` version (parsing its
  `### Task N:` headings into Kanban cards); if there is no plan, the swarm planner is
  run over the goal to generate one. The endpoint is idempotent — re-sending a story
  returns the existing linked project (`created:false`). Back-link: migration 0035 adds
  a nullable, indexed `story_id` to `swarm_projects`; `SwarmProject.story_id` is the
  source story, and `ProductStoryDetail.swarm_link` (`{project_id, swarm_id,
  project_name}` or null) is the reverse view that drives the story's "linked swarm
  project" badge. `ToSwarmResp {swarm, project, tasks[], created}`. Seeded tasks carry a
  `product` label and emit `swarm_task_updated` WS events.
