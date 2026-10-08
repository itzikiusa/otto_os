# Round 2 — design and UX

Reviewer 12. This is the design/UX portion of the second full campaign round; the earlier targeted iterations are Round 1. The worktree is shared and mutable on `review/quality-20261008-next`, starting at `195de62c8`. This reviewer owns the conversation repairs, their named browser regression, and the optional School probe extension. No commit, push, installed-app mutation, production daemon, or real provider write was performed.

## Findings that drove product changes

**R2-UX-1 · medium · fixed — a late send or upload steals the user's current caret.** `ui/src/modules/agents/conversation/Composer.svelte:82`. Submitting a prompt, beginning the next draft, then searching while the request is pending used to move focus back to the composer when either success or failure arrived. Upload completion had the same behavior. This makes subsequent search typing modify the unsent prompt instead. The completion paths now restore composer focus only while it still owns focus, or when its disabled button yielded focus to the document body; detached textareas are excluded. Send, interrupt and upload use the same guard. The normal send path still returns the caret to the composer.

Evidence: [send success/failure red](../evidence/round-2-design-ux/focus-red.log), [upload red](../evidence/round-2-design-ux/attachment-red.log), [current green](../evidence/round-2-design-ux/browser-final.log). The red assertions specifically report search `expected focused / received inactive`. The final tests type again after completion and check both the search and newer unsent draft. An initial test-driver attempt stalled in `response.finished()` despite receipt of the intercepted response; [that log is retained](../evidence/round-2-design-ux/harness-first.log). Removing that unnecessary transport-completion wait exposed the actual focus failures before the production fix.

**R2-DUX-2 · medium · fixed — one Find shortcut opens two competing search surfaces.** `ui/src/modules/agents/conversation/ConversationView.svelte:524`. Screenshot inspection after the first green interaction run revealed the global Find overlay still covering the toolbar after Escape closed conversation search. The global keymap captures Cmd+F before the conversation's bubble handler; without registered ownership both handlers opened their own UI. The conversation now uses the existing focused-search ownership mechanism, releases it on blur/unmount, respects an already handled key event, and returns focus to the field that opened search on Escape. Embedded CodeMirror/terminal controls retain their own find ownership.

Evidence: [search red](../evidence/round-2-design-ux/search-red.log) reports global search `expected count 0 / received 1`. The final regression checks one search surface, repeated Cmd+F, Escape focus return, and global Find after the user moves to the sidebar. This is a concrete reduction in overlapping chrome and keyboard disruption; no cosmetic redesign was added.

**R2-UX-3 · low · fixed — Control-F hijacks macOS text movement.** `ConversationView.svelte:563` accepted raw `ctrlKey`, despite the global keymap deliberately preserving macOS Control chords. A Mac-platform browser negative control confirmed that Control-F opened search from an unsent composer draft. The local handler now uses the same `appModifier`/`isMacPlatform` policy as the app: Cmd+F on Mac and Ctrl+F on non-Mac clients. [Red evidence](../evidence/round-2-design-ux/mac-shortcut-red.log). The regression verifies no search surface, continued composer focus and an unchanged draft; it does not claim physical macOS keyboard delivery.

## Current browser evidence

**Eight new checks passed, zero skips/retries**, with **six relevant existing checks passed, zero skips/retries** after the focus/search ownership changes. The final one-line Mac modifier correction was followed by all eight new checks; the existing suite was not needlessly repeated for that platform guard. [New results](../evidence/round-2-design-ux/browser-final-results.json), [existing results](../evidence/round-2-design-ux/existing-composer-final-results.json), [existing execution log](../evidence/round-2-design-ux/existing-composer-final.log).

The new light/dark journeys exercise a populated real transcript parsed by the isolated daemon, an injected transcript-load failure with inline Retry, unsent multiline draft preservation during recovery, keyboard search/Escape, Chat→Terminal→Chat, a 390×844 phone layout, and reload. Delayed sends/uploads are intercepted rather than delivered to a provider. Existing coverage exercises image upload across composer remount, Enter/Shift+Enter/Stop, previous-message navigation and reuse, code blocks, narrow previews, and six tiled chat panes. These are observed user interactions, not counts of empty module headers.

Runs used Node 26, one Chromium desktop worker, slot `round2-ux`, daemon **17834**, Vite **5207**, orphan sweep disabled, and an isolated throwaway data directory. Daemon SHA-256 was `ef5b20835f96449cb06f55ec757099b206fe63576df3cad6b37a90ec0441c187`. Concurrent correctness compilation makes these functional results unsuitable for latency/performance claims. The fixture uses a shell with a Claude transcript; visible terminal startup text in some captures belongs to this fixture and does not establish real-provider behavior.

`npm run check` passed with **0 errors, 0 warnings**; production UI build exited **0**. [Check](../evidence/round-2-design-ux/ui-check.log), [build](../evidence/round-2-design-ux/ui-build.log). The UI review checklist found no new tokens, styles, inaccessible controls, modal mechanisms or contrast changes in these behavioral repairs. Root owns integrated unit/Rust gates.

**Integration failure retained:** the first full UI unit run passed 1,827/1,828; `composerSendOwnership.test.ts` extracts the production `send()` body into a Node harness and initially omitted its new `restoreComposerFocus()` dependency, producing a ReferenceError. [Original failure](../evidence/round-2-design-ux/unit-integration-initial-failure.log). The coordinator owns repairing that extraction harness to include the real helper and its DOM dependency, then rerunning the complete unit gate. This report's browser/native passes do not conceal or substitute for that gate; no production workaround or assertion weakening is required.

## Visual inspection

