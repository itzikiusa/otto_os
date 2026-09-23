# UI review checklist

The author runs this before asking for review, and the reviewer (a person or
an agent) runs it on every PR that touches `ui/`. The links go to the rule
behind each item.

---

## 1. What the tooling already enforces

`npm run check` (from `ui/`, CI-enforced) runs these in order:

| Step | Fails on |
|---|---|
| `node scripts/ui-guards.mjs` | **Native dialogs:** any `confirm(` / `prompt(` / `alert(` call (they are silent no-ops in the Tauri WKWebView). **Undefined CSS variables:** any `var(--x)` whose `--x` isn't defined anywhere under `ui/src/`, *even with a fallback*. |
| `svelte-check` (`tsconfig.app.json`) | Type errors, including **unknown icon names** (`IconName`), wrong component props, and Svelte a11y warnings that aren't suppressed |
| `tsc` (node, e2e, unit tsconfigs) | Type errors in config, e2e specs and unit tests |

About the guards:

- A component-local knob counts as defined if you declare its default on the
  component root (`--knob: 8px;`), or pass it as a `style:--x` / `--x=` prop.
- Third-party variables are allow-listed by prefix (`EXTERNAL_PREFIXES`,
  currently `--xy-`).
- Escape hatch: a `ui-guards: allow` comment on the same line. Every use needs
  a reason in the PR description. Reviewers should push back on it by default.

**What the tooling does not catch.** Reviewers must look for these by hand:

- literal hex and rgb colours in module styles
- `font-size` in px, and anything below 11 px
- invented z-index values
- hand-rolled modals and popups
- `outline: none` with no replacement
- raw errors in toasts
- new `svelte-ignore a11y_*` comments
- contrast of tinted surfaces

Quick greps over the diff:

```bash
# from the repo root, against the PR's base
B=integration/2026-09-23-overhaul
git diff "$B"... -- ui/src | grep -E '^\+.*#[0-9a-fA-F]{3,8}\b'                   # literal colours
git diff "$B"... -- ui/src | grep -E '^\+.*font-size:\s*[0-9.]+px'                  # px font sizes
git diff "$B"... -- ui/src | grep -E '^\+.*z-index:\s*[0-9]+'                       # z-index literals
git diff "$B"... -- ui/src | grep -E '^\+.*outline:\s*(none|0)'                     # removed focus rings
git diff "$B"... -- ui/src | grep -E '^\+.*svelte-ignore a11y'                      # new a11y suppressions
git diff "$B"... -- ui/src | grep -E '^\+.*toasts\.error\([^)]*(String\(e|e\.message)' # raw errors as toast titles
git diff "$B"... -- ui/src | grep -E '^\+.*(100vh|position:\s*fixed)'               # hand-rolled overlays / WKWebView 100vh trap
```

Each hit isn't automatically wrong (a brand mark may use hex, and a shadow may
use `rgba(0,0,0,…)`), but each one needs a reason.

## 2. The checklist

### Structure

