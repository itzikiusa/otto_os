# Patterns

Recurring interaction patterns. Otto is an app where **agents do real work next
to people**, so most of this doc is about making that work visible,
attributable, reviewable and safe. The rest covers dense data views, destructive
actions and keyboard use.

---

## 1. Agent presence and live status

People need to see at a glance **which agents exist, what each one is doing,
and whether one needs them**.

| Signal | Component and token | Meaning |
|---|---|---|
| Working | `StatusDot status="working"`, `--status-working`, pulsing | Actively producing output right now |
| Running | `StatusDot status="running"`, `--accent` | Alive, not currently producing |
| Idle | `StatusDot status="idle"`, `--status-idle` | Waiting, nothing expected |
| Needs you | `StatusDot needsYou`, amber pulse (`--status-warn`) plus a "Needs you" label | Blocked on a person: a question, an approval, or input |
| Exited / failed | `StatusDot status="exited"`, `--status-exited` | Stopped. Say why in words nearby. |
| Reconnectable | `StatusDot status="reconnectable"`, `--status-warn` | Can be resumed |

Rules:

- **"Needs you" is the only attention state that interrupts.** It may raise a
  notification (the bell) and filter the Agents list (`ws.needsYouFilter`).
  Everything else is ambient. Don't raise "awaiting input" for sessions where
  idleness is normal, such as a plain `shell` provider.
- **An agent is identified by a provider mark plus a name.** Use `ProviderIcon`
  (a Claude, Codex or Antigravity mark, or a monogram tile for any custom
  provider) next to the session title, role or persona name, and the model when
  it matters ("claude · Sonnet"). Named agents (Swarm roles, Personal Agents)
  may use a monogram avatar tile. **No emoji avatars.**
- **Activity lines** read as *who · what · when*: "Iris drafting *Social kit – IG
  story* · 2m ago · Open session". Use `rel()` from `lib/stores/now.svelte.ts`
  for the time; it ticks by itself.
- **The live count lives in the chrome, not in every page.** The status bar
  shows "● N working" and the Agents row shows a count. A page shows presence
  only for agents that belong to it.
- **Long-running work shows progress honestly.** A stage rail (see
  `run-with-otto/RunStageRail.svelte`) or "step 3 of 5" plus elapsed time is
  better than an indeterminate spinner. **Cancel/Stop is always reachable**
  while something runs.
- **Stale is a state.** If live updates stop (the WebSocket drops, the daemon
  restarts), say so ("Reconnecting…", "Last update 5m ago"). Don't keep
  showing "working" forever.

## 2. Agent-authored content

Anything an agent wrote (a message, a draft, a code change, a rule, a note, a
PR description) must be:

1. **Attributed.** Show the agent's identity (`ProviderIcon` plus name, model if
   relevant) and the time. On versions: `v13 (Otto)` vs `v12 (you)`.
2. **Visually distinct from human content, but not louder.** An agent message
   gets a small `AGENT` label (`.chip`, `--fs-xs`) next to the name and a 2 px
   inline-start rule. Human messages have neither.
3. **A draft until a person applies it.** Agent output that changes something
   shared (a document, a design, a config, a repo) arrives as a proposal with
   explicit **Apply / Discard** (or Approve / Edit / Reject). Applying creates a
   new version; it never overwrites without a trace.
4. **Editable after the fact, and tracked.** If a person edits agent output,
   the version records both ("v15 · Otto, edited by you").

**Agent colour: TBD.** The Design Hall mockup uses a dedicated violet
(`--agent`) for agent labels, avatars and the message rule. That token isn't in
`tokens.css`, and using it would fail `npm run check`. Until it lands (defined
per scheme and contrast-checked like the tones), use:

```css
.msg.agent { border-inline-start: 2px solid var(--border-strong); padding-inline-start: 10px; }
.agent-label { /* on a .chip */ color: var(--text-dim); }
```

Don't use `--accent` for agent identity. Accent means *selected* or *primary*,
and an agent message is neither.

## 3. Variants

When an agent offers alternatives (hero designs, commit messages, fixes):

- Show **2–4 options side by side as equal cards**: a thumbnail or preview, a
  2–3 word name ("Bold tilt"), and one line on how it differs.
- Each option has the same actions: **Apply** · **Compare** · a reject control
  that asks *why* (a small popover of reason chips such as "Too busy",
  "Off-brand", "Not accessible", "Other"). The reason feeds the learning loop.