All initial six light/dark captures were opened and inspected, which discovered the second finding. The four final populated/phone captures were then reopened: readable attributed responses and code, opaque reading/editing surfaces, bounded multiline composer and Send/Stop controls, no page-wide horizontal scroll, and no leftover global Find overlay. The initial and final images remain separate so the improvement is reviewable.

| Surface | Before | After |
|---|---|---|
| Light desktop | [overlapping Find](../evidence/round-2-design-ux/conversation-populated-desktop-light.png) | [restored toolbar and focus](../evidence/round-2-design-ux/final-conversation-populated-desktop-light.png) |
| Dark desktop | [overlapping Find](../evidence/round-2-design-ux/conversation-populated-desktop-dark.png) | [restored toolbar and focus](../evidence/round-2-design-ux/final-conversation-populated-desktop-dark.png) |
| Light phone | [covered toolbar](../evidence/round-2-design-ux/conversation-draft-phone-light.png) | [clear toolbar, retained draft](../evidence/round-2-design-ux/final-conversation-draft-phone-light.png) |
| Dark phone | [covered toolbar](../evidence/round-2-design-ux/conversation-draft-phone-dark.png) | [clear toolbar, retained draft](../evidence/round-2-design-ux/final-conversation-draft-phone-dark.png) |

Load-error presentation: [light](../evidence/round-2-design-ux/conversation-error-desktop-light.png), [dark](../evidence/round-2-design-ux/conversation-error-desktop-dark.png). Retry stays in the conversation and the unsent draft remains available below it.

## Native bounded acceptance

**Passed once, exit 0, ten cycles; no retry or timeout widening.** The final production UI was built and embedded in the freshly compiled native SPA probe. [Build](../evidence/round-2-design-ux/native-build.log), [execution](../evidence/round-2-design-ux/native-probe.log), [source/binary manifest](../evidence/round-2-design-ux/manifest.json). The existing School helper accepts `OTTO_NATIVE_SCHOOL_CYCLES` in **1..20**, defaults to **3**, and retains its strict visibility/frame/identity/room assertions. This run used **10**, with an additional assertion that the camera does not move during the hidden interval.

The actual child WKWebView used **Apple GPU / WebGL 2**, loaded the production assets without missing files, and entered the fixture classroom through its pointer handler. All ten Add Workspace→Cancel cycles held hidden frame count and camera stable, then resumed visible rendering; first hidden/restored frame counts were 36→38 and the tenth 66→68. The detached child was physically exposed beside the host, established `document.hidden=false` and `document.hasFocus=false`, settled for 200 ms, then advanced by more than one frame across a further 600 ms. Scene identity and room remained unchanged on Return. The probe also passed real SPA detach/return continuity and native menu zoom 100→200→190→100%, preserving the unsaved Add Workspace field, focus and viewport bounds.

This uses isolated incognito native stores and throwaway daemon **7821**; teardown completed and no listener remained. The daemon is the explicitly recorded `ef5b208…` baseline, not the later concurrent Round 2 evaluator build. No scoring endpoint was exercised. Native AX was deliberately disabled because this is the bounded visibility replay, not a claim that the prior unavailable AX/VoiceOver acceptance has become green. These are current functional observations, not a production repair of the historical Cancel timeout or a long-duration reliability benchmark.

## Separate assessments and actionable closure

The scored basis is the actual deficiencies above, their red→green repairs, final visual inspection, and bounded native acceptance. Scores express quality on this round's reviewed surfaces; confidence expresses how far that evidence can be generalized. A 9.8 acceptance here means no remaining confirmed material defect in the named journeys, with real repair and regression evidence. It is not a claim of 98% test coverage, defect probability, or exhaustive whole-product certification.

| Lens | Observed quality | Confidence and basis |
|---|---:|---|
| **Design** | **9.8/10** | High for these desktop/phone conversation and error states: competing Find chrome removed, the toolbar remains exposed, readable content and bounded draft controls inspected in light/dark. Medium for extrapolation to modules not inspected in this round. No remaining observed design deficiency is assigned an unexplained deduction. |
| **UX / usability** | **9.8/10** | High for the named draft, asynchronous focus, Find/Escape, Mac shortcut, Retry, view-switch/reload and native modal/visibility journeys. Three concrete defects are closed. Native confidence is bounded by one current ten-cycle replay; historical timeout causality, physical keyboard delivery and assistive technology remain explicitly unestablished. |

These assessments do not numerically penalize absent evidence as if it were a known broken product interaction. A whole-product 9.8 cannot be inferred from this report alone; the coordinator combines the independent performance/correctness verticals and integrated gates.

| Gap | Concrete acceptance | Current state |
|---|---|---|
| Asynchronous caret takeover | Success, failure and attachment completion keep the chosen search field focused and leave the newer composer draft intact; ordinary send still focuses composer | Closed, red→green |
| Duplicate search chrome and Escape continuity | One search surface per Cmd+F; repeated shortcut does not duplicate; Escape returns to composer; sidebar retains global Find | Closed, red→green and visual inspection |
| Mac text-editing shortcut | Control-F does not open search or remove composer focus; the draft is unchanged | Closed, red→green |
| Historical native Cancel uncertainty | One bounded current native run: ten actual modal Cancel cycles, hidden frames/camera stable, visible frames resume, scene/room retained, settled visible-unfocused interval advances | Current acceptance passed once; original cause remains unknown |

The native replay can strengthen confidence in current behavior; even a clean result cannot retroactively assign the original timeout a cause. VoiceOver speech/rotor operation, signed install acceptance, and real provider workflows are not claimed by this review.
