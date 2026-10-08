# R16 — Independent design and usability review

**Approve with scoped fixes; whole-product 9.8 is not established.** Two material UX finding families were independently verified after the domain repair pass. The coordinator repaired both; final mounted badge verification passed. No additional architecture blocker was established in this bounded review.

Base: PR94 remote merge `a0bd718b9fbc008d72c164ce643a24ed78e6368c`; branch `review/quality-20261008`, uncommitted shared-worktree repairs. This report does not reopen the completed PR94 merge. R16 owned only this report/evidence and, by explicit coordinator extension, `ui/e2e/desktop-review16-chat-drafts.spec.ts` plus consent-test assertions. All production edits were made by the coordinator. No commit, publication, installed app, real daemon or user state was changed by R16.

## Findings, ordered by user impact

### R16-01 — Major: an old failed chat send replaces a newer unsent draft; clearing a draft is undone

Locations at discovery: `ui/src/modules/product/DiscoveryChat.svelte:80,140` and `ui/src/modules/product/RefineChat.svelte:53,109` (line numbers shift with the correction).

Both modules saved a departing composer only when its text was truthy, leaving the previous map entry behind when the user deliberately cleared it. A→B→A, clear A, B→A restored the old A text. More seriously, send A and hold its response, switch B→A, compose a newer A draft, switch to B, then fail the old send: its unconditional `drafts.set(target, body)` destroyed the newer stored A draft. Switching conversations resets the local sending flag, so this is reachable through the mounted interface despite disabling the composer during the original send.

Independent reproduction: four real mounted browser cases, two per conversation kind, **all failed at the final user-visible value assertion**. Each received `Old A text` instead of the requested empty string or `Newer A draft`. Requests were routed to deterministic delayed fixture responses; real isolated Product story creation and production Svelte components were used. No provider turn was executed. Evidence: `../evidence/R16/chat-drafts-red.log` and failure screenshots.

Correction: remember explicit empty drafts when leaving an editable composer; distinguish the temporary empty composer of an in-flight send; restore an old failed send only if no newer stored draft decision exists. The coordinator implemented the correction in both modules. All four independent regressions passed, and the earlier delayed-failure restoration control also passed. This proves both the preservation and ordinary recovery branches. It does not establish draft persistence across component destruction or app restart.

Structural assessment: these two modules duplicate the same ownership policy, and this pass found the same two defects in both. Keep their regression vectors paired. A small conversation-draft policy helper could concentrate future policy changes if more shared behavior is added, but replacing the different transcripts/providers with a generic chat framework is not justified by this finding.

### R16-02 — Medium: an approved older version makes the current draft appear approved

Locations: `ui/src/modules/design-hall/brand/BrandEditor.svelte:496-497`; the same presentation at `ui/src/modules/design-hall/ArtifactView.svelte:740-741`.

Independent visual evidence from `../evidence/R12-design/brand-phone.png` showed adjacent toolbar text **v2 Approved** while the same page said **v2 is saved but not approved** and that following designs still use v1. Source inspection confirmed the version label used `head.seq` but the status pill used artifact status, whose approved pointer could still refer to v1. The repaired immutable approval target was correct; the visible summary attributed approval to the wrong apparent version. A user scanning the toolbar could reasonably believe the latest brand changes were accepted.

Correction: when artifact status is approved but the current head is not the approved version, render that current head as Draft; retain persisted approval and non-approved workflow states. The coordinator applied this presentation correction to both BrandEditor and ArtifactView. The consent test now checks the current-head Draft badge after approving v1 while v2 is already current, in addition to asserting the actual POST still names v1. Brand light/dark/phone captures are written under `../evidence/R16/`, preserving the original contradictory R12 evidence.

## Independent coverage and positive evidence

This is a risk-oriented cross-cutting review, not a line-by-line re-review of every file assigned to R01–R14. Read PLAN/README/VALIDATION, the domain findings and coverage limits, design README/checklist, architecture-review and devex-review guidance. Inspected production diffs and their callers around:

- Draft ownership and navigation: Product Discovery/Refine and their parent tabs; Skills editor/detail; Workbench store and page; Assistant paged selection and load-more; API import/upload/proto/reflection and stream ownership.
- Target identity and truthful feedback: Git force-push consent; AWS/Kubernetes captured targets; replay partial-success evidence; Proof and Run-with-Otto actions; Design/Brand approval and collection mutation; daemon settings submitted-value acknowledgement.
- Recovery and shared structure: Browser initial load/Retry, Personal Agent schedules, Vault note/refine/preview ownership, Proof/Run stores, auth transition shell, shared LoadState/Modal and latest-request helpers.
- Visual samples independently opened: Brand phone, replay partial-success phone dark, Skills desktop light, Workbench desktop dark. They retain readable content hierarchy, semantic feedback and shared chrome. The Brand status contradiction above was reported rather than accepted because screenshots existed.

