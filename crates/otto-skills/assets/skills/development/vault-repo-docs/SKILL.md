---
name: vault-repo-docs
description: Document a repository into the Otto Vault as an OKF knowledge bundle — read the code, author linked concept docs (services, endpoints, datasets, decisions, runbooks, metrics), validate conformance, and verify claims against the source. This REPLACES embedding-based repo indexing; "index a repo into the vault" now means writing durable, diffable OKF docs. Use when asked to index/scan/document a repo, map a feature's dependencies, or build the team's repo knowledge base.
category: development
version: 3
metadata:
  version: "3.0.0"
---

# Vault Repository Documentation

Turn a repository into a durable, linked, source-backed OKF bundle. Load
`okf-authoring` for the format contract. A full scan means every discovered
candidate is reconciled in `coverage.md`; it never means a representative
sample.

## Required workflow

1. Read [full-scan-method.md](references/full-scan-method.md). Establish repo
   path, commit, scan mode, bundle path, and prior coverage ledger.
2. Run `scripts/inventory_repo.py REPO --format json > manifest.json`. Treat
   candidates as leads, not facts. Supplement the inventory by reading
   entrypoints, route registration, DTOs, migrations, query builders, broker
   wiring, workers, configuration, and tests.
3. Write `coverage.md` before concept docs. Give every candidate exactly one
   status: `documented`, `irrelevant`, `generated`, or `uncertain`. Include its
   evidence, destination document, and a concrete reason.
4. Document breadth first, then depth. Use the completion contracts in:
   - [api-documentation.md](references/api-documentation.md)
   - [datastore-documentation.md](references/datastore-documentation.md)
   - [flows-messaging-workers.md](references/flows-messaging-workers.md)
   - [evidence-and-citations.md](references/evidence-and-citations.md)
5. Use `otto_vault_write` for Markdown. Use `otto_vault_write_file` for
   approved text artifacts such as `api-openapi.yaml`; never hide YAML in a
   Markdown note merely because the writer lacks the correct tool.
6. Run the OKF validator and `scripts/audit_repo_bundle.py BUNDLE --manifest
   manifest.json`. Resolve every error and every unexplained coverage gap.
7. Perform a second source pass: re-check at least the route registration plus
   DTO for each API, every DB access path cited, and every trigger/side effect
   for runtime flows. Update the ledger and scan marker only after this pass.

## Completion contract

A full scan is complete only when:

- `index.md`, `overview.md`, `coverage.md`, and `log.md` exist and link to all
  generated concepts;
- the ledger accounts for every manifest candidate, with no duplicate IDs and
  no `uncertain` row silently presented as complete;
- APIs contain real request and response bodies, validation, errors, auth,
  examples, side effects, flow links, and matching OpenAPI operations;
- data assets contain schema, grain, indexes/TTL, actual read and write paths,
  consistency boundaries, field-level impact, examples, and citations;
- messaging, workers, startup/shutdown, and reconciliation flows are inventoried;
- every load-bearing claim cites `relative/path:line` and has been rechecked;
- validators and audits are clean. If evidence is unavailable, say what remains
  uncertain and finish as partial rather than fabricating completeness.

## Modes

- **Full:** inventory the current tree and reconcile all candidates.
- **Focused:** inventory the entire repo, document the requested lens deeply,
  and mark out-of-scope rows `irrelevant` with the focus as the reason.
- **Incremental:** diff the recorded commit to HEAD, inventory changed files,
  update only affected concepts, and reconcile changed/new/removed candidates.

See [full-scan-manifest.json](examples/full-scan-manifest.json),
[api-flow-bundle](examples/api-flow-bundle), and
[datastore-impact-bundle](examples/datastore-impact-bundle) for compact output
examples. Scripts are conservative accelerators; source reading remains the
authority.

Do not use this skill for a code review, implementation change, or one-off
question about a repository that does not request durable Vault documentation.
