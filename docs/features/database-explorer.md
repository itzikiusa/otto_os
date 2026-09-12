# Database Explorer

A TablePlus/Navicat-class database browser built into Otto. It connects to
**MySQL, PostgreSQL, Redis, MongoDB, and ClickHouse** — over plaintext, TLS/SSL,
or an SSH tunnel — and gives you a lazy schema tree, per-engine autocomplete,
multiple query tabs (with true multi-statement batches, auto-pagination, and a
structured query-plan panel), a virtualized results grid with approval-gated
inline editing, a visual JOIN builder, a read-only relationship diagram (ERD),
Superset-style ClickHouse dashboards/widgets, and an "examine this with an
agent" hand-off.
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
and a **Schema / Saved / History** switch — **Saved** queries can be renamed,
updated in place ("Save as new" forks), and searched; **History** has a search
box and a **Load more** pager. The main area is a tab strip —
**Query · Builder · Structure · Diagram · Dashboards** — over the active view.
The workbench **restores itself on reload** (open connection tabs, the selected
connection, each connection's active view, its open query tabs — pinned ones
first — and the **result view** each tab was left in come back), and a
connection tab shows a green **health dot** with server version + connect
latency once ready. Results render in one of three views — **Grid**, **Vertical**
(one record per block) or **JSON** — and the view is chosen for you unless you
pick one: **MongoDB opens in Vertical**, the SQL engines and Redis in Grid, and
any result **wider than N columns** (a user setting, default 10) switches to
Vertical so a record is readable without a horizontal scroll (§5).
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
| Per-engine drivers | `crates/otto-dbviewer/src/drivers/{mysql,postgres,redis,mongodb,clickhouse}.rs` |
| Mongo query parsing + SQL→Mongo translation | `crates/otto-dbviewer/src/drivers/{mongo_parse,mongo_sql}.rs` |
| String-aware statement splitter (multi-statement + write-guard) | `crates/otto-dbviewer/src/split.rs` |
| Query-plan normalizer (per-engine EXPLAIN → `DbQueryPlan`) | `crates/otto-dbviewer/src/plan.rs` |
| File import (batched INSERT / Mongo `insertMany`) | `crates/otto-dbviewer/src/import.rs` |
| Streaming export writer + formats | `crates/otto-dbviewer/src/export.rs` |
| REST routes | `crates/otto-dbviewer/src/http.rs` (+ `crates/otto-server/src/modules.rs` for examine-with-agent) |
| Profile parsing (host/port/TLS/SSH/`secure`) | `crates/otto-dbviewer/src/config.rs` |
| SSH tunnel (`-L` local forward / `-D` SOCKS5) | `crates/otto-ssh/src/lib.rs` |
| Shared types (engine, schema tree, query req/res, capabilities) | `crates/otto-dbviewer/src/types.rs` |
| UI page + components | `ui/src/modules/database/*.svelte` |
| Results orchestrator (toolbar, view switch, filter/sort, pager) + the three views | `ResultsGrid.svelte` → `GridView.svelte` / `VerticalView.svelte` / `JsonView.svelte` |
| Edit flow (pending changes, review modal, doc editor, cell viewer) + per-engine statement builders | `EditFlow.svelte.ts`, `edit-{sql,mongo,redis}.ts` |
| View-mode precedence, tab/pin persistence, connection view memory | `ui/src/lib/stores/database.svelte.ts` (`effectiveViewMode`) |
| API client types (mirror the contract) | `ui/src/lib/api/types.ts` |

### Per-engine support matrix

Values are the literal `Capabilities` each driver reports (drives which UI
affordances appear) plus the introspection each driver actually performs.

| Capability | MySQL | PostgreSQL | Redis | MongoDB | ClickHouse |
|---|:--:|:--:|:--:|:--:|:--:|
| `sql` (SQL editor + completion) | yes | yes | no | no | yes |
| `joins` (Builder + ERD edges enabled) | yes | yes | no | no | yes |
| `transactions` | no | no | no | no | no |
| `multi_statement` | yes | yes | yes | yes | yes |
| `cancel` (server-side per-query cancel) | yes | yes | no | yes | yes |
| `explain` (query-plan panel) | yes | yes | no | yes | yes |
| `default_port` | 3306 | 5432 | 6379 | 27017 | 8123 |
| `query_language` (editor mode) | `sql` | `sql` | `redis` | `mongo` | `sql` |
| Schema-tree levels (`schema_levels`) | Database → Table → Column | Schema → Table → Column | Database → Namespace → Key | Database → Collection → Field | Database → Table → Column |
| Builder tab (visual JOIN) | yes | yes | — | — | yes |
| Diagram tab (ERD) | yes (FK edges) | yes (FK edges) | — (no Diagram) | yes (cards, no edges) | yes (FK edges) |
| Visual JOIN / FK relationships | yes | yes | no | no | yes |
| Inline editing (approval-gated) | yes (single-table SELECT w/ PK) | yes (single-table SELECT w/ PK) | yes (`GET`/`HGETALL`/`HGET`/`LRANGE`/`SMEMBERS`/`ZRANGE` results) | yes (single-collection find by `_id`; nested fields via dotted `$set`/`$unset`/`$rename`) | yes (`ALTER … UPDATE`) |
| Streaming export to file | yes (sqlx cursor) | yes (sqlx cursor) | no (buffered, 100k cap) | yes (cursor) | yes (HTTP `FORMAT` splice) |
| File import | yes (batched `INSERT`) | yes (batched `INSERT`) | no | yes (`insertMany`) | yes (batched `INSERT`) |
| Query-plan source | `EXPLAIN FORMAT=JSON` | `EXPLAIN (FORMAT JSON)` | — (no plan) | `explain` (queryPlanner) | `EXPLAIN json=1` (+plain fallback) |
| Triggers browse | yes (`SHOW CREATE TRIGGER`) | — | — | — | — |
| Engine-native query cancel | yes (`KILL QUERY`) | yes (`pg_cancel_backend`) | no-op | yes (`$currentOp` comment tag + `killOp`; needs `inprog`/`killop`) | yes (`KILL QUERY`, HTTP transport) |
| Automatic read `LIMIT` | yes (1000) | yes (1000) | n/a (`SCAN` caps) | n/a (`.limit()`/cap) | yes (1000) |
| Keyset pagination (Next = `_id > last`) | no (`OFFSET`) | no (`OFFSET`) | n/a | yes (`_id` sort only; Prev is offset-based) | no (`OFFSET`) |
| Type fidelity (typed round-trip) | native | native | strings | ObjectId / Date / Decimal128 / Long > 2⁵³ / UUID / Binary / Timestamp | native |
| Default result view | Grid | Grid | Grid | **Vertical** | Grid |