The repairs generally preserve module locality: async ownership guards sit beside the await and publication, stores own request ordering, pages own confirmation text, and shared LoadState/Modal implement consistent error/focus behavior. Different actor/resource semantics make a single universal async-mutation wrapper unjustified. No new public UI API migration or caller-facing configuration footgun was established in the sampled surfaces. Existing domain architecture notes about duplicate API preparation and DB edit provenance remain narrow maintenance observations, not mandates for a whole-product rewrite.

## Executed checks

All R16 execution used Node 26.10.0, Chromium Playwright (phone profile where noted), isolated slot `r16`, daemon port 17816/UI 5186, `OTTO_E2E_SWEEP_ORPHANS=0`, and fixture-owned data. The daemon was the earlier R04-built binary, **not a fresh R08–R14 backend build**. These results validate current UI behavior and routed responses; they do not validate the later server repairs. There was no competing heavy browser run.

1. `desktop-review16-chat-drafts.spec.ts`, desktop-browser, workers 1: four expected reds, then four greens after the coordinator fix. Existing `desktop-product-chat-ownership.spec.ts` remained green. First combined green batch was6 pass / 1 test-locator failure: the newly written brand assertion mistakenly searched inside the brand grid for its sibling PageHeader. That test error was corrected; it is not hidden as a product regression or a passing run.
2. `desktop-modal-focus-resize.spec.ts` plus five-name filter over `desktop-ux-r4-shell.spec.ts`: **5/5 passed in 9.5 s**. Covered workspace Modal focus/draft across breakpoints and focus return, Notes focus/selection/draft through desktop/tablet/phone, failed Notes save→Retry→reload, standalone plan cancellation with zero execution and error recovery, API request draft/focus through breakpoints. The selection included five total cases, not the full shell spec.
3. `rtl.spec.ts --project=iphone-portrait --grep 'rtl — (product|api|assistant|vault):'`: **4/4 passed in 12.1 s**. Verified RTL applied, no horizontal page overflow and the repository axe gate on four initial page states. The axe helper rejects critical and new serious violations against its existing baseline; it does **not** prove zero WCAG violations, all populated states, or assistive-technology usability.
4. `desktop-design-consent.spec.ts`, desktop-browser, workers 1: **2/2 passed in 5.9 s**. Both immutable v1 approval requests remain correct after the live v2 update; both current-head badges now show Draft. Brand light/dark/390 px captures were regenerated under R16 evidence and the phone capture was independently opened: v2 Draft now agrees with the v1 approval explanation.

Durable logs and source hashes: `../evidence/R16/`. The hash manifest pins the final sampled chat/approval implementations and added test sources; the shared branch as a whole remained mutable during review. Test durations are functional execution times, not UI latency benchmarks.

## Scores and remaining evidence

| Vertical | Independent assessment | Confidence and limit |
|---|---:|---|
| Design | 9.3/10, inspected repair surface | Medium: coherent local ownership and shared UI conventions; corrected misleading approval state; representative visual samples and source seams. Not every module/theme/populated state was independently rendered. |
| UX/usability | 9.3/10 after these fixes | Medium: independently reproduced and repaired draft loss, preserved recovery controls, keyboard/focus/resize and four RTL initial states. Native VoiceOver, all populated phone/tablet workflows and external-provider recovery are not verified. |

These numbers are bounded engineering judgments, not arithmetic averages of domain scores or percentages inferred from green tests. They do not certify the entire repository at 9.3, and they do not establish the requested 9.8. R13 supplies actual native window/child-pane lifecycle evidence, which is stronger than browser emulation, but its report explicitly excludes VoiceOver, hardware-rendered School composition and prolonged native soak. R07 retains native browser/remote-login breadth limits; R02/R03 retain broader session/Git journey limits. Final fresh-daemon integration is also still pending with the coordinator.

To support a stronger product-wide claim, close those named critical-journey evidence gaps using representative real native/accessibility/provider fixtures, reconcile final source hashes with the integrated build, and re-evaluate material findings. Absence of those measurements is an evidence limit, not an invented assertion that those paths are broken. No score inflation, baseline increase or test suppression is recommended.
