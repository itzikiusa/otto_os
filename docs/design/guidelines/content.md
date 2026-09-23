# Content: voice and copy

How Otto talks: labels, messages, empty states, numbers and dates. Good copy is
part of the design. A clear interface with vague or jargon-heavy words still
feels broken.

---

## 1. Voice

- **Calm, direct and exact.** Write as a senior engineer would to a colleague:
  say what happened, why, and what to do next.
- **Plain words over jargon.** Technical terms are fine when they are the
  user's terms (branch, pod, topic, migration). Otto's internal names are not
  fine: enum values, route ids, crate names.
- **No hype, no cheer.** No "Awesome!", no exclamation marks, no emoji in UI
  chrome, no "Oops". Success is quiet: "Saved", "PR draft opened".
- **Say "you"** for the user, and name the agent when an agent did something
  ("Otto drafted…", "Iris is reviewing…"). Avoid "we".
- **Be short.** A subtitle is one line. An empty-state body is one or two
  sentences. A toast title is 2–6 words.

## 2. Verbs

Buttons and menu items are **verbs**, plus the object whenever the verb alone
is ambiguous. Use one word per concept everywhere:

| Verb | Means | Not |
|---|---|---|
| **New** | Opens a creator: "New workflow" | "Create new workflow", "+ Add" for creation |
| **Create** | Commits a creator (the modal's primary): "Create workflow" | "Submit", "OK", "Save" (for new things) |
| **Add** | Attaches an existing thing: "Add account", "Add artifact" | using "New" for this |
| **Save** | Persists edits to an existing thing | "Apply" (for a settings form), "Update" |
| **Apply** | Accepts a proposal or variant into the current state | "Accept" in one place and "Use" in another |
| **Delete** | Permanently destroys | "Remove" (for a real delete) |
| **Remove** | Detaches or unregisters; the thing survives elsewhere | "Delete" (for a detach) |
| **Archive** | Hides and keeps recoverable | |
| **Refresh** | Fetches the data again | "Reload" (that means reloading the UI: ⌘⇧R) |
| **Run** / **Stop** | Starts / ends running work | "Execute", "Kill" (except for an actual process kill) |
| **Cancel** | Dismisses a dialog, or aborts a pending operation that hasn't started | |
| **Open** | Navigates to or reveals | "View", "Go" |

- **Never** use "OK", "Yes", "Submit" or "Confirm" as a primary label. The
  confirm button repeats the verb: dialog "Delete workflow?" → button
  "Delete".
- A trailing **ellipsis `…`** (the single character, never `...`) means the
  action asks for more input before it happens: "Rename…", "Hand over…",
  "Delete…" in a menu that then confirms.

## 3. Capitalisation and punctuation

- **Sentence case for everything:**
  - buttons
  - page titles
  - tabs
  - menu items
  - ⌘K commands
  - section titles (written in sentence case in the source even when CSS
    uppercases them)
  - dialog titles

  "Add account", not "Add Account"; "Scheduled tasks settings", not "Scheduled
  Tasks Settings".
  - This deliberately departs from macOS's Title Case menus. Sentence case is
    Otto's established convention (165 of 190 button labels already use it),
    and consistency matters more.
  - Proper nouns and product names keep their own casing: GitHub, Kubernetes,
    ClickHouse, Otto, MCP, AWS, Jira, Mission Control and other module names.
  - Some existing ⌘K titles are Title Case ("Restart Focused Session"). That is
    debt; new commands use sentence case.
- No full stop at the end of labels, titles, tooltips or single-sentence
  hints. Use full stops in multi-sentence bodies.
- Use typographic characters: `…`, `—`, `·` as a separator, `→` in flows,
  curly quotes in prose. Use `×` only for dimensions.
- Use numerals for all numbers ("3 runs", not "three runs").

## 4. Error messages

Every error answers **what happened, why (if known), and what to do**, in the
user's terms.

| Bad | Good |
|---|---|
| `Error: Request failed with status code 409` | **Couldn't rename the workflow.** A workflow named "deploy" already exists. Choose another name. |
| `TypeError: Cannot read properties of undefined` | **Couldn't load proof packs.** The daemon returned an unexpected response. Retry, or check Settings → Logs. |
| `git error` | **Pull failed: your branch has diverged.** Your local changes are still stashed. Rebase or merge, then pull again. |
| `Forbidden` | **You don't have access to Kubernetes.** Ask a workspace admin to grant the Kubernetes feature. |

- **Title:** what failed, as a sentence fragment that starts with the verb the
  user tried: "Couldn't delete…", "Pull failed…". Never the raw exception.
- **Body:** the cause and the next step. Raw detail (status codes, stderr) goes
  last, dim, or behind a "Details" disclosure.
- **Don't blame the user**, and don't apologise at length. "Couldn't" is
  enough.
- Map known `ApiError` codes to human text at the call site (403 → access, 404
  → gone, 409 → conflict, network → "Otto can't reach the daemon").
- Where the error is shown: an inline error for failed loads and invalid
  fields, and a toast for failed *actions*
  ([components.md → Toasts](./components.md#10-toasts-vs-inline-messages)).

## 5. Empty states, placeholders, tooltips

- **Empty-state title** states the situation: "No proof packs yet", "No
  workflows match 'deploy'". The **body** says what the thing is for, in one
  sentence, and doubles as the explanation of a jargon module name ("Verified
  evidence — tests, diffs, CI, reviews, approvals — assembled for each piece of
  work."). The **CTA** is the verb from [§2](#2-verbs).
- **Placeholders** are examples ("Weekly dependency report",
  "https://api.example.com"), not instructions and never the label.
- **Tooltips** (`title`) name the control or add one fact. For an icon button
  the tooltip equals its `aria-label`. For disabled controls, the tooltip says
  why they are disabled.
- **Loading text** names the thing: "Loading proof packs…", "Assembling…",
  "Preparing…". Not a bare "Loading…".

## 6. Names on screen

- Show the **label, never the id**. Module names come from `SIDEBAR_MODULES`
  labels and `moduleLabel()`, not from route ids ("Mission Control", not
  "Mission-Control" or "mission-control").
- **Enum values** are mapped to words, not just de-underscored:
  `pending_approval` → "Waiting for approval", `dry_run` → "Dry run". A shared
  mapper per domain (like `McpPill`) keeps them consistent.
- Cadences read as people say them: "Weekly (Mondays 09:00)", "Every 15
  minutes", not "every 10080 min". Replace an empty target like "→ none" with
  what it means ("Not delivered anywhere").
- Seeded or example content is neutral and generic (no customer-specific
  examples in a fresh install).

## 7. Numbers, dates and units

Use the shared helpers. Don't write another local `timeAgo` (the audit found 21
of them).

| Need | Helper | Output |
|---|---|---|
| Relative time that ticks | `rel(ts)` from `lib/stores/now.svelte.ts` (reactive; read it in markup or `$derived`) | "now", "12s ago", "3m ago", "in 2h", a date after about 30 days |
| Byte sizes | `formatBytes(n)` (`lib/metric-format.ts`) | "512 B", "1.2 MB" |
| Large counts | `formatCount(n)` | "980", "12.4k", "3.1M" |
| Durations (seconds in) | `formatSeconds(s)` | "38 ms", "4.2 s", "1.5 min", "2.1 h" |
| Chart values by unit | `formatMetric(v, unit)` / `formatTimeTick` | "—" for null |

- **Relative in lists, absolute on hover:** show `rel(ts)` and put
  `new Date(ts).toLocaleString()` in the `title`.
- Show absolute dates in the user's locale (`toLocaleDateString()` /
  `toLocaleString()`), and never hand-build `MM/DD`.
- **Money:** `$` with 2 decimals under $1,000 ("$0.42", "$12.30"), and
  grouping above that. Don't mix mono and sans for money in one row.
- **Percentages:** whole numbers unless the difference matters ("99.9%").
- Put units in column headers or labels, not on every cell.
- **TBD:** the code audit proposes one `lib/format.ts` (relTime, dateTime,
  duration, bytes, count, usd, pct, with cached `Intl` formatters). Until it
  exists, the helpers above are the canonical ones.