> **Honesty notes.** `transactions` is `false` for **every** engine: the explorer
> acquires each query from a connection pool, so there is no pinned session to hold
> a `BEGIN…COMMIT` open (the flag was `true` for MySQL/Mongo before with nothing
> behind it). `multi_statement` is `true` for all five (Mongo already ran
> `;`-separated scripts). `cancel` labels whether **Stop** hits the engine or is a
> client-side drop — MongoDB is now engine-side too (§4), Redis remains the one
> client-side drop; `explain` gates the **Explain** query-plan button (hidden for
> Redis).

> **Why the matrix matters for the UI.** The **Builder** and **Diagram** tabs are
> gated on `joins`; Redis (no `db:`-rooted tree, no relationships) gets neither a
> Diagram tab nor FK edges; Mongo gets a Diagram with collection cards but **no
> edges** and a "no relationships" hint. The editor language mode and autocomplete
> are driven by `query_language`; the **Explain** button by `explain`; the **Stop**
> tooltip by `cancel`.

---

## 2. Connecting a database

DB connections are created and managed **from inside the Database Explorer**
(they're intentionally hidden from the general Connections page). In the left
sidebar's **Connections** section, use **New connection** (plus icon) — pick one
of `mysql`, `postgres`, `redis`, `mongodb`, `clickhouse`. Group profiles with **New
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

**Opening and closing.** A "+" New-connection button sits in the sidebar's
tab strip itself so it's reachable from every side tab (Schema/Saved/History
empty states also carry "Browse connections" / "New connection" actions, and the
sidebar lands on Connections whenever no tab is open). **Closing a connection
tab disconnects for real**: the UI drops the tab from its persisted workbench
(a close clicked while tabs are still restoring wins over the restore) and calls
`POST /connections/{id}/db/close`, which cancels the connection's in-flight
queries, closes the driver's cached pool, and drops the SSH tunnel. A closed tab
stays closed across app restarts.

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
  (3306 / 5432 / 6379 / 27017 / 8123) when omitted. (`db` may also be spelled
  `database`.) A **PostgreSQL** connection targets one database and is browsed
  **by schema**: its tree top level is that database's schemas (`public` first,
  `pg_catalog`/`information_schema` hidden) and the active-database selector maps
  to `SET search_path`. TLS maps `TlsMode` → `PgSslMode`; the tunnel is a plain
  `-L` local forward (single endpoint, like MySQL); a session time zone applies
  via `SET TIME ZONE` on connect. Importing a `jdbc:postgresql://` URL
  (DataGrip/DBeaver) maps to this engine.
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

Add an `ssh` object (`host`, `port` default 22, optional `user`, optional
`identity_file` — the SSH agent / `~/.ssh` is used when no key is given; an
empty `user` is resolved the way your terminal resolves it: `~/.ssh/config`
`User`, else your local login name) and the daemon
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

- **MySQL** — Databases → a **Tables** folder and a **Views** folder (always),
  plus **Procedures** / **Functions** / **Triggers** folders when the database has
  any (each with a count) → individual objects → columns (column listing is
  filterable by name). A routine leaf's detail shows its parameters + `SHOW
  CREATE` body; a **trigger** leaf's detail is `SHOW CREATE TRIGGER` plus
  `{timing, event, table}`. System schemas (`mysql`, `information_schema`,
  `performance_schema`, `sys`) sort last.
- **PostgreSQL** — the connection's **schemas** (`public` first;
  `pg_catalog`/`information_schema` hidden) → **Tables** / **Views** /
  **Materialized Views** / **Functions** folders → objects → columns. Object
  detail synthesizes DDL from the catalog (`pg_get_viewdef` / `pg_get_functiondef`
  / a composed `CREATE TABLE`) with PK / FK / index markers.
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

### Structure view: indexes (new / edit / drop)

The **Indexes** block of a table's or collection's Structure view
(`StructureView.svelte`) lists every index; clicking a row expands its full
engine-native definition (Mongo's `listIndexes` doc, Postgres' `pg_get_indexdef`
string). Each row also carries **Edit** and **Drop** actions, and the block header
a **New index** button.

Like every other mutating action in the explorer, these **prepare** a statement in
a new query tab — nothing is applied until you press **Run** (and, on a guarded
connection, confirm — see §13).

- **New index** — pick the fields from a **searchable list** (selected fields pin
  to the top in key order — the order is what makes a compound index useful;
  Mongo offers nested paths like `address.city`, and a collection routinely
  samples 60+ of them), optionally name it, add a **Condition** (below) and mark
  it **Unique**, and get a `CREATE INDEX` / `db.coll.createIndex(…)`.
- **Edit** — no engine alters an index in place, so this prepares a **drop +
  recreate**: the builder opens pre-filled from the existing index and emits the
  DROP followed by the CREATE. What the columns-and-unique summary can't carry is
  taken from the stored definition — Mongo key **directions** (`-1`, `"text"`,
  `"2dsphere"`) and options (`sparse`, `expireAfterSeconds`,
  `partialFilterExpression`, `collation`), and the SQL access method (Postgres
  `USING gin`, MySQL `FULLTEXT`/`SPATIAL`). The table is unindexed on that key
  between the two statements.
- **Drop** — one statement, spelled per engine:
  `db.coll.dropIndex("name")` (Mongo), ``ALTER TABLE `t` DROP INDEX `name` ``
  (MySQL/ClickHouse), `DROP INDEX "name"` (Postgres). The primary key gets its own
  form — MySQL exposes it as an index literally named `PRIMARY`
  (`ALTER TABLE … DROP PRIMARY KEY`), and a Postgres index that *backs* the PK
  constraint can only be dropped via `ALTER TABLE … DROP CONSTRAINT`.
- MongoDB's **`_id_`** index can't be changed or dropped, so both actions are
  disabled on that row.

#### Conditions (partial indexes)

A **Condition** narrows *which* rows or documents an index covers — the classic
case being a UNIQUE index that should only apply where a field is actually set.
Add one or more rows (ANDed together), each a field plus one of two operators:
**exists** and **in**. Values for `in` are comma-separated and typed, so
`a, 3, true` becomes `["a", 3, true]` — a stringified `3` would never match.

The three engines that can express this differ sharply, and the generated
statement says which you got:

| Engine | Emitted |
|--------|---------|
| **MongoDB** | native `partialFilterExpression: { f: { $exists: true } }` / `{ $in: […] }` |
| **PostgreSQL** | native `CREATE INDEX … WHERE "f" IS NOT NULL` |
| **MySQL** | **emulated** — see below |
| **ClickHouse** | no equivalent; the section is hidden |

**MySQL has no partial index** — there is no `CREATE INDEX … WHERE`. The
condition is emulated with a **functional key part**: the leading column is
wrapped in a `CASE` that yields `NULL` outside the condition, so those rows never
enter the b-tree (and, for a UNIQUE index, don't collide — MySQL permits many
NULLs). The statement carries a comment saying so, and the builder warns inline,
because the optimizer only uses such an index for queries written with the *same*
expression:

```sql
-- MySQL has no partial indexes; this emulates one with a functional key
-- part. The optimizer uses it ONLY for queries written with the same CASE
-- expression.
CREATE UNIQUE INDEX `idx_orders_status`
  ON `orders` ((CASE WHEN `status` IN ('paid', 'shipped') THEN `status` END));
