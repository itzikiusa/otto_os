# Design review 09 — API, Browser, integrations

Reviewer: Grok (design). Date: 2026-10-18. Read-only code review against `docs/design/guidelines/`. No screenshots, no `npm run check`. Light/dark judged from token use (`ui/src/lib/tokens.css`): these modules do not invent hex colours; status, method, and danger/warning copy use the semantic tones. Phone/tablet judged from the 640/1024 rules and from controls that are not the shared `.btn` / `.icon-btn` (those get the coarse-pointer hit pad in `ui/src/app.css`).

Inspected: request builder, response (empty / sending / failure / body), collections, history, automations, import, git sync, browser tabs, reader, live setup / reconnect / approval / page dialog, agent dock and ask bar, Settings → Slack, Telegram & webhooks, Settings → Sharing (Gmail sender, public domain, active links), Settings → Browser engine. Connect, save, test, and load-error paths were traced, not clicked.

## Scores

| Area | Score | Defects |
|---|---|---|
| API request, response, tabs | 8.4 | 3 |
| API Git sync | 8.1 | 1 |
| Browser | 8.3 | 4 |
| Slack, Telegram, webhooks | 8.5 | 2 |
| Sharing (email sender and links) | 8.4 | 2 |
| **This lens** | **8.3** | **12** |

Not 9.8. The chrome, empty/error states, and outward confirms that do exist (test message, open-to-everyone, private addresses, live approvals) are in good shape. The gaps below are what a person actually hits.

## Defects

### 1. Phone: Send does not bring the response on screen

`ui/src/modules/api/ApiPage.svelte:686-698`

On a phone the builder has no max height and the response sits under it (`min-height: 320px` inside a scrolling column). Desktop keeps both panes visible. Nothing scrolls the response into view when a result or a failure arrives.

Impact: after Send, the status, error, and body are below the URL, headers, and body editor. A failed call looks like nothing happened.

Fix: when `viewport.isPhone` and the active tab’s result or failure changes, `scrollIntoView({ block: 'start' })` on `.resp-pane`. Keep the builder collapsed to a summary row until the user expands it.

### 2. Turning off TLS verification does not ask

`ui/src/modules/api/RequestBuilder.svelte:1136-1139`

Unchecking “Verify TLS certificates” writes `verify_ssl: false` on the draft with only the hint “Turn off only for self-signed development servers.” Allowing private addresses, two dozen lines below, uses `confirmer.ask` (`663-668`).

Impact: the next Send, including one with an auth token, accepts any certificate. One mistap is enough. There is no “where / what” confirm.

Fix: when the box goes from on to off, `confirmer.ask` (danger) naming this request’s host and that tokens on it are still sent. Restore the check if the user cancels. Same pattern as `setAllowLocal`.

### 3. Request-tab close targets are 20 px and overlap the tab

`ui/src/modules/api/ApiPage.svelte:633-636` (button at `396`)

`.req-tab-close` is 20×20. The shared coarse-pointer pad (`app.css` `:where(.icon-btn)::after`, inset −6 px) grows a 20 px control to 32 px, still under `--hit-min` (36), and the extra box covers the tab label beside it. Browser tabs do this correctly (`TabStrip.svelte:156-168`).

Impact: on a phone, closing a tab and switching tabs are the same gesture. Dirty-tab discard (`ApiPage.svelte:104-108`) then fires for the wrong tab.

Fix: do not shrink `.icon-btn` below 24 px. On `(pointer: coarse)`, give the close control `inline-size/block-size: var(--hit-min)` the way `TabStrip` does, and keep it from overlapping `.req-tab-main`.

### 4. Git Pull adds a duplicate tree and closes even when it fails

`ui/src/modules/api/GitSyncDialog.svelte:45-49` and `80-85`; create path `ui/src/lib/stores/apiClient.svelte.ts:1063` and `1183-1196`

The sheet says Pull “imports the collection files found there.” `importParsed` always `saveCollection`s a new root. `gitPullCollections` swallows errors into a toast titled “Couldn’t git pull”, and the dialog calls `onclose()` either way.

Impact: a second Pull duplicates every collection. A failed Pull dismisses the sheet, so the repo, branch, and error are gone. The toast title is git jargon, not what happened or what to do.

Fix: before Pull, confirm with the count: “This adds a new copy of each collection. It does not update ones you already have.” Keep the sheet open. On failure, show `LoadState` / `role="alert"` in the sheet with Retry, title “Couldn’t pull collections”, and the reason. Prefer matching by name and updating, if that is the intended sync.

### 5. Reader mode and Live mode are 28 px on a phone

`ui/src/modules/browser/BrowserView.svelte:890-893` and `948-965`

`.seg` is 28×30. It is not `.icon-btn` or `.segmented > button`, so the global coarse-pointer pad does not apply. The phone rule only enlarges `.icon-btn.tool`.

Impact: the control that switches Reader and Live is smaller than Back, Reload, Summarize, and Save, and sits in the wrapped toolbar where mis-taps change mode (Live also starts or closes a Chromium session, `BrowserView.svelte:552-557`).

Fix: on `(pointer: coarse)`, set `.seg` to `min-inline-size` and `min-block-size: var(--hit-min)`.

### 6. Deleting a mark does not ask

`ui/src/modules/browser/NotesRail.svelte:38-44` and `94-96`

`remove` calls `browser.deleteAnnotation` immediately. Credential delete in the same module asks first (`CredentialsPanel.svelte:127-131`).

Impact: the trash icon sits in a 24 px column next to Send. One tap deletes the passage and the note. There is no undo.

