# Otto walkthrough — builder brief (READ FIRST)

You are building **one** Remotion composition (one feature walkthrough video) for
the Otto desktop app. A shared design kit already exists and is validated. Your
job: compose it into a polished, on-brand, accurate scene sequence. Match the
quality of `src/compositions/Intro.tsx` — **read that file first** as the
reference for tone, structure, and animation.

## Hard rules (non-negotiable)

1. **One file only:** `src/compositions/<Name>.tsx`. Do NOT edit any shared file,
   `Root.tsx`, `theme.ts`, or another composition. Do NOT run renders or `npm`.
2. **Exports (exact names):**
   ```ts
   export const <name>Duration: number = scenesDuration(SCENES);
   export const <Name>: React.FC = () => <Scenes scenes={SCENES} />;
   ```
   The composition is a `SceneDef[]` fed to `<Scenes>`. Because the registered
   duration equals `scenesDuration(SCENES)`, **there is never a blank tail** — so
   every scene's content MUST stay visually alive until the end of its `dur`
   (nothing disappears before the scene ends; no element animates out and leaves
   emptiness). The **last scene is always a `WalkOutro`** (closing brand card).
3. **Use the real Otto design.** App mockups use the default native-dark theme
   `T` (imported from `../theme`). Use the kit's `OttoWindow` / `Navigator` /
   `Rail` / `PhoneFrame` / `TabletFrame` for chrome, and real Otto colors
   (`T.*`, `status`, `providers`, `brand`). NEVER invent an off-brand palette.
4. **Every app scene needs a `<Caption step={n} title sub />`** lower-third that
   explains what's happening (concise, specific, real feature language).
5. **Accuracy:** depict the actual feature behavior from the FEATURE FACTS in
   your task. Use realistic, specific text (real-looking repo names, queries,
   topics, story keys) — never lorem ipsum.
6. **Mobile:** if your spec calls for a mobile beat, use `PhoneFrame` and/or
   `TabletFrame` to show Otto's responsive shell. Otherwise desktop is fine.
7. **TypeScript must be clean** — only import names that exist (see API below).
   No unused imports. Strict mode is on.

## Scene/duration conventions (30 fps)

- Title scene: ~70–90f. App-demo scenes: ~150–260f each. Outro: ~120–150f.
- Aim for the TARGET DURATION in your spec (sum of scene `dur`s, in frames).
- Give each scene a `name`. Keep 4–6 scenes total.
- Start with a `TitleCard` scene (kicker + title + subtitle), then demo scenes,
  then a `WalkOutro`.
- Animate content in with `Appear` / `Stagger` (spring) and `track(frame,...)`.
  Within a Sequence, `useCurrentFrame()` is local (starts at 0). Stagger reveals
  so the scene fills its whole `dur` with motion, then holds.

## Kit API (import from these modules)

### `../theme`
- `T: Theme` (native dark, default for app UI). `themes` (nativeDark, nativeLight,
  proDark, warmLight, warmDark). `Theme` interface fields:
  `bg, bgSidebar, surface, surface2, border, text, textDim, termBg, shadow, accent, accentContrast, scheme, name`.
- `fonts.ui`, `fonts.mono`; `radius.{s,m,l,xl}`.
- `status.{working,idle,exited,needsYou,reconnectable}` (hex), `traffic`.
- `brand.{purple,purpleDeep,violet,cyan,ink,ink2,mist,grad,gradSoft,glow}`.
- `providers.{claude,codex,agy,gemini,shell}` (chip colors).
- `series` (chart palette array). `navActive.{bg,fg,edge}`.
- `alpha(hex, a)` → rgba string. `VIDEO`, `cinematicBg`.

### `../components/scene`
- `Scenes`, `SceneDef` (`{ dur:number; node:ReactNode; name?:string }`),
  `scenesDuration(scenes)`, `Stage` (centers a device on the cinematic bg;
  props `scale?, enter?:'up'|'fade'|'none', float?, y?`), `FloorGlow`,
  `WalkOutro` (props `title, tagline?, pills?:{label,color?,icon?}[], accent?`).

### `../components/Frame`
- `OttoWindow` (props: `nav?, right?, tabs?:{label,icon?,active?,dot?}[], title?, t?, width?, height?, children, contentStyle?`).
- `TabBar`, `RightPanel` (`t?, width?, title?, icon?, children`).
- `PhoneFrame` (`t?, title?, active?, time?, height?, children, workingBadge?, showPanelBtn?`).
- `TabletFrame` (`t?, nav?, title?, height?, children`).

