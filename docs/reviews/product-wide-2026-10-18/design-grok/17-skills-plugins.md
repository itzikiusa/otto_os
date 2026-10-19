# Skills Lab, Skills Eval, Plugins

Reviewed the current frames (not a diff): `ui/src/modules/skills-lab/`, `ui/src/modules/skills-eval/`, `ui/src/modules/plugins/PluginFrame.svelte`, against `docs/design/guidelines/`. Light/dark tokens, phone (≤640), tablet (641–1024), RTL.

States, chrome, and copy are mostly already right: one `PageHeader`, `LoadState` / `EmptyState`, confirms on stop and delete, logical properties, reduced-motion on the live dots, skills push-navigation on the phone. Defects below are the ones that still mislead or block.

## Scores

| Module | Score | Defects |
|---|---|---|
| Skills Lab | 9.4 | 3 |
| Skills Eval | 9.0 | 5 |
| Plugins | 9.6 | 1 |
| **Total** | | **9** |

## Skills Lab — 9.4

**1. Phone review is a 40% list with no back button.** `SkillReviewPanel.svelte:696` stacks the reviews list at `max-height: 40%` above the report. `SkillsLabPage.svelte:91` only puts the list toggle in the header for Skills and Evaluator, and only when `!viewport.isPhone`. Review’s hide control is a second 32px toolbar (`SkillReviewPanel.svelte:360`) that the Skills list does not have. On a phone the findings sit under that list plus the section tabs, and there is no header Back the way Skills has (`SkillsLabPage.svelte:95`, `SkillsBrowser.svelte:734`).

Fix: same push as Skills. On phone, hide the list once a review or the new-review form is open, put Back in `PageHeader` `leading`, and delete `.review-toolbar`.

**2. The filters and copy switcher are 22px targets.** `SkillsBrowser.svelte:570` sets `.fchip { height: 22px }` with no phone override (the skill rows do grow to 44px at `:741`). `SkillDetail.svelte:511` sets `.variant { height: 22px }`. On a phone those chips are how you filter the list and how you switch Library / Claude / Codex. A miss-tap changes the list or the copy you are about to edit.

Fix: under `@media (max-width: 640px)`, give `.fchip` and `.variant` `min-height: var(--hit-min)` and horizontal padding, without growing the 12px icons.

**3. The Evaluator separator points the wrong way in RTL.** `SkillsLabPage.svelte:114` uses `chevronRight`, which `Icon` already mirrors (`Icon.svelte:213`). `:177` also does `scaleX(-1)` on `.lab-tabs-sep`, so the two flips cancel. In RTL the mark between Evaluator and Runs/Golden/Matrix still points right.

Fix: delete the `.lab-tabs-sep` RTL rule.

## Skills Eval — 9.0

**1. Runs and Matrix never become a phone page.** `SkillsEvalPage.svelte:523` and `MatrixView.svelte:617` keep the list at `max-height: 40%` above the report, the compare table, or the new-matrix form. The header collapse that would hide that list is desktop-only (`SkillsLabPage.svelte:91`). After two tab rows, the run or the grid is the bottom slice of the phone. Compare is worse: you pick two runs inside that strip (`SkillsEvalPage.svelte:297`).

Fix: push navigation. Opening a run, a matrix, or compare hides the list; Back goes in `PageHeader` `leading` (Runs and Matrix, including compare). Do not stack at 40%.

**2. “Promote” is the primary action for people who cannot promote.** `promote_skill` is root-only (`crates/otto-server/src/skill_eval.rs:2718`). `RunDetail.svelte:471` still shows **Promote winning skill** as `.primary`, and `:664` / `:675` offer **Promote** on each iteration. The gate override at `:737` (“Force promote anyway (root override…)”) is not behind `auth.isRoot`. A non-root person fills the sheet and gets a failure toast. Copy and Download do work for them.

Fix: render Promote, the sheet, and the force checkbox only when `auth.isRoot`. For everyone else, leave Copy and Download and one line: saving to the library is an admin action.

**3. Matrix “Iterations” does nothing, next to copy that says so.** The form always posts `mode: 'score_only'` (`MatrixView.svelte:211`) and the lede says there are no improvement iterations (`:338`). `run_skill_eval_core` returns before the iteration loop when mode is `score_only` (`skill_eval.rs:1312`). The Iterations field (`MatrixView.svelte:368`) is still sent (`:217`). Changing it does not add runs or improve the skill.

Fix: remove the Iterations field while the matrix is score-only. If a later mode uses it, show it only then and say what a round does.

**4. Editing a golden task throws away tags, and New throws away the form.** Regression cases are stored with `tags: ["regression"]` (`skill_eval.rs:3043`). `openEdit` (`GoldenTasksView.svelte:110`) never copies `tags` or `build_cmd`, and `submit` (`:136`) does not send them. `GoldenTaskReq` defaults tags to `[]` and build command to empty, and update replaces both columns (`eval_lab.rs:155`). Save changes deletes the tag chips on the card. Separately, `toggleCreate` (`:100`) while an edit is open calls `resetFields()` with no confirm, so an in-progress edit is discarded, and the header **New golden task** stays `.primary` (`:209`) beside the form’s **Save changes** (`:282`).

Fix: round-trip `tags`, `build_cmd`, and `enabled` on edit. If the form is dirty, confirm before New or Cancel resets it. While the form is open, the header button is not `.primary`.

**5. Compare hides a failed load, and the rating stars are 24px.** A compare fetch that fails is ignored (`SkillsEvalPage.svelte:130`). `compareRuns` then drops that run, so two selected runs can still show “Select two or more runs…” (`:355`). The stars that write the rating are `width/height: 24px` (`RunDetail.svelte:783`) with no phone size. They sit in a row and save on click (`:637`).

Fix: keep a per-id error on the compare pane with Retry, and don’t use the empty “select more” state when the selection is already ≥ 2. Give `.star` `min-width/min-height: var(--hit-min)` on phone.

## Plugins — 9.6

**1. Warm and Pro Dark never reach the plugin.** `themeVars` (`PluginFrame.svelte:95`) and both posts (`:109`, `:135`) send `scheme` and a subset of tokens. They do not send `themeName`. The bundled Team Performance page applies Warm and Pro Dark only from `m.themeName` (`examples/plugins/team-performance/ui/views/app.js:73`), which selects `--cat-*` and the theme fallbacks the host does not inline (`app.css:107`, `:137`). Inline `--bg` / `--text` match; charts and any token outside the pick list stay on Native while the shell is Warm or Pro Dark. The frame also never sends `dir` or reduced motion, so a plugin cannot follow RTL or `prefers-reduced-motion`.

Fix: on `otto:init` and `otto:theme`, send `themeName: ui.theme`, `dir: document.documentElement.dir`, and `reducedMotion` from `matchMedia('(prefers-reduced-motion: reduce)')`. Keep the token pick, and add the soft tones and type scale the plugin already paints with (`--danger-soft`, `--warning-soft`, `--success-soft`, `--info-soft`, `--surface-3`, `--hover`, `--fs-xs` through `--fs-l`).

## Not defects

Empty libraries, failed loads, and missing plugins already use `EmptyState` / `LoadState` with Retry. Stop and delete confirm, and say what is kept. Skills list/detail pushes on the phone and restores the list with Back. Search focus is on the field wrapper, not `outline: none` alone. Wide tables scroll inside `.table-wrap` / `.cmp`, not the page. The plugin iframe stays on opaque `--bg`.
