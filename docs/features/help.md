# Help — section guides and the tour film

The in-app **Help** page (sidebar footer → **Help**, the native **Help → Otto
Help** menu item, or ⌘K → "Open Help") replaces the old per-feature walkthrough
videos with:

- **one README per part of Otto** — every sidebar module (Work, Automate,
  Build, Infrastructure, Insight, Plugins) plus shell-wide **Basics** (getting
  started, the command bar, keyboard shortcuts, the desktop app, phone and
  remote, settings, snipping). Each has what it's for, getting started, every
  capability, the real keyboard shortcuts, limits and related guides;
- **one short tour film** on top of the default view, with chapters that link
  to the matching guides.

The route stays `#/walkthroughs` so old links, the menu item and saved
routes keep working.

## Using it

- `#/walkthroughs` opens the film and **Getting started** (never an empty
  pane). `#/walkthroughs/<id>` deep-links to a guide (`#/walkthroughs/git`).
  An unknown id shows an inline "no guide called …" with a way back.
- The left rail is grouped like the sidebar, Basics first. The search box
  matches titles, summaries, the full text and **shortcut keys** — type `⌘K`,
  `cmd shift d` or `ctrl 1`. Results show why they matched (a key chip or a
  snippet). `Enter` opens the top hit, `Esc` clears the search, `↑`/`↓`
  (and `Home`/`End` in the list) move through the rail.
- **Open <module>** (the header's primary action) goes to the module. It's
  disabled with an explanation when your role can't view that module.
- **Watch this part** on a guide shows the film above it and plays the chapter
  mapped to that guide. **Read the guide** on a chapter opens its guide.
  **Watch the tour** in the header brings the film back on any guide.
- ⌘K has one **"Guide: <title>"** command per README, from anywhere in the app.
- Phone: push navigation — the film and the list are the page; opening a guide
  replaces it, with a back button in the header.

## How it's built

| Piece | File |
|---|---|
| Page | `ui/src/modules/help/Walkthroughs.svelte` |
| Film player | `ui/src/modules/help/TourFilm.svelte` |
| Loader (Vite glob, eager, `?raw`) | `ui/src/modules/help/sections.ts` |
| Frontmatter parser, search ranking, film manifest (pure, unit-tested) | `ui/src/modules/help/guide.ts`, `ui/unit/helpGuide.test.ts` |
| Markdown → HTML (marked + the allowlist sanitizer, then `<kbd>` chips, table wrappers) | `ui/src/modules/help/render.ts` |
| Guides | `ui/src/modules/help/sections/<id>.md` |
| Film manifest | `ui/src/lib/walkthroughs/film.json` |
| ⌘K "Guide: …" commands | `ui/src/shell/App.svelte` (`registry.register('guides', …)`) |
| E2E | `ui/e2e/desktop-help.spec.ts` |

### Adding or editing a guide

Drop a file in `ui/src/modules/help/sections/`; the page lists whatever exists
(no code change). The id is the sidebar module id (`SIDEBAR_MODULES` in
`ui/src/lib/sidebar.ts`) or a Basics id.

```markdown
---
id: agents
title: Agents
group: Work            # Basics | Work | Automate | Build | Infrastructure | Insight | Plugins
route: agents          # router path "Open …" goes to (may be a sub-path: settings/channels); omit for Basics
summary: One sentence, what it's for.
---
## What it's for
## Getting started
## Everything it can do
## Keyboard shortcuts
## Tips and limits
## Related
```

- Frontmatter is flat `key: value` lines; a trailing `# comment` and
  surrounding quotes are dropped. A file without a `title` is skipped; a
  missing/unknown `group` falls back to the module's sidebar section, then
  Basics.
- Shortcut tables are `| Keys | Action |` with each key combo in a code span
  (`` `⌘K` ``, alternatives `` `⌘]` / `⌘[` ``, ranges `` `⌃1` – `⌃4` ``). They
  render as key chips and feed the shortcut search. Only list shortcuts that
  exist in the code; write "None specific to this page." otherwise.
- Link other guides as `[Git](#/walkthroughs/git)`.
- Ground every statement in the current code; `docs/features/` is the deeper
  reference but the code wins where they differ.

### The tour film

`film.json`:

```json
{ "file": "otto-tour.mp4", "poster": "otto-tour-poster.jpg", "captions": "otto-tour.vtt",
  "duration": 150, "chapters": [ { "id": "agents", "section": "agents", "title": "Agents", "start": 12, "duration": 20 } ] }
```

- The MP4 and poster are **not bundled**. They are streamed from
  `VITE_WALKTHROUGHS_V2_BASE` ?? `VITE_WALKTHROUGHS_BASE` ?? the rolling
  `walkthroughs` GitHub release. For GitHub the URL is resolved through
  `GET /walkthroughs/resolve` first (WebKit refuses a `<video src>` that
  redirects), falling back to the raw URL.
- **Captions:** GitHub's asset host sends no CORS headers, so a remote
  `<track>` can't load from it. Put the VTT next to the manifest
  (`ui/src/lib/walkthroughs/otto-tour.vtt`, a few KB) and the player serves it
  from a same-origin `blob:` URL, on by default with a CC toggle. With a
  non-GitHub base, the VTT is loaded from the host in CORS mode instead.
- A chapter's `section` should name a guide id; then the chapter shows **Read
  the guide** and that guide shows **Watch this part**.
- States: no/invalid manifest → a one-line "not part of this build" note;
  loading → poster + "Loading the tour…"; unreachable (offline, 404, blocked)
  → an inline "can't load right now" with **Retry** (re-resolves the signed
  URL) and **Open in browser**; never a raw error or a toast.

## Limits

- Guides are bundled at build time; changing one needs a UI rebuild.
- The film needs a network connection (the guides don't).
- The desktop app's CSP (`media-src`) allows GitHub's release hosts and
  `blob:`; a custom film base on another host must be added there too.
- `ui/src/lib/walkthroughs/catalog.json` (the old per-feature catalog) is no
  longer read by the app; it stays because `marketing/videos/studio-v2`
  still imports it.
