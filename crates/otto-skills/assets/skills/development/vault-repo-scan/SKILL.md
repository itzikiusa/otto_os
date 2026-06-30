---
description: Scan a repository into the Otto Vault (Repo Brain) — build the tree-sitter symbol index, the code dependency graph (HTTP/DB/import/call/test edges), and embeddings, then write a linked feature doc and VERIFY the result with a second pass. Use when asked to "index a repo", "scan a repo into the vault", map a feature's dependencies (e.g. a login flow), or build the repo brain. Works over the Otto MCP server or directly against ottod.
category: development
version: 1
---

# Vault Repo Scan (with verification)

Turn a repository into a queryable **Repo Brain**: a symbol index, a typed
dependency graph, embeddings, and a short linked feature doc — then **verify** the
graph against the real code with a second agent/pass before declaring done.

You drive this through Otto's API/MCP — you never reimplement the indexer. The
engine lives in `otto-memory`; you orchestrate it.

## Tools you use

Over the **Otto MCP server** (`otto.*` tools), or directly against **ottod**
(`http://127.0.0.1:7700/api/v1`, Bearer `$OTTO_API_TOKEN`):

| Step | MCP tool | HTTP |
| --- | --- | --- |
| Index a repo | `otto.vault_index_repo` | `POST /workspaces/{ws}/vault/repos/index {root,name?}` |
| List indexed repos | `otto.vault_list_repos` | `GET /workspaces/{ws}/vault/repos` |
| Search symbols | `otto.vault_search_symbols` | `GET /workspaces/{ws}/vault/symbols?q=` |
| Read the dep graph | `otto.vault_code_graph` | `GET /workspaces/{ws}/vault/graph?repo_id=` |
| Walk a node's deps | `otto.vault_node_neighborhood` | `GET /workspaces/{ws}/vault/graph/{node_id}?depth=` |
| Write a linked doc | `otto.vault_upsert_doc` | `POST /workspaces/{ws}/vault/docs` |
| Assemble the brain | `otto.vault_brain` | `POST /workspaces/{ws}/vault/brain {focus}` |

Index/doc tools are mutating (approval-gated); the rest are read-only.

## Procedure

1. **Index.** Call `vault_index_repo` with the absolute `root` (e.g.
   `~/go_admission`) and a `name`. Capture `{files, symbols, edges, chunks}`. If
   `edges` is 0 or `symbols` is suspiciously low, stop and report — the path is
   probably wrong or unsupported.

2. **Pick the flow.** Identify the entry symbol for the feature you were asked
   about (e.g. `Login`). Find it with `vault_search_symbols?q=login`.

3. **Trace dependencies.** From the entry symbol's graph node, call
   `vault_node_neighborhood` (depth 2–3). Read the typed edges:
   - `http_call` → the external service it calls (e.g. `LIMITS` / `go_casino_kit`)
   - `db_call` → the table it reads/writes (e.g. `MdlGm_tblLimits`)
   - `imports` → cross-repo dependencies
   - `calls` → intra-repo functions it invokes
   Write down the chain in prose: `login → limits (HTTP, go_casino_kit) → limits table (DB, default if absent) → …`.

4. **Write the feature doc.** Call `vault_upsert_doc` with a **brief** summary
   (2–4 sentences) plus a **details** section (the traced flow, each hop, defaults
   and edge cases). Pass `documents:[<node_ids>]` for the entry symbol + key hops
   so the doc is LINKED into the graph (it shows up as a `doc` node with
   `documents` edges). Keep the doc about *embeddings + links*, not an essay — the
   long-form description can be added later.

5. **VERIFY (second pass — required).** Do NOT trust the heuristics blindly. Run a
   verification pass — ideally a *separate* agent/subagent so the check is
   independent of the author:
   - For each `http_call`/`db_call`/`imports` edge the graph asserts, open the
     cited `file:line` and confirm the call really exists and targets what the
     edge says. Flag false positives (e.g. a string literal that isn't really a
     service) and false negatives (a real call the scan missed).
   - Re-run `vault_brain {focus:"<feature>"}` and confirm the assembled brain
     names the real dependencies and reads correctly.
   - Record the verification result IN the doc (a short "Verified:" line listing
     what was confirmed and any corrections). If you found errors, correct the doc
     and note them — never silently pass.

6. **Report.** Summarize: counts, the traced flow, the doc id/title, and the
   verification verdict (confirmed / corrected). Link the user to the Vault →
   Graph view to explore it visually.

## Notes
- The scan is heuristic (tree-sitter symbols + content signals for HTTP/DB). It is
  meant to bootstrap the graph + embeddings + links — the *authoritative* feature
  description is the human/agent-written doc you attach in step 4, refined over time.
- Big repos are bounded (file/byte/time caps + a chunk cap). For a focused flow,
  indexing the relevant sub-path is faster and cleaner than the whole monorepo.
- Multi-layer storage is automatic: SQLite is the system of record; if the
  workspace has a Qdrant/SurrealDB backend enabled, vectors/graph are mirrored
  there too. You don't manage that here.
