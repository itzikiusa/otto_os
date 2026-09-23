# Otto design guidelines

The rules every Otto UI feature follows, whether a person or a coding agent
builds it. They are grounded in the code that exists today: every token,
component and helper named here is real. Where something is still on an
unmerged branch or only proposed, it is marked (see [Status markers](#status-markers)).

Otto is used mostly by technical people, and that is never a reason for it to
look unfinished. The goal is a tool that feels like a well-made Mac app: calm,
dense where it has to be, consistent from one module to the next, and quiet
enough that the user's work (code, sessions, data) is the loudest thing on the
screen.

---

## Principles

1. **macOS-native, not a desktop-in-a-window.** Otto should behave like Finder,
   Mail or Xcode, not like an operating system drawn inside a web page:
   - a grouped source-list sidebar
   - one unified title bar and toolbar row per page (`PageHeader`)
   - ⌘K as Spotlight
   - sheets for dialogs
   - vibrancy on chrome only
   - real native windows when something pops out

   No in-app window managers, docks, taskbars or draggable inner windows. Home
   is the one "desktop": a grid of live widgets.
2. **Content first, chrome quiet.** The eye should land on the user's data or on
   the single primary action. Accent colour is for selection, focus and the
   primary action, and nothing else. If the navigation is the brightest thing on
   screen, the design is wrong. (It used to be: see the lime active item in the
   [visual review](#background-reading).)
3. **Dense but readable.** Technical users want a lot on screen, so get density
   from tight spacing and good alignment, never from tiny text:
   - body text is 13 px
   - nothing a user has to read is smaller than 11 px
   - there is one type scale
4. **One product, not 30 pages.** Every page:
   - picks one of the [page archetypes](./layout.md#4-page-archetypes)
   - renders the shared `PageHeader`, `PageBody` and `EmptyState`
   - uses the same tokens

   A user moving between Git, Swarm and Usage should never notice a different
   header height, title size, empty-state style or button placement.
5. **Restraint.**
   - One primary action per view.
   - Colour carries meaning (danger, warning, success, info, selection) and is
     never decoration. No rainbow category chips.
   - No second call to action that does the same thing as the first.
   - Nothing is added to the chrome that a menu could hold instead.
6. **Agents are first-class, and visible.**
   - Anything an agent produced is attributed and reviewable.
   - Agent output stays a draft until a person applies it.
   - Anything that leaves the Mac asks first. This is the UI side of the
     [AGENTS.md](../../../AGENTS.md) rule "ask before outward-facing actions".
     See [patterns.md](./patterns.md).
7. **Works everywhere it ships.** Every view must work in:
   - all five theme and scheme combinations (Native light/dark, Pro Dark, Warm
     light/dark)
   - right-to-left layouts
   - phone, tablet and desktop widths
   - a custom accent colour the user picked

## Visual target

The **Design Hall mockup** (published; its sources and screenshots are in the
design-hall scratchpad of the 2026-09-23 design pass) is the reference for how a
finished Otto surface looks:

- a grouped sidebar with section labels and a quiet accent-tint active row
- a single 46–48 px toolbar row: title, inline segmented control, search, one
  primary action
- hairline borders rather than shadows on cards
- status shown with small dots and soft-tinted pills
- agent messages marked with a consistent agent label and provenance chips
- an outward action ("Publish as claude.ai artifact") that always goes through a
  confirmation sheet saying where the content goes and who can see it

The user also pointed to two outside references:

| Reference | Take | Don't take |
|---|---|---|
| **cnvs.dev** | A full-bleed ambient (blurred) backdrop seen through translucent chrome. Almost no chrome. One floating glass pill at the bottom ("Type or speak… ⌘K · model picker · spaces 01–04"). | Chrome-less layouts on data-dense tool pages. Glass behind text-heavy content. |
| **bridgemind.ai** | Confident, calm dark UI; generous spacing around a few strong elements; agent activity presented as a first-class feed. | Marketing-page scale type and hero sections inside the app. |

The floating command bar and ambient backdrop are written up as a **proposed**
pattern for future canvas/studio and Home surfaces in
[layout.md → Floating command bar](./layout.md#7-floating-command-bar-proposed).
They are not a licence to restyle list, detail or settings pages.

## How to use these docs

| Doc | Read it when you… |
|---|---|
| [foundations.md](./foundations.md) | pick a colour, font size, spacing, radius, shadow, layer, animation, icon or translucency |
| [layout.md](./layout.md) | add a page or module, place a header, choose a page archetype, handle phone/tablet |
| [components.md](./components.md) | need a button, form, tabs, chip, table, dialog, confirm, menu, toast, or an empty/loading/error state |
| [patterns.md](./patterns.md) | build agent-facing UI (presence, agent-authored content, variants, provenance, approvals), dense data views, destructive actions or keyboard flows |
| [content.md](./content.md) | write any user-visible text: labels, errors, empty states, numbers, dates |
| [accessibility.md](./accessibility.md) | check contrast, focus, keyboard, ARIA, RTL; run axe |
| [review-checklist.md](./review-checklist.md) | open or review a UI PR (humans and agents both run it) |

If a rule here conflicts with the surrounding code, the rule wins for **new**
code. Existing drift is fixed when you touch that file, not in drive-by
repo-wide rewrites. If a rule here conflicts with reality, for example because
a component API changed, fix the doc in the same PR.

### Status markers

| Marker | Meaning |
|---|---|
| (none) | Shipped. Use it. |
| **Proposed** | A design direction with no code yet. Don't reference tokens or components marked Proposed. `npm run check` fails on an undefined `var(--x)`. |
| **TBD** | An open decision or a planned follow-up. |

## Checklist for a new feature or page

A short version. The full list is [review-checklist.md](./review-checklist.md).

1. **Place it.**
   - Add the module to `SIDEBAR_MODULES` (`ui/src/lib/sidebar.ts`) with a
     unique icon, a short label (module names are proper nouns: "Scheduled
     Tasks"), its `group` and `keywords`.
   - Don't hand-write a ⌘K "Go to" entry; it is generated from the registry
     (see [layout.md → Sidebar](./layout.md#2-sidebar)).
2. **Pick an archetype.** List/detail, workbench/editor, dashboard, settings
   form, or canvas/studio. Follow that archetype's rules in
   [layout.md](./layout.md#4-page-archetypes).
3. **Use the chrome.**
   - One `PageHeader` (a title, at most one primary action, overflow priorities
     on the rest).
   - `PageBody` with `width="full"` or `"readable"`.
   - `EmptyState variant="page"` for an empty page.
4. **Use tokens only.** Semantic colours (`--danger`, `--success`…), the
   `--fs-*` scale and `--radius-*`. No hex values in module styles and no font
   sizes below 11 px.
5. **Use the shared components.**
   - `.btn` and `.icon-btn` for buttons, and `Modal` for dialogs.
   - `confirmer` for confirmations and text prompts. Never native
     `confirm()`/`prompt()`.
   - `ctxMenu` for menus and `toasts` for action results.
   - Design all four states: loading, empty, error and loaded.
6. **Make it keyboard-reachable.**
   - Register ⌘K commands for the main verbs.
   - Every icon button has an `aria-label` and a `title`.
7. **Agents.** Attribute agent output, keep it a draft until applied, and put a
   confirmation in front of anything outward-facing.
8. **Verify.**
   - `npm run check` passes.
   - Test in light and dark at 1440 px and on a phone.
   - Floating UI passes `expectFullyInViewport` with enough items to overflow.
   - No horizontal page scroll.

## Background reading

The 2026-09-23 design pass that motivated these guidelines produced:

- **Visual review**: a screenshot sweep of all 30 routes in both schemes, plus
  1100/1024 px widths and a phone. It has the top-10 fixes and per-module
  notes: the lime active item, the flat 27-item sidebar, "pick one" detail
  panes, four empty-state styles, 25 font sizes, and more.
- **Code audit**: counts and file:line evidence.
  - 55 CSS variables used but never defined; the ui-guards now block these.
  - 583 raw colours.
  - 15 native `confirm()`/`prompt()` calls; now blocked.
  - 1,040 custom-styled buttons.
  - 358 local chip/badge style blocks.
  - 35 z-index values.
  - 172 suppressed a11y warnings.
  - a proposed roadmap for primitives.

Both are working documents from that pass. This directory is the durable
outcome.
