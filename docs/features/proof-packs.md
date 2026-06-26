# Proof Packs

> An agent may not declare a task "done" on assertion alone. Every meaningful unit
> of agent work carries a **Proof Pack** — a bundle of inspectable evidence whose
> **status is derived from the evidence, not claimed by the agent.** Otto assembles
> what it can automatically; agents and humans add the rest through a small API.

## What it is

A proof pack belongs to one **work item** (`UNIQUE(work_item_kind, work_item_id)`):
a `session`, `goal_loop`, `review`, `workflow_run`, `task`, or a `manual` pack. It
holds **artifacts** (evidence) and a **derived status**:

| Status | Meaning |
|--------|---------|
| `missing` | no evidence at all |
| `partial` | some evidence, but the required set for this work-item kind isn't met |
| `passed` | required evidence present and nothing failed |
| `failed` | at least one artifact failed (e.g. a test command exited non-zero) |
| `waived` | a human explicitly waived the requirement (`waived_by` recorded) |

Status, `risk_score` (0–100), and the badge set are computed by **pure functions in
`otto-core::proof`** (`derive_status` / `compute_risk` / `compute_badges`) — the single
source of truth — and recomputed on every mutation. The agent never sets the status.

### Required evidence (per work-item kind)

`derive_status` reaches `passed` only when the kind's required set is met *and* nothing
failed:

- **session / goal_loop / task** — a `diff` **and** ≥1 passing **recognized test** command
  (a `command` artifact whose command matches a real test runner — `cargo test`,
  `go test`, `npm test`, `jest`, `vitest`, `pytest`, …). A trivial green no-op like
  `true` does **not** count, so `passed` isn't gameable.
- **review** — ≥1 `review` artifact and none failed (a clean/resolved review).
- **workflow_run** — ≥1 node artifact, none failed, and every `approval` artifact present
  is `passed`.
- **manual** — lenient: ≥1 artifact and none failed.

### Artifact kinds & badges

Kinds: `command · log · screenshot · diff · ci · api · db · review · approval · self_review`.

Badges (derived, shown in the UI): `no_proof`, `tests_passed`, `tests_failed`,
`human_approved`, `risky_change` (risk ≥ 50 or a risky file present), `ci_missing` (code
changed but no CI artifact), `db_api_verified` (a `db`/`api` artifact explicitly marked
`passed`), `review_unresolved` (a `review` artifact `failed`), `waived`.

Risky files (raise `risk_score` + the `risky_change` badge) are matched by path **segment /
extension / basename** — `migrations/`, `*.sql`, `.github/`, lockfiles, and security words
(`auth`, `rbac`, `keychain`, `netguard`, `policy`, `secret`, `password`, `crypto`, `token`)
as whole words in the basename — so `author.rs`/`tokenizer.rs` are **not** false-flagged.

### Evidence integrity

All persisted artifact content runs through `otto_core::redact` (Bearer tokens, JWTs, AWS
keys, PEM blocks, emails, sensitive JSON keys) **before** storage — a trust layer must not
itself leak secrets — and the redaction hit count is recorded in `metadata.redactions`.
Content is stored in full up to **2 MiB** (larger is truncated with `metadata.truncated`);
list/detail responses carry an 8 KiB preview, and `GET /proof-artifacts/{id}/content`
returns the full stored content.

## Enforcement — "no done without evidence"

Otto **always** packages a pack + derives a status when a work item reaches "done"; the
status and badges are the visible, non-bypassable signal. Two integration points add **hard
teeth**:

1. **Goal Loops** (machine-checked). The loop already ground-truths declared
   `verify_kind=command` acceptance criteria (an "achieved" verdict with any unmet command
   criterion is coerced to "continue"). On success it now also **packages** each verify
   command (with its captured output), the worktree diff, a self-review (the evaluator's
   feedback/rationale), and a criteria summary. With `OTTO_PROOF_REQUIRE_GOAL_LOOP=1` the
   loop additionally **refuses to finalize "achieved"** without a passing machine-checked
   test — bounded by the iteration cap, accepting a `partial` pack on the last iteration so
   a genuine goal is never failed purely for lack of a command.
2. **PR creation** (default on). Opening a PR whose request carries a `proof_pack_id` is
   **rejected `409`** when that pack is not `passed`/`waived`, unless the caller passes
   `allow_unproven: true` — which is recorded as an audit `approval` artifact on the pack.
   Disable with `OTTO_PROOF_REQUIRE_PR=0`. A PR with no linked pack is not gated (Otto
   can't enforce evidence it can't locate).

