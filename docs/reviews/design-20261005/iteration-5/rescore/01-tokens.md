## Lens: Foundations — iteration-5 re-score

**Score: 9.77/10.** Start 10.00, minus 2 Partial minors (0.2) and 1 Open nit (0.03) = 9.77.

I checked this at `/Users/itziklavon/claude_ade-design5` by read and grep only, with no builds or tests. I could not confirm the HEAD commit from here. The re-scoped scale and guard counts below come from files in that worktree.

Judgement: none. The arithmetic stands.

| # | Iteration-4 status | Iteration-5 status | Evidence |
|---|---|---|---|
| 1 | [major] Letter-spacing, Partial | **Fixed** | Off-scale `letter-spacing` is now only `SharePage.svelte:494` (0.18em, marked `ui-guards: allow` as one-time-code digits) and `site.css`, which is the exported-site stylesheet and out of scope. `ui-guards.mjs:249,379` adds a `letter-spacing-literal` rule that allows only `.06em`, `-0.01em` and 0/normal. |
| 3 | [major] Off-grid spacing, `--sp-*`, ratchet, Partial | **Fixed** | `tokens.css:36-44` defines `--sp-1` to `--sp-9`. `foundations.md:288-291` redefines the scale as 2 px steps to 24 px and 4 px steps above. `ui-guards.mjs:248,376` adds an `off-grid-spacing` rule with a baseline of only 4 hits (`GridView` ×2, `JsonView`, `VerticalView`). Odd-px spacing outside `site.css` is down to zero. My earlier count of 548 included now-legal even values (10, 14). |
| 6 | [minor] Off-scale font sizes, Partial | **Partial** | Brokers, Schema, Database and `TermKeysBar` are mostly cleaned up. Still present: `Chart.svelte:260,274` (34 px, 28 px), `TermKeysBar.svelte:144,172,176` (14, 18, 16 px), `AppliedPreview.svelte:166,221` (17, 14 px), and em sizes in `LearningsView.svelte:919`, `DiscoveryTab.svelte:495`, `DiscoveryChat.svelte:409`, `RefineChat.svelte:301` and `McpServers.svelte:499`. The 16 px mobile-input hits are intentional, and there is no guard for non-token px sizes at 11 px or above. |
| 7 | [minor] Radius literals, Partial | **Fixed** | The `radius-literal` baseline is `{}`. The only literals left are 4 px/6 px inside srcdoc strings (`FileTree.svelte:241-242`, `WorkflowsPage.svelte:294-295`), the `DeviceFrame` device bezel, and `site.css`, none of which are app chrome. |
| 8 | [minor] Literal colours, Partial | **Fixed** | The `color-literal` baseline is `{}`. `--on-scrim` is defined at `tokens.css:338`. |
| 9 | [minor] Dead fallbacks and physical shorthand, Partial | **Fixed** | `token-fallback` and `physical-shorthand` baselines are both `{}`. |
| 12 | [minor] Shadows and legacy `--shadow`, Partial | **Partial** | The only literal shadow left is `DeviceFrame.svelte:85`, a device preview. `var(--shadow)` is still used 53 times in 36 files (was 55). `--shadow` is now a pure alias of `--glass-shadow` (`tokens.css:213`, `227`, `243`, `259`, `277`, `400`), so there is no visual drift, but it has not been retired. |
| 13 | [minor] `--fs-2xl` content headings, Partial | **Fixed** | `foundations.md:241` now documents `--fs-2xl` as allowed for the top heading of long-form content (walkthrough step, reader view). `Walkthroughs.svelte:520` and `ReaderView.svelte:348` match that. |
| 15 | [nit] Status tokens duplicate tone tokens, Partial | **Fixed** | `--status-idle` is `var(--text-dim)` and `--status-warn` is `var(--warning)` in dark and light (`tokens.css:83,87,184-186,383-385`). Working and exited keep their own indicator-only values. |
| 16 | [nit] Negative letter-spacing undocumented, Open | **Fixed** | `foundations.md:266` documents `-0.01em` on titles at `--fs-l` and above, and the guard allows exactly that. |
| 17 | [nit] Vault graph fallbacks, Open | **Fixed** | `GraphView.svelte:215` is now `v('--accent')` with no hex fallback, and `:1029` uses `theme.dim`. |
| 18 | [nit] NUL byte in `DiagramView.svelte`, Open | **Fixed** | A NUL search returns 0, grep no longer treats the file as binary, and the 8.5 px `.badge` rule is gone. The `font-size-small` baseline is `{}`. |
| 19 | [nit] `--sp-*` and `--agent` tokens, Open | **Partial** | `--sp-*` is done (see #3). `--agent` is still documented as "Proposed" (`foundations.md:151`) and not defined. This is the 1 nit I still count Open, because the rule says Partial counts at its original severity. |

Spot-check of items I marked Fixed last time (none regressed):
- Focus: 0 `:focus*` rules with raw `border-color: var(--accent);`.
- Weights: no 650 or 7xx weights outside `site.css`.
- `--control-border`: defined at `tokens.css:123,214,260,395`.
- Warm light accent-text: `tokens.css:264` (50% mix).
- Motion: 194 `--dur-*` uses, and the `transition-literal` baseline is `{}`.
- Keyframes: 1 `@keyframes` left in modules (`git/GraphView.svelte`), allowed with an empty baseline.

### New findings

None found. The tightened ratchets introduced no regressions I could see, and the baselines are empty or near-empty.

### Remaining to reach 9.8

- Move the last off-scale font sizes onto `--fs-*`: `Chart.svelte:260,274`, `TermKeysBar.svelte:144,172,176`, `AppliedPreview.svelte:166,221`, and the 0.88–0.92em sizes in the product and settings files. Add a guard for non-token px/em sizes at 11 px or above, with an allowlist for the 16 px mobile inputs.
- Retire the 53 `var(--shadow)` uses (36 files) by switching them to `--glass-shadow` or `--shadow-card`, then delete `--shadow`.
- Define `--agent` (the agent-identity colour), or drop the "Proposed" item from `foundations.md:151`.
