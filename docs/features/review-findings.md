# Review Findings Workflow

Otto's [AI Code Review](./code-review.md) fans out agents over a change and
produces a deduplicated list of findings. The **Review Findings Workflow** is the
layer that turns each of those findings into a **tracked, triable record** — with a
stable identity across re-runs, a human disposition (a 6-state machine), an
immutable event timeline, agent-backed **fix / verify / regression-test** actions,
Jira filing, durable repo rules, and an exportable evidence bundle. It is how a
review stops being a one-shot list and becomes a closed loop: *found → triaged →
fixed → verified → remembered*.

This document is the authoritative end-user + operator reference. The domain
contract lives in `crates/otto-core/src/finding.rs`; the engine detection
lifecycle + fingerprinting in `crates/otto-state/src/review_findings.rs`; the
HTTP handlers in `crates/otto-server/src/routes/findings.rs` and
`crates/otto-server/src/routes/proof_pack.rs`; and the UI in
`ui/src/modules/git/FindingsBoard.svelte` + `FindingActions.svelte` +
`ProofPackView.svelte`. The wire surface is specified in
`docs/contracts/api.md` ("Review findings workflow") and
`docs/contracts/ws.md` (`finding_updated` / `finding_action_started` /
`proof_pack_exported`).

---

## 1. What it is

When an AI review run completes, the summarizer's deduped comments are **upserted**
into the persistent `review_findings` store, each with a stable **fingerprint**.
From then on a finding is a first-class object you act on. Two axes live on every
finding, with **disjoint writers** — neither can stomp the other:

| Axis | Field | Vocabulary | Written by |
|---|---|---|---|
| **Workflow disposition** (the human triage) | `status` | `open · accepted · false_positive · fixed · verified · waived` | the **action endpoints** only |
| **Engine detection lifecycle** (the machine's view) | `state` | `open · fixing · resolved · regressed · declined` | the **review engine** only (upsert / resolve) |

This separation is the heart of the design: your triage (`status`) survives a
re-review, and the engine's re-detection (`state`) never silently rewrites your
decision. The UI exposes `status` as the live workflow chip and `state` only as a
derived `regressed` flag (a finding that reappeared after it was closed).

> **Relationship to AI Code Review.** This workflow sits *on top of*
> [`./code-review.md`](./code-review.md). The review engine produces the findings;
> this layer tracks them. `GET /reviews/{id}/findings` was **widened** from the old
> `ReviewFindingRow[]` to the full `Finding[]` (a non-breaking superset — every old
> field is retained), and the legacy
> `POST /reviews/{id}/findings/{fingerprint}/state` transition is **deprecated** in
> favour of the id-keyed `/findings/{id}/*` actions below.

---

## 2. The Finding — its 11 fields

`Finding` (`crates/otto-core/src/finding.rs`) carries identity fields
(`id`, `review_id`, `workspace_id`, `repo_id`, `pr_number`, `fingerprint`) plus
**the 11 tracked workflow fields**:

| # | Field(s) | Meaning |
|---|---|---|
| 1 | `title` + `body` | the finding's headline + full description |
| 2 | `severity` | normalized `critical · high · medium · low · info` (see §2.1) |
| 3 | `category` | optional lens/category tag from the reviewing agent |
| 4 | `path` + `line` + `line_end` | the code location (file + line range) |
| 5 | `evidence` | the code excerpt / proof the agent cited |
| 6 | `agent_reasoning_summary` | *why* the agent thinks it's a problem |
| 7 | `suggested_fix` | the agent's proposed remedy (optional) |
| 8 | `status` | the 6-state workflow disposition (§3) |
| 9 | `linked_commit` | the commit that fixed it (set by the fix flow) |
| 10 | `linked_test` | the guard test added for it (set by regression-test) |
| 11 | `reviewer` | the current disposition owner (who triaged it) |

Alongside those sit the workflow/gate/artifact fields: `state` + `regressed`
(the read-only detection axis), `requires_human_approval`, `approval_decision`,
`approved_by`, `approved_at`, `jira_key`, `jira_url`, `produced_by_agent`,
`repo_rule_id`, `fix_session_id`, `occurrence_count`, `created_at`, `updated_at`.

Each finding also has an immutable audit trail: a `FindingEvent` per
action/transition (`kind`, `actor`, `from_status`, `to_status`, `detail`,
`created_at`). `GET /findings/{id}` returns a `FindingDetail {finding, events}` —
the finding plus its full timeline.

### 2.1 Severity normalization

Reviewer agents emit a grab-bag of severity tokens (`info|warn|bug`,
`blocker|major|minor|nit`, `critical|high|…`). `FindingSeverity::normalize` is a
**total** mapping that folds all of them into the five-level scale on both write
and read (so legacy `bug`/`warn` rows never break): `critical|blocker → critical`;
`bug|high|error|major → high`; `warn|warning|medium → medium`; `minor|low → low`;
everything else (incl. `nit`, `info`, unknown) → `info`.

---

## 3. The 6-state workflow machine

`FindingStatus` has exactly six values, and `can_transition` enforces a strict
table — the action endpoints reject illegal moves, and the UI buttons mirror the
same gates:

```
            ┌──────────── (reopen) ────────────┐
            │                                   │
  open ──accept──▶ accepted ──fix(async)──▶ fixed ──verify──▶ verified
   │  \           │   │   \                   │   \              │ (re-verify
   │   \          │   │    └──── verify ──────┘    \             │  idempotent)
   │    └─false_positive / waive    │               └─verify─────┘
   │                                │
   └─ waive / false_positive        └─ waive / false_positive
```

Legal transitions (the exact set in `can_transition`):

| From | Allowed to |
|---|---|
| `open` | `accepted` · `false_positive` · `waived` |
| `accepted` | `fixed` · `verified` · `false_positive` · `waived` · `open` |
| `fixed` | `verified` · `false_positive` · `waived` · `open` |
| `verified` | `verified` (idempotent re-verify) · `false_positive` · `open` |
| `false_positive` | `open` (reopen only) |
| `waived` | `open` (reopen only) |

Notably **illegal**: `open → fixed` (a fix always goes via `accepted`), `open →
verified` (you can't verify an untouched finding), and any move *out of* a terminal
state except a reopen to `open`. The `verified → verified` self-edge lets a
re-verify be idempotent.

---

## 4. How findings are born (and re-detected)

When a review completes, the summarizer iterates its comments and **upserts** each
into `review_findings`:

- **Fingerprint** = `SHA-256` over
  `repo_id | pr_number | path | category | body[:512]` (path/category/body
  lower-cased and trimmed, body capped at 512 chars on a char boundary). The
  fingerprint is the *same-finding-across-runs* identity.
- **Dedup scope.** PR reviews dedup by `(workspace_id, repo_id, pr_number,
  fingerprint)`; local reviews dedup by `(review_id, fingerprint)`.
- **New finding** → `status = open`, `state = open`, `occurrence_count = 1`.
- **Re-detected** (same fingerprint reappears) → `occurrence_count += 1`, and the
  human `status` is **NOT reset** — your triage survives a re-review.
- **Regression** — a finding that reappears after it was `resolved`/`declined` is
  flipped to `state = regressed` by the engine; the UI shows a `regressed` chip and
  tints the card.
- **Resolution leg** — `resolve_absent(...)` marks every `open`/`fixing` finding
  that is **not** seen in the new run as `resolved` (if it's gone, it was fixed).
  This is the verification leg: re-running the review is what verifies a fix.

### Merge readiness

The `review_merge_readiness` view is rebuilt on the new workflow vocabulary:
**unresolved** = `open | accepted | fixed`; a **blocker** is an unresolved finding
of `critical`/`high` severity. The Review panel's merge-readiness banner (see
[`./code-review.md`](./code-review.md) §6.4) reads these counts.

---

## 5. The findings board (UI)

`FindingsBoard.svelte` mounts inside both the PR **ReviewPanel** and the
**LocalReviewPanel** for a completed review (`reviewId` + `workspaceId`). It loads
`GET /reviews/{id}/findings` and renders one expandable card per finding:

- **Header chips** — severity chip (red for `critical`/`high`), the workflow
  `status` chip, a `regressed` chip when applicable, a `needs approval` chip when
  the human-approval gate is set, and the title (or first line of the body).
- **Meta row** — `category`, the location `path:Lstart–Lend` (monospace), the
  `reviewer`, `seen ×N` when `occurrence_count > 1`, and artifact chips: the
  `linked_commit` (short SHA), a `test` chip when `linked_test` is set, and the
  `jira_key` (linked to `jira_url` when present).
- **Expanded detail** — the `evidence`, `agent_reasoning_summary`, and
  `suggested_fix` blocks, then the **Timeline** (the `FindingEvent` list, fetched
  via `GET /findings/{id}`), then the **action bar** (§6).
- **Filters + counts** — pill filters by `status` (the 6 values) and `severity`
  (the 5 values), each with a live count; a header count and a **Proof Pack**
  button (§7).

The board subscribes to the `findingBus` (driven by the WS events in §8) and
silently re-fetches when a finding under *this* review changes — the same pattern
the Review panel uses for `review_changed`.

---

## 6. The 7 action buttons (+ overflow)

`FindingActions.svelte` renders the per-finding action bar. Each button is
**disabled per the legal status transition** (mirroring `can_transition`), and
agent-backed actions return `{finding, session_id?}` so the spawned session is
openable in **Agents**. The seven headline buttons:

| Button | Endpoint | Effect | Enabled when |
|---|---|---|---|
| **Ask agent to fix** | `POST /findings/{id}/fix` | `open\|accepted → accepted`, spawns a fix agent in a worktree; async stamps `fixed` on commit (sets `fix_session_id`) | `status ∈ {open, accepted}` |
| **Verify resolved** | `POST /findings/{id}/verify` | spawns a verify agent; on pass `accepted\|fixed\|verified → verified` | `status ∈ {accepted, fixed, verified}` |
| **Convert to Jira** | `POST /findings/{id}/jira` | creates a Jira issue, stores `jira_key`/`jira_url` (inline project-key input) | `jira_key` not set |
| **Mark false positive** | `POST /findings/{id}/false-positive` | `→ false_positive` | `status ∈ {open, accepted, fixed, verified}` |
| **Require human approval** | `POST /findings/{id}/require-approval` | sets the human-approval gate; **status unchanged** | gate not already set |
| **Add to repo rule** | `POST /findings/{id}/repo-rule` | generalizes the finding into a durable **repo rule** (Context Engine), links `repo_rule_id` | `repo_rule_id` not set |
| **Add regression test** | `POST /findings/{id}/regression-test` | spawns an agent to add a guard test; sets `linked_test` | `linked_test` not set |

An **overflow menu** (`⋯`) holds the disposition moves that don't need a dedicated
button:

- **Accept** → `POST /findings/{id}/accept` (`open → accepted`).
- **Waive** → `POST /findings/{id}/waive` `{reason?}` (`→ waived`); enabled for
  `open|accepted|fixed`.
- **Approve / Reject** (only when the approval gate is set and not yet decided) →
  `POST /findings/{id}/approve` `{decision: approve|reject, note?}`. **Approve**
  clears the gate (`open → accepted`); **Reject** → `false_positive`.

Every action validates the transition server-side, appends a `finding_events` row,
emits `finding_updated`, and returns the updated `Finding`. The Jira action returns
**400 `{code:"invalid"}`** when no Jira account is configured.

---

## 7. Repo rules + the review Proof Pack

### 7.1 Repo rules (durable lessons → Context Engine)

**Add to repo rule** generalizes a finding into a `RepoRule` — a durable lesson
(`title`, `body`, optional `category`/`severity`/`glob`, `source_finding_id`,
`enabled`) that is **materialized into future agent sessions' instruction files**
via the Context Engine, so the same class of mistake is steered away from next
time. Repo rules are managed under the **`Context`** feature:

| Method & path | Auth | Effect |
|---|---|---|
| `GET /workspaces/{ws}/repo-rules` | ws viewer (Context) | list the workspace's rules |
| `POST /repo-rules/{id}/toggle` | ws editor (Context) | `{enabled}` — re-materializes the rules block |
| `DELETE /repo-rules/{id}` | ws editor (Context) | 204 |

### 7.2 The review Proof Pack (assemble + export → memory)

The board's **Proof Pack** button opens `ProofPackView.svelte`, which calls
`GET /reviews/{review_id}/proof-pack` to **live-assemble** a `ReviewProofPack`: a
`summary` (counts — total, by status, by severity, `verified`, `fixed`, `open`,
`with_commit`, `with_test`), every finding with its event timeline, and the
review's repo rules.

**Export** calls `POST /reviews/{review_id}/proof-pack/export` `{format?}`, which:

1. persists a **markdown snapshot** to the `review_proof_packs` table
   (`ReviewProofPackExport {id, review_id, format, markdown, created_at}`), and
2. **ingests the verified findings into Vault memory** — each `verified` finding
   becomes a memory record (collection `findings`, kind `finding`, workspace scope,
   tagged with its severity + category, with the linked commit/test attached as
   refs), so the closed loop is searchable later (see [`./vault.md`](./vault.md)).

It then emits the `proof_pack_exported` WS event.

> This review Proof Pack is **namespaced** (`ReviewProofPack`,
> `review_proof_packs`) and is *distinct* from the generic, work-item
> [Proof Packs](./proof-packs.md) feature (`proof_packs`/`proof_artifacts`). They
> were built in parallel; the AI-review gate in Proof Packs writes a `review`
> artifact into a *generic* pack, while this snapshot is the review-finding-centric
> evidence bundle.

---

## 8. API & WebSocket reference

Authoritative: `docs/contracts/api.md` ("Review findings workflow") +
`docs/contracts/ws.md`. Findings reads are **`Git` viewer**, finding writes are
**`Git` editor**, and the repo-rule routes are **`Context`** viewer/editor — there
is **no separate `Feature::ReviewFindings`**.

### Endpoints

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `GET /findings/{id}` | ws viewer (Git) | — | `FindingDetail {finding, events}` |
| `POST /findings/{id}/accept` | ws editor (Git) | — | `Finding` (`open → accepted`) |
| `POST /findings/{id}/waive` | ws editor (Git) | `{reason?}` | `Finding` (`→ waived`) |
| `POST /findings/{id}/false-positive` | ws editor (Git) | `{reason?}` | `Finding` (`→ false_positive`) |
| `POST /findings/{id}/require-approval` | ws editor (Git) | — | `Finding` (sets the gate; status unchanged) |
| `POST /findings/{id}/approve` | ws editor (Git) | `{decision, note?}` | `Finding` — `approve` clears gate (`open → accepted`); `reject → false_positive` |
| `POST /findings/{id}/jira` | ws editor (Git) | `{project_key, issue_type?, account_id?}` | `Finding` (stores `jira_key`/`jira_url`; **400 invalid** if no Jira account) |
| `POST /findings/{id}/repo-rule` | ws editor (Context) | `{title?, body?, glob?}` | `RepoRule` (links `repo_rule_id`) |
| `POST /findings/{id}/fix` | ws editor (Git) | — | `FindingActionResp {finding, session_id?}` |
| `POST /findings/{id}/verify` | ws editor (Git) | — | `FindingActionResp {finding, session_id?}` |
| `POST /findings/{id}/regression-test` | ws editor (Git) | — | `FindingActionResp {finding, session_id?}` |
| `GET /reviews/{review_id}/findings` | ws viewer (Git) | — | `Finding[]` (widened from `ReviewFindingRow[]`) |
| `POST /reviews/{review_id}/findings/{fingerprint}/state` | ws editor (Git) | `{state, fix_session_id?}` | updated finding — **deprecated** legacy lifecycle transition |
| `GET /reviews/{review_id}/merge-readiness` | ws viewer (Git) | — | `MergeReadiness` |
| `GET /workspaces/{ws}/repo-rules` | ws viewer (Context) | — | `RepoRule[]` |
| `POST /repo-rules/{id}/toggle` | ws editor (Context) | `{enabled}` | `RepoRule` |
| `DELETE /repo-rules/{id}` | ws editor (Context) | — | 204 |
| `GET /reviews/{review_id}/proof-pack` | ws viewer (Git) | — | `ReviewProofPack` (live-assembled) |
| `POST /reviews/{review_id}/proof-pack/export` | ws editor (Git) | `{format?}` | `ReviewProofPackExport` (+ memory ingest) |

All routes are under `/api/v1`.

### WebSocket events

Workspace-scoped; emitted from `routes/{findings,proof_pack}.rs` and routed
through the `findingBus` in `ui/src/lib/events.svelte.ts`. The board re-fetches
`GET /reviews/{id}/findings` when the event's `review_id` matches the open board.

- **`finding_updated`** — a finding's workflow `status` (or a tracked field)
  changed; carries `finding_id`, `review_id`, and the new `status` (snake_case).
- **`finding_action_started`** — an agent-backed action (fix / verify /
  regression-test) spawned a session; carries `finding_id` and `session_id` (null
  for non-agent actions).
- **`proof_pack_exported`** — a review's Proof Pack snapshot was persisted (and
  verified findings ingested into memory); carries `review_id` + `proof_pack_id`.

---

## 9. Data model & the FK gotcha

`crates/otto-state/migrations/0082_review_findings_workflow.sql` builds the
workflow on top of the existing fingerprinted-findings store (migrations
`0049`/`0054`), and **fixes a latent bug** along the way:

> Migrations `0049`/`0054` declared `review_findings` with foreign keys to
> **`reviews(id)`** and **`git_repos(id)`** — but **those tables don't exist**; the
> real names are **`pr_reviews`** and **`repos`**. With SQLite FK enforcement on
> (the sqlx default), *every* insert into `review_findings` would fail with "no such
> table", and even an `ALTER`/`RENAME` re-validates the bad FKs and errors. The bug
> was dormant only because the upsert had no call sites until this feature.

So the migration **rebuilds** the table: it builds `review_findings_v2` with FKs to
the **real** tables (`repos`, `pr_reviews`) plus all the workflow columns inline,
copies any rows over, `DROP`s the old table, then `RENAME`s the new one into place
(building a fresh table avoids re-validating the old bad FKs). It then recreates the
indexes and rebuilds the `review_merge_readiness` view on the new `status`
vocabulary.

Tables created/changed:

- **`review_findings`** (rebuilt) — identity (`id`, `workspace_id REFERENCES
  workspaces`, `repo_id REFERENCES repos`, `pr_number`, `fingerprint`), the two
  axes (`status`, `state`), the 11 workflow fields + gates, and
  `first_seen_review_id`/`last_seen_review_id`/`review_id` (all `REFERENCES
  pr_reviews(id) ON DELETE CASCADE`).
- **`finding_events`** — the loop-closing audit trail (`finding_id REFERENCES
  review_findings(id) ON DELETE CASCADE`, `kind`, `actor`, `from_status`,
  `to_status`, `detail_json`, `created_at`).
- **`repo_rules`** — durable lessons (`workspace_id REFERENCES workspaces`, `title`,
  `body`, `category`, `severity`, `glob`, `source_finding_id`, `enabled`).
- **`review_proof_packs`** — persisted markdown snapshots (`review_id`,
  `workspace_id`, `format`, `content`, `summary_json`, `created_by`, `created_at`).
- **`review_merge_readiness`** (view) — unresolved/blocker/resolved counts per
  `(repo_id, pr_number)`.

---

## 10. Capabilities & limitations

**Can:**

- Persist every reviewed finding as a tracked record with a stable fingerprint, a
  6-state human disposition, and an immutable event timeline — independent of the
  engine's re-detection lifecycle.
- Triage from the board with seven one-click actions + an overflow menu, each gated
  by the legal transition table.
- Spawn **agent-backed** fix / verify / regression-test sessions you can open and
  watch; link the resulting commit (`linked_commit`) and guard test (`linked_test`).
- File a finding to **Jira**, gate it behind **human approval**, and generalize it
  into a durable **repo rule** that the Context Engine injects into future sessions.
- Assemble a per-review evidence bundle and export it as markdown, ingesting the
  **verified** findings into Vault memory for later recall.

**Limitations / honest caveats:**

- **The verification leg is a re-review.** `fixed → verified` is confirmed by the
  finding being *absent* from a subsequent run (`resolve_absent`) or by the verify
  agent passing — there is no single fully-automatic close from a code change to
  `verified`.
- **No `Feature::ReviewFindings`.** Permissions piggyback on `Git` (findings) and
  `Context` (repo rules); there is no dedicated feature flag to scope this
  independently.
- **Two distinct "proof packs."** This workflow's namespaced `ReviewProofPack` is
  separate from the generic [Proof Packs](./proof-packs.md) feature — don't conflate
  them.
- **Jira filing needs a configured account** (400 otherwise), and acts through the
  caller's owned issue account (see [`./code-review.md`](./code-review.md) §10).
- **Findings are LLM output.** A finding can be a false positive; the `false_positive`
  disposition and the `requires_human_approval` gate exist precisely because severity
  and validity are the reviewing agent's judgment, not ground truth.

---

## 11. Security & permissions

- **Workspace RBAC.** Reading findings = **`Git` viewer**; all finding actions =
  **`Git` editor**; repo-rule routes = **`Context`** viewer/editor. See
  [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).
- **Disjoint writers.** The human `status` is written only by the action endpoints;
  the engine `state` only by the review engine. Neither path can silently overwrite
  the other, so triage and re-detection can't race to corrupt each other.
- **Audited.** Every transition appends an immutable `finding_events` row (actor +
  from/to status), and `approve`/`reject` records `approved_by`/`approved_at`/
  `approval_decision`.
- **Credential ownership.** Jira filing reuses the AI-review credential-ownership
  rule — the token used belongs to the caller (or root), never another member's
  account.
- **Agent sessions** spawned by fix/verify/regression-test are ordinary Otto agent
  sessions; the same trust/prompt-guard posture applies.

---

## 12. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| "Ask agent to fix" is disabled | Only legal from `open`/`accepted`. **Accept** (overflow) an `open` finding first, or it's already `fixed`/`verified`. |
| "Verify resolved" disabled | Only from `accepted`/`fixed`/`verified`. Fix or accept it first. |
| A closed finding shows a `regressed` chip | It reappeared in a later review run after being `resolved`/`declined`; the engine flipped `state` to `regressed`. Re-triage it. |
| `seen ×N` keeps climbing but status won't change | Re-detection bumps `occurrence_count` and **never resets your `status`** by design — your triage is sticky across re-reviews. |
| "Convert to Jira" → 400 `invalid` | No Jira account is configured. Connect one under Settings → Issue Accounts ([`./jira-confluence.md`](./jira-confluence.md)). |
| An agent action didn't open a session | `session_id` is only returned for agent-backed actions (fix / verify / regression-test); the others are pure status moves. |
| Board not updating live | It refetches off `finding_updated`/`finding_action_started`/`proof_pack_exported` via the `findingBus`; if the socket dropped, it re-fetches on the next match. |
| Export ran but nothing in Vault | Only **verified** findings are ingested into memory. Verify the relevant findings first, then re-export. |
| Approve/Reject not shown | They only appear once the **Require human approval** gate is set and the finding isn't already decided. |

---

## 13. Related docs

- [`./code-review.md`](./code-review.md) — the multi-agent AI review that *produces*
  these findings (fan-out, lenses, merge-readiness, handoff).
- [`./proof-packs.md`](./proof-packs.md) — the generic work-item evidence layer
  (distinct from this workflow's namespaced review proof pack).
- [`./vault.md`](./vault.md) — the knowledge store that verified findings are
  ingested into on export.
- [`./jira-confluence.md`](./jira-confluence.md) — connecting the account used by
  "Convert to Jira".
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — the `Git` / `Context` feature
  roles enforced here.
- `docs/contracts/api.md` ("Review findings workflow"), `docs/contracts/ws.md` —
  authoritative endpoint + WS contract; design at
  `docs/superpowers/specs/2026-06-26-review-findings-workflow-design.md`.
</content>
