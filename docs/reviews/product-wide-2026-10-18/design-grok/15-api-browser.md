# API client and in-app browser — design review

Read-only review of `ui/src/modules/api/` and `ui/src/modules/browser/` against `docs/design/guidelines/`. Source was traced for loading, empty, error, loaded, phone (≤640), tablet, light/dark tokens, and keyboard. The app was not run, so contrast and overflow are from the CSS, not screenshots. `ui/src/modules/settings/BrowserSettings.svelte` was read only to see whether site credentials live there. They do not.

Target was 9.8. These scores are not inflated to meet it.

| Module | Score | Defects |
|---|---|---|
| API client | 8.9 | 8 |
| Browser | 8.6 | 6 |
| **Total** | | **14** |

What already holds the scores up: one `PageHeader` per module page, `PageBody`, `EmptyState` / `LoadState` on the empty and failed paths, `Modal` + `confirmer` + `ctxMenu`, semantic tokens (no hex in module CSS), `--fs-*` with nothing readable under 11px, logical properties, phone push between the API list and the editor, container queries in the request builder and environments split, and the live approval card (who / what / where / when, Take over, Deny, Approve). Private-address opt-in, cookie clear, history clear, retention, autofill, and Git push already say what will happen. Those are not defects.

`overlay.js` hard-codes `#6a5acd` and `#e0b400`. That CSS is injected into the page being picked, which does not have Otto tokens. Not counted.

## API client — 8.9

### 1. Two primary buttons on the same view

`ui/src/modules/api/ApiPage.svelte:325` keeps **New request** as `.btn.small.primary` whenever onboarding is over. The editor’s send control is also primary (`ui/src/modules/api/RequestBuilder.svelte:798`). Opening Environments adds a third: **Save changes** (`ui/src/modules/api/EnvironmentsView.svelte:284`). Opening an automation adds **Run** (`ui/src/modules/api/AutomationEditor.svelte:284`). A connected WebSocket adds **Send message** (`ui/src/modules/api/ResponseViewer.svelte:351`).

The eye never gets a single action. New request is already on the tab strip (`ApiPage.svelte:409`).

**Fix:** one `.primary` per view. Header **New request** stays a normal `.btn.small` (the onboarding `EmptyState` keeps the only primary there). **Send** / **Invoke** / **Connect** is the primary on a request. **Save changes** is the primary on Environments. **Run** is the primary on an automation. **Send message** stays secondary while Connect is the primary, or it becomes the only primary once the socket is open.

### 2. On a phone, Send leaves the response below the fold

Below 640px the builder is not height-capped and the response is a later sibling (`ui/src/modules/api/ApiPage.svelte:686-698`). `send()` (`ui/src/modules/api/RequestBuilder.svelte:580-597`) never scrolls `.resp-pane` (`ApiPage.svelte:439`) into view. Automation runs do (`AutomationEditor.svelte:231-234`). A phone user taps Send and still sees the URL and tabs.

**Fix:** when a tab result, failure, or stream item arrives and `viewport.isPhone`, scroll the response section into view with `scrollBehavior()` (auto under reduced motion). Keep the stacked layout.

### 3. Phone hit targets in the list and on request tabs

Collection and request rows are a fixed 28px (`ui/src/modules/api/CollectionsTree.svelte:438-443` and `:479`). That list is the whole phone pane. Request-tab close buttons are 20×20 (`ui/src/modules/api/ApiPage.svelte:633-636`) with no coarse-pointer rule. Browser tabs already grow to 36px (`ui/src/modules/browser/TabStrip.svelte:156-168`). A thumb tap closes the wrong tab or opens the wrong request.

**Fix:** under `(pointer: coarse)`, collection rows and their buttons at least `var(--hit-min)`, and raise the virtual-list `estimateHeight` to match. Same for `.req-tab-close` and `.req-tab-main` (minimum block size 36px, glyph stays 12px).

### 4. Request-tab state is a dot, and Close doesn’t name the tab

`ui/src/modules/api/ApiPage.svelte:392-394`. Unsaved is a 7px warning circle. Last outcome is a 7px success or danger circle. Both `aria-label`s sit on plain `<span>`s inside a button whose accessible name is the method and title, so they are not announced. The close control is always “Close tab” (`:396`), unlike Browser’s “Close tab {title}”.

**Fix:** put “unsaved” and “last send failed/succeeded” in the tab button’s accessible name. Draw unsaved as the word “Edited” (the name row already says that) and the outcome as a check or × icon, not colour alone. `aria-label` on close includes the tab label.

### 5. Turning off TLS verification does not ask

Allowing private addresses confirms, and names that everyone in the workspace is affected (`RequestBuilder.svelte:663-670`). Unchecking **Verify TLS certificates** (`:1136-1139`) only has helper text. One miss sends that request’s tokens to any certificate.

**Fix:** when the box goes from checked to unchecked, `confirmer.ask` with `danger: true`, title “Don’t verify TLS?”, body that this request will trust any certificate, `confirmLabel` “Don’t verify”. If the user cancels, put the check back. Turning it back on stays silent.

### 6. Run sends every step with no where / what / who

`ui/src/modules/api/AutomationEditor.svelte:194-208` saves if dirty, then `runAutomation` immediately. The button (`:284-286`) says **Run**. Steps are real HTTP calls with the chosen environment’s secrets, including POST and DELETE. Stop is correctly unconfirmed; Run is not a single local edit.

