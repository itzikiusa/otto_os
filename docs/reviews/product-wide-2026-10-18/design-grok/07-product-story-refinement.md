# Design review — Product stories, refinement, discovery chat, attachments

**Score: 7.1 / 10. Defects: 12.** Target was 9.8. Not met. The page shell, empty/error split, and publish sheet are in good shape. Phone navigation, the chat split, and refinement’s “already saved” story are not.

Reviewed source only (no Cargo, npm, browser, or screenshots). Light, dark, Pro Dark, and Warm were checked by tracing these styles against `ui/src/lib/tokens.css`. Contrast of one-off `color-mix` badges was not measured in a browser.

## Covered

- Story list and detail shell: `ui/src/modules/product/ProductPage.svelte`, `OverviewTab.svelte` (header, draft, Jira/Confluence body, publish entry), `ImportDialog.svelte`.
- Refinement: `RefineTab.svelte`, `RefineChat.svelte`, `ui/ListPane.svelte`. Backend write that the chip reports: `crates/otto-product/src/refine.rs`.
- Discovery chat: `ChatTab.svelte`, `DiscoveryChat.svelte`, `ActionCard.svelte` (apply stays a draft; that part is fine).
- Discovery runs (the tab next to chat): `DiscoveryTab.svelte`.
- Attachments: `AttachmentsPanel.svelte`, paste from the draft body in `OverviewTab.svelte`.
- Guidelines used: `docs/design/guidelines/` list/detail phone push, one primary, EmptyState/LoadState, 36 px phone hits, agent draft-until-apply, logical CSS, `runStatus` / `rel` / `formatBytes`.

## What holds

- One `PageHeader` and `PageBody`. Stories | Learnings is a real tablist. Desktop/tablet restores the last story and never opens a “pick one” pane. An empty library hides the list and uses one page `EmptyState`.
- Failed story-list and story-open loads use `LoadState` plus Retry, not a fake empty state. Delete, detach-style confirms, and publish go through `confirmer` / `Modal`. Publish names the account, project or space, the body, and who can see it.
- Discovery **action cards** stay drafts until Apply, with confirm on a non-empty replace and Undo. That is the pattern refinement should follow.
- Scoped styles use tokens (`--accent-soft`, `--accent-solid`, `--accent-contrast`, `--danger`). No hex in these files. Inputs that clear `outline` put the accent ring back. The story list is a resizable `PaneDivider` (hidden at ≤640 px). The draft/Jira columns stack at ≤1024 px.

## Defects

### 1. Phone story detail is an accordion under a stack of titles

`ProductPage.svelte:224-230`, `:622-689` (no `leading` slot), `:692-800`, `:1374-1459`. `OverviewTab.svelte:1353-1410`. `ImportDialog.svelte:120-122`.

Opening a story on a phone does not replace the list. Both accordion bars stay: “Stories” and the story title. Above them the shell already has the phone title, `PageHeader` (title + stage + key), and the Stories | Learnings tabs. Below them the workflow strip adds another one or two rows. `OverviewTab` then repeats the title as an `h2`, plus stage and source key. First screen is chrome, not the story. Import calls `product.select` and never sets `mobileSection = 'content'`, so a phone import highlights a row and leaves the user in the list.

**Fix:** At `viewport.isPhone`, render only the list or only the detail. Put a 36 px back button in `PageHeader` `leading` (`aria-label="Back to stories"`) that returns to the list without clearing selection. Drop the accordion bars. On detail, do not repeat title, stage, or key in the overview header; keep Edit title as a header action. Any select path (import included) should open the detail on a phone.

### 2. Refinement and discovery chat stay a 220 px split at every width

`ui/ListPane.svelte:24` (`width: {width}px`, default 220, `flex-shrink: 0`). `RefineTab.svelte:161-260` and `ChatTab.svelte:124-195` have no `@media (max-width: 640px)` and no container query. `RefineTab.svelte:204-258` also shows two empty states at once (“No threads yet” in the list and “No refinement threads yet” in the pane).

On a phone the story pane is full width, then this split still runs. The transcript column is about 140 px, so bubbles and the Send button wrap into a sliver. On a 768 px tablet with the sidebar open, the story list (~280 px) plus this 220 px pane leaves the composer the same way. `layout.md` says a list pane is user-resizable and phone list/detail is push; this pane is neither.

**Fix:** Give the thread/chat list a `PaneDivider` on desktop. Below 640 px, or when the chat column is under ~480 px (`container-type: inline-size`), show either the list or the open transcript, with a back control in the pane header. When there are zero threads, hide the list pane and let one `EmptyState` own the single New action.