```

Two round-trip guarantees when **editing** an index that already has a condition:

- **MongoDB** — filter terms this builder doesn't model (`$gt`, `$type`, …) are
  **preserved verbatim** and called out, so an edit never silently widens what the
  index covers. Adding a condition also drops `sparse`, because MongoDB rejects an
  index that sets both it and `partialFilterExpression` (the builder warns).
- **PostgreSQL** — the catalog normalizes predicates (`= ANY (ARRAY[…])`, `::text`
  casts), so an existing `WHERE` is kept as **text** rather than parsed back into
  rows; it is shown, and replaced only if you add conditions of your own.

### Per-engine autocomplete (smart & context-aware)

The editor requests suggestions from `POST …/db/completion` (debounced ~120 ms;
or explicitly on **Ctrl+Space**). It also opens **automatically** when you type a
`.` (member access — `alias.`, `db.coll.`, or a Mongo embedded `x.`) or a space
right after a clause keyword (`from `, `where `, `and `, …). Each request carries
the text **before** and **after** the cursor; the daemon parses it to decide what
you actually want and ranks the results so the useful ones come first.

**What "smart" means:**

- **Intent by clause.** After `FROM`/`JOIN`/`UPDATE`/`INSERT INTO` you get
  **tables/collections**; after `WHERE`/`AND`/`OR`/`ON`/`HAVING`/`SELECT`-list you
  get **columns**; a qualified `alias.` / `table.` narrows to that one table's
  columns. The `FROM` list is resolved from the *whole* statement, so a column in
  the `SELECT` list still knows its table even though `FROM` comes after the cursor.
- **Indexes first.** Columns are ranked by index membership — **PRIMARY KEY →
  UNIQUE → other index → plain** — via a `score` the editor maps to CodeMirror's
  `boost`. So in a `WHERE` the columns you'll actually filter on float to the top.
  (ClickHouse uses `is_in_primary_key` / data-skipping indexes; the ORDER-BY/PK
  columns lead.)
- **Mongo, including embedded fields.** `db.` → collections; `db.coll.` → methods
  (`find`/`aggregate`/…); inside `find({ … })` or a pipeline stage at a **key**
  position → that collection's fields, **index fields first** (from `listIndexes`),
  then sampled fields. Embedded paths are first-class: an index on `address.city`
  offers **`address`** first, and once you type `address.` it offers
  **`address.city`** / `address.zip` (nested paths are sampled up to depth 3). A
  dotted key is inserted **quoted** (`{ "address.city": … }`) because the bare form
  is a Mongo parse error; a simple key inserts bare. When no database is selected
  yet (a connection with no default db), Mongo falls back to the first user database
  so `db.` still lists collections instead of nothing.
- **Mongo in the SQL dialect, too.** The Mongo runner also accepts
  `SELECT … FROM <coll> WHERE …` (translated to `find`/`aggregate`), so completion
  speaks SQL there as well: `FROM` → **collections**, `WHERE` → that collection's
  **fields, index-first** (an embedded `WHERE address.` still refines to
  `address.city`) — exactly like MySQL/ClickHouse, never the collection list.
- **Never worse than before.** Anything the heuristic doesn't recognise falls back
  to the old keyword + function + table dump, so completion always returns
  *something*.

**Fast & cached.** The schema each engine needs (databases, tables/collections,
index-ranked columns) is introspected **once per connection + database** and
reused for every subsequent keystroke — only the cheap text analysis runs per
keystroke. The snapshot is held **until you refresh** (the schema tree's
**Refresh** button / context-menu — which now also clears this cache via
`POST …/db/completion/refresh`), with a ~5-minute TTL as a self-heal safety net.
Mongo samples a collection's fields only when that collection is in context, then
caches them.

What each engine contributes to the candidate pool:

- **MySQL** — ~50 SQL keywords, ~65 functions, plus live database / table /
  index-ranked column names from `information_schema`.
- **ClickHouse** — ~60 keywords, ~100+ functions, plus live database / table /
  column names from `system.*` (primary-key & skip-index columns ranked first).
- **MongoDB** — ~60 query/aggregation **operators**, the collection **methods**,
  collection names, and index-first sampled field **paths** (incl. embedded).
- **Redis** — the ~80 Redis **commands** (static; no per-keystroke keyspace scan).

---

## 4. Query tabs, running queries, LIMIT, cancel

### Multiple query tabs

The **Query** view (`QueryEditor.svelte`) has a tab strip: each tab is an
independent statement + result. Add tabs with the **+** button (⌥⌘T); close with
the **×** (hidden when only one tab remains) or ⌥⌘W; **double-click** a tab to
rename; ⇧⌥⌘W reopens the last closed tab. A tab shows a pulsing dot while
running and a red dot on error. Tab titles auto-derive (explicit name → the
table after `FROM`/`UPDATE`/`INTO` → a verb snippet → "Query N"). Each tab also
remembers its **result view** (Grid / Vertical / JSON — §5): the view is stored
with the tab, so switching tabs never resets it, and **⇧⌘V** cycles
Grid → Vertical → JSON on the active tab.

**Right-click a tab** for **Pin tab / Unpin tab**, **Rename**, **Close others**
and **Close all**. A **pinned** tab (📌 glyph, no ×) survives *Close others* /
*Close all* and refuses × / ⌥⌘W with an "Unpin to close" toast until you unpin
it — the two bulk actions say "(keeps pinned tabs)" whenever a pin exists.
Pinned tabs group at the front of the strip; pins, tab order, statements,
variables and view picks are all persisted per connection (`otto_db_tabs`), so
a pinned tab is still there after a reload or an app restart. Closed tabs'
running queries are cancelled (server-side too) exactly like a single close.

### Running a query

Press **Run** (the toolbar button) or **⌘↵ / Ctrl+Enter** — this runs the
**selection** if there is one, else the **statement under the cursor**. To run
the whole buffer as one **multi-statement batch**, use **Run all** (appears when
the buffer holds >1 statement) or **⇧⌘↵**: the backend splits the script with a
string/comment-aware splitter and returns **one result set per statement** — the
grid shows a segmented **Result 1…N** switcher (tooltip = the statement; a red
dot marks an errored entry). Execution stops at the first failure and the
completed results are still returned. While a query is in flight the grid dims
under a **running overlay** (elapsed seconds + Cancel); **Esc** cancels. When a
bare SELECT was auto-limited, the footer grows a **pager**
(`‹ Prev · rows a–b · Next ›`) that re-runs server-side with `OFFSET`/`skip`
(with an "unordered" hint when the statement has no `ORDER BY`; an explicit
user `LIMIT` disables it; on MongoDB **Next** walks the collection by `_id`
instead of `skip` — see §5). The **⌨** toolbar popover lists every shortcut
(⌘↵, ⇧⌘↵, ⌘S save, ⇧⌘F format, ⇧⌘V cycle the results view, Esc cancel, ⌥⌘→/←
switch query tabs, ⌥⌘T new tab, ⌥⌘W close tab, ⇧⌥⌘W reopen). The toolbar also
has:

- **Save** — name and store the statement as a workspace **saved query** (visible
  in the sidebar's **Saved** switch; see §11).
- **Explain** — opens the **query-plan panel** (§4c): a normalized EXPLAIN tree
  with red badges on costly access patterns. Hidden for Redis (no plan surface);
  falls back to the raw-`EXPLAIN`-into-the-grid path if a plan can't be normalized.
- **Format** — beautify the statement. Engine-aware: SQL engines use sql-formatter,
  **Mongo** uses a structural JS/JSON re-indenter (`db.coll.op({…})` / pipelines —
  the SQL formatter can't parse it), Redis (one-line commands) is a no-op.
- **Ask AI** — the examine-with-agent hand-off (see §9).
- **Variables** — a statement may reference `:name`, `{name}` or `{{name}}`
  placeholders; the bar under the editor holds a value + type (`string` quoted /
  `number` raw / `raw` verbatim) per variable, per tab. **Running with a missing
  value prompts for it** (a small **Query variables** dialog, one row per
  unfilled name, Enter runs) instead of failing; opening a **saved query** that contains
  placeholders opens the same prompt straight away. Values are persisted with
  the tab.
- **Active database** selector — scope queries to a DB (or a Redis keyspace) so
  you can drop the `db.` prefix.
- **Limit** selector — the automatic row cap (below).
- **Timeout** (ms) — per-statement wall-clock timeout (MySQL applies it as a
  `MAX_EXECUTION_TIME(ms)` hint; blank/0 = no limit).
- **Mask** toggle — when on, the **server** runs result cells through
  `otto_core::redact` before they leave the daemon (emails, tokens, keys); the
  grid shows a **🔒 Masked** badge. Raw values never reach the browser.

**Syntax highlighting.** The editor colours your query by engine: **SQL** for
MySQL/ClickHouse, a small **Redis** highlighter for Redis, and **JavaScript** for
Mongo — native Mongo queries (`db.coll.find({…})`, aggregate pipelines, BSON
literals) are JS-shaped, so JS highlighting reads naturally (the SQL subset Mongo
also accepts still renders fine).

### MongoDB: running full mongosh scripts

Pasting a real **mongosh script** — variables, functions, control flow,
`db.getSiblingDB(…)`, `print(…)` (think a seed/bootstrap `.js` file) — into a
Mongo query tab runs it through the **actual `mongosh` CLI** rather than the
explorer's command parser: the statement is detected as a script
(`types::looks_like_mongosh_script` — line-anchored and comment-aware, so a
multi-line `insertOne({…})` body or JSON command is never hijacked), written to
a temp `.js` file, and executed as `mongosh <uri> --quiet --file …` against the
**same resolved endpoint** the native driver uses — including the SSH tunnel
(the SOCKS5 proxy rides the URI as `proxyHost`/`proxyPort`, which mongosh
honors), TLS material, credentials and the selected database. The shell's
stdout comes back as the result grid (one line per row); a non-zero exit
surfaces the tail of both streams as the error.

- A script **always classifies as a write** — arbitrary JS can't be proven
  read-only — so the production / read-only typed-confirmation gate applies
  before anything runs, and the MCP read-only surface refuses scripts outright.
- Needs the `mongosh` CLI on the daemon's PATH (`brew install mongosh`) — the
  same binary Otto's Mongo **terminal sessions** spawn. The moment the editor
  detects a script it shows a **notice bar** ("mongosh script detected — Run
  executes it through the real mongosh CLI") and probes `GET /db/mongosh` for
  the binary: ✓ + version when present, or the install hint inline **before**
  you run — a missing binary is never a silent failure or a surprise run-time
  error.
- **Run sends the WHOLE buffer** for a script. A script is indivisible — the
  usual "run the statement under the cursor" split (on top-level `;`) would cut
  real JavaScript into fragments that share no scope, so ⌘↵ used to execute the
  leading `const …;` alone (silently doing nothing) or a middle fragment that
  died on a variable defined earlier in the file. Select a region first if you
  genuinely want to run only part of it; "Run all" is hidden for scripts because
  the plain Run already is one.
- Scripts are capped at 30 minutes and 10,000 output lines (truncated badge
  beyond).

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
- **PostgreSQL** → `pg_cancel_backend(<pid>)`.
- **ClickHouse** → `KILL QUERY WHERE query_id = '<id>'` (HTTP transport only).
- **MongoDB** → every `find` / `aggregate` / `count` issued with a `query_id`
  is tagged with a **comment** (`otto:<query_id>`); **Stop** runs
  `$currentOp` on a second connection, matches `command.comment` against that
  tag and issues `killOp` for each matching op id. Best-effort: it needs the
  `inprog` and `killop` privileges — without them the cancel logs a warning and
  succeeds as a client-side drop (the query keeps running on the server until it
  finishes). Writes and full mongosh **scripts** are not tagged, so they cannot
  be cancelled server-side.
- **Redis** → no engine-native per-query cancel; cancel is a **no-op success**
  (the UI just drops the in-flight request).

Cancelling an unknown / already-finished query, or one that belongs to a
different connection, is always a benign `204` — never an error.

---

## 5. Results grid

Results render through one orchestrator (`ResultsGrid.svelte` — toolbar, view
switch, client-side filter/sort, selection bar, pending-edits bar, footer pager)
over three interchangeable views of the same rows:

- **Grid** (`GridView.svelte`) — a **virtualized** columnar table: only the rows
  in view are in the DOM, so 100k-row results scroll smoothly. Complex cells
  (objects/arrays) show as compact JSON with click-to-expand (or **Expand JSON**
  for all of them); `NULL` renders as a dimmed `∅`.
- **Vertical** (`VerticalView.svelte`) — **one record per block**, each field on
  its own `field: value` row, nested documents rendered as nested rows (below).
  The Postgres `\x` / ClickHouse `FORMAT Vertical` way of reading a wide or
  ragged record.
- **JSON** (`JsonView.svelte`) — **one JSON object per row**: each row is its own
  bordered, numbered, copyable block rather than a single big array, so row
  boundaries are unmistakable.

The server payload is identical in all three — only the rendering differs. The
Vertical and JSON views are not virtualized (one document can be enormous on its
own), so they draw records in **batches** (a "Show N more" button grows the
window; 500 is the hard ceiling per view). A **Search rows…** box filters the
current view in every mode. The footer shows the row count (annotated
"(filtered)" / "(sorted)" when active) and the query duration in ms, plus the
**truncated** and **🔒 Masked** badges when applicable.

### View mode & auto-Vertical

The **Grid · Vertical · JSON** switch in the results toolbar is a per-tab
choice, and the view a result actually renders in is resolved by
`effectiveViewMode` (`ui/src/lib/stores/database.svelte.ts`) with this
precedence — first match wins:

1. **Your explicit pick for this tab** (the switch, or **⇧⌘V** which cycles
   Grid → Vertical → JSON → Grid) — stored on the tab, persisted with it, never
   reset by switching tabs or reloading.
2. **The auto-Vertical threshold** — a result with **more than N columns**
   renders in Vertical (N is *Settings → Appearance → Database Explorer*,
   default **10**, `0` = never). It never overrides a pick made on the tab.
3. **The view remembered for the connection** — the last explicit pick made on
   any tab of that connection (persisted in its `otto_db_view` entry next to its
   main/side pane), so the next tab you open there starts the same way.
4. **The engine default** — **MongoDB → Vertical**; MySQL, PostgreSQL,
   ClickHouse and Redis → Grid.

The switch's tooltip names the rule in force ("your pick for this tab",
"auto: 12 columns > 10", "remembered for this connection", "engine default").
Once you have picked a view on a tab a dimmed **Auto** chip appears next to the
three modes; it clears **that tab's pick only** (the connection memory stays)
and its tooltip says which view that would restore. The threshold lives in the
browser profile (localStorage), like the other Appearance preferences — it is
not synced through `PUT /settings`. Dashboard widget mini-grids and the AWS
Athena view mount the same component without a query tab and keep a local
Grid-first switch.

### Vertical view: nested documents

In Vertical view a sub-document or array renders as **nested `field: value`
rows** (indented under its parent), **expanded by default** — you read
`meta.brand_id` where it sits instead of clicking into a `{…}` summary. Large
documents stay responsive through a **node budget** (default 400 nodes per
record, breadth-first, arrays in 50-item chunks) rather than the old
"collapse past depth 2 or 20 keys" cutoff: branches that don't fit the budget
render as a one-line summary you can open; **Expand all** (warns first when the
records currently DRAWN are estimated past 20,000 nodes in total — the estimate
is summed over the batch, not per record), **Collapse all** and **Reset** sit in
the view's header. Expansion is **sticky by path across records**: opening
`items.0.meta` in record 1 keeps `items.*.meta` open in every other record on
screen (array indices are normalised, so a pick on the first element applies to
its siblings). The JSON view uses the same plan and the same three buttons. The
expansion state resets when the result's columns change.

### Editing in Vertical view

**Double-click any value** — top-level or nested — to edit it inline. The editor
is **typed** (ObjectId · Date · number · long · decimal · bool · null · string ·
JSON, pre-selected from the current value; ObjectId = 24 hex, Date parses to
ISO, long/decimal validated) and the edit **parks as a pending change** exactly
like a grid cell (amber row marker, pending-edits bar, **Review & apply**). A
field's right-click menu adds **Set null**, **Delete field ($unset)…**,
**Rename field…**, **Add field here…**, **Copy path** and **Copy value**; the
record's **⋯** menu adds **Add field…**, **Insert document…**, **Copy as JSON**,
**Export…**, **Compare** and **Replace document (JSON)…** (the whole-document
editor). On **MongoDB** the review builds one `updateOne` per touched document
with **dotted paths** — `{"$set": {"items.0.qty": 7}, "$unset": {"legacy": ""},
"$rename": {"old": "new"}}` — so concurrent edits to other fields are left
alone (the old whole-document `replaceOne` is now only the explicit *Replace
document* action). Conflicting parks are refused with a toast (rename a path
then edit the new name; a `$set` under a path being `$unset` replaces the
earlier change). On the **SQL engines** the same double-click produces the
existing `UPDATE … SET col = v` path (top-level column), and an edit *inside* a
JSON column rewrites that whole column value in the same `UPDATE` — no separate
code path. The review modal shows a **diff table** (path · before → after, per
operation) above the editable statement for every change, including
*Replace document* (computed by flattening old vs new).

### Compare two records

Select **exactly two rows** (checkboxes in the grid, or **Compare with…** from
two record ⋯ menus in Vertical/JSON) and press **Compare** in the **selection bar**
— the grid's path, it appears as soon as rows are selected. The toolbar's
**Compare…** button is the Vertical/JSON path: it is rendered only outside Grid
and enabled at exactly two. Either opens a side-by-side **diff modal**: one row per
leaf path, classed *same / changed / only-left / only-right*, an "only
differences" toggle (on by default), and **Copy as JSON patch** (`set`/`unset`
operations that turn the left record into the right one). Read-only — it is a
review aid, not an edit path.

### Mongo filter bar

On a MongoDB connection, when the active statement is a single `find(…)`, a
one-line **filter bar** sits above the results: type a `{ field: value }`
object (column chips insert `"col": ` at the caret), choose **Replace** (the
new object becomes the `find` filter) or **AND** (merged into the existing
one), and press **Run** / Enter — the statement is rewritten in the editor and
re-run. It reuses the same `find(` splicer as *Query by value* / *Add to
query*; an unbalanced object is rejected with a toast; the bar hides for
aggregates, multi-statement buffers and scripts.

### Aggregate pipeline builder

**Pipeline…** in the results toolbar (MongoDB only) opens a stage-by-stage
builder: add `$match` / `$project` / `$sort` / `$limit` / `$skip` / `$group` /
`$unwind` / `$lookup` / `$addFields` / `$count` (or a raw stage), reorder or
delete stages, and watch the formatted `db.<collection>.aggregate([...])`
preview update live. **Insert** writes it into the editor; **Run** inserts and
runs. The collection defaults to the one the current result was read from
(else the object selected in the tree, else a text box); the draft is kept per
connection in localStorage so closing the dialog loses nothing.

### Single-document import/export

From a record's **⋯** menu (Vertical/JSON): **Copy as JSON** copies the
pretty-printed document, **Export…** downloads it as `<collection>-<id>.json`.
The toolbar's **Insert from JSON…** (and the record menu's **Insert
document…**) opens the document editor in insert mode — paste or type a JSON
object and **Save** builds `db.<collection>.insertOne(<doc>)` (SQL: an
`INSERT INTO … VALUES` from the keys that match result columns) through the
usual review modal. Whole-result import/export (files, streaming) is §10/§10b.

### Client-side filter & sort

Filtering and sorting happen **in the browser** against the loaded rows (no
re-query). Click a column header to cycle **none → ascending → descending →
none** (type-aware: numeric vs string; nulls sort last). Header right-click adds
**Sort ascending/descending**, **Clear sort**, **Filter by {column}…**, and
**Copy column name**. A cell right-click offers **Filter: col = value** /
**Exclude: col ≠ value**, **Expand value**, and **Copy value**. (Column filters
that *re-shape the query* show as chips with a "press Run to apply" hint —
distinct from the client-side row search.)

#### Query by value / Add to query

A cell right-click also offers two actions that rewrite the **active query text**
directly (for **MySQL, ClickHouse and MongoDB** — the engines with a filterable
query language):

- **Query by value: `col` = value** — sets the query's `WHERE` clause (Mongo: the
  `find(…)` filter) to `col = value`, replacing any existing one.
- **Add to query: AND `col` = value** — ANDs `col = value` onto the existing
  `WHERE` (Mongo: merges the key into the filter object). e.g. from
  `… WHERE a = 'x'`, adding `b = 'y'` gives `… WHERE a = 'x' AND b = 'y'`.

Both **write the rebuilt query into the editor _and_ the clipboard** and **never
run it** — you review it and press **Run**. They reuse the same top-level
`WHERE`-splicer as the quick-filter chips (`splitStatement`/`rewriteWhere`),
preserving `ORDER BY`/`LIMIT`/`GROUP BY`/ClickHouse `SETTINGS`·`FORMAT` tails,
parenthesizing an existing `OR` before the `AND`, and `IS NULL` for null cells.
The items are hidden when the active statement can't be safely filtered (a
non-`SELECT`, a multi-statement buffer, or a Mongo aggregate) and for engines
without a query language (Redis). The base for the rewrite is the statement that
PRODUCED the rows on screen (`QueryTab.ran_statement`), not the editor's live
text — the grid always describes what you are looking at. That is also why
editability, the "Copy as INSERT" target and "Export all rows…" no longer
re-derive (and visibly flicker, one `object_detail` probe per keystroke) while
you type the next query.

### Approval-gated inline editing

Editing is **never applied directly.** Otto detects when a result is safely
editable and, if so, lets you **double-click a cell** to edit it (or duplicate /
delete rows). Committing a cell **parks it as a pending change** (the cell shows
the draft with an amber marker) so you can edit **several fields — in one row or
across rows — before anything is prepared**. A bar above the footer counts the
pending changes; **Review & apply** builds **one statement per touched row with
every changed column in a single `SET`** (`$set` for Mongo; several rows become
a multi-statement batch) and opens the "Review SQL" modal — the statement only
runs when you press **Run** there. **Discard** drops the drafts; re-editing a
parked cell back to its stored value un-parks it. After a successful run the
grid **re-runs the active query** so values reflect the database (no optimistic
patching — and any drafts left over are invalidated by the refresh).

A result is editable only when Otto can target a row unambiguously:

- **SQL (MySQL/ClickHouse)** — a **single-table `SELECT`** (no JOIN / GROUP BY /
  DISTINCT / UNION / aggregates) **whose primary-key column(s) are in the
  result.** Composite keys are supported; PK columns are read-only. If the table
  has no primary key, or a PK column is missing from the SELECT, editing is
  disabled with an inline reason. ClickHouse edits generate an `ALTER … UPDATE`
  mutation.
- **MongoDB** — a **single-collection** `find` (or translated `SELECT`) **that
  includes `_id`**; edits build an `updateOne` targeting `_id` whose `$set` /
  `$unset` / `$rename` carry **dotted paths** for nested fields (a cell edit in
  the grid is a top-level `$set`; the Vertical view reaches any depth — see
  *Editing in Vertical view*), duplicates an `insertOne`, deletes a
  `deleteMany`. Typed values round-trip as EJSON (`{"$oid"}`, `{"$date"}`,
  `{"$numberDecimal"}`, `{"$numberLong"}` …).
- **Redis** — the result of a single `GET`, `HGETALL`, `HGET`, `LRANGE`,
  `SMEMBERS` or `ZRANGE` is editable: a value edit reviews as
  `SET k "v" KEEPTTL` (the key's TTL is preserved), `HSET k field v`,
  `LSET k i v` or `ZADD k score member`; a row delete as `DEL` / `HDEL` /
  `SREM` / `ZREM`; a hash's **Add field** as `HSET`. Set members can be removed
  but not edited in place, and the list **index** column is read-only. Editing a
  **hash field** or a **zset member** name is a **rename**, so it reviews as two
  commands — `HDEL` + `HSET` (the value is carried over) / `ZREM` + `ZADD` (the
  score is) — and is skipped with a note when the row cap cut the reply between
  the name and its value. Anything else Redis returns (`SCAN`, `KEYS`, `INFO`,
  multi-line scripts) stays read-only.

The review modal is titled for the operation ("Review UPDATE", "Review DELETE",
"Review INSERT (duplicate row)", "Review updateOne", "Review ALTER … UPDATE
(mutation)", …), shows a **diff table** of every change (path · before → after)
above the editable statement, and warns it will run against the connection. On
a **production / read-only** connection a **typed confirmation** is required
first (see §13).

### Copy & export from the grid

Toolbar actions reflect the **current filtered + sorted view**: **Copy** (TSV to
clipboard), **CSV**, and **JSON** (browser downloads of the in-memory rows).
When a result was capped, a **Full Export** button re-runs the statement
uncapped server-side and downloads it. **Download…** opens the streaming
local-file export (see §10). A **→ Agent** button pastes the query + result into
a running agent. **Import file…** opens the file→table import dialog (see §10b).

### Foreign-key navigation

When a result is a single-table SELECT (the same detection that gates inline
editing), cells in a **foreign-key column** get a right-click **"→ Go to
`<ref_table>`"** action. It opens a new query tab running
`SELECT * FROM <ref_table> WHERE <ref_col> = <value> LIMIT 1` (every column of a
**composite** FK is ANDed using that row's values), so the grid feels relational
instead of flat — the DataGrip/DBeaver staple. It reuses the foreign-key metadata
already fetched for edit-ability (no extra round-trip); a `NULL` foreign-key value
has no navigable target.

### Generate SQL from selected rows

With one or more rows selected in an editable single-table result, the selection
bar offers **Copy as INSERT** (opens `INSERT INTO <table> (cols) VALUES (…)` for
every selected row in a new tab — **not** run) and **WHERE pk IN (…)** (copies a
primary-key predicate to the clipboard; composite keys become an OR of per-row
ANDs). Both reuse the same identifier/literal escaping as inline edits.

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
- **Edit widget** — the same sheet reopens via the card's edit button / "Edit…"
  menu entry (title, statement, viz, mapping; the connection is fixed at
  creation and shown as a chip on the card). Widget delete asks for
  confirmation. A failed auto-refresh shows an inline "showing last data" pill
  and backs off exponentially instead of toasting per tick.

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

### Ask in English (verified NL→SQL)

The Query toolbar's **Ask in English** button opens an input where you describe
what you want in plain language; **Generate** drafts a query and returns it only
**after it has been validated with `EXPLAIN`** against the live schema — never a
hallucinated, unrunnable guess. The server-side loop (`POST …/db/nl-to-sql`):

1. Asks the configured drafter (the agent/LLM) for a candidate, grounding it in a
   compact schema summary.
2. **Rejects any write/DDL before it touches the engine** — this feature is
   **read-only by contract**; a non-read draft is discarded and retried.
3. Validates the candidate with `EXPLAIN` (a read, so it's guard-safe even on a
   Prod/read-only connection) and, on an engine error, feeds that error back to
   the drafter for a **bounded retry** (default 3, max 4 attempts).

On success the panel shows the read-only **SQL**, a collapsible **"Validated with
EXPLAIN"** plan, the attempt count, and any warnings, plus **Insert into editor**
(places the SQL without running) and **Run** (inserts + runs via the normal
path). **No draft ever reaches the editor until it has a valid plan.** It is
**Editor**-gated (validation runs `EXPLAIN` live) and **unavailable for Redis**
(no plan surface). If the server has no drafter wired the panel shows "Ask AI is
not set up on this server"; if the retry loop is exhausted it shows the last
engine error verbatim.

---

## 10. Export

Two export paths, both gated at the same role as running a query (workspace
**Editor**; global connections: `Database:Edit`):

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

## 10b. Import

The mirror of the export: import a **local file on the daemon host** into an
existing SQL table (`POST …/db/import`). Launch it from a table node's
right-click **"Import into…"** (prefills the table name) or the results-grid
toolbar **Import file…**. The dialog picks a **format**, a **file** (via the
shared file picker — the extension auto-selects the format), the **target
table**, and a **batch size** (default 500, clamped 1..=5000), then streams the
result.

- **Formats.** `csv` / `tsv` take the **first row as the header** (column names);
  `ndjson` (one JSON object per line) and `json` (an array of objects) carry keys
  per record — columns are the **union of keys** in first-seen order, and a
  missing key becomes `NULL`.
- **Batched INSERTs.** Rows are parsed and inserted as
  `INSERT INTO <table> (cols) VALUES (…),(…)` of `batch_size` rows each, with
  engine-aware quoting (identifiers and string literals escape `'` **and `\`**
  per dialect — MySQL and ClickHouse honor backslash escapes), so no single
  statement is unbounded and nothing is injection-prone. The file is capped at
  **100 MiB**, and a delimited row whose cell count doesn't match the header is
  **rejected with its row number** (it is never silently padded or clipped).
  The dialog's Cancel button aborts a running import mid-stream.
