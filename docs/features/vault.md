# Vault — the workspace knowledge store

The **Vault** is Otto's workspace-scoped knowledge store: a place where distilled
notes (decisions, facts, requirements, constraints, learnings, answered questions,
entities, summaries) and raw evidence (chunks) accumulate, link to each other with
`[[backlinks]]`, and become recallable by **hybrid search** — keyword (SQL `LIKE`)
fused with semantic (vector) similarity. The engine (`otto-memory`) is
domain-agnostic: the **Product** section is its first consumer, ingesting story
artifacts so agents recall a compact background brief each turn instead of
re-reading every Jira/Confluence artifact, but any feature can write to and read
from the same store.

This document is the end-user and operator reference for the Vault. It documents
what the code actually does today; where a capability is scaffolded but not wired
into the running daemon, that is called out explicitly.

> **At a glance.** The Vault is a single SQLite store living in your Otto data
> directory. By default it embeds notes with a **local, dependency-free
> deterministic stub embedder** — no model download, no API key, no network. Real
> local (`fastembed`) and remote (OpenAI / Voyage) embedders are designed in
> behind a trait but are **not** wired into the shipped daemon. Reads need
> workspace **Viewer**; writes need **Editor**. The UI is the **Vault** section in
> the left rail (un-gated — visible to any authenticated member).

---

## 1. Overview & where it lives

The Vault is one of Otto's crates (`otto-memory`) plus a UI module
(`ui/src/modules/vault/`). The daemon mounts its REST router under `/api/v1`; the
persistence is plain SQLite tables in the shared state DB. Everything is scoped to
a **workspace** (`workspace_id`), and rows cascade-delete when a workspace is
removed.

| Concern | Where it lives |
|---|---|
| Engine crate | `crates/otto-memory/` |
| Service (save / search / recall / governance) | `crates/otto-memory/src/service.rs`, `governance.rs` |
| Embedder seam | `crates/otto-memory/src/embed.rs` |
| Vector index (brute-force cosine) | `crates/otto-memory/src/index.rs` |
| Retrieval math (RRF + re-rank) | `crates/otto-memory/src/retrieve.rs` |
| Collection ingest / graph import | `crates/otto-memory/src/ingest.rs` |
| Obsidian-vault write-through / re-index | `crates/otto-memory/src/vault.rs` |
| Shared-host remote backend | `crates/otto-memory/src/remote.rs` |
| Core REST router | `crates/otto-memory/src/http.rs` |
| Governance REST router | `crates/otto-server/src/memory_gov.rs` |
| Product → memory ingest route | `crates/otto-server/src/routes/product_memory.rs` |
| Persistence (DTOs + repo + SQL) | `crates/otto-state/src/memory.rs` |
| SQLite schema | `crates/otto-state/migrations/0038_memory.sql`, `0039_memory_sharing.sql`, `0056_memory_lifecycle.sql` |
| Daemon wiring (backend selection) | `crates/ottod/src/main.rs` (≈ lines 248–268) |
| UI page + store | `ui/src/modules/vault/VaultPage.svelte`, `vault.svelte.ts` |
| UI dialogs | `ImportGovDialog.svelte`, `MergeDialog.svelte`, `SplitDialog.svelte`, `MemoryStateBadge.svelte` |
| TypeScript contract mirror | `ui/src/lib/api/types.ts` (`Memory`, `NewMemory`, `MemoryPatch`, `MemoryQuery`, `MemoryHit`, `RecallBrief`, `MemoryLink`, `MemoryGraphData`) |
| Authoritative API contract | `docs/contracts/api.md` → *Memory layer (workspace-scoped knowledge store)* |
| Stored data | SQLite tables `memories`, `memory_vectors`, `memory_links`, `governed_imports` in the Otto state DB |

### Data model in one screen

A row in `memories` carries:

- **Identity & scope** — `id`, `workspace_id`, `collection` (`product` | `code` |
  `docs` | `confluence` | `platform-map`, default `product`), `record_type`
  (`item` = distilled, `chunk` = raw), `scope` (`workspace` | `story` | `entity`),
  optional `story_id`.
- **Content** — `kind` (the taxonomy below), `title`, `body`, `tags`, `entities`,
  and `refs` (provenance pointers: a `{kind, ref, url?, label?}` list).
