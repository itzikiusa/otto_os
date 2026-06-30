# Vault / Context Engine v2 — "Repo Brain"

> Status: implemented in this branch. This doc is the design + the feature guide.
> Source of truth for the API surface remains `docs/contracts/`.

## Why

Vault v1 was real SQLite storage but with three weak organs:

1. a **deterministic stub embedder** (FNV-1a hashed bag-of-words),
2. **brute-force** vector search, and
3. **`LIKE`** keyword search.

Only Product ever consumed recall, and recall was effectively a ghost feature
(defined, zero callers on the spawn path). There was no code intelligence
(symbols, dependency graph, git/test context) and no way for the vault to explain
*why* it surfaced something.

v2 turns the Vault into a **Repo Brain**: a layered, pluggable knowledge + code
store that every agent consumes, reachable over MCP (locally and externally), with
real embeddings, ANN search, an FTS5 keyword index, a tree-sitter symbol index, a
code dependency graph, git/test context, and per-result "why selected"
explanations.

## The three databases — where each one is used

The Vault is **layered**. SQLite is always present and is the system of record;
Qdrant and SurrealDB are **optional remote layers** you can turn on per-workspace.

| Concern | SQLite (local, always-on) | Qdrant (optional, remote) | SurrealDB (optional, remote) |
|---|---|---|---|
| Metadata / system of record (memories, docs, governance, repo-index state) | ✅ authoritative | — | mirror (optional) |
| Keyword search | ✅ **FTS5** | — | — |
| Vector embeddings + ANN | ✅ BLOB + in-proc **HNSW** (+ exact fallback) | ✅ **primary vector engine at scale / shared** | ✅ (alt vector engine) |
| Code dependency / knowledge **graph** | ✅ edge rows + in-proc traversal | — | ✅ **primary graph engine** (RELATE + graph traversal) |
| Symbol index | ✅ | — | mirror (optional) |
| External / team sharing | via host Otto (remote.rs) | ✅ shared cluster | ✅ shared cluster |

**Decision rules baked into config (`vault.*`):**

- **SQLite** — the default for *everything*. Zero-config, offline, hermetic. The
  authoritative store for metadata, FTS5 keyword search, vectors (as f32 BLOB) with
  an in-process HNSW ANN built from them, the symbol index, and the dependency
  graph as edge rows. Otto works fully with only SQLite.
- **Qdrant** — turn on when the vector set gets large or must be **shared across
  machines**. It becomes the `vector_backend`: embeddings + a small filter payload
  live in Qdrant; ANN runs there; SQLite stays the metadata system of record and
  still serves FTS5. Purpose-built ANN = better recall/latency than in-proc HNSW at
  scale. Reached over its REST API (SSRF-guarded), so no heavy gRPC/native deps.
- **SurrealDB** — turn on when you want **rich graph traversal** over the code /
  knowledge graph (multi-hop "what does `login` depend on, transitively?") and/or a
  single multi-model remote store. It becomes the `graph_backend` (and optionally a
  `vector_backend`), using `RELATE` + SurrealQL graph queries. Reached over its HTTP
  `/sql` endpoint (SSRF-guarded).

Backends are independent: e.g. `vector_backend = qdrant` + `graph_backend = surreal`
+ metadata in SQLite is a valid, supported layering. Any unconfigured backend
silently falls back to SQLite, so nothing ever breaks if a remote is down.

## Automatic installation

Remote backends and the local neural embedder can be **installed from Otto** (UI
button → API → MCP), never required up front:

- **Qdrant** — `docker run -d -p 6333:6333 -v <data>:/qdrant/storage qdrant/qdrant:<pinned>`;
  health-checked on `GET /readyz`; config saved on success.
- **SurrealDB** — Homebrew (`brew install surrealdb/tap/surreal`) or the official
  install script, else Docker (`surrealdb/surrealdb:<pinned> start`); health-checked.
- **Ollama** (local neural embeddings) — `brew install ollama` / install script,
  then `ollama pull nomic-embed-text`; health-checked on `GET /api/tags`.

The installer is a **planned, approval-gated action**: `POST …/vault/backends/{kind}/install`
first returns a *plan* (the exact commands + prerequisites it detected); execution
runs only after explicit approval (and is exposed as a DANGEROUS MCP tool). Default
deploys never need any of it.

## Architecture

```
                          ┌──────────────────────────────────────────┐
   agent session spawn ──▶│ otto-context::materialize::provision()    │
   (all 9 features)       │   builds the "Repo Brain" context block   │
                          └───────────────┬──────────────────────────┘
                                          │ recall()
                          ┌───────────────▼──────────────────────────┐
                          │ otto-memory::MemoryService                │
                          │  hybrid: FTS5 ⊕ vector(ANN) ⊕ graph ⊕     │
                          │          symbols ⊕ git ⊕ tests  → RRF      │
                          │  every hit carries `why: [ContextReason]` │
                          └──┬─────────┬─────────┬─────────┬──────────┘
            Embedder trait   │         │ Vector  │ Graph   │ CodeIndex
        ┌──────────┬─────────┘  ┌──────┴──┐  ┌───┴────┐ ┌──┴─────────┐
   local-code  ollama  remote   sqlite/HNSW qdrant   sqlite surreal  tree-sitter
   (default)  (local) (oai/voy)  (default)  (remote) (def)  (remote)  symbols+deps
```