### 3. A refinement reply can land off-screen

`RefineChat.svelte:145-189`. `DiscoveryChat.svelte:39-44` scrolls to the newest message. `RefineChat` never does.

The transcript scroller starts at the top. After Send, the optimistic bubble and the agent reply are appended below the fold. The composer re-enables with no guarantee the reply is in view. On the crushed phone column (defect 2) that reply is easy to miss.

**Fix:** Same effect as discovery chat: after messages or the thinking row change, scroll the scroller to the end only if the user was already near the bottom, so reading history is not yanked.

### 4. Refinement writes the story, then only offers a “Story updated” chip

`RefineChat.svelte:167-174`. The write is `refine.rs:422-440`: a non-empty `updated_story_md` becomes a `suggested` version before the UI returns. Publish and discovery treat newest `suggested` as the story body (`refine.rs:5-12`).

The chip is a check icon and the words “Story updated to v{n}”, and it only navigates to Rewrite. There is no diff, Apply, or Discard in the thread. For a Jira issue the live issue did **not** change, so the label is wrong. For every story, the text other Product actions will use **did** change, with no review. Discovery chat already does this correctly via `ActionCard.svelte` (Apply, confirm, Undo).

**Fix:** Do not commit on the turn. Return the proposed markdown and render an `ActionCard`-style draft: diff, Apply, Discard. Apply creates the suggested version. Until then the chip must not say the story was updated. Copy if a version exists: “Suggested v{n} — review”, not “Story updated”.

### 5. A running turn cannot be stopped

`RefineChat.svelte:180-214` and `DiscoveryChat.svelte:244-280`. `sending` disables the textarea and Send. There is no Stop. A refinement turn’s no-progress window is 150 s (`refine.rs:42`), and the hard cap is longer. Rewrite at least has “Stop waiting” (`RewriteTab.svelte:286-288`). Here the composer is dead until the request ends. Switching threads drops the local wait; staying on the thread does not.

**Fix:** While `sending`, replace Send with Stop. Stop aborts the in-flight request, restores the typed text, and re-enables the composer. If the daemon turn cannot be cancelled, say “Stop waiting” and do not leave the control disabled.

### 6. A failed transcript load still sends

`DiscoveryChat.svelte:194-196` and `:255-283`. `RefineChat.svelte:146-148` and `:193-214`. `send()` does not look at `loadError`.

The history area shows `LoadState`, but the composer stays live. A send appends an optimistic bubble onto an empty local list while the server still has the thread. The reply can depend on messages the user cannot see, and a later Retry replaces the local list.

**Fix:** Disable Send while `loadError` is set, or while the first load has not succeeded. Keep Retry as the only action in that state.

### 7. Discovery chat keeps the previous story’s list, and hides a failed reload

`ChatTab.svelte:27-63` and `:137-141`. `RefineTab.svelte:35-48` clears threads before the fetch. `ChatTab` only clears `activeCid`.

Switching stories leaves story A’s chats on screen until B’s request returns. The right pane says “No chat open”, so the obvious click opens A’s conversation under B’s header. If B’s load fails, `loadError` is set but the template only shows it when `chats.length === 0`, so A’s list stays and there is no Retry.

**Fix:** Clear `chats` at the start of a story change (same as refinement). On a failed refresh with rows already showing, use `LoadState`’s stale bar and Retry instead of the `length === 0` branch.

### 8. Archive is instant, one-way, and always tappable on a phone

`RefineTab.svelte:124-138` and `:230-238`. `ChatTab.svelte:87-101` and `:157-165`. Touch rule at `:393-397` and `ChatTab.svelte:303-307` (`@media (hover: none)` forces the Archive control visible). No unarchive control exists in either file.

Archive is not confirmed. It immediately leaves the open thread. On a phone the control sits on every active row at about 20 px tall (`padding: 2px 8px`, `--fs-xs`), beside the row you tap to open. There is no Undo toast and no way back.

**Fix:** `confirmer.ask` (“Archive this thread? You can still read it.”, `danger: false`, confirmLabel “Archive”). Add Unarchive on archived rows. Give the control `min-block-size: var(--hit-min)` on a phone, or move it into the row ⋯ menu.

### 9. Discovery run refresh fails silently, and task status is a dot

`DiscoveryTab.svelte:171-199` and `:519-536`. `RefineTab.svelte:72-82` and `:185-188`.

Refresh sets `loadError`, but the error view renders only when `runs.length === 0`. A failed Refresh on a list that is already showing does nothing visible. The run picker then treats a failed discovery-run load as “no runs” (`catch` clears the list, no Retry), so “New from discovery run…” disappears.