- **Provenance** — `source_kind` (e.g. `jira`, `confluence`, `analysis`,
  `transcript`, `question`, `learning`, `code`, `manual`, `graphify`) and an
  optional `source_ref`.
- **Ranking priors** — `confidence` (default 0.7) and `salience` (default 0.5).
- **Lifecycle** — `active` (soft-delete flag), `state`
  (`suggested`|`accepted`|`stale`|`contradicted`, default `accepted`),
  `superseded_by`, `version`, `provenance_json`, `forgotten_at`, `undo_token`,
  `expires_at`.
- **Visibility** — `shared` (all workspace members; the default) or `private`
  (creator-only).
- **Bookkeeping** — `created_by`, `created_at`, `updated_at`, `last_accessed_at`,
  `access_count`, `content_hash` (a normalized SHA-256 of the body, used for
  exact-duplicate detection).

The **`kind` taxonomy** (`crates/otto-memory/src/types.rs::kind`): `fact`,
`decision`, `requirement`, `constraint`, `qa`, `learning`, `summary`, `entity`,
`snapshot`, `glossary`, `chunk`.

---

## 2. Notes & backlinks

### Creating notes

A "note" is a `memories` row. There are several ways one is created:

1. **Manually**, via `POST /workspaces/{ws}/memories` with a `NewMemory` body
   (`source_kind: "manual"`). This is the direct API path; the engine assigns the
   `id`, computes the `content_hash`, embeds the note on write, and (if a vault
   directory is configured) writes a markdown file.
2. **By ingesting a story** — Product extracts a story's answered questions,
   learnings, latest analysis summary, and newest version into typed memories
   (see §7).
3. **By chunking text** into a collection — `POST .../memory/ingest-text` splits a
   file's content into overlapping 40-line windows (8-line overlap) and stores each
   as a `chunk` record tied back to the source path.
4. **By importing a graph** — a graphify `graph.json` becomes `entity` memories +
   links (see §4).
5. **By importing a governance file** — AGENTS.md / CLAUDE.md / .cursorrules are
   parsed into `suggested` memories (see §4).

**Exact-duplicate saves are a NOOP.** Before inserting, the service looks up the
`(workspace, collection, scope, story_id, content_hash)` tuple; if a live row with
the same normalized body already exists, the existing row is returned unchanged
(no new id, no duplicate). This is why repeated ingests are safe and idempotent.

**Editing** a note (`PATCH .../memories/{id}`) accepts a partial `MemoryPatch`
(`title`, `body`, `tags`, `entities`, `confidence`, `salience`, `active`). On
update the `content_hash` and embedding are recomputed and `version` is bumped.

**Soft-delete semantics.** `DELETE .../memories/{id}` flips `active=false` (a
`204`); the row is never hard-deleted by this path. The richer governance
`forget` endpoint (see §4) additionally mints an undo token and sets `state=stale`.

### Backlinks and the link graph

Links live in a separate `memory_links` table: `(src_id, dst_id, rel, weight,
certainty)`. The `rel` vocabulary is `relates_to`, `supersedes`, `derived_from`,
`about_entity`, `duplicates`, `blocks`; `certainty` is graphify-style
(`extracted` | `inferred` | `ambiguous`).

- `GET .../memories/{id}/links` returns **all** links touching a note (where it is
  either source or destination).
- The Vault UI computes **backlinks** as the subset where `dst_id == selected.id`
  — i.e. the notes that point *to* the one you're reading, shown under
  **"Linked by (N)"**.

When the Vault writes Obsidian-style markdown files, links are rendered as
`[[wikilinks]]` at the top of each note's body; the SQLite store is the derived
index and the markdown files are the human-/git-facing representation.

> **README divergence (intentional).** The README advertises "notes with
> `[[backlinks]]`". The `[[…]]` wiki-link syntax is produced and parsed in the
> **markdown vault file** representation (`vault.rs`). Inside the SQLite store and
> the UI, links are first-class `memory_links` rows; the UI surfaces them as a
> "Linked by" backlinks list rather than rendering inline `[[…]]` tokens in the
> note body. Both describe the same link graph — the wiki-link text is the
> file-format spelling of it.

---

## 3. Collections & the graph view

### Collections

A **collection** is a free-form bucket (`collection` column) that groups related
memories: `product` (the default), `code`, `docs`, `confluence`, `platform-map`.
They're used for filtering (`?collection=` on list/search/graph) and as the
default ingest target (`code` for `ingest-text`/`import-graph`, `platform-map` for
governance imports). In the UI, the left sidebar shows **collection chips** (only
when more than one collection is present) to filter the index and the graph.

