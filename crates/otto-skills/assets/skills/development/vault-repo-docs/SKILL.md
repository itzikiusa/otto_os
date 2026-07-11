---
description: Document a repository into the Otto Vault as an OKF knowledge bundle — read the code, author linked concept docs (services, endpoints, datasets, decisions, runbooks, metrics), validate conformance, and verify claims against the source. This REPLACES embedding-based repo indexing; "index a repo into the vault" now means writing durable, diffable OKF docs. Use when asked to index/scan/document a repo, map a feature's dependencies, or build the team's repo knowledge base.
category: development
version: 2
---

# Vault Repo Docs — index a repo by DOCUMENTING it (OKF)

The old Repo Brain (tree-sitter symbols + embeddings) is gone. The vault is a
**docs home**: a directory of markdown files with an index the daemon keeps
(links, tags, full text). Indexing a repo = an agent reading the code and
writing a linked **OKF bundle** into the vault. Docs are durable, greppable,
diffable, and every claim is verifiable against the source — none of which
embeddings gave us.

Load the companion `okf-authoring` skill for the format doctrine; this skill
is the repo→bundle workflow.

## Tools

Session MCP (`otto_*`) or Otto MCP control plane (`otto.vault_*`) or HTTP
(`http://127.0.0.1:7700/api/v1`, Bearer `$OTTO_API_TOKEN`):

| Step | Session MCP | HTTP |
| --- | --- | --- |
| Find the vault | `otto_vault_list` | `GET /workspaces/{ws}/vault/vaults` |
| Browse | `otto_vault_dir` | `GET …/vaults/{id}/dir?path=` |
| Read a note | `otto_vault_read` | `GET …/vaults/{id}/note?path=` |
| Write a note | `otto_vault_write` | `PUT …/vaults/{id}/note {path,content,if_hash?}` |
| Search | `otto_vault_search` | `POST …/vaults/{id}/search {query}` |
| Backlinks | `otto_vault_backlinks` | `GET …/vaults/{id}/backlinks?path=` |
| Link graph | `otto_vault_graph` | `GET …/vaults/{id}/graph?mode=local&path=` |
| Validate | `otto_vault_okf_validate` | `POST …/vaults/{id}/okf/validate` |
| Regenerate indexes | — | `POST …/vaults/{id}/okf/indexes` |

## Workflow

1. **Pick the bundle home.** `otto_vault_list` → use the workspace's OKF vault
   (ask which if several). Place the repo's bundle under a top-level folder
   named after the repo (e.g. `go-admission/`).
2. **Survey the repo before writing.** Read the README, entrypoints, route
   tables, schema/migrations, config. Build a mental inventory of concepts:
   - `services/` — one doc per deployable/service (grain: what it is, its
     endpoints table, trust/dependency prose with links).
   - `endpoints/` or a `# Endpoints` table inside the service doc (mint
     separate docs only for endpoints with real behavioral depth).
   - `datasets/`/`tables/` — one doc per table/store with a `# Schema` table,
     the grain ("one row per X"), and `# Joins` links.
   - `decisions/` — ADRs you can EVIDENCE from the code/history
     (`# Context / # Decision / # Consequences`).
   - `runbooks/` — operational flows visible in the code (retries, health,
     failure modes).
   - `references/` — enums/status codes/metrics that ≥2 docs would cite
     (four-gate test from `okf-authoring`).
3. **Write concepts** with `otto_vault_write`, OKF frontmatter always
   (`type`, one-sentence `description`, `resource` = repo URL/path, `tags`,
   `timestamp`). Cross-link generously — links are the graph. Cite files as
   `path/to/file.rs:123` in Citations.
4. **Wire the bundle**: repo root `index.md` (or run the index generator
   endpoint), dated `log.md` entry (`**Creation**`/`**Update**` bullets).
5. **Validate**: `otto_vault_okf_validate` → fix EVERY error (E1/E2/E3);
   fix warnings you introduced. Never report done with errors.
6. **Verify claims (second pass).** Re-read each doc you wrote and check 2-3
   load-bearing claims per doc against the actual source (endpoint paths,
   column names, config keys). Fix what you got wrong — never leave invented
   facts. This pass is NOT optional.
7. **Maintain mode** (repo changed): find affected docs via
   `otto_vault_search` (by `resource`/path/topic), update bodies +
   `timestamp`, add concepts for new assets, `**Deprecation**` log entries
   for removed ones, re-validate.

## Rules

- **Never invent**: no fabricated endpoints, columns, enum values or URLs —
  every fact traces to a file you read.
- **Augment, don't rewrite** existing docs (headings survive verbatim; tags
  union-merge; `type`/`title`/`resource` copy verbatim).
- Standard markdown links (`[auth](/go-admission/services/auth.md)` or
  relative) — not wikilinks — inside OKF bundles.
- Keep each doc scannable: prose ≤3 paragraphs, then tables/lists/fenced code.
- Big repos: document breadth-first (all services shallow) before depth; a
  bundle that covers everything thinly beats three perfect docs.
