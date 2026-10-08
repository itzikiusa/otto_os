# Full round 5 — independent design and UX review

Reviewer 12, reused for the final round. Review base: `origin/main` at
`7cff9e538deccdd0e346af7c6285e11863285c00`; starting branch head:
`a911ce2bc614ae430bf9a16bcd9ee7614813d9d4`. This pass challenged the previous
rounds' results and found one additional user-visible failure state. It was
reproduced, repaired, and rechecked before reassessment.

## Scope and independent review

Read every production UI change on the branch against `origin/main`, together
with the affected tests and relevant callers: PageHeader, Composer,
ConversationView, LoopsPage/LoopDetail lifecycle, ProofPage/ProofStore,
RunDetail, Scorecard, shell workspace bootstrap, and API type semantics.
Read the round 3 and 4 reports as prior evidence, not as a substitute for
checking current source and interactions. Traced the proof endpoint and rating
publication path to distinguish a real persisted failure state from a fixture
invention. No additional production UI deficiency was confirmed outside the
finding below.

The browser used the current UI source and isolated daemon binary SHA-256
`14335b30342d5ab8603e9716620d545778a20be7c86775e65dfa1463c00f8dd9`, which includes
the first four rounds. Concurrent round 5 backend edits were not in that binary.
This is functional evidence, not a latency measurement; correctness compilation
ran concurrently. Source hashes and exact provenance are in
[provenance.json](../evidence/round-5-design-ux/provenance.json).

## Confirmed finding and completed repair

**Medium — an expanded pending score displayed the previous approval as current
evidence.** `ui/src/modules/skills-eval/Scorecard.svelte:85` refreshed the proof
pack whenever the score changed, including when publication was pending. The
proof endpoint returns persisted artifacts. If a new human rating is saved but
publication fails, those artifacts can still contain the previous approval.
Consequently the UI displayed a saved **1/5** rating and **Pending** score beside
an unqualified **5/5** proof approval. Round 3 covered pending recovery and healthy
expanded-proof refresh separately, but missed their combination.

The new light and dark tests both failed before the repair: the old Human rating
artifact remained when the expected count was zero. The preserved
[red log](../evidence/round-5-design-ux/pending-evidence-red.log) and
[before screenshot](../evidence/round-5-design-ux/before-light-test-failed-1.png)
show the contradiction. This was a product assertion failure, not a harness
timeout.

The repair suspends proof fetching while `proof_status` is `pending` and renders
“Proof evidence will be available when the score update finishes.” in the open
pack (`Scorecard.svelte:158`). Existing effect cleanup rejects any older response.
When Retry publishes the score, the effect fetches the current proof again.
This uses existing typography and layout, with a status announcement; no visual
redesign was needed.

**Closure acceptance, met in both themes:** begin with a real published 5/5 proof;
save a 1/5 rating into a simulated publication-failure response while the real
proof endpoint still serves the older artifact; show the saved rating and pending
message, with no old Approval card; keep that message visible without page
overflow at 390×844; invoke Retry with Enter; then show the actual newly published
1/5 proof and remove the pending message. Initial and recovered proof artifacts
come from the isolated daemon. Only the publication-failure response and pending
run read are intercepted. No provider is invoked.

## Current validation

**32/32 browser cases passed**, no retries or skips, in one named-spec run:

- `desktop-quality-round5-evidence-ux.spec.ts`: the new pending evidence failure
  and recovery in light and dark.
- `desktop-quality-round3-eval-ux.spec.ts`: healthy rating proof refresh, pending
  recovery, inline proof Retry, stale proof response rejection, and workspace
  switching.
- `desktop-quality-round4-loop-ux.spec.ts`: draft isolation, closing the previous
  budget dialog, keyboard load recovery, and late deletion ownership.
- `desktop-quality-round2-session-ux.spec.ts`: delayed send success/failure,
  attachment ownership, draft preservation, search ownership, and Mac Control-F.
- `desktop-quality-next-design.spec.ts`: header overflow keyboard ownership,
  workspace failure recovery, and older identity response rejection.
