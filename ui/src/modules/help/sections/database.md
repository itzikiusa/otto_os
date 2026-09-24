---
id: database
title: Database Explorer
group: Infrastructure
route: database
summary: Browse, query and carefully edit MySQL, PostgreSQL, MongoDB, Redis and ClickHouse databases, with every change reviewed before it runs.
---
## What it's for

Database Explorer is Otto's built-in database client. It lives inside the **Connections** hub: open a database connection there and you get a schema tree, query tabs, a results grid, a visual query builder, a relationship diagram and dashboards. Otto never changes data behind your back. Every edit becomes a statement you read, and production connections ask you to type a confirmation first.

Supported engines: **MySQL**, **PostgreSQL**, **MongoDB**, **Redis** and **ClickHouse**. Connections can go over plaintext, TLS/SSL or an SSH tunnel. Passwords are stored in the macOS Keychain, never in Otto's database.

## Getting started

1. Open **Connections** in the sidebar (`#/database` opens the same page).
2. Choose **New connection** in the header, or the plus button in the sidebar's Connections section. You can also **Import connections** from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster.
3. Pick a kind (mysql, postgres, redis, mongodb, clickhouse), then fill in host, port, user, database and password, or a connection string for MongoDB. Set **Environment** (dev, staging, prod), and turn on **Read-only** if the connection must never write. Add TLS or an SSH tunnel if you need them.
4. Open the connection. A green health dot shows the server version and connect latency once it's ready. Use **Test** in the toolbar to check it again.
5. Expand the schema tree, right-click a table and choose **Select rows** (MongoDB: **Find rows**), or type a query in the **Query** tab and press ⌘↵.

## Everything it can do

**Workbench layout**
- One tab per open connection. Each tab shows its section path, and a **PROD** or **RO** badge with a red or amber rail when the connection is guarded.
- Views per connection: **Query**, **Builder** (SQL engines only), **Structure**, **Diagram** (not for Redis) and **Dashboards**.
- The sidebar has **Schema**, **Saved** and **History**. Saved queries can be searched, renamed, updated in place or saved as new. History has search and **Load more**.
- Group connections into nested sections and drag them between sections.
- **Open beside agents (split)** from a connection's right-click menu docks the explorer next to an agent session.
- The workbench restores itself after a reload: open connections, views, query tabs and the result view of each tab.

**Schema tree**
- Lazy, per engine. SQL engines show database or schema, then tables, then columns. MongoDB shows database, collections, fields. Redis shows database, namespaces, keys.
- Type in the tree's search box to filter objects.
- Right-click a node for: **Set as active database**, **Select rows** / **Find rows** / **Get value**, **Send to editor**, **Open structure**, **Explain with agent**, **Copy name**, **Copy create statement**, **Import into…**, **Refresh**, and for SQL tables **Truncate table…** and **Drop table…** / **Drop view…**. These only fill in a query tab, so you still review and run them yourself.

**Query tabs and running queries**
- Several query tabs per connection. Double-click to rename. Right-click for **Pin tab**, **Rename**, **Close others**, **Close all** and **Open in Builder**. Pinned tabs survive bulk closes and restarts.
- **Run** runs the selection, or the statement under the cursor. **Run all** runs the whole buffer as a batch and shows one result per statement (Result 1…N). The batch stops at the first error and keeps the results so far.
- Autocomplete and syntax highlighting per engine: SQL, Redis commands, or JavaScript-style MongoDB queries.
- **Variables**: `:name`, `{name}` or `{{name}}` placeholders get a value and type (string, number, raw) per tab. Otto asks for any missing value when you run.
- Toolbar: **Save**, **Explain** (a query-plan tree with warnings on costly steps; not for Redis), **Format**, **Ask AI**, **Ask in English**, **Active database**, **Limit** (100 to 50,000, or All; default 1,000), **Timeout** (MySQL), and **Mask** (the daemon redacts emails, tokens and keys before results leave it).
- A bare read without its own `LIMIT` gets the row cap added. When results are capped, a **‹ Prev · Next ›** pager appears. It shows an "unordered" hint when there is no `ORDER BY`.
- **Stop** cancels on the server for MySQL, PostgreSQL, ClickHouse (HTTP transport) and MongoDB (needs the `inprog` and `killop` privileges). For Redis, Stop only drops the request on the client.
- MongoDB accepts shell-style queries, JSON commands, a SQL subset (translated to find or aggregate), and full **mongosh scripts**. Scripts run through the real `mongosh` CLI, and Otto tells you before you run if it isn't installed.