- [ ] The page follows one [archetype](./layout.md#4-page-archetypes) and its
      rules.
- [ ] There is exactly one `PageHeader` (Agents excepted) and one `<h1>`. The
      title is the module name or the selected item's name.
- [ ] There is at most one `.primary` per view. Destructive actions collapse
      first (`data-overflow` lowest) and don't sit next to the primary.
- [ ] Icon-only header actions have `aria-label`, `title` and `data-icon`
      (`data-label` if the menu text should differ).
- [ ] `PageBody` uses `width="full"` or `"readable"`. No centred narrow island.
- [ ] A new module is registered in `SIDEBAR_MODULES` with a **unique icon**,
      its `group`, `keywords` and an RBAC gate. There is no hand-written ⌘K
      "Go to" entry.
- [ ] List/detail pages open on an item (`initialSelection` /
      `rememberSelection`), never on a "pick one" void. An empty list hides the
      list pane.

### Tokens and colour ([foundations.md](./foundations.md))

- [ ] Only tokens: no hex colours in module CSS (except the documented
      exceptions), and no fallbacks on tokens that don't exist.
- [ ] Semantic tones for meaning (`--danger`, `--warning`, `--success`,
      `--info`, plus `-soft`). Status tokens only for dots and bars.
- [ ] Accent used correctly: `--accent-text` for text, `--accent-solid` +
      `--accent-contrast` for fills, `--accent-soft` for selection. No literal
      accent.
- [ ] No `--text-dim` on `--surface-3`. No text on translucent material
      without a ≥ 78% tint.
- [ ] Vibrancy only on chrome, never behind data.
- [ ] Colour is never the only signal.

### Type, spacing, shape

- [ ] Font sizes come from `--fs-*`. Nothing a user must read is under 11 px.
      Weights are 400, 500 or 600.
- [ ] Spacing is on the 4 px grid. Radius uses `--radius-s/m/l` or pill.
      `--shadow` only on floating layers.
- [ ] Icons: `Icon` with a typed name, sizes 12/14/16 (24–26 only in empty
      states), no glyph characters as icons, `ProviderIcon` for providers.
- [ ] Any animation has a `prefers-reduced-motion` override. Continuous
      animation only for live state.

### Components ([components.md](./components.md))

- [ ] Buttons use `.btn` variants and `.icon-btn`. There is no scoped `.btn`
      redefinition and no module button system.
- [ ] Dialogs use `Modal`. Confirms and prompts use `confirmer`
      (non-destructive: `danger: false` plus a real `confirmLabel`).
- [ ] Menus use `ctxMenu`. Any custom floating UI clamps into the viewport,
      caps its height against `window.innerHeight`, and never relies on
      `100vh`.
- [ ] Toasts only for action results, with a human title and no raw exception
      as the title. Failed loads and invalid fields are shown inline.
- [ ] Every data region has **loading, empty, error and loaded** states.
      `EmptyState` with one CTA. Specific loading text. Inline error with
      Retry.
- [ ] Nothing was re-implemented that exists in `lib/components` (pickers,
      editors, diff, terminal, charts, virtual list).

### Agents and outward actions ([patterns.md](./patterns.md))

- [ ] Agent output is attributed (who, model, when), visually marked, and
      stays a draft until a person applies it.
- [ ] Variants are equal cards with Apply, Compare, and a reject-with-reason
      control.
- [ ] Anything outward-facing (PR, Jira/Confluence, Slack/Telegram, email,
      webhook, publish, prod) confirms with **where**, **what** and **who**, and
      the button names the action.
- [ ] Approvals show who asked, what, why and when; Approve and Deny are
      recorded. Nothing is auto-approved silently.
- [ ] Live work shows honest progress, can be stopped, and shows staleness.

### Copy ([content.md](./content.md))

- [ ] Sentence case. Verbs on buttons (New/Create/Add/Save/Apply/Delete/Remove
      used for their meanings). No "OK", "Yes" or "Submit". `…` for actions
      that ask for more input.
- [ ] Errors say what happened, why and what to do.
- [ ] No route ids, enum values or ULIDs as primary text. Cadences and dates
      are human.
- [ ] Numbers and dates use `rel()`, `formatBytes`, `formatCount` and
      `formatSeconds`. No new local formatters.

### Accessibility ([accessibility.md](./accessibility.md))

- [ ] Every control is a real control. No `onclick` on a `div`. No new
      `svelte-ignore a11y_*`.
- [ ] Focus is visible everywhere. No `outline: none` without a
      `:focus-visible` replacement.
- [ ] Keyboard: the whole flow works without a mouse. Tablists and menus
      support arrow keys. Esc closes the top-most layer. Focus returns to the
      trigger.
- [ ] Labels on every input. `aria-pressed`, `aria-selected` and
      `aria-expanded` where they apply.
- [ ] Logical properties (`inset-inline-*`, `margin-inline-*`…). Directional
      icons flip in RTL. Code and paths stay LTR.

### Responsive

- [ ] Works at phone (≤ 640 px), tablet (641–1024 px) and desktop. Only the
      640/1024 breakpoints are used; split panes use container queries.
- [ ] No horizontal page scroll. Wide content scrolls inside its own container.
- [ ] On phone, list/detail uses push navigation with a back button in
      `PageHeader`'s `leading` slot. Hit areas are at least 36 px.

### Keyboard and command surface

- [ ] The main verbs are registered in ⌘K (`registry.register(owner, …)` in an
      `$effect`, with `group` and `keywords`).
- [ ] New global shortcuts are added to `lib/keys.ts` **and** `KEYMAP`, and
      don't clash with system, WebKit, terminal or session chords.

## 3. Tests and evidence

**E2E helpers** (`ui/e2e/helpers.ts`):

| Helper | Use it for |
|---|---|
| `expectFullyInViewport(page, locator, what?)` | Every new menu, popover, dropdown or typeahead. **Seed enough items to overflow the window** (see `desktop-git-add-menu.spec.ts`). |
| `expectNoHorizontalOverflow(page)` | Every new page, at phone and tablet sizes |
| `expectContentHasHeight(page, min?)` | The page actually renders content, not a collapsed pane |
| `expectAccessible(page)` | axe, which fails on `critical`. Also assert no `serious` `color-contrast` for new surfaces (see [accessibility.md §8](./accessibility.md#8-testing)). |
| `openPage(page, id)` / `PAGES` | Navigate. Add a new module to `PAGES` so `pages.spec.ts` (mobile/tablet sweep), `theme.spec.ts` (light) and `rtl.spec.ts` cover it. |

**Specs.**

- Desktop-only specs must be named `desktop-*.spec.ts` (the `desktop-browser`
  project only runs those).
- New daemon routes need a fresh binary: `OTTO_E2E_BIN=target/debug/ottod`.
- The e2e harness always starts an isolated throwaway daemon; it never touches
  real sessions.
- The page-chrome spec on `feat/page-chrome` (`desktop-page-chrome.spec.ts`)
  asserts one header per route, a fixed header height, overflow into ⋯, and
  auto-select. Add a new route to its `ROUTES` list.

**Light and dark.**

- Force the scheme with an init script before navigating, as `theme.spec.ts`
  does:

  ```ts
  await page.addInitScript(() => localStorage.setItem('otto_scheme', 'light')); // or 'dark'
  // theme: localStorage 'otto_theme' = 'native' | 'pro-dark' | 'warm'
  ```

  Remember that init scripts run again on every reload, so make any reset
  one-shot.
- **Attach screenshots to the PR:** the new or changed surface at 1440×900 in
  **light and dark**, plus one phone screenshot. For anything tinted or
  accent-filled, add one in **Warm dark**. Reviewers compare them against the
  Design Hall visual target: quiet chrome, content first, one primary.

**Before you request review:**

1. `cd ui && npm run check` passes.
2. The relevant e2e specs pass locally (a slot-isolated run is fine).
3. Keyboard-only walk-through of the main flow.
4. Screenshots attached (light, dark, phone).
5. Any rule you deliberately broke is called out in the PR description with a
   reason. If the rule itself is wrong, update these docs in the same PR.
