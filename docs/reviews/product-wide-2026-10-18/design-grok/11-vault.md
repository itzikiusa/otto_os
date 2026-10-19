# Vault — design review

**Score: 8.0 / 10. Defects: 5.** Not 9.8.

Desktop chrome is in good shape: one `PageHeader`, tokens (no module hex except the allowed HTML-file canvas), global focus rings, `EmptyState` when no vault is registered, `LoadState` on tags and on a first graph failure, and a missing-note banner with Retry. Light and dark graph colors come from `tokens.css`. The score drops on states the user actually hits: a library that looks empty when it is loading or failed, search that keeps the wrong notes, a graph that hides a failed reload and covers itself on a phone, and reading-view tags that are not controls.

## 1. An empty or failed file listing is shown as an empty library

`ui/src/modules/vault/vault.svelte.ts:318` clears `roots` before `loadRoot` (`485-490`) returns. `loadRoot` and `toggleDir` (`495-514`) have no catch. `select` (`330`) already set `current`, so a thrown directory read never reaches the page `LoadState` (`VaultPage.svelte:376`), which only renders when `current` is null. `FileTree.svelte:286-289` treats `flat.length === 0` as “No notes yet” plus New note.

The user opens Vault, or a folder, and sees an empty library. A slow root listing flashes that empty state before the notes appear. A failed listing stays there: no error, no Retry, and New note looks like the right next step. A folder whose children failed to load shows an open chevron and no rows.

A loaded vault with zero notes also shows two empties: that tree copy, and the center “No note open” (`VaultPage.svelte:509-523`), each with New note.

**Fix:** Keep a `treeLoading` / `treeError` on the store. `FileTree` shows `LoadState` (`what="files"`, Retry calls `loadRoot`) until the root listing settles; only then, if it is empty, one `EmptyState` for the page (hide the center “No note open”). On a folder failure, leave the row closed and show the error on that row with Retry. Do not clear `roots` until the replacement listing succeeds.

## 2. Phone and a narrow tablet column never become one pane

`VaultPage.svelte:870-882` stacks the file tree, search, and tags into a strip of `min(25%, 180px)` under 640px. There is no `PageHeader` `leading` back control. From 641px to 1024px nothing changes: the tree stays 220–420px beside the note. Split panes are supposed to use a container query, and list/detail on a phone is push navigation.

Search results, the empty tree, and a missing-note retry all sit in that strip. Opening a hit does not take the strip away, so the note is whatever height is left above the status bar. Tree rows are 26px (`FileTree.svelte:462`) and tab close is 16×16 (`VaultPage.svelte:787-793`), under the 36px phone hit size.

**Fix:** When the panes container is phone-width (container query, not a new breakpoint), show either the sidebar or the center. Opening a note, file, or graph hides `.left` and puts Back in `PageHeader` `leading` (returns to Files, Search, or Tags). Give that sidebar the full body height. At `max-width: 640px`, raise tree-row and tab-close hit areas to at least 36px by padding, not icon size.

## 3. The graph covers itself, and a failed reload looks like success

`GraphView.svelte:97` starts the controls panel open. It is at least 200px wide and `max-height: calc(100% - 16px)` (`1664-1677`), pinned to the top end (`1355`). The status strip, including Stop layout, is the bottom start (`1635-1649`). On a phone graph, and on a short desktop window, the opaque panel covers the canvas, the empty copy, and Stop.

A later fetch stores `errorMsg` (`693-694`) but the error `LoadState` renders only when `nodeCount === 0` (`1315-1318`). A vault that already drew nodes keeps the stale graph and drops the “Loading graph…” badge. Retry never appears.

**Fix:** Below a container width of ~720px, start the panel closed; a “Graph” button opens it as a scrollable sheet that stops above the status strip. If `errorMsg` is set while nodes are on screen, keep the canvas and show a compact error bar with Retry (do not clear the last good payload until the retry succeeds). Clamp the hover tooltip (`1197-1199`, `1588`) inside the canvas so a node at the end edge is not clipped.

## 4. A failed search still lists the previous query’s notes

`runSearch` (`vault.svelte.ts:1261-1262`) sets `searchError` with `msg(e)` and leaves `searchHits` and `searchedQuery` from the last success. `SearchPanel.svelte:37-56` renders the error `LoadState` with `empty` (so it does not use `LoadState`’s stale-data bar) and then still renders every old hit as a button. There is no label that those rows are for a different query.

The user searches, the daemon fails, and they open a note from the list under the error. It is not a match for what they just typed. The detail line can also be the raw `Failed to fetch` string, because this path does not use `loadErrorText`.

**Fix:** Hide `.hits` while `searchError` is set or the typed query is not `searchedQuery`. On failure, set `searchError = loadErrorText(e)` and do not keep the previous rows. Retry stays on the `LoadState`.

## 5. Tags and unresolved embeds in the reading view are not controls

`mdRender.ts:198` emits `<span class="tag">`. Unresolved embeds are `<span class="wikilink unresolved">` (`148`). `NoteView.svelte:466-472` hangs the click on a static div and suppresses the a11y warnings. Resolved wikilinks are real links; tags are not.

A keyboard or VoiceOver user cannot run the tag search those chips advertise. The same click target is how an unresolved embed offers to create the note, and it is not in the tab order.

**Fix:** Render tags and unresolved embeds as `<button type="button">` (keep resolved targets as `<a>`). Handle them from the button’s own `click`, and delete the static-div handler and the `svelte-ignore` once nothing else needs it.
