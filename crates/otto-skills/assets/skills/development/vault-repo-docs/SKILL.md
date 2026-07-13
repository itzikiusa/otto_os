---
name: vault-repo-docs
description: Use when indexing, scanning, or documenting a repository into the Otto Vault as a durable OKF bundle, including full, focused, and incremental scans or feature dependency maps; not for code review or one-off repository questions.
category: development
version: 5
metadata:
  version: "5.0.0"
---

# Vault Repository Documentation

Turn a repository into a durable, linked, source-backed OKF bundle. Load
`okf-authoring` for the format contract. A full scan means every discovered
candidate is reconciled in `coverage.md`; it never means a representative
sample.

## Required workflow

1. Read [full-scan-method.md](references/full-scan-method.md). Establish repo
   path, commit, scan mode, bundle path, and prior coverage ledger.
2. For a full or focused scan, run `scripts/inventory_repo.py REPO --format
   json > manifest.json`. For an incremental scan, use `--changed-since COMMIT`
   and repeat `--include-file PATH` for affected registration/contract files.
   An invalid or diverged baseline must produce `mode: full-fallback`, never a
   guessed incremental scope. Preserve `manifest.json` in the bundle. Review
   `scanned_files`, `exclusions`, counts, and suspicious zero-candidate output.
   Treat candidates as leads, not facts; inspect entrypoints, registration,
   DTOs, migrations, queries, broker wiring, workers, configuration, and tests.
3. Write `coverage.md` before concept docs. Give every candidate exactly one
   status: `documented`, `irrelevant`, `generated`, or `uncertain`. Include its
   evidence, destination document, and a concrete reason. Add manually found
   candidates to both the manifest and ledger; an unknown ledger row is an
   audit error.
4. Document breadth first, then depth. Use the completion contracts in:
   - [api-documentation.md](references/api-documentation.md)
   - [datastore-documentation.md](references/datastore-documentation.md)
   - [flows-messaging-workers.md](references/flows-messaging-workers.md)
   - [evidence-and-citations.md](references/evidence-and-citations.md)
   - [cross-repo-dependencies.md](references/cross-repo-dependencies.md)
   Keep output consumable with linked concepts, dense tables, diagrams, and
   examples; do not restate the repository line by line.
5. Resolve internal/platform dependencies to local checkouts and DEEP-DIVE
   into them where a flow crosses the boundary: document what the called code
   actually does for this flow (cited as `<dep-repo>:path:line`), link the
   dependency's vault bundle at the mention — forward-linking
   `../<dep-repo>/index.md` even when that repo is not scanned yet — and write
   `dependencies.md`. See
   [cross-repo-dependencies.md](references/cross-repo-dependencies.md).
6. Use `otto_vault_write` for Markdown. Use `otto_vault_write_file` for
   approved text artifacts such as `api-openapi.yaml`; never hide YAML in a
   Markdown note merely because the writer lacks the correct tool.
7. Run the OKF validator and `scripts/audit_repo_bundle.py BUNDLE --manifest
   BUNDLE/manifest.json`. The audit resolves document links, checks kind-specific
   depth, and reconciles API operations with OpenAPI. Resolve every finding; an
   uncertain row deliberately fails the completion gate and means partial.
   Forward links into not-yet-scanned dependency bundles are the one accepted
   class of unresolved link.
8. Perform a second source pass: re-check at least the route registration plus
   DTO for each API, every DB access path cited, and every trigger/side effect
   for runtime flows. Update the ledger and scan marker only after this pass.

## Completion contract

A full scan is complete only when:

- `index.md`, `overview.md`, `coverage.md`, and `log.md` exist and link to all
  generated concepts;
- the ledger accounts for every manifest candidate, with no duplicate IDs,
  missing target documents, or `uncertain` rows presented as complete;
- APIs contain real request and response bodies, validation, errors, auth,
  examples, side effects, flow links, and matching OpenAPI operations;
- data assets contain schema, grain, indexes/TTL, actual read and write paths,
  consistency boundaries, field-level impact, examples, and citations;
- messaging, workers, startup/shutdown, and reconciliation flows are inventoried;
- every flow note fills the required skeleton in
  [flows-messaging-workers.md](references/flows-messaging-workers.md): numbered
  steps naming each store as engine + table/collection, request/response
  examples for HTTP-triggered flows, and a diagram whose store nodes carry the
  engine name (the audit enforces all of these);
- `dependencies.md` maps every internal dependency to its vault bundle (live
  or forward link), flow steps that cross into a resolved dependency document
  what happens inside it (cited `<dep-repo>:path:line`), and a library scan's
  `consumers.md` links back to documented importers;
- every load-bearing claim cites `relative/path:line` and has been rechecked;
- validators and audits are clean. If evidence is unavailable, say what remains
  uncertain and finish as partial rather than fabricating completeness.

## Modes

- **Full:** inventory the current tree and reconcile all candidates.
- **Focused:** inventory the entire repo, document the requested lens deeply,
  and mark out-of-scope rows `irrelevant` with the focus as the reason.
- **Incremental:** use `--changed-since` only when the recorded commit exists
  and is an ancestor of HEAD. Include affected registration/contract files,
  update changed/new/removed candidates, and accept `full-fallback` when the
  baseline is unsafe.

See [full-scan-manifest.json](examples/full-scan-manifest.json),
[api-flow-bundle](examples/api-flow-bundle), and
[datastore-impact-bundle](examples/datastore-impact-bundle) for compact output
examples. Scripts are conservative accelerators; source reading remains the
authority.

Do not use this skill for a code review, implementation change, or one-off
question about a repository that does not request durable Vault documentation.
