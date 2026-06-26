# Mission Control / Work Graph — Requirements Matrix

This is the authoritative checklist extracted from the feature request. Every item
must be traceable to a design decision, an implementation artifact, and a test.
Status column is updated as work proceeds: ☐ todo · ◐ in progress · ☑ done.

## R1 — Core concept

| # | Requirement | Status | Design ref | Impl ref | Test ref |
|---|-------------|--------|-----------|----------|----------|
| R1.1 | A single **work graph**: every agentic activity is one traceable unit | ☐ | | | |
| R1.2 | New internal concept / crate **`otto-workgraph`** | ☐ | | | |
| R1.3 | A **Mission Control** page in the UI | ☐ | | | |

## R2 — Work item kinds (every activity type is representable)

| # | Kind | Example | Status |
|---|------|---------|--------|
| R2.1 | `session` | "Claude fixing login integration test" | ☐ |
| R2.2 | `swarm` (swarm project) | "Implement tournament notification flow" | ☐ |
| R2.3 | `goal_loop` | "Fix failing Kafka consumer test until green" | ☐ |
| R2.4 | `workflow` (workflow run) | "Nightly repo health check" | ☐ |
| R2.5 | `review` (PR review) | "Security review of payment changes" | ☐ |
| R2.6 | `product_story` | "Add bonus recommender MVP" | ☐ |
| R2.7 | `pr` | a pull request as a unit of work | ☐ |
| R2.8 | `external_trigger` | "Slack thread → agent task" | ☐ |

## R3 — Per-work-item traceability fields (the "one traceable unit" promise)

Each work item must surface:

| # | Attribute | Source | Status |
|---|-----------|--------|--------|
| R3.1 | **who/what started it** (owner / actor) | work_items.owner + initiating event | ☐ |
| R3.2 | **what repo/story/session/PR it belongs to** | work_edges (belongs_to) + repo_id/branch | ☐ |
| R3.3 | **what context was used** | work_events (context event) / artifacts | ☐ |
| R3.4 | **what tools were called** | work_events (tool_call events) | ☐ |
| R3.5 | **how much it cost** | work_items.cost_so_far (rolled up) | ☐ |
| R3.6 | **what changed** | work_events / result_summary / artifacts (diffs, commits) | ☐ |
| R3.7 | **what evidence exists** | artifacts (test runs, diffs, PR links) | ☐ |
| R3.8 | **what needs human approval** | status=needs_approval + approval record | ☐ |

## R4 — Enhancement-plan attributes (explicitly enumerated in the request)

| # | Attribute | Status |
|---|-----------|--------|
| R4.1 | visible **owner** | ☐ |
| R4.2 | **goal** | ☐ |
| R4.3 | **context** | ☐ |
| R4.4 | **cost** | ☐ |
| R4.5 | **result** | ☐ |
| R4.6 | **evidence** | ☐ |
| R4.7 | **status** | ☐ |
| R4.8 | **policy** (risk level / governance) | ☐ |
| R4.9 | **audit** (event trail) | ☐ |
| R4.10 | **approval** | ☐ |
| R4.11 | **trace artifacts** | ☐ |

## R5 — Data model (exact entities from the request)

| # | Entity & fields | Status |
|---|-----------------|--------|
| R5.1 | `work_items`: id, workspace_id, kind, title, goal, status, owner, repo_id, branch, cost_so_far, risk_level, result_summary, created_at, updated_at | ☐ |
| R5.2 | `work_edges`: from_item_id, to_item_id, relation ∈ {spawned, depends_on, fixes, reviews, verifies, blocks, belongs_to} | ☐ |
| R5.3 | `work_events`: work_item_id, timestamp, actor ∈ {user, agent, system, integration}, event_type, payload_json | ☐ |

## R6 — Integration ("every module emits events into this graph")

| # | Module → emits | Status |
|---|----------------|--------|
| R6.1 | sessions → session work item lifecycle + events | ☐ |
| R6.2 | swarm → swarm-project work item + task/agent events | ☐ |
| R6.3 | goal loops → goal_loop work item + iteration events | ☐ |
| R6.4 | workflows → workflow-run work item + node events | ☐ |
| R6.5 | reviews → review work item + finding events | ☐ |
| R6.6 | product stories → product_story work item | ☐ |
| R6.7 | PRs → pr work item | ☐ |
| R6.8 | channels (Slack/Telegram/webhook) → external_trigger work item | ☐ |

## R7 — Process requirements

| # | Requirement | Status |
|---|-------------|--------|
| R7.1 | In-depth design, reviewed | ☐ |
| R7.2 | Careful plan, every requirement mapped, reviewed | ☐ |
| R7.3 | Implementation fulfilling every requirement | ☐ |
| R7.4 | Full E2E tests | ☐ |
| R7.5 | Isolated worktree (done: `feat/workgraph-mission-control`) | ☑ |
| R7.6 | Merge to branch, verify all gates green | ☐ |
| R7.7 | If green, merge to main (LOCAL only — do not push) | ☐ |
| R7.8 | Rebuild + reinstall the app, replace the running daemon | ☐ |

## Non-negotiable engineering gates (from AGENTS.md)

- `cargo build --workspace` + `cargo test --workspace` green
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cd ui && npm run check` (svelte-check + tsc) clean
- `cd ui && npm run build` succeeds
- `cd ui && npm run test:e2e` green (incl. new Mission Control specs)
- Migrations append-only (new numbered file), never edit existing
- Contracts updated in lockstep: `docs/contracts/api.md`, `ws.md`, `ui/src/lib/api/types.ts`
- New routes classified in otto-server `policy.rs` (else 403)
- No AI attribution in commits/PRs