### The graph view

`GET .../memory/graph?collection=` returns `GraphData { nodes, edges }`:

- **nodes** — every active memory (`{id, label, kind, collection}`), capped at
  5000 per workspace.
- **edges** — every `memory_links` row in the workspace.

The Vault UI renders this as a **dependency-free SVG graph**: nodes are laid out on
a circle, colored by `kind` (entity = blue, decision = teal, constraint/requirement
= orange, qa = magenta, chunk = grey, default = light blue), and edges are drawn as
lines — dashed when `certainty == "inferred"`. A footer shows the node/edge counts.
Toggle between the **Index** (list) and **Graph** views with the buttons at the top
of the sidebar.

There is also an **entity neighborhood** endpoint —
`GET .../memory/entities/{id}/graph` — which returns just one memory's immediate
links plus the memories on the other end (`{links, neighbors}`). This is the
narrow, per-node traversal; the full graph endpoint is the workspace-wide view.

---

## 4. Lifecycle governance

Beyond create/read/update, the Vault has a governance layer (migration
`0056_memory_lifecycle.sql`) for curating accumulated knowledge. All governance
routes require workspace **Editor**.

- **Lifecycle state** — `POST .../memory/{mid}/state` with `{state}`. Valid states
  are `suggested` (auto-ingested, awaiting review), `accepted` (active/approved —
  the default for manual creation), `stale` (soft-deprecated), `contradicted`
  (overridden by a newer memory; set automatically by merge/split). Anything else
  is a `400`. The UI shows the state as a chip and offers a state filter
  (all / suggested / accepted / stale / contradicted).
- **Forget with undo** — `POST .../memory/{mid}/forget` soft-deletes the row
  (`active=0`, `state=stale`, `forgotten_at` set) and returns an opaque
  `undo_token`. `POST .../memory/{mid}/forget/undo` with that token restores it
  (`active=1`, `state=accepted`, token cleared — single use). The UI surfaces a
  7-second **"Undo forget"** affordance after a forget. The undo token is a
  SHA-256-derived 32-byte hex string; the undo is workspace-scoped.
- **Merge (N→1)** — `POST .../memory/merge` with `{ids, title, body}`. Creates one
  new memory inheriting the first source's collection/scope/story_id/visibility,
  marks all sources `contradicted` and points their `superseded_by` at the new row,
  and records `provenance_json = {op:"merge", source_ids:[…]}`. Requires ≥2 ids.
  In the UI: enter **Merge…** mode, tick two or more notes, confirm the merged
  title/body.
- **Split (1→N)** — `POST .../memory/{mid}/split` with `{parts:[{title,body},…]}`.
  Creates N children inheriting the parent's metadata, marks the parent
  `contradicted` (pointing `superseded_by` at the first child), and records
  `provenance_json = {op:"split", parent_id}`. Requires ≥2 parts.
- **Provenance diff** — when a note has a `superseded_by`, the UI can fetch the
  successor and show a word-level diff of the two bodies ("this → superseded"),
  using the shared `DiffView` component.
- **Governed import** — `POST .../memory/import` with
  `{kind: "agents-md"|"claude-md"|"cursorrules"|"custom", content, label?}`. Splits
  the markdown on level-2 (`##`) headings (or treats the whole file as one section
  if there are none), creating one memory per non-empty section in the
  `platform-map` collection, tagged with the import kind. AGENTS.md/CLAUDE.md
  sections become `fact`, `.cursorrules` become `constraint`, `custom` become
  `learning`. Every imported memory is set to `state=suggested` (awaiting review),
  and the batch is recorded in `governed_imports` for auditability. Returns
  `{imported, import_id}`. The UI exposes this as the **Import…** button →
  `ImportGovDialog`.

> Note: the `governed_imports` table records a `reverted_at` column and the engine
> can list import batches (`list_governed_imports`), but a one-call "revert this
> import" endpoint is not exposed; reverting today means forgetting the listed
> memory ids.

---

## 5. Hybrid recall (keyword + vector), explained

`POST /workspaces/{ws}/memory/search` takes a `MemoryQuery` and returns ranked
`MemoryHit[]`. The query carries `text`, optional filters (`collection`,
`story_id`, `kinds`, `tags`, `entities`, `scope`, `include_inactive`), `k` (result
count, default 20), and a `mode`:

| `mode` | Behaviour |
|---|---|
| `hybrid` (default) | Run keyword **and** vector retrieval, fuse with RRF, then re-rank. |
| `keyword` | Keyword only (SQL `LIKE`). |
| `semantic` | Vector KNN only (skipped if there is no text or no embedder). |

### How a query is matched and ranked

1. **Keyword candidates** (`MemoriesRepo::search_keyword`). The query is tokenized
   on non-alphanumerics into terms of length ≥2. A SQL prefilter selects rows where
   `lower(title || ' ' || body) LIKE %term%` for *any* term (capped at 2000 rows),
   then rows are ranked by how many distinct terms they contain. Up to `k×4`
   candidates feed the fusion stage.
2. **Semantic candidates** (`VectorIndex::knn`). If `mode != keyword` and there is
   query text, the query string is embedded and the **brute-force cosine index**
   scores it against every stored vector for the workspace + active rows + the
   current embedder's model id, returning the top `k×4` by cosine similarity.
3. **Fusion** (`retrieve::rrf_fuse`). For `hybrid`, the two id-rankings are combined
   with **Reciprocal Rank Fusion** — each list contributes `1/(k0 + rank + 1)` per
   id with damping `k0 = 60` — so an item ranked highly by either method (or
   moderately by both) floats up. For pure `keyword`/`semantic`, the base score is
   simply `1/(1 + rank)`.
4. **Filter + re-rank** (`MemoryService::search` + `retrieve::rerank_score`). Each
   fused candidate is loaded and dropped if it fails the post-filters (active,
   visibility, `story_id`, `collection`, `kinds`). Survivors get a light prior on
   top of the fused base score:

   ```text
   score = base × (1 + 0.3·recency + 0.05·ln(1+access_count)
                     + 0.2·(confidence·salience))
                 + scope_bonus
   ```

   where `recency = 0.5 ^ (recency_days / half_life)` (half-life defaults to 30
   days, overridable via `recency_half_life_days`), and `scope_bonus = 0.15` when
   the hit's `story_id` matches the queried one. Results are sorted descending and
   truncated to `k`.
5. **Access bookkeeping.** Every returned hit's `access_count` is incremented and
   `last_accessed_at` updated, so frequently-recalled memories are gently boosted
   over time.

Each `MemoryHit` carries the `memory`, its final `score`, and a `why` array (an
explanation hook — currently emitted empty by the engine).

### Recall brief (token-budgeted)

`POST /workspaces/{ws}/memory/recall` with `{story_id, focus?, token_budget?}`
assembles a compact **`RecallBrief`** for an agent: it runs grouped hybrid searches
(Constraints & Requirements, Decisions, Key Facts, Answered Questions, Learnings,
Background) scoped to the story, packing as many hits as fit under the token budget
(default 2000; Product calls it with 4000). The returned brief is a list of
markdown sections plus the `token_estimate` and the `used` memory ids. Untrusted
text is defanged (backticks/newlines neutralized) so a stored note can't act as a
prompt instruction when the brief is composed into an agent's context.

---

## 6. Embeddings (local-first)

**Embeddings are computed on every write** (`MemoryService::embed_one` embeds
`title + "\n" + body`) and stored as little-endian `f32[dim]` BLOBs in the
`memory_vectors` table, keyed by `memory_id` + `model_id` + `dim`. Search embeds
the query the same way and compares with cosine similarity.

### What actually runs today

The shipped daemon uses **`MemoryService::with_defaults`**, which wires the
**`StubEmbedder`** (`crates/otto-memory/src/embed.rs`):

- **Model id:** `stub-v1`, **dimension:** 256.
- **Algorithm:** a deterministic, unit-normalized hashed bag-of-words — each token
  is FNV-1a-hashed into one of 256 feature buckets. Notes that share tokens land on
  closer vectors.
- **Local-first by design:** no model download, no network call, no API key, zero
  extra dependencies. This is what makes the Vault work out of the box and entirely
  offline.

It is explicitly *not* a substitute for a real semantic model — for short,
keyword-overlapping notes it behaves much like a fuzzy keyword matcher, which is
why `hybrid` mode (keyword ⊕ vector, RRF-fused) is the default and gives the best
results with the stub in place.

