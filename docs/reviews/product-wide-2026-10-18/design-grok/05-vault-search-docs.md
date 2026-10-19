# Lens: Vault and Help (search, reading, graph, empty library, missing doc)

**Score: 8.0/10** — 10 − (5 × 0.3) − (5 × 0.1). Module CSS stays on tokens (the only hex is the allowed HTML-canvas white in `FileViewer.svelte`). Help’s phone list/detail is real push navigation. Vault search, missing notes, graph failure, and the phone vault are not.

Defects: **10** (5 major, 5 minor).

### Findings

1. **[major] Phone vault is a 180px strip, not push navigation.** `ui/src/modules/vault/VaultPage.svelte:868-882`, hit sizes `FileTree.svelte:460-462`, `SearchPanel.svelte:89-97`, `TagsPanel.svelte:66-75`, `VaultPage.svelte:787-799`, `NoteView.svelte:556-561`, graph chrome `GraphView.svelte:1635-1679`. Layout requires list/detail to become a full-height list with a back button at ≤640px, ≥36px targets, and no floating chrome over the content. A phone instead keeps the file tree, search, or tags in a `max-height: min(25%, 180px)` band (rows stay ~26px, tab close is 16×16, note tools are 28px) and paints the graph’s default-open ~210px panel and status chip on top of the canvas. Search results and the graph are not usable. **Fix:** at `viewport.isPhone`, show either the sidebar or the center, with Back in `PageHeader` `leading`; give tree/search/tag/tab-close/note tools `min-block-size: var(--hit-min)`; start the graph panel collapsed and full-width under the canvas, not over it.

2. **[major] Vault search keeps answering the previous query.** `ui/src/modules/vault/SearchPanel.svelte:35-56`, `ui/src/modules/vault/vault.svelte.ts:1254-1265`. Hits render whenever `searchHits` is non-empty. Editing the box, clearing it, or a failed `runSearch` does not clear them (`catch` only sets `searchError`). The “Press Enter” / error line sits above notes from the last success, so a click opens a note the current text did not find. **Fix:** render `.hits` only when `searchedQuery === searchQuery.trim()`, not searching, and no error. On failure set `searchHits = []`. Clearing the field should clear hits without waiting for Enter.

3. **[major] A missing note can be created from one pointer path only.** `ui/src/modules/vault/mdRender.ts:148`, `ui/src/modules/vault/mdRender.ts:172-176`, `ui/src/modules/vault/mdRender.ts:253-257`, `ui/src/lib/sanitize.ts:36`, `ui/src/modules/vault/NoteView.svelte:179-206`, `ui/src/modules/vault/NoteView.svelte:649-651`, `ui/src/modules/vault/NoteView.svelte:673-675`, `ui/src/modules/vault/RightPanel.svelte:87-92`, `ui/src/modules/vault/GraphView.svelte:1206-1215`. Body wikilinks are `<a>` with no `href`, so they are not in the tab order; the read surface is a click-only `div`. An unresolved embed is a `<span class="wikilink">` whose `data-raw` the sanitizer strips (`span` allows only `data-tag` and `data-path`), and the click handler never looks for that span. Outgoing “unresolved” rows are `disabled`. A graph ghost click is a no-op. The dashed link is also `opacity: 0.6`, which drops accent text under the contrast floor in light and dark. Help claims every unresolved click creates the note (`ui/src/modules/help/sections/vault.md:39`). **Fix:** emit `href="#"` on internal links (survives the sanitizer) and `preventDefault` on click; render unresolved embeds as those same anchors. On a missing target, side panel and graph ghost should run the existing Create confirmation. Drop the opacity; keep the dashed underline and add `title="Missing note"`. Update the guide sentence to match.

4. **[major] A failed graph load leaves the previous vault on screen and hides the error.** `ui/src/modules/vault/GraphView.svelte:679-698`, `ui/src/modules/vault/GraphView.svelte:1313-1328`. The fetch clears `errorMsg` but does not clear nodes. The error card renders only when `nodeCount === 0`. After a vault switch or rescan fails, the old graph stays, “Loading graph…” disappears, and Retry never shows. **Fix:** on failure, if this request is still current, call the empty ingest (or keep a `stale` flag) and show `LoadState` with Retry over the canvas even when the last good graph is still drawn. Do not paint vault A’s nodes under vault B’s name.

