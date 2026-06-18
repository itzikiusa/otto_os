# Codebase Mapping Method

A systematic way to trace a story from the user-visible surface down to the
database and across service boundaries. Work top-down, then cross-check bottom-up.

---

## 1. Identify the story surface

Start with what the user or external caller touches. The surface tells you the
entrypoint layer to search first.

| Story type | Where to start |
|---|---|
| UI / screen change | Frontend route files, page/view components |
| REST API change | Router / handler / controller layer |
| Background job or ETL | Worker/job registration, scheduler config |
| Event-driven (publish/subscribe) | Message handler or consumer entry |
| Schema/data-only change | Migration files, DAO/repository layer |
| Config / flag change | Config loader, feature-flag evaluation sites |

---

## 2. Trace entrypoints

Find the file(s) where execution enters for this story. Common patterns by language:

- **Go**: `router.go`, `handler.go`, `main.go` registrations, `cmd/` packages
- **Rust**: `main.rs`, `lib.rs`, `routes.rs`, Tauri `#[tauri::command]` handlers
- **TypeScript/JS**: `app.tsx`/`router.tsx`, Next.js `pages/` or `app/`, Express routers
- **SQL jobs**: `.sql` migration files, ClickHouse materialized view definitions

Use ripgrep / grep to locate them quickly:

```sh
# find HTTP handler registrations
rg 'route\.|router\.|\.get\(|\.post\(|\.put\(|\.delete\(' --type go -l

# find Tauri command handlers
rg '#\[tauri::command\]' --type rust -l

# find event consumers
rg 'subscribe\|consume\|on_message\|handle_event' -l
```

---

## 3. Follow the call path

From each entrypoint, follow the call graph at least **two hops deep** (three for
complex features). Document each hop:

```
entrypoint (path/to/handler.go:42)
  → service method (path/to/service.go:118)
    → DAO / repository (path/to/dao.go:77)
      → SQL query (same file or embedded SQL)
    → external HTTP call (path/to/client.go:33)
```

If a service call crosses a network boundary (HTTP client, gRPC stub, message
publish), that is an integration point — note it separately.

---

## 4. Map shared types and contracts

Shared types are the hidden connective tissue. Search for the domain model (struct,
interface, type alias) that flows through the call chain.

```sh
# Find struct/type definitions for a domain concept
rg 'type Player struct|type PlayerRequest|PlayerDTO' -l

# Find where a type is imported or referenced
rg 'PlayerRequest' --type go
```

For each shared type that crosses a service or schema boundary, ask:
- Who owns the definition? (One service or a shared package?)
- Who are the consumers? (Other services that import or deserialize it?)
- Is the change additive (new optional field) or breaking (rename, removal, type change)?

---

## 5. Locate jobs, ETL, and async paths

Stories that look like simple API changes often have background job implications:

- Aggregation rollups that need refreshing after a schema change
- Kafka/Rabbit consumers that process the same events differently
- Scheduled reports or exports that read the tables being changed
- Cache-warming jobs that depend on the call pattern

Search for job registrations:

```sh
rg 'cron\|scheduler\|worker\|job\|etl\|rollup\|materialize' -l
```

---

## 6. Trace DB schema from both directions

**Top-down** (from code): follow DAO/repository calls to the SQL queries. Note every
table name and column referenced.

**Bottom-up** (from schema): search migration files for the tables identified above.
Confirm the column names, types, and indexes match what the code expects.

```sh
# Find migration files
find . -name '*.sql' -o -name '*migration*' | head -30

# Find all references to a specific table
rg 'MdlGm_tblPlayers\|players_table' --type go
```

---

## 7. Cite evidence

Every finding should have a citation. Use this format:

```
path/to/file.go:42   — brief description of what is at that line
```

For multi-line evidence, quote the minimal relevant excerpt:

```go
// path/to/handler.go:118
func (h *PlayerHandler) GetBalance(ctx context.Context, req *BalanceRequest) ...
```

If you searched and found **nothing**, say that explicitly:

> "Searched for `TransactionEvent` in `crates/otto-dbviewer/` — no matches. This
> consumer does not exist yet; adding it is part of this story."

---

## 8. Screens and UI flows (if applicable)

For frontend work, trace the component tree:

1. Route → page/view component (cite file path)
2. Page component → child components that handle the feature area
3. State management: where does the data come from? (local state, store, server fetch)
4. API calls: which endpoints does the frontend hit? (look for `fetch`, `axios`, `trpc`, etc.)

Note any shared UI primitives (design-system components, form abstractions) that would
be modified, since those have blast radius across other screens.
