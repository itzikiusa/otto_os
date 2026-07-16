---
name: okf-authoring
description: Use when creating, maintaining, consuming, converting, or validating Open Knowledge Format bundles, especially Otto Vault documentation for repositories, services, APIs, data assets, decisions, runbooks, metrics, and references.
metadata:
  version: "3.0.0"
---

# OKF authoring

Build durable, source-backed knowledge as Markdown concepts with YAML frontmatter. Treat conformance as the floor and evidence-backed completeness as the finish line.

Do not use this skill for ordinary Markdown writing, generic code review, or source changes when no OKF bundle is involved.

## Choose a mode

| Mode | Use it to | Required result |
|---|---|---|
| `produce` | Create knowledge from source evidence | New concepts, links, indexes, log entry |
| `maintain` | Reconcile knowledge after a source change | Preserved structure, updated facts, neighborhood, log |
| `consume` | Answer from an existing bundle | Read root `index.md`, follow only relevant concepts; switch to `maintain` for durable discoveries |

## Route the work

Read only the resources needed for the task:

- Read [references/spec-v0.1.md](references/spec-v0.1.md) before validating, converting, or resolving frontmatter/reserved-file questions.
- Read [references/concept-patterns.md](references/concept-patterns.md) before writing a service, endpoint, flow, datastore, runbook, ADR, metric, or reference.
- Read [references/linking-indexes-logs.md](references/linking-indexes-logs.md) when adding, moving, renaming, deprecating, or converting concepts.
- Read [references/quality-gates.md](references/quality-gates.md) before declaring produce or maintain work complete.
- Read [examples/complete-api-endpoint.md](examples/complete-api-endpoint.md) for an endpoint contract, [examples/complete-data-asset.md](examples/complete-data-asset.md) for datastore depth, or [examples/maintain-before-after.md](examples/maintain-before-after.md) for structure-preserving edits.

## Workflow

1. Inspect the source and the existing bundle before writing. Record unknowns; never invent URLs, fields, joins, enum values, behavior, or citations.
2. Choose a plural domain directory such as `services/`, `endpoints/`, `flows/`, `datasets/`, `runbooks/`, `decisions/`, `metrics/`, or `references/`.
3. Write one concept per nameable topic. Use path-minus-`.md` as its ID, a non-empty `type`, a one-sentence `description`, and a canonical `resource` only for a real asset.
4. Link related concepts at their first useful mention. Update the local `index.md` and append a newest-first dated `log.md` entry for produce/maintain work.
5. Run the hard static gate, then the quality audit, without mutating input.

```bash
python3 scripts/validate_okf.py ROOT --strict --format text
python3 scripts/audit_bundle.py ROOT --format text
```

Use `--format json` for automation. Fix every conformance error. Resolve quality findings with cited facts or explicitly mark the fact unknown.

## Hard static enforcement

For every `produce` or `maintain` run:

1. If `ROOT/scripts/check_vault.py` exists, run `python3 ROOT/scripts/check_vault.py ROOT` and require exit code 0.
2. Otherwise run `python3 scripts/validate_okf.py ROOT --strict` and require exit code 0.
3. Run the quality audit only after the static gate passes.

The static gate must have zero errors and zero warnings. A link must name a concrete Vault-relative file such as `endpoints/index.md`; a directory target such as `endpoints/` is a failure. A `file://` target that duplicates a Vault note is a failure because it hides the relationship from Otto; a source-evidence URI outside the bundle remains an external citation.

Do not claim produce or maintain work complete while either static command fails. Do not downgrade, hide, or ignore a deterministic finding; repair the target or report the work as incomplete.

## Completion contract

Report the mode, changed paths, source evidence, unresolved unknowns, static-gate result, and audit result. Produce/maintain work is complete only when the hard static gate has zero findings, the quality audit is clean, indexes and log reflect the changes, and every material claim is supported or explicitly uncertain.
