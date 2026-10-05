## Lens: Foundations — iteration-6 re-score

**Score: 10.00/10.** Start 10.00 with nothing left Open or Partial, so there is nothing to subtract. This is from read and grep only at `/Users/itziklavon/claude_ade-design6`; I did not confirm HEAD 75f1565b and ran no builds or tests.

Judgement: none. The 4 baselined off-grid spacing hits (see Remaining) are the only thing I would still watch.

| # | Iteration-5 status | Iteration-6 status | Evidence |
|---|---|---|---|
| 6 | [minor] Off-scale font sizes, Partial | **Fixed** | The only px or em `font-size` left in `.svelte` files is six 16 px iOS no-zoom inputs. Each carries a `ui-guards: allow` comment with the reason (`ClusterForm.svelte:370`, `PrDetail.svelte:976`, `WipPanel.svelte:1423,1427`, `ReviewPanel.svelte:2279`, `RemoteLiveView.svelte:1062`). `Chart`, `TermKeysBar`, `AppliedPreview` and the 0.88–0.92em product/settings files are gone. A new `font-size-literal` rule (`ui-guards.mjs:98,279,390`) covers px, em and rem at 11 px or above. Its baseline is `{}`. |
| 12 | [minor] Legacy `--shadow` still used, Partial | **Fixed** | `var(--shadow)` appears 0 times in `ui/src`. `--shadow:` is no longer defined in `tokens.css`. |
| 19 | [nit] `--agent` token, Partial | **Fixed** | `foundations.md:153` now says there is no `--agent` token, because agent attribution uses something other than a colour. The "Proposed" item is resolved by decision. |

Spot-check of earlier Fixed items (no regressions found):
- `letter-spacing`: 3 off-scale hits remain, all in `site.css`, the exported-site stylesheet that is out of scope. The `letter-spacing-literal` baseline is `{}`.
- Colour and radius: `color-literal` and `radius-literal` baselines are `{}`.
- Motion: `transition-literal` baseline is `{}`.
- Focus and weights: 0 `:focus` rules with raw `border-color: var(--accent);`, and 0 weights of 650 or 700+ in `.svelte` files.
- Spacing: `off-grid-spacing` is still only 4 baselined hits (`GridView` ×2, `JsonView`, `VerticalView`).

### New findings

None. The new guards and the removal of `--shadow` introduced no regressions I could find. The `--shadow` check relies on the `var(--shadow)` count of 0, because my attempt to also search bare `--shadow` text failed with a regex error. I did not re-run it without the lookaround.

### Remaining to reach 9.8

The score already exceeds 9.8, so nothing is required. Optional cleanup:
- Pay down the 4 baselined `off-grid-spacing` hits (`ui/src/modules/database/GridView.svelte`, `JsonView.svelte`, `VerticalView.svelte`) so the baseline reaches `{}`.
- Pay down the remaining `global-class` and `outline-removed` entries in `ui/scripts/ui-guards-baseline.json`. They belong to the components lens, not foundations.