- **Same write guard.** Every batch runs **through the normal guarded `run`
  path** — there is no parallel guard — so masking/history apply and a
  **Prod/read-only connection refuses the import** until you type the connection
  name (the identical typed-confirmation flow a write query uses); the client
  then retries with `confirm_write`. The dialog streams an `application/x-ndjson`
  line and ends with **"Imported N rows in B batches"** (then re-runs the active
  query / refreshes the table's structure) or the error.
- **Role gate.** Workspace **Editor** (global connections: `Database:Edit`), the same as
  running a query / exporting.
- **v1 scope: SQL engines only** (MySQL/ClickHouse). Mongo `insertMany` and Redis
  are explicit follow-ups (see §12).

---

## 11. API / contract reference

`docs/contracts/api.md` is authoritative; `ui/src/lib/api/types.ts` mirrors it.

**Engine access** (`/connections/{id}/db/*`) — reads = `ws viewer`, live-DB
execution/cancel/export = `ws editor` (global connections: `Database:Edit`):

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
| `POST …/db/close` | tear down the connection's server-side state — cancel in-flight queries, close the cached driver pool, drop the SSH tunnel; idempotent (`ws viewer`; fired on tab close) |
| `POST …/db/mcp-query` | read-only query surface for agents over MCP — writes/DDL refused before any driver call, rows capped at 200, masking forced on (`ws viewer`) |
| `POST …/db/completion` | context-aware, index-first completion (`{prefix, suffix?, database?, node?}` → `{items:[…]}` with per-item `score`) |
| `POST …/db/completion/refresh` | drop the cached completion snapshot for the connection (204) — wired to the schema **Refresh** button |
| `GET …/db/history` | recent query history (per-user for non-root) |
| `POST …/db/explain-with-agent` | spawn an agent to explain a schema/result |
| `POST …/db/export` | uncapped result as a CSV/JSON browser download |
| `POST …/db/export-to-path` | stream an uncapped result to a local file (selectable format) |
| `POST …/db/query-plan` | normalized EXPLAIN tree (`DbQueryPlan`; MySQL/PG/CH/Mongo; Redis 400) |
| `POST …/db/import` | file → table/collection (SQL batched `INSERT`s / Mongo `insertMany`) |

**Saved queries / dashboards / widgets** — workspace-scoped lists under
`/workspaces/{wid}/db/*`, item routes keyed by row id (reads `ws viewer`,
mutations `ws editor`; by-id reads/mutations also require owner / ws-Admin /
root): `…/db/saved-queries`, `…/db/dashboards`, `…/db/widgets`,
`PATCH|DELETE /db/saved-queries/{qid}` (PATCH = rename / update-in-place),
`GET|PATCH|DELETE /db/dashboards/{id}`, `PATCH|DELETE /db/widgets/{id}`, and
`POST /db/widgets/{id}/run`.

`RunQueryReq` notes: `query_id` (client-generated, enables cancel — on MongoDB
it also becomes the `otto:<query_id>` comment tag `killOp` matches on);
`timeout_ms` (MySQL `MAX_EXECUTION_TIME` hint; others = context deadline);
`mask` (server-side redaction); `confirm_write` (typed-confirmation
acknowledgement for a guarded connection); `offset` (server-side paging for
auto-limited statements); `cursor?` (EJSON of the last `_id` of the previous
page — **keyset paging**, MongoDB only, see below). A batch response puts the
first statement's result at the top level with the rest in `more_results[]`
(each with a `statement` preview and an `errored` flag); `auto_limited` carries
the applied cap so the UI can page; `next_cursor?` (EJSON of the last `_id`
returned) is present only when the page was keyset-eligible **and** truncated.
`ExportToPathReq` = `{statement, node?, format?, local_path, max_rows?}` →
`ExportToPathResp` = `{local_path, rows, bytes, duration_ms}`.

**Keyset pagination (MongoDB).** A `find` is keyset-eligible when it has no
explicit `.limit()`, no sort (or `{_id: 1}`), and no `_id` key in its filter.
For such a find the driver forces `sort: {_id: 1}` on *every* page and, when a
`cursor` is sent, ANDs `{_id: {$gt: <cursor>}}` onto the filter instead of
`skip`-ping — so **Next** in the footer pager costs the same on page 200 as on
page 1 and never repeats or skips a document that moved. **Prev** stays
offset-based (`skip`), as does any non-eligible find (a `cursor` sent with one
is ignored and the request falls back to `offset` silently).

**Type fidelity (MongoDB).** Result cells carry BSON types as EJSON sentinels —
`{"$oid"}`, `{"$date"}`, `{"$numberDecimal"}`, `{"$numberLong": "<digits>"}`
(only when |n| > 2⁵³, smaller integers are plain JSON numbers), `{"$uuid"}`,
`{"$binary": {"base64", "subType"}}`, `{"$timestamp": {"t", "i"}}` — and every
one of them is decoded back on the way in (`decode_ejson`), so an edited
ObjectId / Date / Decimal128 / Long / UUID / Binary / Timestamp round-trips
without loss. The UI renders them as `ObjectId("…")`, `ISODate("…")`,
`NumberLong("…")`, `UUID("…")`, `BinData(n, "…")`, `Timestamp(t, i)`.

---

## 12. Capabilities & limitations

- **Engines**: MySQL, PostgreSQL, Redis, MongoDB, ClickHouse. `ssh`/`custom`
  connection kinds are **not** browsable data sources (an SSH connection is a
  terminal, not a DB).
- **Builder & Diagram** require `joins` (SQL engines). Redis has no Diagram; Mongo's
  Diagram shows cards but no edges.
- **Inline editing** needs an unambiguously addressable row (single-table SELECT
  with PK / single-collection find with `_id`). **Redis editing** covers the
  result of one `GET` / `HGETALL` / `HGET` / `LRANGE` / `SMEMBERS` / `ZRANGE`
  only (set members: delete, not edit; a hash field / zset member name edit is a
  rename — `HDEL`+`HSET` / `ZREM`+`ZADD`); everything else Redis returns stays
  read-only. A nested edit inside a **SQL JSON column** rewrites the **whole
  column value** in the `UPDATE` (SQL has no dotted `$set`), so concurrent
  edits to other keys of that same column are overwritten — Mongo's dotted
  paths don't have this limitation.
- **Cancellation** is engine-native for MySQL, PostgreSQL (`pg_cancel_backend`),
  ClickHouse (HTTP transport) and MongoDB (`$currentOp` comment tag + `killOp`,
  which needs the `inprog` + `killop` privileges — without them the cancel
  degrades to a client-side drop with a daemon-side warning, and full mongosh
  scripts / writes are never tagged). Redis cancel is a client-side drop (the
  `cancel` capability flag labels this in the UI).
- **Pagination**: MongoDB pages forward by keyset (`_id > last`) only for a
  `find` with no explicit limit, no sort other than `{_id: 1}` and no `_id`
  filter; **Prev** and every other engine/statement page by `OFFSET` / `skip`.
- **Result view**: the auto-Vertical column threshold (and the rest of the
  Appearance settings) is stored **per browser profile** (localStorage), not in
  the daemon's `PUT /settings` — it does not follow you across devices, and a
  second browser profile starts at the default (10). Per-tab picks and the
  per-connection memory are localStorage too; closing a connection tab forgets
  its remembered view along with its main/side pane (the tab picks survive).
- **Transactions**: the `transactions` capability is `false` for every engine —
  queries run on pooled connections, so there is no pinned session to hold a
  `BEGIN…COMMIT` open across runs.
- **Streaming export** is native for MySQL/PostgreSQL/Mongo/ClickHouse-HTTP;
  Redis, ClickHouse-native, and the ClickHouse JSON-*array* format buffer in RAM
  with a **100,000-row hard cap** (a clear error beyond, never an OOM).
- **ClickHouse native transport** caveats: per-query database scoping and per-query
  cancellation are **not** wired on the native (9000/9440) transport — use the HTTP
  transport (8123/8443) for those.
- **MongoDB** supports a mongosh-style shorthand, raw JSON command documents,
  **full mongosh scripts** (below), **and**
  a SQL→Mongo translation for a single-base-collection `SELECT` (WHERE / ORDER BY /
  LIMIT / COUNT / GROUP BY / aggregates / INNER & LEFT equi-joins → `$lookup`).
  RIGHT/FULL/CROSS joins, non-equi joins, subqueries, UNION, HAVING, and DISTINCT
  are not translated. A translated `SELECT` counts as a **read** for the
  production / read-only write-guard, like the `.find(…)` spelling it compiles to
  (before 2026-07-27 the guard only knew the Mongo spellings, so every `SELECT`
  on a prod/read-only connection came back `write_blocked` — a 409).
- **`approx_row_count`** (MySQL `information_schema.table_rows`) is an InnoDB
  estimate (can be wildly off) and is opt-in because it costs an extra query.
- **File import** (§10b) covers the SQL engines (MySQL/ClickHouse/PostgreSQL —
  batched `INSERT`s with engine-aware identifier quoting) **and MongoDB**
  (`insertMany` batches; CSV/TSV cells are type-coerced). Redis import is not
  supported. The result is a single terminal NDJSON line `{done, rows, batches}`
  (only `export-to-path` streams progress ticks). The file is buffered before
  parsing (fine for typical imports; the batching already bounds per-statement
  size) — a streaming line-by-line parser for very large files is a follow-up.
- **Ask in English (NL→SQL)** (§9) is **read-only by contract** and
  **unavailable for Redis** (no `EXPLAIN`/plan surface); it requires a drafter
  wired on the server (otherwise the panel says so).

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
  dashboard widget** all require workspace **Editor** (global connections: `Database:Edit`) —
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
- **Stop didn't kill the query (Redis)** — Redis has no native per-query cancel;
  the client just drops the request. MySQL/ClickHouse-HTTP issue a real
  `KILL QUERY`, PostgreSQL `pg_cancel_backend`.
- **Cancel didn't stop my Mongo query** — server-side cancel needs the
  connection's user to hold `inprog` (to see the op in `$currentOp`) and
  `killop`; without them Stop only drops the HTTP wait and the daemon logs a
  warning. Full **mongosh scripts** and **writes** are never tagged, so they can
  only be dropped client-side. Grant the privileges (or the `clusterMonitor` +
  `hostManager` roles) and re-run.
- **My result opened in Vertical, I wanted the grid** — MongoDB defaults to
  Vertical, and any engine switches to Vertical past the column threshold
  (*Settings → Appearance → Database Explorer*, default 10; `0` disables it).
  Pick **Grid** on the tab (or ⇧⌘V) — the pick sticks to that tab and is
  remembered for the connection; the **Auto** chip returns to automatic.
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
