---
id: vault
title: Vault
group: Build
route: vault
summary: Your docs home, made of plain Markdown folders (Obsidian vaults work as they are) with links, search, a graph and agents that write and review docs.
---

## What it's for

A vault is a folder of Markdown files on your Mac. Otto indexes it for links, tags, search and a graph, but the files stay yours: you can keep editing them in Obsidian or any editor. Agents can read and write the vault through Otto's tools, and docs agents can write and review whole documentation sets.

## Getting started

1. Open **Vault** in the sidebar and add a vault. Give it a name and pick a folder, or leave the folder blank and Otto creates one under `~/.otto/vault/`.
2. Keep **OKF vault** on if you want Open Knowledge Format checks and templates.
3. Wait for indexing to finish; the header then shows the note, link and unresolved counts.
4. Press `⌘O` to open a note, or `⌘N` to create one.
5. Press `⌘E` to switch between editing and reading.

## Everything it can do

**Vaults**
- Register several vaults; every workspace can see all of them. Switch with the vault button in the header.
- **Rescan** re-indexes the folder. **Unregister vault (keeps files)** removes Otto's index only.
- Changes made outside Otto (for example in Obsidian) appear within a few seconds.

**Files and tabs**
- A file tree that loads folders as you open them. Right-click for **New note here**, **New folder here**, **Rename**, **Move to…**, **Open in new tab**, **Delete (→ .trash)** and **Docs agent here**.
- Renaming or moving a note rewrites every link to it across the vault and tells you how many links changed. Drag a file onto a folder to move it.
- Deleting moves the file to the vault's `.trash/` folder. **Trash and restore** brings it back, to its old path or a new one, and never overwrites an existing file.
- `⌘`-click selects several notes. Then **Send to agent…** with an instruction, or **Review + fix** them as one set.
- Open notes and files as tabs. Middle-click or **Open in new tab** keeps the current tab. Tabs and the current view are restored per vault.

**Editing and reading**
- The editor autosaves shortly after you stop typing. `⌘S` saves at once.
- Type `[[` to link to a note (matches titles, aliases and paths) and `#` to pick an existing tag.
- If the file changed on disk while you were editing, a banner lets you reload the disk version or keep yours. Otto never overwrites silently.
- Reading view supports wikilinks in every form (`[[note]]`, `[[note|alias]]`, `[[note#heading]]`, `[[#heading]]`), embeds (`![[note]]`, images), callouts, `%%comments%%`, tags as chips, and Mermaid and D2 diagrams. Clicking an unresolved link creates that note.
- Other files open in a matching viewer: OpenAPI and Swagger specs, JSON, CSV and TSV tables, images, PDFs, and code.

**Right panel**
- **Backlinks** with context, **Outgoing** links, **Outline** and **Properties**. **Edit properties** changes common frontmatter fields and keeps everything else as it was.
- In an OKF vault: **Validate** checks the vault and lists findings you can click, and **Generate indexes** rebuilds each folder's `index.md`.
- The status bar shows backlinks, words, characters, the OKF badge and the vault path.

**Search, tags and the switcher**
- Full-text search with highlighted snippets. Combine words with `tag:`, `path:` and `type:` (for example `deploy tag:runbook`).
- **Tags** lists every tag with its count; click one to search for it.
- The quick switcher matches titles, aliases and paths. `⇧Enter` creates a note with the name you typed.

**Graph**
- A graph of the whole vault that stays fast with very large vaults. Choose the whole vault or the current note's neighbourhood, and set how many hops.
- Filter by title, show or hide tags, orphans, unresolved links and reserved files, and colour by folder or OKF type.
- Tune the forces and display, stop or resume the layout, drag to pin a node, double-click to unpin. Click a node to open the note.

**History**
- **Edit history** keeps before and after versions of edits made in Otto and by agents. **Note edit history** filters it to one note. Compare any revision and restore either side. Restoring always adds a new revision.

**Agents**
- **Docs agent** (the ✨ button): 1 to 4 writer agents, a summarizer, and ready-made prompts for a repository deep dive, flows, APIs with OpenAPI, datastores, messaging and incremental updates. Written notes are listed when the run ends.
- Optional **Review outcomes**: 1 to 4 read-only reviewers, each with its own provider, model, method and focus. The author fixes their findings for up to 3 rounds by default (1 to 10). You can retry a single reviewer or revision, or cancel the run.
- **Refine with AI** on an open note starts a session that edits that note on request.
- Agents in any session can list, read, search, write, rename and delete vault notes through Otto's tools, with the same permissions as their owner.
- A chip in the header shows how many docs-agent runs are active.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `⌘O` | Open the quick switcher |
| `⌘N` | New note |
| `⌘E` | Switch between editing and reading |
| `⌘S` | Save the note now |
| `↑` / `↓` | Move through switcher results |
| `Enter` | Open the selected switcher result |
| `⇧Enter` | Create a note named after the switcher query |
| `Esc` | Close the quick switcher |
| `⌘`-click | Select several notes in the tree |

## Tips and limits

- Vault access follows the **Product** feature: reading and searching need View, any change needs Edit.
- Notes larger than 4 MB are indexed by name only (no body search, tags or links) until they shrink.
- Only edits made through Otto are versioned; changes from other editors are picked up but not kept in history.
- The scanner skips `.obsidian/`, `.git/`, `.trash/`, hidden files and `node_modules`.
- Reviewer agents can't write to the vault; only the author does.
- Vault history is kept in the hidden `.otto-history/` folder inside the vault.

## Related

- [Product](#/walkthroughs/product)
- [Agents](#/walkthroughs/agents)
- [Design Hall](#/walkthroughs/design)
- [Skills Lab](#/walkthroughs/skills-eval)
- [Browser](#/walkthroughs/browser)