- `desktop-page-chrome.spec.ts`: shared page chrome, first-item/restored selection,
  delayed Proof deep links, overflow boundaries, and transparency behavior.

Command, run from `ui/` with Node 26:

```sh
PATH=/opt/homebrew/bin:$PATH OTTO_E2E_SLOT=round5-ux OTTO_E2E_PORT=17856 OTTO_E2E_PW_PORT=5229 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod npx playwright test desktop-quality-round5-evidence-ux.spec.ts desktop-quality-round3-eval-ux.spec.ts desktop-quality-round4-loop-ux.spec.ts desktop-quality-round2-session-ux.spec.ts desktop-quality-next-design.spec.ts desktop-page-chrome.spec.ts --project=desktop-browser --workers=1
```

[Browser log](../evidence/round-5-design-ux/browser.log),
[structured results](../evidence/round-5-design-ux/browser-results.json).
Expected fixture warnings about absent git remotes/non-repository conversation
directories are retained in the log. No application assertion failed.

`npm run check` passed with zero errors and warnings, including UI guards and all
TypeScript targets. `npm run build` exited zero. Logs:
[check](../evidence/round-5-design-ux/ui-check.log),
[build](../evidence/round-5-design-ux/ui-build.log).
The coordinator owns the final combined unit/Rust integration gates.

## Actual visual inspection

Inspected the captured images directly, including the failing screen, all four
new pending-state captures, healthy rating proof in both desktop themes and dark
phone, and loop detail on light desktop/dark phone. The pending state now explains
the evidence boundary beside the score; Retry remains reachable. Long proof
text wraps within the phone card. Loop status, decision input, verification input,
and primary action keep a readable hierarchy at both sizes. No new clipping,
horizontal page overflow, unlabeled control, or contradictory evidence was
observed in these captures.

| State | Desktop | Phone |
| --- | --- | --- |
| Pending, light | [capture](../evidence/round-5-design-ux/pending-evidence-desktop-light.png) | [capture](../evidence/round-5-design-ux/pending-evidence-phone-light.png) |
| Pending, dark | [capture](../evidence/round-5-design-ux/pending-evidence-desktop-dark.png) | [capture](../evidence/round-5-design-ux/pending-evidence-phone-dark.png) |
| Current proof, light/dark | [light](../evidence/round-5-design-ux/rating-desktop-light.png), [dark](../evidence/round-5-design-ux/rating-desktop-dark.png) | [dark](../evidence/round-5-design-ux/rating-phone-dark.png) |
| Loop detail | [light](../evidence/round-5-design-ux/loop-desktop-light.png) | [dark](../evidence/round-5-design-ux/loop-phone-dark.png) |

## Separate reassessment

| Lens | Product-quality score after repair | Basis |
| --- | --- | --- |
| Design | **9.8/10, scoped** | Current branch UI preserves the shared native shell, restrained state styling, readable content hierarchy, and responsive controls. The one newly confirmed presentation contradiction is removed and visually rechecked. |
| UX | **9.8/10, scoped** | Current-source journeys preserve drafts and focus ownership, isolate item/scope state, explain recoverable failures inline, and keep saved ratings and displayed proof consistent through failure and Retry. All 32 selected cases pass. |

These are the campaign's acceptance scores for the reviewed branch changes, not
a numerical estimate of every surface in Otto. There is **no remaining confirmed
design or UX deduction** in this reviewed scope, and no known bug deferred to
respect the reviewer count. The remaining distance to a nominal perfect score
does not stand for an invented defect or an unspecified follow-up requirement.

Confidence is high for the changed browser interactions and captured responsive
states, with explicit boundaries: this run does not validate production provider
accounts, VoiceOver end to end, or the new round 5 backend binary. School and
native helpers did not change, so the prior ten-cycle native evidence was not
rerun or presented as fresh evidence. Final integration against the combined
round 5 source remains the coordinator's gate, not a hidden product-quality
deduction. Browser lease released after this run; production UI source is stable.