### Real embedders (designed, not wired)

The `Embedder` trait has additional implementations in the codebase, intended to
swap in behind the same seam:

- **Remote** (`RemoteEmbedder`) — an OpenAI-/Voyage-compatible client that POSTs
  `{model, input}` to `<base>/embeddings` with a bearer token. Presets:
  OpenAI `text-embedding-3-small` (1536-dim) at `https://api.openai.com/v1`, and
  Voyage `voyage-3` (1024-dim) at `https://api.voyageai.com/v1`. API keys are
  passed in as resolved strings (sourced from the macOS Keychain) and **never**
  stored in the DB.
- **Local** — comments reference a `fastembed`-backed local model as the intended
  on-device option.

**However:** `crates/otto-memory/Cargo.toml` declares **no `[features]` and no
`fastembed` dependency**, and `ottod/src/main.rs` only ever constructs the stub
(`with_defaults`) or a remote *backend* (a different thing — see §7/§10). There is
**no setting, env var, or build feature in the current tree that activates the
`RemoteEmbedder` or a local `fastembed` model in the running daemon.** The
`with_embedder(pool, embedder)` constructor exists as the one-line wiring point for
when that lands. Treat real embedders as a designed-and-tested seam, not a
user-facing option yet.

> **Bottom line on the embedding question:** today the Vault embeds **locally with
> a built-in deterministic stub** (`stub-v1`, 256-dim, no key, offline). API-based
> embedders (OpenAI/Voyage) and a local `fastembed` model are implemented behind
> the `Embedder` trait but are **not** connected to the shipped daemon.

### Vectors & the index

`sqlite-vec` and an HNSW ANN index are mentioned only as future seams in code
comments. The active `VectorIndex` is **`BruteForceIndex`**: it loads every
workspace vector for the current model and computes cosine in-process. This is
sub-millisecond at single-user scale; for very large collections it is the
documented place to swap in an ANN index behind the same trait.

---

## 7. How other features use the Vault

The whole point of the Vault is to let agent-driven features recall a small,
relevant brief instead of re-fetching and re-stuffing raw context every turn.

- **Product** is the first and primary consumer (`otto-product`,
  `ProductMemory` in `memory_facade.rs`):
  - **Ingest** — `POST /workspaces/{ws}/product/stories/{sid}/memory/ingest`
    extracts a story's **answered questions**, **learnings**, **latest analysis
    summary**, and **newest version** into typed memories (`qa`, `learning`,
    `summary`, etc.) tagged with the `story_id`. Dedup and embedding happen inside
    `save`, so repeated ingests are idempotent. Returns `{ingested}` (the number of
    candidate memories submitted). Editor-gated.
  - **Recall** — `ProductMemory::recall_brief` calls the engine's `recall_brief`
    with a 4000-token budget, producing the grouped background brief an agent reads
    in place of the raw Jira/Confluence artifacts.
- **Agents / swarm.** The engine and the `recall`/`search` endpoints are
  domain-agnostic and available to any caller with Viewer access, but at the time
  of writing **only Product wires the recall façade** — `otto-swarm`,
  `otto-context`, `otto-sessions`, and `otto-orchestrator` do not yet call the
  Vault directly. Agents reach Vault content through Product's recall brief (and, of
  course, through the HTTP API if scripted via an API token).
- **Governed config import.** Any feature can fold an AGENTS.md / CLAUDE.md /
  .cursorrules into the `platform-map` collection (see §4), making the project's
  own operating rules recallable knowledge.

---

## 8. API / contract reference

All paths are relative to the `/api/v1` mount and require a bearer token. The
authoritative contract is `docs/contracts/api.md` (*Memory layer*) and the
TypeScript mirror in `ui/src/lib/api/types.ts`. Reads require workspace **Viewer**;
mutations require **Editor**.

