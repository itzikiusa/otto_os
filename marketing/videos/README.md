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

`Intro` · `Sessions` · `MissionControl` · `Git` · `Review` · `ProofPacks` ·
`Product` · `Canvas` · `Swarm` · `GoalLoops` · `Connections` · `Database` ·
`Brokers` · `Channels` · `Workflows` · `ScheduledTasks` · `Mcp` · `Vault` ·
`Skills` · `SkillsEval` · `UsageInsights` · `Api` · `Plugins` · `TeamMobile` ·
`Platform` · `Outro`

Together they cover the full Otto surface — one walkthrough per gatable feature
plus the brand intro/outro: agent sessions, Mission Control's unified work graph,
git/PRs, multi-agent review + tracked findings, Proof Packs, Jira/Confluence
product workflows, the Canvas, the agent swarm, Goal Loops, connections, the
database explorer, Kafka brokers, Slack/Telegram channels, workflows, scheduled
tasks, the MCP control plane, the knowledge vault, the skill library +
self-improvement, the skills evaluator, usage/budgets/insights, the API client,
custom plugins, RBAC + remote + mobile, and the platform polish (palette,
theming, RTL, auto-update). The in-app **Walkthroughs** page
(`ui/src/modules/help/Walkthroughs.svelte`) lists the same set and **streams
them from GitHub** — the MP4s are not bundled into the app (see "Publishing").

## Commands

```bash
npm install
npm run studio                 # open Remotion Studio (preview/scrub)
npx remotion still src/index.ts Intro out/intro.png --frame=120   # one still
node render-all.mjs            # render every composition → out/*.mp4
node render-all.mjs Intro Git  # render a subset
```

`out/` is the render output only (gitignored; ~5 MB per 1080p master). Nothing
copies it into `ui/` — the app never ships the videos.

## Publishing (how the app gets the videos)

The Walkthroughs page loads `<base>/<Id>.mp4` where `<base>` defaults to the
rolling GitHub release **`walkthroughs`** on `itzikiusa/otto_os`
(`https://github.com/itzikiusa/otto_os/releases/download/walkthroughs/<Id>.mp4`);
`VITE_WALKTHROUGHS_BASE` overrides it at UI build time. Keeping them out of the
bundle saves ~135 MB in *each* of `ottod` (`--features embed-ui`) and `Otto.app`.

After re-rendering, publish with:

```bash
packaging/publish-walkthroughs.sh            # encode out/*.mp4 → 720p, upload --clobber
packaging/publish-walkthroughs.sh some/dir   # or a different source dir
ENCODE_ONLY=1 OUT_DIR=/tmp/wt packaging/publish-walkthroughs.sh   # just encode
```

The script re-encodes to 1280x720 H.264 (`-crf 30 -preset slow -pix_fmt yuv420p
-movflags +faststart`, audio copied) — roughly a 3–4× reduction from the 1080p
masters — creates the release if missing (`--prerelease`, so it never shows as
"Latest"), and replaces the assets in place so the URLs stay stable. Asset names
must match `file:` in `Walkthroughs.svelte`; add a row there when you add a
composition.
