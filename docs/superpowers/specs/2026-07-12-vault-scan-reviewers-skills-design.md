# Vault scan iterative reviewers and skill depth

Date: 2026-07-12 · Branch: `feat/vault-scan-reviewers`

## Problem

Vault docs runs can fan writers out and consolidate their drafts, but the
consolidator is an author, not an independent reviewer. A run can therefore
finish after every writer misses the same route, request/response model,
datastore access path, worker, or side effect. The caller cannot assign
independent review agents, give each one a focus, or watch authors respond to
review feedback.

The two bundled authoring skills are also too shallow as packages:

- `okf-authoring` and `vault-repo-docs` have no required `name` metadata,
  focused references, deterministic scripts, realistic examples, or evals.
- Full-repo prompts ask for breadth but do not leave a deterministic coverage
  ledger proving that discovered routes, payloads, stores, messages, workers,
  and flows were accounted for.
- Multi-file resources would not reliably reach every provider. Non-Claude
  providers currently receive only concatenated `SKILL.md` bodies. Claude's
  out-of-tree stager resolves installed/global skills but not compiled-in
  bundled packages, while its prompt claims the skills were staged.

## Goals

1. Add an optional, visible, multi-reviewer loop to every docs run.
2. Let the caller configure each reviewer independently by provider, model,
   review-method skill, and free-text focus.
3. Review and revise for up to a user-selected number of rounds (default 3),
   stopping early only when every reviewer is clean in the same round.
4. Make full repo scans auditable through a source-backed inventory and
   coverage ledger, with explicit API bodies and deep datastore impact paths.
5. Turn the OKF/repo-doc skills into complete, progressively disclosed skill
   packages and add one generic plus four focused reviewer skills.
6. Deliver each complete skill tree to every provider and verify the behavior
   with scripts, eval fixtures, Rust tests, and focused UI tests.
7. Make non-Markdown documentation deliverables such as OpenAPI YAML writable
   through the same guarded Vault API/MCP boundary as notes.

## Non-goals

- Reviewers do not directly edit vault notes. The final author owns revisions.
- This does not add a generic PR/code-review engine or reuse review findings as
  product workflow findings.
- The inventory scripts do not claim semantic truth from regex matches. They
  produce candidates with evidence; agents trace and classify them.
- A failed review does not delete or roll back documentation already written.

## Run lifecycle

The existing authoring stages remain:

1. One to four writers inspect the repository and write finals (single writer)
   or isolated drafts (multi-writer).
2. A multi-writer summarizer consolidates drafts into the final target. This
   summarizer is the final author for the rest of the run; for single-writer
   runs, that writer is the final author.

When review is configured, the run continues with review rounds:

1. Every configured reviewer independently reads the original request, the
   current final bundle, the coverage ledger, the repository, and its selected
   review-method skill.
2. Each reviewer writes a structured JSON findings artifact. An empty array is
   clean. Missing or malformed output is an error, never a clean verdict.
3. When every reviewer is clean in the same round, the run stops immediately.
4. Otherwise, the server sends the combined findings to the same final-author
   session. That author repairs the final notes, refreshes indexes/coverage,
   validates OKF, and reports changed paths.
5. Reviewers inspect the revised result. The loop repeats until clean or the
   configured maximum (1–10, default 3) is reached.

The UI renders this sequence explicitly: authors, round N reviewers and their
findings, round N revision and changed notes, then the next round. Every
session remains openable inline.

## Request and persisted DTOs

`RunReq` gains a backward-compatible optional block:

```json
{
  "review": {
    "max_iterations": 3,
    "reviewers": [
      {
        "provider": "claude",
        "model": "sonnet",
        "skill": "vault-api-review",
        "focus": "Prioritize externally consumed contracts"
      }
    ]
  }
}
```

- `reviewers`: required and 1–4 when `review` is present.
- `max_iterations`: optional, defaults to 3, accepted range 1–10.
- `skill`: one of the bundled Vault reviewer methods; defaults to
  `vault-docs-review`.
- `focus`: optional additional instructions; it narrows the method but never
  removes its mandatory checks.

`VaultDocsRun` gains a defaulted review object so old persisted payloads still
deserialize:

- `state`: `skipped | pending | reviewing | revising | clean | exhausted |
  error | cancelled | interrupted`.
- `max_iterations`, `current_iteration`, `outcome`.
- `reviewers`: resolved configurations used by the run.
- `rounds[]`: reviewer states/findings and the revision state/session/changed
  paths for every completed or active round.

Run states add `reviewing`, `revising`, and terminal
`done_with_findings`. `done` means review was skipped or every reviewer was
clean. `done_with_findings` means the round cap was reached with unresolved
findings. `error` means a reviewer/revision did not produce valid output or a
required session failed. Existing `cancelled`/`interrupted` semantics extend
to active reviewer and revision slots.

The existing `vault_docs_runs.payload` JSON persists the expanded structure;
no new table or migration is needed. `list_unfinished()` must include the two
new active states.

## Findings contract

Each reviewer writes a JSON array. Every item has:

```json
{
  "severity": "blocking",
  "category": "api",
  "summary": "POST /widgets omits its 422 response body",
  "evidence": [
    {"repo_path": "src/routes/widgets.rs", "line": 91},
    {"doc_path": "widgets/api.md", "section": "POST /widgets"}
  ],
  "missed_item": "ValidationError response schema and example",
  "required_fix": "Document the 422 schema/example and add it to OpenAPI"
}
```

Review prompts require proof against real code, reject speculative findings,
and ask reviewers to reconcile deterministic audit output rather than blindly
repeat it. The server stores the parsed findings in the durable run payload.

## Reviewer skill roster

Writers use `okf-authoring` plus `vault-repo-docs`. Reviewer rows choose one
method, with free-text focus layered on top:

| Skill | Responsibility |
| --- | --- |
| `vault-docs-review` | Default all-lens completeness review: original request, coverage, API, data, flows, operations, auth, side effects, evidence, OKF, links, contradictions, and reader usability. |
| `vault-api-review` | HTTP/RPC contracts: every operation, auth, parameters, request body, success/error response bodies, examples, validation, side effects, and OpenAPI parity. |
| `vault-data-review` | SQL/NoSQL/cache assets: fields and types, grain, queries, reads/writes, joins, indexes, TTL, transactions, field-level impact paths, and diagrams. |
| `vault-runtime-review` | API/message/worker/reconciliation/startup flows: dependencies, retries, idempotency, failure handling, schedules, and external side effects. |
| `vault-evidence-review` | Coverage reconciliation, source citations, unsupported claims, uncertainty, link/index integrity, OKF quality, examples, and diagram validity. |

Reviewer sessions are read-only. Their session MCP capability set excludes
vault write/rename/delete tools; their only mutation is the server-specified
temporary JSON artifact. The final author is the sole vault writer during
review iterations.

## Skill package upgrades

### `okf-authoring`

Keep `SKILL.md` concise: trigger metadata, core produce/maintain/consume
workflow, routing table, and completion contract. Add:

- `references/spec-v0.1.md` — exact format/conformance rules and provenance.
- `references/concept-patterns.md` — service, endpoint, flow, datastore,
  runbook, ADR, metric, and reference patterns.
- `references/linking-indexes-logs.md` — links, reserved files, neighborhood
  maintenance, deprecation, and conversion rules.
- `references/quality-gates.md` — content depth, citations, examples,
  diagrams, uncertainty, and augment-don't-rewrite gates.
- `scripts/validate_okf.py` — stdlib offline conformance check.
- `scripts/audit_bundle.py` — deterministic structure/depth audit with JSON or
  text output; it flags missing API/data sections but never invents facts.
- `examples/` — complete API endpoint, data asset, and maintain-before/after.
- `evals/evals.json` plus fixtures.

### `vault-repo-docs`

Keep `SKILL.md` focused on modes, source survey, coverage ledger, write order,
and finish gates. Add:

- `references/full-scan-method.md`.
- `references/api-documentation.md`.
- `references/datastore-documentation.md`.
- `references/flows-messaging-workers.md`.
- `references/evidence-and-citations.md`.
- `scripts/inventory_repo.py` — stdlib candidate inventory with `file:line`
  evidence for routes/types/queries/stores/messages/workers/config/clients.
- `scripts/audit_repo_bundle.py` — reconcile inventory candidates with the
  coverage ledger and check API/OpenAPI/data/flow depth.
- `examples/full-scan-manifest.json`, API-flow bundle, datastore-impact
  bundle, and focused/incremental examples.
- `evals/evals.json` plus fixtures.

Every new reviewer skill follows the same shape: compact method, focused
reference/checklist, one excellent findings example, deterministic audit
integration where relevant, and runnable eval fixtures.

## Full-scan coverage contract

`inventory_repo.py` runs before writing and produces source-backed candidates
for routes and request/response types; SQL/migrations/tables/columns,
collections and keys; messages and payloads; jobs/schedules/reconciliation;
startup/shutdown; external clients; configuration; authorization; and side
effects.

The writer traces candidates and writes a visible `coverage.md` mapping every
candidate to `documented`, `irrelevant`, `generated`, or `uncertain`, with a
link to the final note or a reason. A full scan cannot silently omit a
candidate. The scripts are heuristic discovery aids; source reading and
reviewer verification remain authoritative.

API documentation is complete only when every operation includes auth,
parameters, request schema/body and realistic example, success response
schema/body and example, material error responses, validation, side effects,
flow link, and source citation. OpenAPI must match.

Datastore documentation is complete only when every touched table,
collection, or key pattern includes its grain/purpose, full known fields and
types, reads and writes, actual query/access patterns, indexes/TTL,
transaction/consistency behavior, joins/relationships, field-level impact
paths, examples, diagrams where useful, and citations. Unknown details are
marked unknown instead of guessed.

## Guarded text artifacts

The current `otto_vault_write` path accepts only `.md`, while the shipped full
scan template requires `api-openapi.yaml`. Direct filesystem writes would
bypass Vault path checks, optimistic concurrency, rescanning, and the stated
agent contract. Add a dedicated guarded text-artifact write instead of
weakening note semantics:

- `PUT /workspaces/{ws}/vault/vaults/{id}/file` and session MCP
  `otto_vault_write_file` accept `{path, content, if_hash?}`.
- Accepted extensions are documentation text assets: `.yaml`, `.yml`,
  `.json`, `.d2`, `.mmd`, `.txt`, and `.csv`; Markdown remains on the note
  endpoint so it always receives note parsing/index behavior.
- The same traversal, hidden-segment, symlink-escape, optimistic concurrency,
  and 4 MiB limits apply. Parent folders are created. The response is
  `{path,size,hash}` and the Vault rescans before returning.
- The tool is Editor-gated and classified as a Vault mutation. Reviewer
  sessions do not receive it.
- Binary assets remain read/import-only; this endpoint never accepts encoded
  binary content.

The full/API scan templates use this tool for `api-openapi.yaml`, and the
OpenAPI viewer continues to read it through the existing asset endpoint.

## Complete skill delivery

Replace the Claude-specific staging assumption with a provider-neutral
materialized bundle:

1. Resolve each skill from the Library, then global runtime locations, then
   the compiled-in `otto-skills` tree.
2. Copy the entire package into a per-run bundle, preserving references,
   scripts, examples, assets, and eval metadata.
3. Keep the `.claude/skills/<name>` view for Claude first-class invocation.
4. Also expose a provider-neutral `skills/<name>` view. Non-Claude prompts get
   the exact package path and file manifest, and must read `SKILL.md` plus only
   the task-relevant resources. Do not inline large packages into prompts.
5. Scripts remain executable; package paths are read-only inputs to reviewers
   and writers.

The UI's skill viewer shows the full package file list and can open bundled
resources, not only `SKILL.md`.

## UI

The Docs agent form adds an optional **Review outcomes** section:

- Toggle off by default for backward compatibility.
- Enabling it creates one generic `vault-docs-review` row.
- Each row has method, provider, optional model, optional additional focus,
  and remove; up to four rows.
- Maximum review rounds defaults to 3 and explains early clean exit.

The live/history detail renders authors first, then every review round. Each
reviewer row shows method/focus, provider/model, state, finding count, Open,
View findings, and Retry. Revision rows show the final-author session, state,
changed paths, Open, and Retry. The run header shows `reviewing · round N/M`,
`revising · round N/M`, `done`, or `done with findings`.

## Failure, retry, cancel, and recovery

- Reviewers in a round fan out concurrently and are isolated while running,
  but the round cannot be clean unless every configured reviewer returns a
  valid result.
- A failed/malformed reviewer makes the run `error` after its retry window;
  docs and prior findings remain available.
- A failed revision makes the run `error`; docs and findings remain available.
- Dedicated reviewer/revision retry routes mirror existing writer/summarizer
  retry semantics and caps.
- Cancel terminates active writer, summarizer, reviewer, and revision sessions
  and marks their non-terminal rows cancelled.
- Startup recovery flips active review/revision state and slots to
  `interrupted`; it does not delete final docs or review artifacts.
- Reaching the maximum iterations with findings produces
  `done_with_findings`, not a false success or destructive rollback.

## Contracts and documentation

Update in lockstep:

- `docs/contracts/api.md` request/response bodies, states, retry routes, and
  durable semantics, plus the guarded text-artifact write contract.
- `ui/src/lib/api/types.ts` and `ui/src/lib/api/vault.ts`.
- `docs/features/vault.md` form, lifecycle, skill roster, full-scan contract,
  iteration outcomes, and troubleshooting.
- Bundled skill versions and package tests.

## Verification

### Code

- Rust unit tests: request validation/defaults, DTO backward compatibility,
  state transitions, early clean exit, exhaustion, malformed findings,
  interruption, retry/cancel, prompt construction, complete bundled-resource
  materialization, and provider-specific delivery.
- Vault engine/API/MCP tests: allowed and rejected text extensions, traversal,
  symlink escape, optimistic concurrency, size limit, rescan visibility, RBAC,
  and reviewer-session tool filtering.
- State tests: `reviewing`/`revising` count as unfinished.
- Script tests: fixture repositories/bundles with missing API bodies, shallow
  DB coverage, missed workers, false citations, and clean controls.
- UI: `npm run check`, `npm run build`, and focused Vault Playwright specs for
  reviewer configuration, visible rounds, early exit, exhaustion, retry, and
  history reload. Do not run the full Playwright suite.

### Skill TDD/evals

For every upgraded/new skill:

- Capture no-skill/current-skill baseline failures before editing.
- Validate package structure and references.
- Run positive/negative activation cases.
- Run seeded omission cases and clean-bundle false-positive controls.
- Run iteration convergence: findings → author repair → clean reviewer result.
- Forward-test on fresh agents without leaking intended findings.

### Completion

Run focused Rust tests, all new script/eval suites, UI check/build, and the
named Vault E2E specs. Then run the repository deployment flow to rebuild,
sign, install, replace the running app, and verify the installed daemon/UI via
a newly added API surface rather than a stale route.
