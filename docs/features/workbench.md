# Workbench

Scratch files for side scripts, snippets and notes, with strong formatting and
previews and a **full edit history**. It replaces a separate editor (VS Code,
a notes app) for the "prepare a script, tidy it, run it somewhere" loop. A file
can go straight to the DB Explorer's **Run on…** (multi-target / parameter
sweep), the API client, a terminal or agent session, or the Vault.

- Route: `#/workbench`. Sidebar entry **Workbench** (Build group). ⌘K: "Go to Workbench".
- RBAC: rides the **Agents** feature (reads = View, writes = Edit). Files are
  **per user** inside a workspace. Other members never see your files.
- Code: UI `ui/src/modules/workbench/`, API client `ui/src/lib/api/workbench.ts`,
  store `crates/otto-state/src/workbench.rs` (migration `0165_workbench.sql`),
  routes `crates/otto-server/src/routes/workbench.rs`.

## Walkthrough

1. **New file** creates `Untitled N` with language **Auto-detect** and opens it.
   The language is detected from the extension first (`.json`, `.md`, `.sql`,
   `.d2`, `.mmd`, `.csv`, …), then from the content (JSON, markup, Mermaid
   keywords, D2 edges, SQL keywords, `curl`, Markdown, CSV, YAML, shebangs). The
   language menu overrides it per file.
2. **Type.** Autosave runs about 0.8 s after you stop typing. It also runs on tab
   switch, when the page is hidden and before unload. Unsaved buffers are backed
   up in the browser and restored after a reload. A failed save keeps the buffer
   dirty and shows **Retry**.
3. **Format** (⇧⌥F or the toolbar):
   - JSON (pretty-print, errors show line:col)
   - YAML (`yaml`)
   - SQL (`sql-formatter`; `:name`, `{name}`, `{{name}}` placeholders survive)
   - HTML / XML / SVG (built-in pretty-printer; `<pre>`, `<script>`, `<style>`
     and `<textarea>` stay verbatim)
   - CSV / TSV (RFC 4180 normalisation)
   - TOML, Markdown, JS/TS/CSS, Mermaid, D2 (conservative whitespace clean-up)

   Formatting is idempotent and is a single undo step. JSON, YAML, XML and CSV
   are validated while you type, and clicking the error's line:col jumps there.
4. **Preview** (split view):
   - Markdown: the Vault renderer, sanitised, including Mermaid/D2 fences and
     `![alt](asset:<id>)` images.
   - HTML: an iframe with an empty `sandbox` and a strict CSP, so no scripts run.
   - SVG: rendered as an image, so no scripts run.
   - Mermaid and D2: the Canvas renderers.
   - JSON: a collapsible tree.
   - CSV/TSV: a table, first 2,000 rows.
   - Images: pasted or dropped PNG/JPEG/GIF/WebP/SVG files are stored as
     workbench assets.
5. **History.** Every save lands in an append-only revision timeline. Select a
   revision to see it or diff it against the current text or another revision.
   **Restore this version** adds a new `restore` revision, so nothing is
   overwritten.
6. **Placeholders.** The side panel lists the placeholders it detects. In SQL
   these are `:name`, `{name}` and `{{name}}`, using the DB Explorer's parser.
   In other languages only `{name}` and `{{name}}` count. Values are remembered
   per file. A comma-separated value (`1,2,3,4`) becomes a sweep for **Run on…**.
7. **Send to…**
   - **Database: Run on…** picks the connection: the selected one, the only
     one, or a choice. It then opens the DB Explorer with the script in a new
     query tab and opens the **Run on…** sheet with the placeholder values as
     parameters. Nothing runs until you start it there.
   - **API client: new request.** Text starting with `curl` goes through the
     curl importer. Anything else becomes an unsaved POST draft with the content
     as the body.
   - **Terminal / agent session.** Pick a live session and confirm. The text is
     pasted as a bracketed paste with no Enter, so it is never executed.
   - **Vault: save as note.** Choose the vault and path; the default is
     `workbench/<name>.md`. Overwriting an existing note asks first.
   - **Copy with placeholders filled.**
8. **Trash.** **Move to trash** is a soft delete, and the history is kept.
   **Show trash** lists trashed files with **Restore** and **Delete forever**.
   Only Delete forever removes the file's history. It asks first, and only a
   trashed file can be purged.

Keyboard (only while the Workbench page is active):

| Keys | Action |
|------|--------|
| ⌘S | Save now as a checkpoint revision |
| ⌘W | Close the Workbench tab. This does not close the shell tab. |
| ⌘P | Quick open |
| ⇧⌥F | Format |

Open tabs and the active tab are remembered per workspace. The page opens on
the last tab you had open, or else on the most recent file.

## History model

- **Revisions** (`workbench_revisions`) are keyed `(doc_id, seq)`. `kind` is one of:
  - `create`: the first content.
  - `auto`: autosaves within about 60 s of the burst's first save fold into one
    revision. `saves` counts how many folded in, and the latest content
    overwrites the burst's revision, so the newest text is never lost.
  - `checkpoint`: ⌘S. Saving new content appends a revision. Saving unchanged
    content **seals** the open `auto` burst by turning it into a checkpoint, so
    the next edit starts a new revision.
  - `restore`: an older revision restored. `restored_from` holds the source seq.
- **Content** is stored in content-addressed blobs (`workbench_blobs`, sha256,
  deduplicated). When a coalesced save replaces a blob that no revision
  references any more, that blob is collected. Nothing else is ever collected.
- **Retention:** no retention job touches the workbench tables. A test pins this.
  History is removed only by `DELETE …?permanent=true` on a trashed file.
- **Crash safety:** each save is one SQLite transaction (doc row + revision +
  blob), and concurrent autosaves serialise. A test pins this too.
- Metadata changes (rename, pin, language, folder, tags) never create revisions.

## API & events

REST under `/api/v1/workspaces/{ws}/workbench/…`. See
[api.md → Workbench](../contracts/api.md):

- Docs: list (`?trash=true`), create, get, autosave/patch, trash / `?permanent=true`, restore.
- Revisions: list, get, restore, and a line `diff?from=&to=`.
- Assets: upload a raw image body, and fetch it (SVG is served with a sandbox CSP).

WebSocket: `workbench_doc_changed { workspace_id, user_id, doc_id, action, rev,
updated_at, client_id }` is delivered only to the owner. Other windows refetch,
and a window ignores its own `client_id`. A tab with local edits shows
**Reload / Keep mine** instead of being overwritten.

MCP (agents):

- `workbench_list` and `workbench_get` are read-only.
- `workbench_write` creates a file or updates its content, with the same
  approval rules as other mutating tools.

## Capabilities & limits

- Content limit is 5 MB per file and 20 MB per image asset. Diffs are capped
  for very large inputs.
- Formatting for JS/TS/CSS is whitespace-only, because no heavy formatter is
  bundled.
- The DB hand-off needs at least one DB connection in the workspace. With more
  than six connections it opens the DB page and asks you to pick one.
- Files are personal: there is no sharing between users yet.

## Troubleshooting

- **"Not saved" with Retry.** The daemon refused or was unreachable. The buffer
  stays dirty and is backed up locally, so press Retry or ⌘S.
- **"Changed in another window".** Another window saved this file while you had
  unsaved edits. **Reload** discards your unsaved edits and asks first.
  **Keep mine** keeps your buffer, and your next save becomes the newest revision.
- **A revision you expected is missing.** Saves less than about a minute apart
  fold into one `auto` revision. Press ⌘S to pin an exact state.
