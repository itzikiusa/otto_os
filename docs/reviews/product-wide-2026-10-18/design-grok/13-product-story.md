# Product stories — design review

**Score: 8.8 / 10. Defects: 6.**

Reviewed the story list, overview, discovery chat, refinement, and attachments in `ui/src/modules/product/` against `docs/design/guidelines/` (PageHeader, EmptyState, Modal, confirmer, real buttons, focus, loading / empty / error, long text, phone ≤640 and tablet 641–1024). Light and dark both read through `ui/src/lib/tokens.css` (Native, Pro Dark, Warm). These surfaces use semantic tokens; no theme-only contrast break in the scoped UI. List load, empty, filter-empty, and detail error already use `LoadState` / `EmptyState`, and outward publish goes through `Modal`.

## 1. Discovery and refine stay a 220px list beside the transcript

`ui/src/modules/product/ui/ListPane.svelte:24` locks `width: {width}px` (default 220) and `:41` sets `flex-shrink: 0`. `ChatTab.svelte:198` and `RefineTab.svelte:264` are a row with no breakpoint. `ProductPage.svelte:1379` only turns the outer story list into an accordion at 640px.

On a phone the chat column is about 70–140px, so bubbles, starter chips, and the composer are unusable. At 641px it is worse: the 280px story sidebar is back, and the transcript is ~100px. Archive is forced on for `(hover: none)` (`ChatTab.svelte:303`, `RefineTab.svelte:393`), so the thread title loses the rest of that 220px.

**Fix:** container-query the main column (set `container-type: inline-size` on `.product-main`). Under ~720px, show either the list or the open chat, full width. A back control above the transcript returns to the list. Do not add a second `PageHeader`.

## 2. A refinement thread never follows the latest turn

`DiscoveryChat.svelte:41` scrolls to the newest message when the count changes. `RefineChat.svelte:145` has no equivalent. Opening a long thread lands on the oldest bubble. Send and the “Thinking…” row (`RefineChat.svelte:181`) render below the fold, so a reply looks like a hang until the user scrolls.

**Fix:** bind the message scroller and, on count change, set `scrollTop = scrollHeight`, same as discovery. Keep the composer usable while that scroll happens.

## 3. A failed attachment preview looks like it is still waiting

`AttachmentsPanel.svelte:128` logs the preview error and clears the loading flag. The card then shows “Load preview” again (`:328`) with no reason. The same swallow is in `OverviewTab.svelte:983` for Jira files (`:1852` “Load preview”, `:1870` “Preview PDF”, `:1889` “Download”). An in-body `attachment:<id>` image that fails (`OverviewTab.svelte:452`) stays as raw markdown in the description.

**Fix:** keep a per-id error string (`loadErrorText`). Show “Couldn’t load this preview” plus Retry on that card. For a body token, replace the markdown image with the same inline failure instead of the raw token.

## 4. Long tags and MIME types scroll the page sideways

`ProductPage.svelte:1163` and `:1192`: tag chips are `white-space: nowrap` with no max-width, inside a sidebar that does not clip. One long tag widens the story list and the page. `OverviewTab.svelte:3266`: `.att-meta` is nowrap, so a MIME such as `application/vnd.openxmlformats-officedocument…` becomes the min-content width of the Jira attachment card.

**Fix:** on the chips, `max-width: 100%`, `overflow: hidden`, `text-overflow: ellipsis`, and `title` set to the full tag. On `.att-meta`, `min-width: 0`, ellipsis, and `title` set to the full mime. The filename already ellipsizes (`OverviewTab.svelte:3257`).

## 5. Two primary buttons on the empty library and inside chat

`ProductPage.svelte:672` says the empty state owns the one import CTA, but `:678` leaves **New** as `.btn.small.primary` while `noStories` is true. `EmptyState` at `:853` also defaults to a primary **Import story**. Inside a story, **New chat** (`ChatTab.svelte:129`) and **New thread** (`RefineTab.svelte:166`) are a second `.primary` next to the header **New**.

**Fix:** when `noStories`, drop `primary` from header New so Import is the only filled button. On the chat and refine tabs, drop `primary` from header New and keep it on New chat / New thread — that is the action for the open view.

## 6. “New from discovery run” shows the raw status word

`RefineTab.svelte:187` prints `{dr.derived_status}` in the option label. Discovery itself maps that through `runStatus` (`DiscoveryTab.svelte:210`). The picker shows `waiting_on_user` instead of “Waiting on user”, in a 220px select.

**Fix:** label the option with `runStatus(dr.derived_status).label`. Keep the id as the option value.
