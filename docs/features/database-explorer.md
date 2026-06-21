# Database Explorer

A TablePlus/Navicat-class database browser built into Otto. It connects to
**MySQL, Redis, MongoDB, and ClickHouse** — over plaintext, TLS/SSL, or an SSH
tunnel — and gives you a lazy schema tree, per-engine autocomplete, multiple
query tabs, a virtualized results grid with approval-gated inline editing, a
visual JOIN builder, a read-only relationship diagram (ERD), Superset-style
ClickHouse dashboards/widgets, and an "examine this with an agent" hand-off.
Every connection runs locally through the `ottod` daemon; **database credentials
are stored in the macOS Keychain, never in the app's database or in the repo.**

> **Where this lives in the code.** Engine: `crates/otto-dbviewer/`. SSH tunnels:
> `crates/otto-ssh/`. Connection profiles: `crates/otto-connections/` +
> `otto_core::domain::Connection`. UI: `ui/src/modules/database/`. The REST
> contract is authoritative in `docs/contracts/api.md` (DB Explorer engine
> access + saved queries/dashboards/widgets).

---

## 1. Overview

The Database Explorer is its own top-level page in Otto. Its left sidebar is a
**connection picker** (grouped into draggable sections) plus a **schema tree**
and a **Schema / Saved / History** switch. The main area is a tab strip —
**Query · Builder · Structure · Diagram · Dashboards** — over the active view.
You can also dock a connection's full explorer *beside an agent* in the Agents
split ("Open beside agents (split)" from a connection's right-click menu), so an
agent and a live DB sit side by side.

Architecture: the Svelte UI talks to `ottod` (loopback `127.0.0.1:7700`) over
HTTP; `ottod` owns the native drivers, resolves each connection's Keychain
secret, establishes any SSH tunnel, dispatches to the per-engine driver, records
query history, and persists saved queries / dashboards / widgets in SQLite.

### Where each piece lives

| Concern | Source |
|---|---|
| Engine orchestration (resolve profile + tunnel, dispatch, history) | `crates/otto-dbviewer/src/service.rs` |
| Engine-agnostic driver contract | `crates/otto-dbviewer/src/driver.rs` |
| Per-engine drivers | `crates/otto-dbviewer/src/drivers/{mysql,redis,mongodb,clickhouse}.rs` |
| Mongo query parsing + SQL→Mongo translation | `crates/otto-dbviewer/src/drivers/{mongo_parse,mongo_sql}.rs` |
| Streaming local-file export sink + formats | `crates/otto-dbviewer/src/export.rs` |
| REST routes | `crates/otto-dbviewer/src/http.rs` (+ `crates/otto-server/src/modules.rs` for examine-with-agent) |
| Profile parsing (host/port/TLS/SSH/`secure`) | `crates/otto-dbviewer/src/config.rs` |
| SSH tunnel (`-L` local forward / `-D` SOCKS5) | `crates/otto-ssh/src/lib.rs` |
| Shared types (engine, schema tree, query req/res, capabilities) | `crates/otto-dbviewer/src/types.rs` |
| UI page + components | `ui/src/modules/database/*.svelte` |
| API client types (mirror the contract) | `ui/src/lib/api/types.ts` |

### Per-engine support matrix

Values are the literal `Capabilities` each driver reports (drives which UI
affordances appear) plus the introspection each driver actually performs.

| Capability | MySQL | Redis | MongoDB | ClickHouse |
|---|:--:|:--:|:--:|:--:|
| `sql` (SQL editor + completion) | yes | no | no | yes |
| `joins` (Builder + ERD edges enabled) | yes | no | no | yes |
| `transactions` | yes | no | yes | no |
| `multi_statement` | yes | yes | no | yes |
| `default_port` | 3306 | 6379 | 27017 | 8123 |
| `query_language` (editor mode) | `sql` | `redis` | `mongo` | `sql` |
| Schema-tree levels (`schema_levels`) | Database → Table → Column | Database → Namespace → Key | Database → Collection → Field | Database → Table → Column |
| Builder tab (visual JOIN) | yes | — | — | yes |
| Diagram tab (ERD) | yes (FK edges) | — (no Diagram) | yes (cards, no edges) | yes (FK edges) |
| Visual JOIN / FK relationships | yes | no | no | yes |
| Inline editing (approval-gated) | yes (single-table SELECT w/ PK) | no | yes (single-collection find by `_id`) | yes (`ALTER … UPDATE`) |
| Streaming export to file | yes (sqlx cursor) | no (buffered fallback) | yes (cursor) | yes (HTTP `FORMAT` splice) |
| Engine-native query cancel | yes (`KILL QUERY`) | no-op | no-op | yes (`KILL QUERY`, HTTP transport) |
| Automatic read `LIMIT` | yes (1000) | n/a (`SCAN` caps) | n/a (`.limit()`/cap) | yes (1000) |

> **Why the matrix matters for the UI.** The **Builder** and **Diagram** tabs are
> gated on `joins`; Redis (no `db:`-rooted tree, no relationships) gets neither a
> Diagram tab nor FK edges; Mongo gets a Diagram with collection cards but **no
> edges** and a "no relationships" hint. The editor language mode and autocomplete
> are driven by `query_language`.

---

## 2. Connecting a database

DB connections are created and managed **from inside the Database Explorer**
(they're intentionally hidden from the general Connections page). In the left
sidebar's **Connections** section, use **New connection** (plus icon) — pick one
of `mysql`, `redis`, `mongodb`, `clickhouse`. Group profiles with **New
section** (folder icon); sections nest into a tree and connections drag between
them. The connection's section path (e.g. `PLATFORM / STG`) is shown on its tab
so the target environment is unmistakable.

A profile stores only **non-secret** parameters in SQLite (`Connection.params`,
JSON); the password/secret is held in the macOS Keychain and the row keeps only
an opaque `secret_ref`. The driver receives a fully `ResolvedConfig` — the
service has already fetched the secret and, if configured, opened the tunnel and
rewritten `host`/`port`.

**Test the connection** with the **Test** button next to the engine chip in the
connection's status row (shows "Testing…" then a green/red dot). It runs a cheap
probe (`SELECT 1` / `PING`) and reports latency and the server version.

### Profile parameters

`params` is per-engine but shares a common shape (parsed in
`otto-dbviewer/src/config.rs`):

```json
{
  "host": "db.internal", "port": 3306, "user": "otto", "db": "shopdb",
  "conn_string": "mongodb+srv://app:{secret}@cluster.mongodb.net/shopdb",
  "tls":  { "mode": "required", "verify": true, "ca_cert": "-----BEGIN…" },
  "ssh":  { "host": "bastion", "port": 22, "user": "ec2-user",
            "identity_file": "/Users/me/.ssh/id_ed25519" },
  "secure": true
}
```

- **`host` / `port` / `user` / `db`** — the endpoint. Port defaults per engine
  (3306 / 6379 / 27017 / 8123) when omitted. (`db` may also be spelled
  `database`.)
- **MongoDB `conn_string`** — a full URI *wins* over host/port. `{secret}` in the
  URI is substituted with the Keychain password at resolve time; a `+srv` URI is
  fully supported (see SSH below).
- **`tls`** — see Plaintext / TLS below.
- **`ssh`** — attach an SSH tunnel (see below).
- **`secure: true`** — shorthand for "TLS required" (used by ClickHouse HTTPS and
  Redis `rediss://`); equivalent to `tls.mode = "required"`.

### Plaintext

The default. No `tls` object and no `secure` flag → the driver connects in the
clear (`TlsMode::Disabled`). Use this for local/dev databases.

### TLS / SSL

Set a `tls` object (or `secure: true`). Modes:

| `tls.mode` | Meaning |
|---|---|
| `disabled` (default) | No TLS. |
| `preferred` | Use TLS if the server offers it; don't fail if it doesn't. |
| `required` | TLS is mandatory. |

Additional `tls` fields: `verify` (default **true** — verify the server cert
against `ca_cert` or system roots), `ca_cert` (inline CA PEM, for private CAs /
RDS bundles), `client_cert` + `client_key` (inline PEM for mutual TLS / X.509
auth), and `server_name` (override the SNI / verification hostname). All three
SQL/document engines honor these:

- **MySQL** — custom CA + client cert/key, verify on/off.
- **Redis** — `rediss://` TLS with an inline custom CA + client cert/key (PEM
  strings, not file paths); `verify:false` flips the `insecure` flag.
- **ClickHouse** — HTTPS over the HTTP transport (custom CA PEM, `verify:false` ⇒
  accept invalid certs) **or** rustls on the native transport (custom root store,
  SNI from `tls.server_name`). Ports are auto-detected: 8123/8443 = HTTP(S),
  9000 = native plaintext, 9440 = native TLS; `"transport":"native"` forces the
  native protocol.
- **MongoDB** — TLS via the connection string or `TlsOptions` (CA file, client
  cert/key, `allow_invalid_certificates` when `verify:false`).

### SSH tunnel

Add an `ssh` object (`host`, `port` default 22, `user`, optional `identity_file`
— the SSH agent / `~/.ssh` is used when no key is given) and the daemon
establishes the tunnel for you, transparently rewriting the endpoint the driver
dials. Tunnels are **cached and kept alive** between operations (idle ones are
evicted after 10 minutes, which kills the `ssh` child), so a stable local
forward keeps the driver's cached connection valid.

Two tunnel shapes are used automatically:

- **MySQL / Redis / ClickHouse** → a **local forward** (`ssh -L`). The driver is
  pointed at `127.0.0.1:<local-port>`, but the *original* host/port is stashed so
  TLS-sensitive drivers (e.g. SNI-routed managed ClickHouse) still present the
  real hostname for SNI/Host while the TCP rides the tunnel.
- **MongoDB** → a **dynamic SOCKS5 proxy** (`ssh -D`). A `mongodb+srv` (Atlas)
  profile resolves its replica-set shard hosts at runtime and Atlas routes by
  SNI, so there is no single endpoint to rewrite; the driver dials each real host
  *through the SOCKS proxy* with the real SNI preserved. This is why Mongo/Atlas
  tunnels behave differently from the other engines.

> **Setup details** — generating keys, `ProxyJump`/bastion configuration, agent
> forwarding, and verifying a tunnel — live in **[SSH connections & SFTP](./connections-ssh-sftp.md)**.
> Here you only *attach* an existing SSH config to a DB profile via the `ssh`
> object above; the explorer reuses the same tunnel machinery.

---

## 3. Schema tree & autocomplete

### Lazy schema tree

The sidebar tree (`SchemaTree.svelte`) loads **lazily** — only the top level is
fetched up front (`GET …/db/schema`); each node's children are fetched on expand
(`POST …/db/schema/children`). A **Filter schema…** box narrows the listing. The
tree shape is per engine:

- **MySQL** — Databases → a **Tables** folder and a **Views** folder → individual
  tables/views → columns (column listing is filterable by name). System schemas
  (`mysql`, `information_schema`, `performance_schema`, `sys`) sort last.
- **ClickHouse** — Databases → tables/views (each tagged with its engine; views
  detected) → columns. System schemas (`system`, `INFORMATION_SCHEMA`) sort last.
- **MongoDB** — Databases → collections → **sampled** top-level fields (inferred
  by `$sample`-ing up to 100 documents; system DBs `admin`/`local`/`config` last).
- **Redis** — logical keyspaces (`db0`, `db1`, …, from `INFO keyspace`, each
  showing its key count) → **namespaces** grouped by the prefix before the first
  `:` → keys. Large keyspaces are sampled (a `SCAN`-based sample, capped) and
  flagged with a "type a prefix to filter" hint.

**Interactions.** Clicking a **database/keyspace** sets it as the *active
database* (rendered bold) so subsequent queries are scoped to it without a
`db.`/`USE` prefix. Clicking a **table/view/collection/key** opens the
**Structure** view. Right-click offers (varies by node kind):

- **Set / Clear active database** (database & Redis keyspace nodes).
- **Select Rows (Limit N)** / **Find Rows (Limit N)** / **Get value (VERB)** — run
  immediately against the object (SQL / Mongo / Redis respectively).
- **Send to SQL Editor / Send to Editor** — drop a starter statement
  (`SELECT * FROM … LIMIT N`, `db.coll.find({})`, the read command for a key) into
  the editor without running it.
- **Open structure**, **Explain with agent** (see §9), **Copy name**, **Refresh**.
- **SQL tables only, destructive (danger-styled): Truncate Table… / Drop Table… /
  Drop View…** — these *pre-fill* a query tab; you must press **Run** yourself
  (and, on a guarded connection, confirm — see §13).

For Redis, the keyspace expander has an inline **filter by prefix…** box
(`RedisKeyFilter.svelte`): typing a prefix and pressing Enter re-scans the
keyspace server-side with `SCAN … MATCH <prefix>*`.

### Per-engine autocomplete

The editor requests suggestions from `POST …/db/completion` (debounced ~120 ms;
or explicitly on **Ctrl+Space**), scoped to the active database and the
selected node. What each engine returns:

- **MySQL** — ~50 SQL keywords, ~65 functions (aggregates, string, date/time,
  math, JSON, casts, conditionals, hashes), plus **live identifiers**: database
  names, table names (scoped to the active DB), and column names (capped).
- **ClickHouse** — ~60 keywords (incl. `FORMAT`, ASOF/ARRAY joins), ~100+
  functions (aggregates, array ops, date/time, `dictGet`, window funcs), plus live
  database / table / column names (capped).
- **MongoDB** — ~60 BSON/aggregation/query **operators** (`$match`, `$group`,
  `$lookup`, `$set`, `$in`, `$regex`, …), the collection **methods** (`find`,
  `aggregate`, `updateOne`, `insertMany`, `createIndex`, …), plus collection names
  and sampled field names.
- **Redis** — the ~80 Redis **commands** (static; no per-keystroke keyspace scan).

---

## 4. Query tabs, running queries, LIMIT, cancel

### Multiple query tabs

The **Query** view (`QueryEditor.svelte`) has a tab strip: each tab is an
independent statement + result. Add tabs with the **+** button; close with the
**×** (hidden when only one tab remains); **double-click** a tab to rename. A tab
shows a pulsing dot while running and a red dot on error. Tab titles
auto-derive (explicit name → the table after `FROM`/`UPDATE`/`INTO` → a verb
snippet → "Query N").

### Running a query

Press **Run** (the toolbar button) or **⌘↵ / Ctrl+Enter**. The toolbar also has:

- **Save** — name and store the statement as a workspace **saved query** (visible
  in the sidebar's **Saved** switch; see §11).
- **Explain** — runs the *real* query plan (`EXPLAIN` / Mongo `explain`).
- **Ask AI** — the examine-with-agent hand-off (see §9).
- **Active database** selector — scope queries to a DB (or a Redis keyspace) so
  you can drop the `db.` prefix.
- **Limit** selector — the automatic row cap (below).
- **Timeout** (ms) — per-statement wall-clock timeout (MySQL applies it as a
  `MAX_EXECUTION_TIME(ms)` hint; blank/0 = no limit).
- **Mask** toggle — when on, the **server** runs result cells through
  `otto_core::redact` before they leave the daemon (emails, tokens, keys); the
  grid shows a **🔒 Masked** badge. Raw values never reach the browser.

### Automatic read `LIMIT`

To avoid scanning a huge table by accident, a read with **no explicit `LIMIT`**
gets a cap appended automatically. The **Limit** dropdown offers **100 / 500 /
1,000 / 5,000 / 10,000 / 50,000 / All** (default **1,000**; "All" = no cap). The
SQL drivers inject `LIMIT n` *conservatively* (`inject_row_limit`):

- Only `SELECT` (incl. `WITH … SELECT` and a parenthesized `(SELECT …)`) is
  rewritten. `SHOW` / `DESC` / `DESCRIBE` / `EXPLAIN` / `EXISTS` return rows but
  **reject** a trailing `LIMIT`, so they're left untouched.
- Statements that already constrain rows (`LIMIT n`), span multiple statements, or
  use a clause where a trailing `LIMIT` would be invalid/ambiguous
  (`FORMAT`/`SETTINGS`/`INTO OUTFILE`/`INTO DUMPFILE`/`FOR UPDATE`/`FOR
  SHARE`/`UNION`/`LIMIT BY`) are left exactly as written.
- An identifier like `rate_limit` is *not* mistaken for a `LIMIT` clause.

Drivers fetch `n+1` rows internally to know whether the result was clipped; the
grid surfaces a **truncated** badge + **Full Export** affordance when it was.
Redis caps by `SCAN` sampling instead (key-list caps, bounded SCAN rounds);
Mongo applies the cap via the cursor's `.limit()`.

### Cancelling a running query

While a query runs, the **Run** button becomes **Stop**. Cancellation is
**engine-native** and runs on a *separate* connection (you can't `KILL` on the
blocked one): the client sends a `query_id` with the run; **Stop** posts that id
to `POST …/db/cancel`, and the service issues:

- **MySQL** → `KILL QUERY <connection-id>` (captured from `CONNECTION_ID()`).
- **ClickHouse** → `KILL QUERY WHERE query_id = '<id>'` (HTTP transport only).
- **Redis / MongoDB** → no engine-native per-query cancel; cancel is a **no-op
  success** (the UI just drops the in-flight request).

Cancelling an unknown / already-finished query, or one that belongs to a
different connection, is always a benign `204` — never an error.

---

## 5. Results grid

Results render in a **virtualized grid** (`ResultsGrid.svelte`) — only the rows
in view are in the DOM, so 100k-row results scroll smoothly. Three view modes:
**Grid** (columnar, default), **Vertical** (one record per block), and **JSON**
(the rows as a JSON array). Complex cells (objects/arrays) show as compact JSON
with click-to-expand; `NULL` renders as a dimmed `∅`. A **Search rows…** box
filters the current view. The footer shows the row count (annotated "(filtered)"
/ "(sorted)" when active) and the query duration in ms, plus the **truncated**
and **🔒 Masked** badges when applicable.

### Client-side filter & sort

Filtering and sorting happen **in the browser** against the loaded rows (no
re-query). Click a column header to cycle **none → ascending → descending →
none** (type-aware: numeric vs string; nulls sort last). Header right-click adds
**Sort ascending/descending**, **Clear sort**, **Filter by {column}…**, and
**Copy column name**. A cell right-click offers **Filter: col = value** /
**Exclude: col ≠ value**, **Expand value**, and **Copy value**. (Column filters
that *re-shape the query* show as chips with a "press Run to apply" hint —
distinct from the client-side row search.)

### Approval-gated inline editing

Editing is **never applied directly.** Otto detects when a result is safely
editable and, if so, lets you **double-click a cell** to edit it (or duplicate /
delete rows). Committing an edit **builds the statement and opens a "Review SQL"
modal**; the statement only runs when you press **Run** there. After a successful
run the grid **re-runs the active query** so values reflect the database (no
optimistic patching).

A result is editable only when Otto can target a row unambiguously:

- **SQL (MySQL/ClickHouse)** — a **single-table `SELECT`** (no JOIN / GROUP BY /
  DISTINCT / UNION / aggregates) **whose primary-key column(s) are in the
  result.** Composite keys are supported; PK columns are read-only. If the table
  has no primary key, or a PK column is missing from the SELECT, editing is
  disabled with an inline reason. ClickHouse edits generate an `ALTER … UPDATE`
  mutation.
- **MongoDB** — a **single-collection** `find` (or translated `SELECT`) **that
  includes `_id`**; edits build an `updateOne` (and duplicates an `insertOne`,
  deletes a `deleteMany`) targeting `_id`.
- **Redis** — not editable from the grid.

The review modal is titled for the operation ("Review UPDATE", "Review DELETE",
"Review INSERT (duplicate row)", "Review updateOne", "Review ALTER … UPDATE
(mutation)", …), shows the editable statement, and warns it will run against the
connection. On a **production / read-only** connection a **typed confirmation**
is required first (see §13).

### Copy & export from the grid

Toolbar actions reflect the **current filtered + sorted view**: **Copy** (TSV to
clipboard), **CSV**, and **JSON** (browser downloads of the in-memory rows).
When a result was capped, a **Full Export** button re-runs the statement
uncapped server-side and downloads it. **Download…** opens the streaming
local-file export (see §10). A **→ Agent** button pastes the query + result into
a running agent.

---

## 6. Visual JOIN builder

The **Builder** tab (`QueryBuilder.svelte`, available for SQL engines only) is a
Navicat-style visual JOIN canvas:

- **Palette** (left): a database selector, a **Filter tables…** search, and a list
  of tables (a **+** adds a card). You can add **any** table from **any** database.
- **Canvas** (center): draggable **table cards** (header = editable alias + source
  name; body = a checkbox per column to include it in the SELECT, with a type hint
  and PK/FK badges). **Draw a join** by dragging from one column's connector
  handle to another column's handle. Click a join edge to open a popover that
  switches the join type (**INNER / LEFT / RIGHT / OUTER …**) or deletes it. A
  pinned **Suggested joins** bar offers FK-derived chips (e.g.
  `orders.user_id → users.id`) you can click to add — FK suggestions are an
  optional helper, not required.
- **Bottom panel**: **Filters** (WHERE rows across any canvas column),
  **Sort** (ORDER BY), a **Limit** input (default 100), an **Expressions** section
  (add `IF` / `CASE` / function columns with `AS` aliases), and the **Generated
  SQL** preview. SQL is generated **live** by walking the edge graph from the
  first-added (base) table; tables left unconnected are flagged ("Not connected —
  excluded from SQL") and dropped.
- **Actions**: **Open in Query** (sends the generated SQL to a new Query tab
  without running) or **Run** (executes it immediately).

---

## 7. ERD / relationship diagram

The **Diagram** tab (`DiagramView.svelte`) is a **read-only** entity-relationship
diagram for the active connection, backed by `POST …/db/schema-graph`:

- A **sidebar table picker** (with **Show all / Show related only / Show none**)
  lets you choose which tables to render; the canvas auto-lays-out the selected
  cards (default ~12 shown) with **PK/FK-marked columns** and **FK relationship
  edges** labeled `from.col → to.col`. Pan and zoom are supported.
- The backend walks the **same lazy schema tree** the UI browses
  (`schema_children` + `object_detail`), so the diagram is engine-agnostic and the
  FK data flows through the normal introspection. It introspects each object's
  detail in parallel (concurrency 8) and **caps** the number of tables: `max_tables`
  defaults to **60** and is clamped to **1..200** server-side. When the schema has
  more tables than the cap, the graph is flagged **truncated** so the UI can prompt
  you to pick a subset.
- **Redis** returns an empty graph (its tree has no `db:`-rooted tables) and gets
  **no Diagram tab**. **MongoDB** renders collection cards but **no edges** (no FK
  metadata) and shows a **"no relationships"** hint. Both report
  `relationships:false`.

---

## 8. ClickHouse dashboards & widgets

The **Dashboards** tab (`Dashboards.svelte`) provides Superset-style live
dashboards. Dashboards, widgets, and saved queries are **workspace-scoped** and
**owner-private** (a non-root user sees only their own; root / workspace-Admin
see all — see §13).

- **Create / rename / delete** a dashboard (an editor role is required to mutate).
- **Auto-refresh cadence** per dashboard: **Off / 10s / 30s / 1m / 5m** (default
  Off). Each widget's interval is jittered ±15% to spread load.
- **Add widget** (sheet seeded from the active query tab): a **Title**, a
  **Statement**, a **Visualization** picker, and — for charts — an **X axis** and
  **Y series** column mapping (each defaulting to "(auto)").

**Visualization types** (`Chart.svelte`, hand-rolled inline-SVG, no chart
dependency) — the `DbViz` set is **`table` · `number` · `line` · `bar` · `area`
· `pie`**:

| Viz | Renders |
|---|---|
| `number` | a single large value (mapped value cell) with a label |
| `table` | a mini virtualized grid (up to ~200 rows per widget) |
| `line` / `area` | X-axis labels + Y series traces (area fills under the curve) |
| `bar` | grouped vertical bars per X category |
| `pie` | slices with percentages (first numeric column = values) |

A widget's **Refresh** button re-runs its stored statement (uncapped to 5000
rows for rendering). Although the dashboards are framed around ClickHouse
analytics, a widget runs against whatever connection it stores, through the same
guarded execution path as a normal query.

---

## 9. Examine with an agent

Otto can hand a schema, an object's structure, or a query result to a coding
agent for explanation. The affordances:

- **Ask AI** (Query toolbar) — sends the statement (and result context) to an
  agent.
- **Explain with agent** (schema-tree node right-click) — sends the object's
  path/kind.
- **Explain** (Structure view) — sends the DDL / object metadata.

Each spawns a **new Agent session** (`POST …/db/explain-with-agent`,
`crates/otto-server/src/modules.rs`) in the connection's workspace using the
workspace/global default provider. The server seeds the session with a
database-expert prompt that embeds the connection name + engine, your question
(or a default "describe each table/field, relationships, indexing,
normalization, possible issues, then suggest useful queries"), and the
**`content`** the UI already had — so **no extra DB round-trip** is made just to
build the prompt. Requires workspace **Editor**.

---

## 10. Export

Two export paths, both gated at the same role as running a query (workspace
**Editor**; global connections: root):

**A. Browser download (`POST …/db/export`).** Re-runs the statement **uncapped**
and returns the whole result as a file attachment the browser downloads —
`csv` or `json`. The whole result is buffered in RAM (fine for the UI's "Full
Export" of a modestly-clipped result). This backs the grid's **CSV / JSON**
buttons (which export the in-memory view) and **Full Export** (uncapped re-run).

**B. Streaming to a local file (`POST …/db/export-to-path`).** For results too
big to pull into the browser. The **Download…** dialog picks a **format**, a
destination directory (via the shared folder picker; the last format + directory
are remembered), and an optional row cap. The daemon **streams** the result
row/chunk-by-chunk straight to a `BufWriter` on disk so **daemon memory stays
bounded** regardless of result size:

- **MySQL** — the sqlx row cursor (never `fetch_all`).
- **MongoDB** — iterating the `find`/`aggregate` cursor (columns fixed from the
  first document, `_id` first).
- **ClickHouse (HTTP)** — appends an explicit `FORMAT` and splices the server's
  own bytes through. **For a tunnelled ClickHouse this writes *your* local path —
  not a server-side `INTO OUTFILE` on the tunnel host.**
- **Redis** (and any engine without a native row stream, incl. ClickHouse's JSON
  *array* shape and the native transport) — falls back to buffering the full
  result (logged with a warning).

**Formats** (`ExportFormat`, `format` field, snake_case on the wire) and the
labels shown in the dialog:

| `format` | Label | Output |
|---|---|---|
| `csv` (default) | CSV | comma-separated, **no header**, RFC-4180 quoting |
| `csv_with_names` | CSV (with header) | CSV with a header row |
| `tsv` | TSV | tab-separated, no header (tabs/newlines → spaces) |
| `tsv_with_names` | TSV (with header) | TSV with a header row |
| `json` | JSON (array) | one JSON array of row objects |
| `ndjson` | NDJSON | one JSON object per line |

`local_path` is a path **on the daemon host** (leading `~` expands to the daemon
user's home); an existing directory becomes `<dir>/export.<ext>`, otherwise it's
the full file path (parent created). ClickHouse maps these to native `FORMAT`
names (`CSV`, `CSVWithNames`, `TabSeparated`, `TabSeparatedWithNames`,
`JSONEachRow`); the JSON *array* has no single-pass `FORMAT`, so it buffers.
Only **row-returning reads** are exportable — a write/DDL is rejected, and a
write on a guarded connection is blocked outright (export has no confirmation
path). The response reports `{local_path, rows, bytes, duration_ms}`.

---

## 11. API / contract reference

`docs/contracts/api.md` is authoritative; `ui/src/lib/api/types.ts` mirrors it.

**Engine access** (`/connections/{id}/db/*`) — reads = `ws viewer`, live-DB
execution/cancel/export = `ws editor` (global connections: root):

| Method & path | Purpose |
|---|---|
| `POST …/db/test` | connectivity probe (latency + server version) |
| `GET …/db/capabilities` | engine capability flags |
| `GET …/db/schema` | top-level schema tree (roots) |
| `POST …/db/schema/children` | lazy-expand a node (`{node}`; Redis `filter`) |
| `POST …/db/object` | object detail (columns / keys / indexes / DDL / extra; opt-in `approx_row_count`) |
| `POST …/db/schema-graph` | read-only ERD (`{schema, max_tables?}`; default 60, clamp 1..200) |
| `POST …/db/query` | run a statement (`RunQueryReq` incl. `max_rows`, `node`, `query_id`, `timeout_ms`, `mask`, `confirm_write`) |
| `POST …/db/cancel` | engine-native cancel of an in-flight `query_id` (204) |
| `POST …/db/completion` | autocomplete suggestions |
| `GET …/db/history` | recent query history (per-user for non-root) |
| `POST …/db/explain-with-agent` | spawn an agent to explain a schema/result |
| `POST …/db/export` | uncapped result as a CSV/JSON browser download |
| `POST …/db/export-to-path` | stream an uncapped result to a local file (selectable format) |

**Saved queries / dashboards / widgets** — workspace-scoped lists under
`/workspaces/{wid}/db/*`, item routes keyed by row id (reads `ws viewer`,
mutations `ws editor`; by-id reads/mutations also require owner / ws-Admin /
root): `…/db/saved-queries`, `…/db/dashboards`, `…/db/widgets`,
`DELETE /db/saved-queries/{qid}`, `GET|PATCH|DELETE /db/dashboards/{id}`,
`PATCH|DELETE /db/widgets/{id}`, and `POST /db/widgets/{id}/run`.

`RunQueryReq` notes: `query_id` (client-generated, enables cancel); `timeout_ms`
(MySQL `MAX_EXECUTION_TIME` hint; others = context deadline); `mask` (server-side
redaction); `confirm_write` (typed-confirmation acknowledgement for a guarded
connection). `ExportToPathReq` = `{statement, node?, format?, local_path,
max_rows?}` → `ExportToPathResp` = `{local_path, rows, bytes, duration_ms}`.

---

## 12. Capabilities & limitations

- **Engines**: MySQL, Redis, MongoDB, ClickHouse. `ssh`/`custom` connection kinds
  are **not** browsable data sources (an SSH connection is a terminal, not a DB).
- **Builder & Diagram** require `joins` (SQL engines). Redis has no Diagram; Mongo's
  Diagram shows cards but no edges.
- **Inline editing** needs an unambiguously addressable row (single-table SELECT
  with PK / single-collection find with `_id`); Redis is read-only in the grid.
- **Cancellation** is engine-native only for MySQL and ClickHouse (HTTP
  transport). Redis/Mongo cancel is a client-side drop.
- **Streaming export** is native for MySQL/Mongo/ClickHouse-HTTP; Redis,
  ClickHouse-native, and the ClickHouse JSON-*array* format buffer in RAM.
- **ClickHouse native transport** caveats: per-query database scoping and per-query
  cancellation are **not** wired on the native (9000/9440) transport — use the HTTP
  transport (8123/8443) for those.
- **MongoDB** supports a mongosh-style shorthand, raw JSON command documents, **and**
  a SQL→Mongo translation for a single-base-collection `SELECT` (WHERE / ORDER BY /
  LIMIT / COUNT / GROUP BY / aggregates / INNER & LEFT equi-joins → `$lookup`).
  RIGHT/FULL/CROSS joins, non-equi joins, subqueries, UNION, HAVING, and DISTINCT
  are not translated.
- **`approx_row_count`** (MySQL `information_schema.table_rows`) is an InnoDB
  estimate (can be wildly off) and is opt-in because it costs an extra query.

---

## 13. Security & guards

- **Secrets in the Keychain.** A connection profile stores only non-secret params
  in SQLite plus an opaque `secret_ref`; the password/secret lives in the macOS
  Keychain (`otto-keychain`) and is fetched only when resolving a connection.
  Mongo `conn_string` `{secret}` substitution happens at resolve time.
- **Loopback by default.** The daemon listens on `127.0.0.1` and runs on the
  user's machine, so `export-to-path` writes the **daemon host's** real local disk.
- **Production / read-only write guard.** A connection is *guarded* when its
  environment is **Prod** **or** it is explicitly **read-only** (`is_write_guarded`).
  A guarded connection **refuses any statement classified as a write/DDL** unless
  the request carries `confirm_write`. Classification is **conservative — unknown
  is treated as a write** (so a novel/unparseable statement errs toward refusing):
  SQL is a read only if every statement starts with `SELECT/SHOW/DESC/DESCRIBE/
  EXPLAIN/WITH/USE` (`SET` counts as a write); Redis is a read only if every line
  is in a vetted read-command set (`CONFIG`/`CLIENT`/`EVAL` are writes); Mongo is a
  read only for recognizable `find`/`aggregate`/`count`/`distinct` shapes without
  `$out`/`$merge`. A rejected write returns a `409` tagged `write_blocked:` so the
  UI prompts for a **typed confirmation** (type the connection name). Confirmed
  writes on a guarded connection are **audited** (`db.write_confirmed`). The UI also
  shows danger styling — a red rail / **PROD** badge for production, an amber rail /
  **RO** badge for read-only — and a banner.
- **`explain:true` can't bypass the gate.** A raw write sent with `explain:true` is
  still blocked (the SQL drivers execute by statement text); only a genuine
  `EXPLAIN`-prefixed statement classifies as a read.
- **Execution role.** Running a query, cancelling, exporting, and **running a
  dashboard widget** all require workspace **Editor** (global connections: root) —
  a widget runs arbitrary stored SQL, so it can't be triggered by a mere Viewer.
- **Ownership.** Saved queries / dashboards / widgets are owner-private: a
  same-workspace co-member who learns a resource id still can't read/mutate/run it
  unless they are the owner, a workspace **Admin**, or **root**.
- **Automatic read `LIMIT`** (§4) and the **schema-graph table cap** (§7) bound
  accidental full-table scans / fan-out. **Streaming export** (§10) bounds daemon
  memory.
- **Server-side masking.** The `mask` flag redacts cells in the daemon before they
  leave it (raw values never reach the browser).

> Roles and ownership tiers (Viewer < Editor < Admin, per-session isolation,
> impersonation) are documented in **[Multi-user RBAC](../MULTI-USER-RBAC.md)**.

---

## 14. Troubleshooting

- **"connection kind … is not a browsable database"** — the profile is an `ssh` or
  `custom` kind; only MySQL/Redis/MongoDB/ClickHouse are browsable.
- **TLS handshake stalls on a tunnelled managed DB** — the driver preserves the
  real hostname for SNI through the tunnel; for SNI-routed managed ClickHouse make
  sure `tls.server_name` (or the original host) is correct. For Atlas, confirm the
  tunnel is the **SOCKS5** kind (it is, automatically, for `mongodb+srv`).
- **MongoDB tunnel won't connect** — Mongo uses `ssh -D` (SOCKS5), not `-L`; the
  bastion must allow dynamic forwarding. See
  **[SSH connections & SFTP](./connections-ssh-sftp.md)**.
- **A write is rejected with `write_blocked:`** — the connection is Prod or
  read-only; confirm the write (type the connection name) or run it from an
  unguarded profile. Remember the classifier treats *unknown* statements as writes.
- **`SHOW` / `DESCRIBE` returns no `LIMIT`** — by design; the auto-`LIMIT` only
  touches `SELECT`-shaped reads.
- **Inline editing is disabled** — the result isn't uniquely addressable: include
  the table's primary key (or `_id` for Mongo) and use a single-table/-collection
  query (no JOIN/GROUP BY/aggregate).
- **Stop didn't kill the query (Redis/Mongo)** — those engines have no native
  per-query cancel; the client just drops the request. MySQL/ClickHouse-HTTP issue
  a real `KILL QUERY`.
- **Export wrote to the wrong machine** — `export-to-path` writes the **daemon
  host's** disk; for a remotely-running daemon the file lands there, not on your
  laptop. Use the browser-download export (CSV/JSON) to pull to the client.
- **Approximate row counts look wrong** — `approx_row_count` is an InnoDB estimate;
  run `SELECT COUNT(*)` for an exact number.

---

## 15. Related docs

- **[SSH connections & SFTP](./connections-ssh-sftp.md)** — set up SSH tunnels /
  bastions / keys (then attach via the profile's `ssh` object), and the SFTP file
  browser over the same SSH auth.
- **[Usage & cost](./usage-and-cost.md)** — token/cost tracking, including agents
  spawned by **Examine with an agent**.
- **[Message brokers](./message-brokers.md)** — the Kafka viewer, which shares the
  SSH-tunnel machinery (`otto-ssh`) with the Database Explorer.
- **[Multi-user RBAC](../MULTI-USER-RBAC.md)** — roles, per-session isolation, and
  the ownership tiers that gate saved queries / dashboards / widgets.
- **API contract**: `docs/contracts/api.md` (DB Explorer engine access + saved
  queries/dashboards/widgets).