**Results**
- Three views: **Grid** (virtualized, resizable and reorderable columns), **Vertical** (one record per block, nested documents expanded) and **JSON** (one object per row). **Auto** clears your pick for the tab.
- Default view: Vertical for MongoDB, Grid for everything else. Results wider than a per-engine column count switch to Vertical automatically. Set the count in **Settings → Appearance → Database Explorer**: on at 10 columns for MongoDB, off for the other engines. A view you pick on a tab always wins.
- **Search rows…**, a **Filter row** under each header, click-to-sort headers, and a **Row detail** side panel.
- Right-click a cell for **Filter** / **Exclude** by value, **Query by value** or **Add to query** (rewrites the query's `WHERE` or MongoDB filter but doesn't run it), **Go to** a foreign-key target, copy or expand the value.
- **Copy**: TSV, CSV, JSON or column names. **Export**: download CSV or JSON, **Export all rows…**, or **Import file…**.
- The **⋯** menu has **Aggregate pipeline…** (MongoDB stage builder), **Compare two records…**, **Insert row from JSON…**, **Expand JSON cells**, **Send to running agent…** and **Examine with AI**.
- Select rows to **Copy as INSERT**, copy a `WHERE pk IN (…)` predicate, **Compare** exactly 2 records side by side, or delete them.
- MongoDB has a filter bar above `find` results, and Redis has a key filter.

**Editing, always reviewed**
- Double-click a cell to edit it (Vertical view reaches nested fields). Edits wait as pending changes. You can also duplicate, insert or delete rows.
- **Review & apply** builds the statements (`UPDATE`, MongoDB `updateOne` with dotted paths, Redis `SET`/`HSET`/…, ClickHouse `ALTER … UPDATE`) and shows a before-and-after diff. Nothing runs until you press **Run** in the review.
- Editing needs a result Otto can target exactly: a single-table SQL `SELECT` that includes the primary key, a single-collection MongoDB `find` that includes `_id`, or the result of one Redis `GET`, `HGETALL`, `HGET`, `LRANGE`, `SMEMBERS` or `ZRANGE`. The status bar says why when a result isn't editable.

**Visual query builder (Builder view)**
- Add tables to a canvas and drag between columns to join them (INNER, LEFT, RIGHT, FULL OUTER). Single foreign-key joins are drawn for you.
- **Columns**: plain columns, expressions and aggregates: `COUNT`, `COUNT DISTINCT`, `SUM`, `AVG`, `MIN`, `MAX`, plus `GROUP_CONCAT` (MySQL), `STRING_AGG` (PostgreSQL), `uniq` and `groupArray` (ClickHouse). Output names and **Distinct**.
- **Filters** in nested AND / OR groups, with operators that suit the column type. **Group by** fills itself in when you add the first aggregate. **Having** filters on aggregates.
- **Sort** ascending or descending with **NULLs first / last**. MySQL has no native syntax for this, so Otto emulates it. Then **Limit** (default 100) and **Offset**.
- The live SQL shows in the engine's dialect, and problems are explained in words, such as a column that is neither grouped nor aggregated. **Run** stays disabled until they're fixed.
- **Open in editor** copies the SQL to a new query tab. Going the other way, **Open in Builder** on a query tab parses a `SELECT` back into the builder. If the query uses something the builder can't show (UNION, subqueries, CTEs), it says why and leaves the canvas as it was.