5. **[major] The Vault guide sends Docs agent users to Refine.** `ui/src/modules/help/sections/vault.md:61-63` versus `ui/src/modules/vault/VaultPage.svelte:339-349` (`Icon name="zap"`, label “Docs agent”) and `ui/src/modules/vault/NoteView.svelte:420-428` (`Icon name="sparkle"`, “Refine with Otto”). The guide calls Docs agent “the ✨ button” and names the note action “Refine with AI”. Following Help opens the wrong tool. **Fix:** say the header **Docs agent** button (bolt), and **Refine with Otto** (sparkle) on the note. Same pass: the menu label is “Unregister vault”, not “Unregister vault…” (`vault.md:26`).

6. **[minor] An empty library still says a note is waiting in the tree.** `ui/src/modules/vault/VaultPage.svelte:509-523`, graph twin `ui/src/modules/vault/GraphView.svelte:1319-1327`. The tree correctly says “No notes yet” and offers New note (`FileTree.svelte:286-289`). The center (and the graph overlay, which has no button) still says “No note open” / “Nothing to graph” and tells you to open one from the tree. **Fix:** when `(vault.status?.notes ?? vault.current.notes) === 0`, use “This vault is empty” and a single New note action. On the graph empty state, add that button (`pointer-events` on `.empty` is `none` except `.empty button`).

7. **[minor] Search stops at 50 hits and never says so.** `ui/src/modules/vault/vault.svelte.ts:1256`, `ui/src/modules/vault/SearchPanel.svelte:45-56`. A full page looks complete. **Fix:** when `searchHits.length === 50`, add “Showing the first 50. Narrow with tag:, path:, or type:”.

8. **[minor] A missing guide on a phone tells you to use a list that is not mounted.** `ui/src/modules/help/Walkthroughs.svelte:170-172`, `ui/src/modules/help/Walkthroughs.svelte:314-321`. `showList` is false whenever the route is not the index, including an unknown id. The empty state says “Pick one from the list”. Back works; the sentence does not. **Fix:** on the phone, body “Go back to all guides, or start at the beginning.” Keep the Getting started action.

9. **[minor] Graph labels are 10px.** `ui/src/modules/vault/GraphView.svelte:1103`. Nothing a person must read is below 11px (`--fs-xs`). These are the note titles. **Fix:** `11px` (or the computed `--fs-xs`). The system font stack can stay.

10. **[minor] Graph group colour is the only signal, and it is a raw rainbow.** `ui/src/modules/vault/GraphView.svelte:236-242`, `ui/src/modules/vault/GraphView.svelte:447-452`, `ui/src/modules/vault/GraphView.svelte:1196-1200`. Real notes push `pMetas` as `''` while `pKeys` holds the service or type. The hover tip therefore shows the title alone. Cluster identity is a golden-angle HSL fill, which is not a semantic token and fails in colour-blind and light/dark reading. Touch has no hover at all. **Fix:** set each note’s meta to its colour-group label so the tooltip (and a tap-to-pin callout on a phone) names the group. Keep the fill as a secondary cue.

### What would get this lens to 9.8

- Phone vault becomes push navigation with 36px targets, and the graph panel stops covering the canvas (finding 1). That single change is +0.3.
- Search hits match the text in the box, including after an error (finding 2).
- Missing notes share one Create action from the keyboard, embeds, the side panel, and graph ghosts, without faded type (finding 3).
- Graph failure shows Retry and never another vault’s nodes (finding 4).
- The Vault guide names Docs agent and Refine as they appear (finding 5).

Those five are +1.5, to **9.5**. Three of the minors (empty-library copy, the 50-hit cap, and the phone missing-guide sentence) are the rest of the way to **9.8**. The 10px labels and the group tooltip should land in the same pass; they are not what the score is waiting on if the five majors and those three minors are done.

### Checked, not filed

- No vault and load/error use `EmptyState` / `LoadState` (`VaultPage.svelte:376-389`). Tags, backlinks, switcher, trash, history, and the tour film each have a loading, empty, and error path.
- Help index always opens Getting started; unknown ids use `EmptyState` on desktop, where the rail stays visible. Search empty has Clear. Guides and film chapters cross-link.
- Help and vault colours in CSS are tokens. `FileViewer.svelte` `#fff` is the marked HTML-page exception. Graph strokes read `--text`, `--text-dim`, `--accent`, and `--surface` on scheme change.
- Help phone rows are 44px and chapter controls go to 36px at `max-width: 640px`. Focus uses the global `:focus-visible` ring; the help search and quick switcher replace `outline: none` with a focus ring.