### `../components/Nav`
- `Navigator` (`active?:string, t?, width?, sessions?:NavSession[], activeSessionTitle?, workingCount?, counts?:Record<string,number>, user?`).
  Module ids for `active`: `agents, connections, swarm, git, product, vault, api,
  database, brokers, workflows, skills-eval, insights, usage, walkthroughs, settings`.
- `NavSession` = `{ title, provider, status?, tasks?:[done,total], needsYou? }`.
- `Rail` (`active?, t?, workingCount?`).

### `../components/kit`
- Animation: `Appear` (`delay?,y?,x?,scale?,style?`), `Stagger` (`delay?,step?,y?`),
  `useSpring(delay,opts?)`, `track(frame,[a,b],[c,d],ease?)`, `useTyped(text,start,cps)`,
  `Caret`.
- Cinematic: `Background`, `Kicker`, `BrandWord`, `Caption`
  (`step?,title,sub?,delay?,align?`), `TitleCard` (`kicker?,title,subtitle?,icon?`),
  `FeaturePill` (`label,color?,delay?,icon?`).
- Atoms: `Chip` (`tone?:'default'|'ok'|'bad'|'warn'|'accent', color?, t?`),
  `Button` (`variant?:'primary'|'default'|'ghost'|'danger', size?, icon?, t?`),
  `Card` (`pad?, t?`), `Field` (`label?,value?,placeholder?,focused?,mono?,caret?,icon?,t?`),
  `Toggle` (`on?,t?`), `Segmented` (`options,active?,t?`), `KeyCap`, `Keys` (`keys:string[]`),
  `Avatar` (`name,color?,size?,t?`), `Cursor` (`from,to,startAt,duration?,click?`),
  `Toast` (`text,tone?:'ok'|'bad'|'info',delay?,t?`), `StatusDot` (`kind?,size?,pulse?`).
- Data viz: `MetricStat` (`label,value,delta?,deltaTone?,accent?,t?`),
  `BarChart` (`data:number[],labels?,color?,height?,grow?,width?,t?`),
  `Sparkline` (`data,color?,width?,height?,progress?,t?`),
  `Ring` (`value,size?,color?,label?,t?`).
- Code/diff/table:
  `Terminal` (`lines:TermLine[],fontSize?,delay?,step?,pad?,t?,style?`) where
  `TermLine={text,tone?:'cmd'|'ok'|'dim'|'warn'|'err'|'text'|'accent'}`;
  `Diff` (`lines:DiffLine[],delay?,step?,fontSize?,t?,style?`) where
  `DiffLine={text,kind?:'add'|'del'|'ctx'|'hunk'}`;
  `Table` (`columns:string[],rows:(string|ReactNode)[][],widths?,delay?,step?,fontSize?,t?,style?`).
- Logo: `OttoIcon` (`size?,glowPx?`), `OttoGlyph` (`size?,glow?`), `Icon` (`name,size?,color?,strokeWidth?`).
  Icon names available: terminal, chart, gauge, plug, branch, gear, sidebar, panel,
  plus, x, search, chevronDown/Left/Right, dot, check, user, folder, file, note,
  play, refresh, trash, edit, zap, globe, arrowUp, arrowDown, commit, merge,
  comment, split, eye, key, db, box, clock, command, pr, grid, square, archive,
  maximize, minimize, info, link, external, ticket, slack, bell, send, tag, stash,
  fetch, share, pin.

## Pattern to follow

```tsx
import React from 'react';
import { AbsoluteFill } from 'remotion';
import { T, brand, fonts, providers, status, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import { TitleCard, Caption, Terminal /* … */ } from '../components/kit';

const TitleScene: React.FC = () => (
  <TitleCard kicker="…" title="…" subtitle="…" />
);

const DemoScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="git" />} title="Otto — repo">
        {/* real-looking feature UI built from kit atoms */}
      </OttoWindow>
    </Stage>
    <Caption step={1} title="…" sub="…" />
  </>
);

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />, name: 'Title' },
  { dur: 220, node: <DemoScene />,  name: 'Demo' },
  // …
  { dur: 130, node: <WalkOutro title="…" tagline="…" pills={[…]} />, name: 'Outro' },
];

export const gitDuration = scenesDuration(SCENES);
export const Git: React.FC = () => <Scenes scenes={SCENES} />;
```

Build something you'd be proud to ship on a launch page.