- Nothing is pre-applied. The current state stays until the user applies an
  option.
- **Compare** opens side-by-side, slider or change-list views between any two
  versions. Restoring an old version creates a new one ("nothing is lost").

## 4. Provenance and citation chips

Agent output should show **what it was built from**, so people can check it.

- **Citation chips** (`R1`, `R2`, `Brand Kit v4`, `LOY-142`) sit inline under
  the message or draft that used them. They are neutral `.chip` pills with the
  source's kind icon, and clicking one opens the source.
- **Context chips** sit above a composer and list what the agent will see: the
  selected section, a linked story, references. Each has a remove ✕, so the
  user controls the context before sending.
- **Evidence lines** on derived items ("from 3 accepted variants, 1 edit",
  "picked the bolder variant 3 of 4 times") use `--fs-xs`, `--text-dim`, and a
  `clock` icon.
- A *lineage* view (derived from / uses / embeds / implements) is a plain
  indented list with relation labels. It is not a graph unless the user asks
  for one.
- When a rule was applied automatically ("Applied your team rules: *no stock
  photos in heroes*"), say so and link to the rule.

## 5. Approvals and outward-facing actions

[AGENTS.md](../../../AGENTS.md) requires agents to **ask before irreversible or
outward-facing actions**. The UI enforces the same line for everyone.

**Outward-facing** means anything that leaves this Mac or is seen by other
people:

- opening or merging a PR, pushing
- posting a Jira or Confluence comment, or editing a page
- sending to Slack, Telegram, email or a webhook
- publishing (for example as a claude.ai artifact)
- sharing a session remotely
- running against a **prod** connection

The confirmation for an outward action must show:

- **where** it goes: the destination, repo, channel or URL
- **what** is sent: a preview, a diff, or a size summary
- **who** will see it: its visibility ("Private until you share", "Posts to
  #payments")

The primary button names the action ("Publish", "Post comment", "Open PR"),
never "OK". Use `confirmer.ask(…, { danger: false, confirmLabel: 'Publish' })`
for a short message. Use a `Modal` when there is a preview. The Design Hall
"Publish as claude.ai artifact" sheet is the model: a warning callout, a page /
size / embed summary, then Cancel · Publish.

**Approval queues** (Mission Control, MCP → Approvals, a workflow's
`human_approval` step, Run with Otto's approval gate) all use one shape:

- a pending count in `--warning` wherever the queue is surfaced
- per item: **who asked** (agent identity), **what** (the tool or action and
  its arguments or diff), **why** (the agent's stated reason), and **when**
- **Approve** (the primary action) and **Deny** (asks for an optional reason)
  on each item. Batch approval is allowed only for identical, low-risk items.
- decisions are recorded and visible in an audit list: approved, denied,
  expired, and by whom

Never auto-approve silently. A "remember my choice" option is explicit
(`confirmer.choose(…, { checkboxLabel })`), scoped (per repo, per tool) and
reversible in Settings.

**Prod guardrails.** Connections marked `prod` refuse writes and DDL by default,
and a prod tab shows a red rail and banner (`ConnectionForm.svelte`, the
database tab). Any new feature that can act on a connection, cluster or account
inherits the same environment marking and confirmation.

## 6. Data-dense views

### Logs and terminal output

- `--font-mono` at `--fs-s`, on `--term-bg`. Line numbers and timestamps are
  dim.
- **Follow tail by default, and pause when the user scrolls up.** Show a
  "Jump to latest" pill while paused.
- Support ⌘F (`FindInPage` / the terminal's own find), a wrap toggle, and copy
  of a selection or the whole log.
- Severity is shown by a small tone marker or text label per line (`ERROR` in
  `--danger`). Don't colour whole lines.
- Very long output is virtualised (`VirtualList`) or truncated with "Show all
  N lines".

### Diffs

- Use `DiffView`. Additions and deletions use `--success-soft` and
  `--danger-soft` backgrounds, **plus** the `+`/`−` gutter. Colour is never the
  only signal.
- A file header row shows the path (mono, middle-truncated), change counts, and
  stage, discard or comment actions on hover.
- Agent-proposed diffs follow [§2](#2-agent-authored-content): they are
  attributed, and applied explicitly.

### Grids and results

- Use the Database Explorer grid for query results (virtualised, sticky
  header, copy cell/row/insert). Don't reimplement it.
- Numbers are right-aligned and tabular. Nulls are dim `NULL`. JSON cells open
  a viewer instead of expanding the row.
- Show the result's shape near the grid: "1,204 rows · 38 ms · page 1 of 13".

### Identifiers

- Show IDs (ULIDs, SHAs, ARNs) shortened in mono (`01J9…X4QZ`, `a1b2c3d`), with
  the full value in `title` and copy on click. Don't put raw IDs in list rows;
  they belong in the detail view.

## 7. Destructive actions

| Verb | Meaning | Confirm? |
|---|---|---|
| **Delete** | Permanently destroys the thing (and possibly its data) | Always. Name the object and the consequence. |
| **Remove** | Detaches it from here; the thing still exists (remove from list, unlink, unregister) | Only if re-adding is costly |
| **Archive** | Hides it and keeps it recoverable | No. Offer "Show archived". |
| **Discard** | Throws away unsaved or unapplied changes | If more than a trivial edit would be lost |
| **Stop** / **Cancel** | Ends running work / dismisses a dialog or pending operation | Stop: if it loses work in progress |

- The request can sit in a menu (`danger: true` row) or be a `.btn.danger`.
  The **commit** happens in the confirm dialog, whose red filled button repeats
  the verb ("Delete").
- Say what else goes with it: "Its token is removed from the Keychain", "3
  runs and their logs are deleted".
- Prefer designs that don't need a confirm: archive instead of delete, versions
  instead of overwrite. **TBD:** undo toasts, once toasts support actions.
- Never put a destructive action right next to the primary action, or in a
  spot where a double-click could hit it.

## 8. Selection and state memory

- List/detail pages remember their selection per device
  (`lib/lastSelection.ts`, **in flight: `feat/page-chrome`**). Sidebar layout,
  folded sections, theme and similar live in the `ui` store (localStorage, per
  device).
- **Every `localStorage` access is wrapped in try/catch**: private windows and
  blocked storage just mean nothing is remembered. Nothing that has to be
  durable or shared lives there; that belongs in the daemon.
- A selection is part of the URL when it should survive reload or be shareable
  (`#/insights/health`, `#/agents/<id>`).

## 9. Keyboard-first

Technical users live on the keyboard. Every feature must be fully usable
without a mouse.

**⌘K is Spotlight.** Register your module's main verbs with the command
registry (`lib/commands.svelte.ts`) from an `$effect`, under an owner key. The
effect's cleanup unregisters them.

```ts
import { registry } from '../../lib/commands.svelte';

$effect(() => {
  if (!proof.detail) return registry.register('proof', []);
  const pack = proof.detail.pack;
  return registry.register('proof', [
    { id: 'proof.assemble', title: 'Assemble proof pack', group: 'Proof',
      keywords: 'rebuild refresh evidence', run: () => assemble(pack.id) },
    { id: 'proof.waive', title: 'Waive proof pack…', group: 'Proof', run: openWaive },
  ]);
});
```

- `group` is the module name, `keywords` holds synonyms, and `shortcut` is a
  display-only hint. Titles are sentence case. A trailing `…` means the
  command asks for more input.
- Register **context commands** only while their context exists (the focused
  session, the open item).
- "Go to <module>" commands come from the sidebar registry. Don't add them by
  hand ([layout.md](./layout.md#2-sidebar)).

**Global shortcuts** live in one place: `lib/keys.ts` (`installKeyMap`, the
`KeyAction` union) plus the `KEYMAP` table, which is the only source for the
`?` cheat sheet. Adding a shortcut means changing both.

- Use ⌘ chords for global actions. Bare letters are only for a focused widget
  (a grid, a canvas) and never while a text field or terminal has focus.
- Don't take chords that macOS, WebKit or the terminal already own (⌘Q, ⌘H,
  ⌘`, ⌘C/V/X/Z/A, ⌃C in a terminal). Check `KEYMAP` for clashes. The session
  chords (⌃1…⌃9, ⌘[, ⌘], ⌘D) are taken.
- `Esc` closes the top-most layer (a menu, then a sheet, then a find bar), one
  at a time. `Enter` submits a single-field dialog, and ⌘Enter submits
  multi-line composers.
- After an action, focus lands somewhere sensible: back on the trigger, on the
  new item, or in the next field. Focus is never lost to `<body>`.

**In lists and trees:** ↑/↓ move, Enter opens, Space toggles and ⌫ requests a
delete (which still confirms). Arrow-key navigation inside
`role="menu"`/`tablist` is expected; see
[accessibility.md](./accessibility.md#3-keyboard).