### Embedders (`Embedder` trait, already existed)
- `LocalCodeEmbedder` (**new default**) — code-aware: splits camelCase/snake_case,
  keeps symbol tokens, char n-grams, larger dim, L2-normalized. Deterministic &
  offline — what E2E runs against. A real improvement over the FNV stub.
- `OllamaEmbedder` (**new**) — real *local neural* embeddings via localhost Ollama.
- `RemoteEmbedder` (existed) — OpenAI / Voyage.
- `fastembed` (**new, cargo feature `fastembed`, off by default**) — in-process
  ONNX local neural embeddings. Behind a feature so the default build/deploy never
  links ONNX; flip it on to get true local neural without a server.

### Vector index (`VectorIndex` trait, already existed)
- `HnswIndex` (**new**) — HNSW-style ANN over the SQLite-stored vectors with an
  exact-cosine fallback below a size threshold (keeps small sets / tests exact &
  deterministic). Replaces always-brute-force.
- `QdrantIndex` (**new**) — remote ANN over Qdrant REST.

### Code intelligence (new)
- **Symbol index** — tree-sitter (reusing the repomap extractor) persisted to
  `code_symbols` (name, kind, file, line, signature, lang, repo).
- **Dependency graph** — `code_nodes` + `code_edges`: edges typed
  `calls | imports | http_call | db_call | test_of | defined_in`. Heuristic
  extractors detect Go HTTP-client calls to other services (e.g. `go_admission`
  login → `go_casino_kit` limits) and DB calls (SQL / query builders).
- **Git context** — recent commits, a blame summary, recent PRs touching a file
  (via the `git` CLI; bounded).
- **Test mapping** — source ↔ test heuristics (`x.go`↔`x_test.go`,
  `X.ts`↔`X.test.ts`, `foo.py`↔`test_foo.py`, …).

### Context explanation
Every recall result carries `why: Vec<ContextReason>` — `{kind, detail, score}`
(e.g. `vector 0.82`, `keyword "limits"`, `symbol GetLimits @file:120`,
`depends-on login`, `changed in PR #123`, `test for handler.go`). Surfaced in the
context block, the API, MCP, and the UI.

### All agents consume the vault
`materialize::provision()` (the single PreSpawnHook chokepoint for all 9
session-spawning features) gains an optional `RepoBrain` recall: it builds a
ranked, budgeted "Repo Brain" section (top symbols, dep-graph for the task focus,
relevant memories/docs, git/test context, each with its "why") and injects it into
`CONTEXT.md`. One change → every agent gets the right repo brain.

## MCP surface (local + external)
Read tools (default-on): `vault_search`, `vault_list`, `vault_symbols`,
`vault_graph`, `vault_explain`, `vault_git_context`, `vault_test_map`,
`vault_doc_get`. Write/index tools (DANGEROUS + approval): `vault_ingest`,
`vault_index_repo`, `vault_doc_upsert`, `vault_link`, `vault_backend_install`.
External access is the **existing** opt-in TLS network listener + `kind='mcp'`
bearer token — defaults stay loopback-only; we do not weaken them.

## The worked example — `~/go_admission` login flow
`vault_index_repo` over `~/go_admission` produces: embeddings for the login-flow
files, a symbol index, and a dependency graph —
`login → (http_call) limits/reality-check → go_casino_kit → (db_call) limits table
(default if absent)` — plus a generated **"Login Flow" doc** (brief + detailed)
linked into the graph. The UI renders the graph, the doc, and the per-node "why".
A fixture repo mirrors this structure for hermetic E2E; the real run against
`~/go_admission` is captured in `docs/features/vault-v2-login-example.md`.

## UI
The Vault page gains: a **Repo Index** tab (index a repo, see status), a **Symbols**
browser, a **Code Graph** view (the login-flow dependency graph), **why-selected**
chips on every search hit, **git/test context** panels, a **Docs** view with links,
and a **Backends** panel (status + one-click install of Qdrant/SurrealDB/Ollama).

### Full graph view (Obsidian-style)
A first-class **Graph view** tab: a force-directed graph of the *whole* vault —
memories, docs, symbols, and code dependency nodes — with the same shape as
Obsidian's:

- **Left sidebar**: a hierarchical tree of vault content (collection → folder →
  note / repo → file → symbol), clickable to focus a node.
- **Center**: a force-directed (D3-force) canvas — drag, zoom/pan, click-to-focus,
  hover for the node's "why"/summary, node color by kind, edge style by relation
  (`link`, `calls`, `http_call`, `db_call`, `test_of`).
- **Right**: collapsible control panels mirroring Obsidian —
  - **Filters** — search/text filter, by collection, by kind, show/hide
    orphans, include code nodes vs knowledge nodes.
  - **Groups** — color groups by a query/tag (e.g. group "limits" red).
  - **Display** — node-label visibility, node size by degree/centrality, link
    thickness, arrows, animate.
  - **Forces** — center force, repel strength, link force/distance, collision.

The graph data comes from the unified graph endpoint (knowledge links + code
edges). When SurrealDB is the `graph_backend`, traversal/expansion queries are
served by SurrealDB (RELATE/graph SurrealQL); otherwise SQLite serves the edges.
The *visualization* is ours either way — SurrealDB is the graph data engine, not a
view.
