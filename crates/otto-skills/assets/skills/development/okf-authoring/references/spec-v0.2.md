# OKF v0.2: format, provenance and compatibility

Canonical source: [GoogleCloudPlatform/open-knowledge-format SPEC.md](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md), version 0.2, checked 2026-09-20. This is a concise reference; the upstream specification defines the format. [Legacy v0.1 reference](spec-v0.1.md) remains available for existing bundles.

## Structure and permissive consumption

A bundle is a tree of UTF-8 Markdown files. A concept ID is its bundle-relative path without `.md`. Every concept has YAML frontmatter with a nonempty `type`; unknown types and extension fields are allowed. Preserve unknown metadata and existing body structure when maintaining knowledge.

`index.md` and `log.md` are reserved, not concepts. Nested indexes and all logs have no frontmatter. Only the root index may declare `okf_version: "0.2"`. Preserve an existing version declaration unless an explicit format migration is requested. Versionless and v0.1 bundles remain consumable; new optional fields do not require rewriting the bundle.

Missing optional metadata, unknown fields/types, broken links and missing indexes do not make a bundle nonconformant. Otto's authoring quality gate can request repairs without pretending these are upstream conformance errors. During consumption, surface uncertainty and continue.

## Metadata

| Family | Shape and interpretation |
|---|---|
| `generated` | `{by: actor, at: offset-bearing ISO datetime}` records who produced the current content and its last meaningful change. `by` is required within this optional family; `at` is recommended. |
| `sources` | List of mappings with `resource` (URI, bundle path or population/scope descriptor). Optional stable `id`, `title`, `author`, `usage_count`, `last_modified`; `usage_window: {from, to}` frames usage counts at concept or source level. |
| `verified` | List of `{by, at}` events, or one bare mapping treated as a one-element list. Distinct from generation; record actual checks only. |
| `status` | `draft`, `stable` or `deprecated`; absent means stable, which does not imply verified. |
| `stale_after` | Absolute ISO datetime with explicit UTC offset; stale when `now >= stale_after`. Never a relative TTL. |

Actors use `<producer>/<version>`, `human:<id>` or `process:<id>`. Use the actual author identity/version when known; do not invent a model, person or verification event. Source signals are facts, not a stored credibility score. Trust is advisory: no verification is unverified, only nonhuman events are machine-confirmed, and a human event indicates human-reviewed. These declarations are not authentication or access control and can predate later content edits.

```yaml
---
type: Reference
title: Example service contract
description: Describes the current service interface.
generated: {by: otto/example-v1, at: 2026-09-20T09:00:00Z}
sources:
  - id: contract
    resource: https://example.test/api/openapi.yaml
    title: Service OpenAPI contract
status: draft
---
```

Attribute claims through Markdown footnotes keyed by `sources[].id`: `The service accepts JSON.[^contract]`, with `[^contract]: Service OpenAPI contract`. A footnote label joins to metadata; reordering the source list must not change attribution. Prefer source locations that another reader can inspect. A scope descriptor is permitted but is not a clickable artifact.

## Compatibility and maintenance

For v0.2 authoring, use `generated.at` instead of legacy `timestamp`, and `sources` plus keyed footnotes instead of a `# Citations` list. Consumers may read `timestamp` only when `generated` is absent, and may retain v0.1 Citations. Preserve legacy conventions on focused maintenance unless migration is requested. Never convert an old timestamp into a claim of verification.

A meaningful source change can invalidate facts even if the note was edited recently. Inspect the source, reconcile affected claims and links, update generation metadata, and retain verification history without presenting stale checks as current. Do not generate verification from a successful syntax check or copy a human actor from a prior author.

## Attested Computation

`type: Attested Computation` declares `runtime`, `parameters`, an inline fenced `# Computation` or a `computation` path, plus `executor.resource`/`executor.receipt` and `attester.resource`. Agents supply declared parameter values; they do not rewrite the sanctioned computation. The attester is deterministic code. Receipts and per-run verdicts are runtime artifacts, not bundle metadata. `verified` concerns the definition, not an individual execution. Reading an executor/attester reference does not authorize running arbitrary code.

## Offline checks and limits

The read-only Python validator keeps E1–E3 for structural errors and W1–W6 for advisory metadata/link findings. W3 accepts `generated.at` or legacy `timestamp` when generated is absent. W6 warns on malformed generated/verified/sources/status/stale_after shapes; it never rejects optional families as conformance errors. Unknown fields/types are preserved and not executed.

The stdlib reader handles the documented block and single-line flow mapping/list forms. It is not a complete YAML parser: anchors, tags and multiline flow structures need Otto's full runtime YAML validator. It does not authenticate verification, fetch sources, compute staleness at validation time or execute attestations. Normal mode returns nonzero only for conformance errors; `--strict` additionally fails the author's local quality gate on warnings.