### Core (router: `otto-memory/src/http.rs`)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `GET /workspaces/{ws}/memories` | Viewer | query: `collection?, kind?, story_id?, tag?, include_inactive?, limit?` | `Memory[]` |
| `POST /workspaces/{ws}/memories` | Editor | `NewMemory` | `Memory` (exact-dup save returns the existing row) |
| `GET /workspaces/{ws}/memories/{id}` | Viewer | — | `Memory` (404 for another user's `private` memory) |
| `PATCH /workspaces/{ws}/memories/{id}` | Editor | `MemoryPatch` | `Memory` |
| `DELETE /workspaces/{ws}/memories/{id}` | Editor | — | `204` (soft-delete: `active=false`) |
| `GET /workspaces/{ws}/memories/{id}/links` | Viewer | — | `MemoryLink[]` |
| `POST /workspaces/{ws}/memory/search` | Viewer | `MemoryQuery` | `MemoryHit[]` (hybrid keyword⊕vector, RRF-fused, re-ranked) |
| `POST /workspaces/{ws}/memory/recall` | Viewer | `{story_id, focus?, token_budget?}` | `RecallBrief` |
| `GET /workspaces/{ws}/memory/graph` | Viewer | query: `collection?` | `GraphData {nodes, edges}` |
| `POST /workspaces/{ws}/memory/ingest-text` | Editor | `{collection?, path, content}` | `{chunks}` |
| `POST /workspaces/{ws}/memory/import-graph` | Editor | `{collection?, graph:{nodes,edges}}` | `ImportStats {nodes, edges}` |
| `GET /workspaces/{ws}/memory/entities/{id}/graph` | Viewer | — | `{links, neighbors}` |
| `POST /workspaces/{ws}/product/stories/{sid}/memory/ingest` | Editor | — | `{ingested}` |

### Governance (router: `otto-server/src/memory_gov.rs` — all Editor)

| Method & path | Request | Response |
|---|---|---|
| `POST /workspaces/{ws}/memory/{mid}/state` | `{state}` | `Memory` |
| `POST /workspaces/{ws}/memory/{mid}/forget` | — | `{undo_token}` |
| `POST /workspaces/{ws}/memory/{mid}/forget/undo` | `{undo_token}` | `Memory` (restored) |
| `POST /workspaces/{ws}/memory/merge` | `{ids, title, body}` | `{memory}` |
| `POST /workspaces/{ws}/memory/{mid}/split` | `{parts:[{title,body},…]}` | `{memories}` |
| `POST /workspaces/{ws}/memory/import` | `{kind, content, label?}` | `{imported, import_id}` |

**Query notes** — `MemoryQuery.mode` ∈ `{hybrid (default), semantic, keyword}`;
`k` defaults to 20. `visibility` ∈ `{shared (default), private}`. `viewer` is set
server-side from the authenticated user and is never read from the client body.

---

## 9. Capabilities & limitations

**Capabilities**
- Workspace-scoped notes across multiple collections, with a typed `kind`
  taxonomy and provenance refs.
- A first-class link graph (`memory_links`) with backlinks and a built-in,
  dependency-free SVG graph view.
- Hybrid recall: keyword (`LIKE`) ⊕ vector (cosine), RRF-fused and re-ranked by
  recency, usage, confidence×salience, and story-scope match.
- Exact-duplicate-safe (idempotent) saves via normalized body hashing.
- Token-budgeted recall briefs for agents.
- Full lifecycle governance: state machine, soft-delete with undo, merge, split,
  provenance diff, and audited governed-config import.
- Per-user `shared`/`private` visibility.
- Optional Obsidian-vault markdown write-through + re-index for git-based sharing.
- Optional shared-host backend so a team can share one Vault across machines.

**Limitations / honest gaps**
- **The shipped embedder is a deterministic stub**, not a learned semantic model;
  real local (`fastembed`) and remote (OpenAI/Voyage) embedders are coded behind
  the trait but not wired in or feature-flagged in the current tree.
- **Brute-force vector search** only — fine for single-user scale; no ANN index
  (`sqlite-vec`/HNSW) is wired despite the comment seams.
- **Keyword search is `LIKE`-based**, not FTS5 — robust and always-available but
  not stemmed/ranked like a true full-text index. (The contract's "FTS5" framing
  describes the intended class of feature; the implementation is a `LIKE`
  prefilter + term-count ranking.)
- **Only Product consumes recall** today; other agent features don't yet read the
  Vault directly.
- **No one-click revert** for a governed import (the batch is tracked, but you
  forget the rows manually).
- The `MemoryHit.why` explanation array is currently emitted empty.
- A shared SQLite **file** over a network share is unsupported — use the remote
  backend or the markdown-vault sync route instead (see §10).

---

## 10. Security & permissions

- **Workspace scope is the boundary.** Every query is keyed on `workspace_id`, and
  the tables `ON DELETE CASCADE` from `workspaces`. There is no cross-workspace
  read or search. The Vault has no RBAC *feature key* of its own — it ships
  un-gated, so any authenticated workspace member sees the section — but the
  per-route checks still enforce **Viewer for reads, Editor for writes** at the
  workspace-role level (root bypasses the visibility filter).
- **Per-user visibility.** A `private` memory is visible only to its creator; the
  `get_one` endpoint returns **404** (not 403) for another user's private memory so
  its existence isn't leaked, and `list`/`search` exclude others' private rows
  (the server sets `viewer` from the authenticated user; root sees everything).
- **Local-first data.** By default everything lives in your local SQLite state DB
  and embeddings are computed on-device with the stub — no note text or vector
  leaves the machine, and no API key is required.
- **Secrets stay out of the DB.** When a remote embedder is used, its API key is a
  Keychain-resolved string passed in at construction and never persisted.
- **Prompt-injection defense.** Text composed into a recall brief is defanged
  (backticks/newlines neutralized) so stored notes can't smuggle instructions into
  an agent prompt.
- **Sharing across machines** (operator choice, both opt-in):
  - **Remote backend** — set `OTTO_MEMORY_REMOTE_URL` (and
    `OTTO_MEMORY_REMOTE_TOKEN`) so every member's Otto forwards memory operations to
    one shared host Otto that owns the SQLite (single writer; each member
    authenticates as themselves, so `shared`/`private` is enforced per-user on the
    host). Graph import must run on the host.
  - **Markdown vault sync** — set `OTTO_MEMORY_VAULT_DIR` (ignored when a remote URL
    is also set) to write each saved memory as an Obsidian-style note under
    `<dir>/<workspace>/`. Sync that folder with git/Dropbox/Syncthing and re-index
    it (`reindex_vault`) on other machines. A shared SQLite *file* over a network is
    explicitly unsupported.

---

## 11. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| **"No memories yet"** in the Vault | Nothing has been ingested for this workspace. Run a Product analysis or ingest a story (`POST .../product/stories/{sid}/memory/ingest`), import an AGENTS.md/CLAUDE.md, or create a note via the API. |
| **Search returns weak/odd matches** | The default embedder is the deterministic stub, so `semantic` mode is approximate. Use `hybrid` (the UI default) and include distinctive keywords; vectors mainly help when terms overlap. |
| **A saved note didn't create a new row** | Exact-duplicate detection — the same normalized body in the same `(collection, scope, story_id)` returns the existing row. Change the body or scope to create a distinct memory. |
| **Can't see a teammate's note** | It may be `private` (creator-only). Private rows are invisible to others (404 on direct GET). Ask the creator to set it `shared`. |
| **Write returns 403** | Writes require workspace **Editor**; reads only need **Viewer**. Check your workspace role. |
| **Forgot a memory by mistake** | Use the **Undo forget** affordance (7s window in the UI) or `POST .../memory/{mid}/forget/undo` with the `undo_token` returned by forget. The token is single-use. |
| **Graph view is empty** but the index isn't | The graph only shows **active** memories and existing `memory_links`; freshly ingested notes may have no links yet. Links come from imports, merge/split, or graphify. |
| **Team can't share one Vault** | Don't point multiple instances at one SQLite file. Use `OTTO_MEMORY_REMOTE_URL` (shared host) or `OTTO_MEMORY_VAULT_DIR` + git sync + re-index. |
| **Expecting OpenAI/Voyage embeddings** | Not wired into the shipped daemon. The remote/local embedders are scaffolded behind the `Embedder` trait but there is no setting/feature to enable them in the current build (see §6). |
| **Vault data location** | The `memories`/`memory_vectors`/`memory_links`/`governed_imports` tables in the Otto state SQLite DB (under your Otto data directory). Backing up that DB backs up the Vault. |

---

## 12. Related docs

- [`./product.md`](./product.md) — the Product section; the Vault's first consumer
  (story ingest + recall brief).
- [`./agent-sessions.md`](./agent-sessions.md) — agent sessions, which read recalled
  context instead of re-fetching raw artifacts each turn.
- `docs/contracts/api.md` — authoritative API surface (*Memory layer* + *Must-have
  wave* governance routes).
- `crates/otto-memory/` — the engine; `crates/otto-state/src/memory.rs` and
  `crates/otto-state/migrations/0038–0056` — persistence and schema.