**Structure, diagram and dashboards**
- **Structure** shows columns, keys, indexes, foreign keys and the DDL. You can add, edit and drop indexes, including partial ones. **Design** edits a SQL table's columns and generates an `ALTER TABLE` for you to review in a query tab.
- **Diagram** is a read-only relationship diagram with a table picker. MongoDB shows collections without edges.
- **Dashboards**: widgets (table, number, line, bar, area, pie) built from saved statements, with auto-refresh Off, 10s, 30s, 1m or 5m.

**Agents**
- **Ask AI** and **Ask in English** open the **DB Assistant** beside the editor. It's a live agent session that reads the database read-only. Pick the agent before the first question. Proposed queries appear with **Insert** and **Run**. **Summarize** downloads the investigation as Markdown.
- **Examine with AI** (results) and **Explain with agent** (schema tree, Structure) hand the current context to an agent.

**Export and import**
- **Export all rows…** streams the full, uncapped result to a folder on the Mac as CSV, TSV (with or without a header), JSON array or NDJSON.
- **Import file…** loads CSV, TSV, JSON or NDJSON into an existing table or collection in batches. It works for the SQL engines and MongoDB, not Redis.

**Reviewed database changes (MySQL, PostgreSQL)**
- The **Changes** button opens a review workflow. Draft a script with its targets, validate it for an executor, and submit it. A different person approves it. The executor then runs the exact approved revision. Every attempt and outcome is audited.

## Keyboard shortcuts

These work while the Query view has focus. The ⌨ button in the query toolbar lists them too. For app-wide keys, see [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

| Keys | Action |
|---|---|
| ⌘↵ | Run the selection or the statement under the cursor |
| ⇧⌘↵ | Run all statements as one batch |
| Esc | Cancel the running query |
| ⌘S | Save the query |
| ⇧⌘F | Format the query |
| ⇧⌘V | Cycle the results view: Grid → Vertical → JSON |
| ⇧⌘E | Maximize the editor over the results, and back |
| ⌘B | Hide or show the schema sidebar |
| ⌥⌘T | New query tab |
| ⌥⌘W | Close the query tab |
| ⇧⌥⌘W | Reopen the last closed tab |
| ⌥⌘→ / ⌥⌘← | Next / previous query tab |
| ← → Home End | Move between the Query, Builder, Structure, Diagram and Dashboards views (when the view bar has focus) |
| Arrow keys, PageUp, PageDown, Home, End | Move the cell cursor in the grid |
| ↵ | Edit or open the focused cell |
| ⌘C | Copy the focused cell's value |
| ⇧F10 | Open the menu for the focused cell or tree node |
| ↑ ↓ → ← ↵ | Move through and expand the schema tree |
| ⌘↵ | Save in the cell viewer, document editor and review dialog |

## Tips and limits

- **Production and read-only guard.** On a prod or read-only connection, any statement Otto can't prove is a read counts as a write. Before it runs, you type the connection's name. Imports go through the same check. Confirmed writes are audited.
- **Access.** Connections appear to anyone with the Connections or Database feature. Only the owner (root) creates, imports or configures connections. Running queries, exporting and running widgets need Editor or higher in the workspace. Per-connection access rules can limit which databases and operations each person gets. Open them from **Access** in a connection's menu.
- **Ask AI** starts a coding agent, so it also needs agent access.
- There are no multi-query transactions. Each run uses a pooled connection, so a `BEGIN … COMMIT` can't span two runs.
- Redis has no Builder, Diagram, query plan or import. Stop on Redis doesn't cancel on the server.
- Editing a key inside a SQL JSON column rewrites the whole column value. MongoDB edits only touch the fields you changed.
- The auto-Vertical settings, tab picks and pinned tabs are stored in this browser profile. They don't follow you to another device.
- mongosh scripts always count as writes, need `mongosh` on the Mac (`brew install mongosh`), and stop after 30 minutes or 10,000 output lines.
- Reviewed changes don't roll back automatically. If an outcome is uncertain, the target stays locked until someone reconciles it.

## Related

- [Connections](#/walkthroughs/connections)
- [Message Brokers](#/walkthroughs/brokers)
- [Agents](#/walkthroughs/agents)
- [Settings](#/walkthroughs/settings)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