Expanded tasks use a 7 px dot alone (`status-done` is `--accent`, which means selection; `status-running` is amber). `patterns.md` says running is the info/working treatment, success is not accent, and colour is never the only signal. The run header already uses `StatusBadge` + `runStatus`. The refine picker prints raw `derived_status` (`in_progress`, `error`) instead of `runStatus(…).label`.

**Fix:** Wrap the run list in `LoadState` so a failed refresh keeps the rows and shows Retry. Surface a discovery-run load failure next to the picker. Replace task dots with `StatusBadge` / `runStatus`. Use that same label in the “New from discovery run…” options.

### 10. The attachment panel advertises a paste it cannot take, and hides a broken preview

`AttachmentsPanel.svelte:259-295`, `:121-132`, `:327-333`, `:355-358`, `:273-274`, `:379-394`.

The empty hint says “Drop files here, choose File, or paste a screenshot”. The region is a `div` with `onpaste` and no `tabindex`, so paste only works if focus is already inside it. Clicking the hint does not focus it. Paste into the draft body works (`OverviewTab.svelte:1071-1083`); paste aimed at the panel, or on a Jira story that is not in the draft textarea, does not. The button label is “File”, not a verb.

Image preview failure is `console.warn` only. The row falls through to “Load preview”, which retries with the same silence. Non-image files use a 📄 glyph instead of `Icon`. Delete and “Mark as mockup” are custom controls at `padding: 2px 6px` (well under 36 px) and are not `.btn`. Sizes use a local `fmtBytes` (`:95-99`) instead of `formatBytes`.

**Fix:** Make the drop zone a focusable control (`tabindex="0"`, visible focus, “Paste a screenshot” as its name) or delete the paste sentence and point at the body field. On preview failure, show “Couldn’t load the preview” and Retry. Use `Icon name="file"`. Label the picker “Add file”. Use `.btn.small` with `min-block-size: var(--hit-min)` on phone. Call `formatBytes`.

### 11. Chat bubbles ignore RTL and the agent rule

`DiscoveryChat.svelte:376-383` and `RefineChat.svelte:258-266`.

User bubbles set `border-bottom-right-radius: 3px` and agent bubbles `border-bottom-left-radius: 3px`. `justify-content: flex-end` flips in RTL; those radii do not, so the tail sits on the wrong corner. Agent turns have `AgentByline` (good) but not the specified 2 px inline-start rule on `--border-strong`, so a long bubble reads as a generic card. User text in refinement is also passed through `renderMarkdown` (`RefineChat.svelte:166`); discovery correctly leaves the user’s text plain (`DiscoveryChat.svelte:229-230`).

**Fix:** Use logical radii (`border-end-end-radius` / `border-start-end-radius`). Add the agent rule. Render the user’s refinement text as plain text, like discovery chat.

### 12. Two primary buttons, and the story row is not exposed as the current item

Primary: `ProductPage.svelte:678-686` (New) stays `.primary` while `RefineTab.svelte:165-172` (New thread), `ChatTab.svelte:128-135` (New chat), and `DiscoveryTab.svelte:171-178` (Run discovery) are also `.btn.small.primary`. On those tabs two filled buttons are on screen. New creates a story; the other creates a thread, a chat, or a swarm run.

Selection: `ProductPage.svelte:574-579` toggles `.active` and does not set `aria-current`. `tree-toggle` is 18×22 px (`:1052-1063`); the row ⋯ is 22 px, 30 px on a phone (`:1021-1034`, `:1492-1496`). Phone minimum is `--hit-min` (36 px). The chevron and the row are separate buttons, so a fat finger expands instead of opening.

**Fix:** Keep a single `.primary` (header New, or the empty-state Import). In-pane actions are `.btn.small`. Set `aria-current="true"` on the open story’s button. On a phone, make the chevron and ⋯ at least 36 px (padding, not a bigger glyph).

## Residual (not scored)

- Publish and import account/project errors were read, not exercised against a live Jira or Confluence account.
- No light/dark/phone screenshots and no axe run, so a contrast failure in the warning-tinted mockup badge (`AttachmentsPanel.svelte:488-491`) or discovery status chips would be extra.
- Design arena, mockups, analysis, test cases, and learnings were out of this pass except where overview embeds attachments.

## Score

**7.1 / 10. 12 defects.** Tokens, page chrome, library empty/error, publish confirmation, and discovery action cards are at the guideline. The score stays under 9.8 until phone story detail is real push navigation, refinement and discovery chat collapse to one column, a refinement turn cannot change the suggested story without Apply, and the chat list cannot show another story’s threads after a failed load.