**Fix:** before `runAutomation`, a confirm sheet: step count, environment name (or “the active environment”), and that the calls leave this Mac. `confirmLabel` “Run {n} requests”. Don’t confirm Stop.

### 7. Git pull closes the sheet even when it failed

`ui/src/modules/api/GitSyncDialog.svelte:45-49` always calls `onclose()` after `gitPullCollections`. The store swallows the error into a toast (`ui/src/lib/stores/apiClient.svelte.ts:1193-1196`) and the success line is `1 collection file(s)` instead of `plural()`. Push stays on the sheet until it succeeds (`GitSyncDialog.svelte:51-55`). A failed pull looks like a dismiss, and the only recovery is to reopen **Sync with Git…**.

**Fix:** return success/failure from the pull. Stay open and show the error inline with Retry when it fails. Close only after success. Use `plural(imported, 'collection file')`. Toast title “Couldn’t pull collections”, not “Couldn’t git pull”.

### 8. A timeout offers “Open Settings”, which is not Settings

`ui/src/modules/api/ResponseViewer.svelte:367` calls `onsettings`, which only switches the builder to the Settings tab (`ApiPage.svelte:440`). On a phone that tab is still above the response the user is reading.

**Fix:** label the button **Set a timeout**. After the click, select that tab and scroll it into view on phone.

## Browser — 8.6

### 9. Saved site credentials cannot be created from the product

`CredentialsPanel` says a caller should mount it (`ui/src/modules/browser/CredentialsPanel.svelte:12-14`). Nothing does. `BrowserSettings.svelte` covers the engine and the panel version only. Autofill is rendered only when a credential already matches (`ui/src/modules/browser/BrowserView.svelte:371` and `:736-746`), so the key button never appears for a new workspace. The panel, if mounted as-is, also uses a “+” glyph instead of `Icon` (`CredentialsPanel.svelte:184`) and an empty sentence instead of `EmptyState` (`:188`).

**Fix:** a header overflow action **Site credentials…** (and the same from the autofill control when nothing matches) opening this panel in a `Modal` or a pushed phone page. Empty state: `EmptyState` with **Add credential** and `Icon name="plus"`. Keep the reveal and delete confirms already in the panel. Add `:focus-visible` on the form fields (they draw their own borders and never replace a ring).

### 10. A summary hides a degraded fetch and is set in capitals

`runSummarize` keeps only `resp.summary` and drops `engine` and `degraded` (`ui/src/lib/stores/browser.svelte.ts:406-408`). The panel (`ui/src/modules/browser/BrowserView.svelte:773-783`) says “Summary · drafted by Otto” and the CSS uppercases it (`:974-983`). Reader view does warn on `page.degraded` (`ReaderView.svelte:224-227`). A summary of a no-JavaScript fetch is shown as the page.

**Fix:** store `engine` and `degraded` with the text. Sentence case, no `text-transform: uppercase`. Under the title: “Drafted by Otto” plus the engine, and when `degraded` is true the same warning as Reader (“no JavaScript ran”). The API does not return a model; don’t invent one.

### 11. The address bar wraps only when the viewport is ≤640px

Tools are `flex: none` (`BrowserView.svelte:931-937`). Wrapping, a full-width field, and 36px tools happen only in `@media (max-width: 640px)` (`:948-964`). Tablet widths with the sidebar open make the content column narrower than the viewport, so the field shrinks to a sliver while Back, Reader/Live, Pick, Summarize, and Save to vault stay on one row. The request builder already uses a container query at 560px (`RequestBuilder.svelte:1895-1897`).

**Fix:** `container-type: inline-size` on `.urlbar` and the same wrap rules at `@container (max-width: 560px)`, not only at a 640px viewport.

### 12. Reader/Live, Send, and mark actions stay under 36px on a phone

Reader/Live segments are 28×30 (`BrowserView.svelte:890-900`). The phone rule grows `.icon-btn.tool` only (`:956-959`), not `.seg`. Ask **Send** is 30×30 and has no `:focus-visible` (`ui/src/modules/browser/AskBar.svelte:189-204`). Mark edit/send/delete are forced to 24×24 (`ui/src/modules/browser/NotesRail.svelte:210-212`).

**Fix:** those controls use `min-width` / `min-height: var(--hit-min)` under `(pointer: coarse)` or `max-width: 640px`. `.send:focus-visible` gets a 2px `--accent-text` outline. Don’t grow the glyphs.

### 13. Delete mark does not ask

`ui/src/modules/browser/NotesRail.svelte:38-44` and the trash button at `:94-96` delete the mark and its note immediately. There is no undo. Credential delete in the same module already confirms.

**Fix:** `confirmer.ask` with `danger: true`, title “Delete mark”, body quoting the first line of the excerpt, `confirmLabel` “Delete mark”.

### 14. Sending a mark starts the agent with no preview

`ui/src/modules/browser/SendToSession.svelte:27-31` posts as soon as a session is chosen (`:45-49`). That route submits straight into the session input (`BrowserSendAnnotationReq` in `ui/src/lib/api/types.ts:8162-8167`). The menu shows session titles, not the excerpt, and two agents often share a title. The toast “Sent to session” doesn’t say which one.

**Fix:** after the pick, a confirm sheet: session title, provider, and the excerpt (truncated). `confirmLabel` “Send mark”. Toast “Sent to {title}”.

## Not scored

No light/dark or phone screenshots were taken. Keyboard behaviour of the shared `Tabs`, `Modal`, and `ctxMenu` was not re-tested here. A separate integrations module was out of scope; Postman import and Git sync were reviewed as part of the API client.