The remaining gates **package** evidence and surface badges (the visible signal), without a
hard block:

3. **AI review → lifecycle-driven.** Each completed review now upserts its summarized
   findings into the persistent `review_findings` store (fingerprinted), driving the
   `open → fixing → resolved / regressed / declined` lifecycle: a finding that reappears in
   a later run `regresses`, one that's gone `resolves` (the verification leg; re-run review
   to verify a fix). The review's pack gets a `review` artifact — `failed` while findings are
   unresolved (drives `review_unresolved`), `passed` when clean. Fix-attempt linkage
   (`fix_session_id`) is set via the existing finding-state endpoint — semi-automatic, not a
   fully closed loop.
4. **Workflows.** On run completion each node's output becomes a `log` artifact (status from
   the node status), each `human_approval` node an `approval` artifact (passed iff approved,
   with approver/note/timestamp — drives `human_approved`), and the budget gate a `log`.
5. **Sessions.** On the all-done edge (the agent marks every task complete) Otto packages a
   pack: the working-tree diff always, the repo's tests **opt-in** via
   `OTTO_PROOF_AUTO_TEST=1` (running a repo's suite in the user's live cwd is disruptive, so
   default off), then a one-shot Notice if the proof is incomplete. Goal-loop-spawned session
   packs are linked to the loop's pack (`parent_pack_id`).

### What's auto vs. what an agent/human supplies (honest)

| Proof item | Auto | Via API |
|---|---|---|
| Diff summary | ✅ goal-loop + session | — |
| Test commands + output | ✅ goal-loop (verify cmds); session (opt-in) | ✅ `/assemble {commands}` or `command` artifact |
| Build / lint | partial (classified from the command) | ✅ `command` artifact |
| Screenshots / video | — | ✅ `screenshot` artifact (`content_url`) |
| API evidence | — | ✅ `api` artifact |
| DB evidence | — | ✅ `db` artifact |
| CI status | — | ✅ `ci` artifact (`content_url`) |
| Review findings | ✅ review gate | — |
| Agent self-review | ✅ goal-loop (evaluator) | ✅ `self_review` artifact |
| Human approval | ✅ workflow approval node + PR override | ✅ `approval` artifact |

Auto-capturing API-Client requests / DB-Explorer queries, and video, are follow-ups; the
artifact API supports all of them today.

## API

See `docs/contracts/api.md` (#115–125) and `docs/contracts/ws.md` (`proof_pack_updated`).
All routes are `Feature::ProofPack`-gated (View = reads, Edit = writes) and additionally
check the caller's workspace role. Key endpoints:

```
GET    /api/v1/workspaces/{id}/proof-packs[?status&work_item_kind&work_item_id]
POST   /api/v1/workspaces/{id}/proof-packs           # ensure/create for a work item
GET    /api/v1/workspaces/{id}/proof-summary         # cheap badge map (inline chips)
GET    /api/v1/proof-packs/{id}                       # pack + badges + artifacts + children
POST   /api/v1/proof-packs/{id}/artifacts            # add evidence (agent/human)
POST   /api/v1/proof-packs/{id}/assemble             # re-run diff + commands, recompute
POST   /api/v1/proof-packs/{id}/waive                # human override
GET    /api/v1/proof-artifacts/{id}/content          # full stored content
```

## UI

A **Proof** module (sidebar) lists every pack with its status chip, risk, and badges, and a
detail viewer shows the artifacts (grouped by kind, with lazy full-content load) plus
assemble / add-artifact / waive / delete actions. Each session row in the sidebar shows a
compact proof status chip from the workspace proof summary. Live updates arrive over the
`proof_pack_updated` WS event.

## Configuration (env)

| Var | Default | Effect |
|---|---|---|
| `OTTO_PROOF_REQUIRE_PR` | on | Block PRs over an unproven linked pack (`0` disables) |
| `OTTO_PROOF_REQUIRE_GOAL_LOOP` | off | Hard-require a passing test before a goal loop finalizes "achieved" |
| `OTTO_PROOF_AUTO_TEST` | off | Auto-run the repo's tests on the session all-done edge |

## Data model

`crates/otto-state/migrations/0077_proof_packs.sql` — `proof_packs` (one per work item) +
`proof_artifacts` (cascade). Repo: `otto_state::ProofRepo`. Engine: `otto_server::proof`.

## Limits & non-goals

Video capture; auto-capture of API-Client/DB-Explorer activity into artifacts; on-disk
large-binary artifact storage; a fully-closed review fix→verify loop (today a re-run
verifies). The PR gate enforces only when the PR request links a pack (`proof_pack_id`).