Fix: `confirmer.ask` with `confirmLabel: 'Delete mark'`, `danger: true`, and the excerpt in the body. Only then delete.

### 7. The article title is a second h1

`ui/src/modules/browser/ReaderView.svelte:258` and `ui/src/lib/components/PageHeader.svelte:382`

The module header is already `<h1>Browser</h1>`. The fetched page title is another `<h1>`.

Impact: heading navigation announces two page titles. The article is not nested under the module.

Fix: make `.page-title` an `<h2>`. Keep the current `font-size: var(--fs-2xl)`.

### 8. Ask-bar Send has no focus ring and a 30 px hit target

`ui/src/modules/browser/AskBar.svelte:124-126` and `189-204`

`.send` is a custom button, not `.icon-btn`, so it misses the shared focus ring and the coarse-pointer pad. Styles set `outline` nowhere on it. The textarea correctly replaces `outline: none` with a border and shadow (`152-155`).

Impact: keyboard users cannot see that Send is focused. On a phone the control is 30 px.

Fix: use `.icon-btn` with the accent fill (or add `:focus-visible` outline `2px solid var(--accent-text)`), and `min-block-size: var(--hit-min)` under `(pointer: coarse)`.

### 9. A page summary is not attributed

`ui/src/modules/browser/BrowserView.svelte:777-783`; data available on `BrowserSummarizeResp` (`ui/src/lib/api/types.ts:8155-8159`)

The panel says “Summary · drafted by Otto” and then the text. The response also has `engine` and `degraded`. Neither is shown. There is no time.

Impact: a summary produced by a degraded fetch (no JavaScript) reads the same as a full one. There is no way to tell when it was drafted or that it is not the page itself. The label is the attribution rule in `patterns.md` (who, and when; model only if the API returns one — it does not).

Fix: “Draft · Otto · {engine} · {rel time}”. If `degraded`, add the same sentence Reader already uses (`ReaderView.svelte:226`). Keep it a draft: do not write it into the vault until the person confirms Save.

### 10. Slack names fail silently

`ui/src/modules/settings/Channels.svelte:51-53`

A thrown directory request sets `slackDir = null` and does not surface `loadErrorText`. `channels_error` / `users_error` render only when the payload itself carries them (`754-755`, `798`).

Impact: Choose channel and people’s names never appear. The card and the allow-list stay on raw `C…` / `U…` ids with no error and no retry. A network failure looks like “Slack has no directory.”

Fix: remember the failure. Under Allowed users and Default channel, show the reason and a Retry button. Leave id entry usable.

### 11. “Add an allowed user” is a toast, and “allow everyone” means two different things

`ui/src/modules/settings/Channels.svelte:246-251` and `777-779`

Enabling Slack or Telegram with an empty allow-list and without “Open to everyone” toasts and returns. The allow-list is not marked invalid. The webhook field says “Leave blank to allow everyone” (`777-779`) while the key hint (`693-696`) still requires `X-Otto-Webhook-Key`.

Impact: the toast is the only error, off the field, and it disappears. On a webhook, “everyone” reads as unauthenticated. The key is still required; the caller id is what is unrestricted.

Fix: put `role="alert"` under Allowed users, set `aria-invalid` on the add field, and focus it. Do not use a toast for this. Change the webhook hint to “Leave blank to accept any caller id. Callers still send the webhook key.”

### 12. Locked Gmail address and share Revoke are undiscoverable on a phone

`ui/src/modules/settings/EmailSenderSetup.svelte:216-224` and `ui/src/modules/settings/ActiveShareLinks.svelte:95-127` (no `@media` in that file)

The address input is `disabled` while a password is stored. The only explanation is `title` (“Replace the app password to change the sender”). Touch has no tooltip. Replace sits on the password row, not the address.

Active links are a five-column table. Revoke is `white-space: nowrap` in the last column (`ActiveShareLinks.svelte:151`). The wrap scrolls horizontally (`137`). There is no phone layout.

Impact: changing the sender looks broken. Revoke, the action that disconnects guests, is off the first screen of a phone. Horizontal scroll inside a settings card is easy to miss, so a live link looks permanent.

Fix: under the disabled address, always show “Replace the app password to change this address.” Below 640 px, render each share as a stacked card: session, access, expiry, and Revoke on the card. Drop `display: flex` on the `td` (`.sess` at `149`) so the table stops fighting its cells.

## Checked and not filed

- Test message uses `confirmOutward` with where / what / who (`Channels.svelte:408-423`). Open-to-everyone and Allow sender confirm with danger (`254-259`, `348-351`).
- Private-address block in the response explains why and offers one admin action (`ResponseViewer.svelte:355-370`).
- Live engine setup has missing / installing / failed / unsupported / no-admin states (`LiveEngineSetup.svelte:65-154`). Reconnect and “session ended” are inline, with Retry and Reader (`RemoteLiveView.svelte:936-970`). Approvals name who, what, why, and when (`ApprovalCard.svelte`).
- Email SMTP failure is inline (`EmailSenderSetup.svelte:199-202`), not only a toast. Load failures use `LoadState`.
- API onboarding hides the header primary while `EmptyState` owns the one action (`ApiPage.svelte:324-326`, `333-340`).
- No hex colours, native `confirm` / `prompt` / `alert`, or undefined tokens in these modules. Method colour is backed by the word (`MethodTag.svelte`).

## Unverified

Not run in a browser. Contrast of `--text-dim` on `--surface-2` and of gold `--warning` on `--warning-soft` in Warm dark was not measured; those tokens are claimed text-safe by `tokens.css` and `unit/tokenContrast.test.ts`. RTL was checked only for logical properties already in these files, not by flipping `dir`.
