# Otto walkthrough videos (Remotion)

Feature-walkthrough / marketing videos for **Otto**, the Agentic Development
Environment. 1920×1080 @ 30fps. Everything is code — no external assets beyond
the vector logo (drawn from the real `favicon.svg`).

## Design fidelity

The in-video app mockups are built to match the **real** Otto UI, not an
approximation:

- Colors, fonts, radii come 1:1 from `ui/src/lib/tokens.css` (see `src/theme.ts`).
  Default app theme = **native dark** (`#1e1e23` bg, `#0a84ff` accent, the
  signature `#7ee787` green active-nav row). `themes` also carries native-light,
  pro-dark (purple) and warm.
- The icon set in `src/components/Icon.tsx` is ported verbatim from
  `ui/src/lib/components/Icon.svelte`.
- The window chrome, navigator (real module order), rail, phone and tablet
  shells live in `src/components/Frame.tsx` + `src/components/Nav.tsx`.
- The **brand identity** (cinematic title cards, captions, backgrounds) uses the
  purple→cyan of the actual mark.

## Structure

- `src/theme.ts` — tokens, themes, brand, helpers.
- `src/components/` — the shared kit:
  - `Icon.tsx`, `OttoLogo.tsx` (vector mark + app-icon tile)
  - `Frame.tsx` (`OttoWindow`, `RightPanel`, `PhoneFrame`, `TabletFrame`)
  - `Nav.tsx` (`Navigator`, `Rail`)
  - `kit.tsx` (animation helpers + cinematic + in-app atoms + data-viz + terminal/diff/table)
  - `scene.tsx` (`Scenes` sequencer, `Stage`, `WalkOutro`)
- `src/compositions/*.tsx` — one file per walkthrough. Each exports `<Name>` and
  `<name>Duration`. The composition is a `SceneDef[]`; the registered
  `durationInFrames` is **exactly** `scenesDuration(SCENES)`, so there is never a
  blank tail.
- `src/Root.tsx` — registers every composition.
- `AGENT_BRIEF.md` — the contract + kit API used to build the compositions.

## Compositions

`Intro` · `Sessions` · `Git` · `Review` · `Product` · `Connections` ·
`Database` · `Brokers` · `Swarm` · `Channels` · `UsageInsights` · `Skills` ·
`Workflows` · `Plugins` · `Vault` · `TeamMobile` · `Platform` · `Outro`

Together they cover the full Otto surface: agent sessions, git/PRs, multi-agent
review, Jira/Confluence product workflows, connections, the database explorer,
Kafka brokers, the agent swarm, Slack/Telegram channels, usage/budgets/insights,
skills & context, workflows, custom plugins, the knowledge vault, RBAC + remote +
mobile, and the platform polish (palette, mission control, theming, auto-update).

## Commands

```bash
npm install
npm run studio                 # open Remotion Studio (preview/scrub)
npx remotion still src/index.ts Intro out/intro.png --frame=120   # one still
node render-all.mjs            # render every composition → out/*.mp4
node render-all.mjs Intro Git  # render a subset
```
