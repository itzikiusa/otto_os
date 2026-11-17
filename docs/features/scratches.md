# Scratches

Scratch files for side scripts, queries, snippets and notes — the "prepare a
script, tidy it, send it somewhere" loop without a separate editor. Scratches
are a **project in the IDE**: one real folder per user, the **same in every
workspace**, opened in the normal IDE with the tree, editor, language servers,
run configurations, Local History, search and the agent dock.

- Where: IDE → project switcher → **Scratches**, the **default project**: first
  in the projects list, it can't be renamed or deleted, and the IDE opens it
  when there is no other project (no workspace folder, or the open project was
  deleted). ⌘K: **New scratch file…** from anywhere. `#/workbench` (the old
  Scratches page) opens the IDE on the Scratches project. IDE Settings ›
  Appearance › **Show projects as tabs** puts every project in the IDE header
  as a tab, Scratches first (see [IDE](./ide.md) § 1b).
- Folder: `~/Otto/Scratches/<user id>` (`$OTTO_SCRATCHES_HOME` moves the
  parent). It is an ordinary folder, so Finder, a terminal or another editor
  work on it too.
- RBAC: rides the **Agents** feature, like the IDE. The folder is **per user**:
  the IDE opens a non-root user's own folder and refuses anyone else's, even
  inside a workspace whose root contains it.
- Code: UI `ui/src/modules/ide/shell/ideScratches.svelte.ts` (switcher entry,
  new scratch), file tools `ui/src/modules/ide/filetools/` (format,
  Send to…), preview: the IDE's (`docs/features/ide.md` §12), routes `crates/otto-server/src/routes/scratches.rs` over
  `crates/otto-ide/src/scratches.rs` (folder, export, path rules), archive
  reader `crates/otto-state/src/workbench.rs` (migration
  `0219_workbench_docs_exported.sql`).

## Walkthrough

1. **New scratch file** (⌘K, the tree's **+**, or the empty Scratches tree)
   asks for a name; its extension picks the language (`.sql`, `.json`, `.md`,
   `.py`, `.mmd`, `.d2`, `.http`, …). The file is created at the top of the
   folder and opens. Folders, renames, moves and deletes are the IDE tree's.
2. **Type.** The IDE saves as usual; every save and delete lands in **Local
   History** (tree toolbar clock, or the editor's Local History action), where
   any version can be diffed and restored.
3. **Format** (⌘⌥L, *Reformat code*): JSON, YAML, Markdown, JS/TS/CSS/HTML
   through the IDE's formatters; **SQL** (`sql-formatter`, `:name` / `{name}` /
   `{{name}}` placeholders survive) and **XML / SVG** (built-in pretty-printer)
   are formatted in the app.
4. **Preview.** The IDE's **Editor | Editor and preview | Preview** control
   (end of the tab strip) covers Markdown, HTML, SVG, images, JSON, Mermaid
   (`.mmd`), D2 (`.d2`, Canvas' renderer) and CSV/TSV/JSONL — see the IDE
   guide, §12.
5. **Send to…** (share icon in the editor bar, any file):
   - **Database: Run on…** picks the connection (the selected one, the only
     one, or a choice), opens the DB Explorer with the script in a new query
     tab and the **Run on…** sheet, which fills its own placeholders. Nothing
     runs until you start it there.
   - **API client: new request.** Text starting with `curl` goes through the
     curl importer; anything else becomes an unsaved POST draft with the
     content as the body.
   - **Terminal / agent session.** Pick a live session and confirm. Control
     characters are removed first, and the confirm says how many. An agent or
     shell session gets a bracketed paste with no Enter. A connection session
     (SSH, a database client, a pod shell) takes a single line only; a
     multi-line file is refused there and **Copy** is offered instead.
   - **Vault: save as note.** Choose the vault and path (default
     `scratches/<name>.md`). Overwriting an existing note asks first.

## From the Workbench

Scratch files used to be DB docs on a separate Workbench page. The first time
you open Scratches (or an agent uses a `workbench_*` tool), Otto copies your
docs from **every workspace** into the folder, once:

- a doc's folder becomes sub-folders; its name gets an extension from its
  language (an *Auto-detect* doc is sniffed: SQL, JSON, HTML/XML/SVG, shell,
  Markdown, else `.txt`); a name that already has an extension keeps it;
- an image doc becomes its picture file;
- a name that is taken (including a case-only clash such as `Report` /
  `report`) gets ` (2)`, ` (3)`… — nothing is overwritten;
- docs in the Workbench trash are not copied.

Each copied doc remembers its new path (`workbench_docs.exported_path`), so the
copy never repeats — a copy you delete stays deleted. The Workbench tables,
including every doc's revision history, stay in the database as an archive;
the revisions are not replayed into Local History.

## API

REST: `GET /api/v1/scratches` (`root` + files), `GET /api/v1/scratches/file?path=`,
`PUT /api/v1/scratches/file`. See [api.md → Scratches](../contracts/api.md).
The IDE itself uses the `/ide/*` routes like any project.

MCP (agents), all on the session owner's folder:

- `workbench_list` (`query` filters paths) and `workbench_get` (`path`) are read-only.
- `workbench_write` (`path`, `content`, optional `language` for the extension)
  creates or replaces a file; the old and new content land in Local History.

## Capabilities & limits

- The `/scratches` file API reads and writes UTF-8 text up to 5 MB per file and
  lists up to 2,000 files; the IDE has its own limits.
- Markdown images that pointed at Workbench assets (`asset:<id>`) don't render
  any more; the assets stay in the archive.
- Files are personal: there is no sharing between users. A workspace rooted
  above `~/Otto/Scratches` (e.g. your home folder) can browse into the folder
  through that workspace's own project, like any file under it.

## Troubleshooting

- **"Couldn't open Scratches".** The daemon could not create or read
  `~/Otto/Scratches/<user id>` (permissions, a full disk) — the toast says why.
- **A doc from the Workbench is missing.** Trashed docs are not copied; restore
  it with an older Otto build, then open Scratches again and it is copied.
