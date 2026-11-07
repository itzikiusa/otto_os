# Threads continuity and context

Five additions approved after comparing T3 Code's November 1–7 changes with Otto:
recoverable drafts, structured questions, fresh context within a thread, tool images,
and review/diff context attached to a message.

## Acceptance

- Draft text, attachments, model/effort/access selection, context chips and fresh-session
  intent recover after reload. Saves belong to the authenticated human, workspace and
  thread. A late request cannot overwrite newer typing or resurrect a sent draft.
- Questions support choices and free text through provider-native replies. Pending
  questions remain reachable, cancelled questions close, and requests from a stopped
  provider cannot be answered in a replacement process.
- `/clear` keeps the visible thread and history, marks a new provider-session boundary,
  and does not silently stop active/background work or discard queued messages.
- Expanded tool details show bounded raster images alongside text, with accessible
  previews on phone and desktop.
- Review comment conversations and selected diff context become removable, attributed
  composer chips. Adding context preserves existing work and never sends or fixes it
  without the user's send action.

## Verification record

Implementation and integrated verification are in progress. Results and screenshots
will be recorded here after the corresponding checks run. No physical-phone or
Tailscale verification is implied by browser emulation.

Baseline at `ffa907b27`: `cargo test -p otto-agent-protocol --lib` — 44 passed.

Integrated Threads protocol checks: 62 unit tests and 40 real-runtime/fake-provider
integration tests passed, including fresh-session/question identity boundaries.
Draft persistence state tests passed (revision conflicts, owner separation, expiry,
quota and deletion cascade). Timeline projection checks: 22 passed.

## Review engine and workflow acceptance

Additional authorized scope: adaptive PR lens/provider selection; verified causal
findings and critical-path test assessment; explicit prior-finding reconciliation
and rescoring; local fix-agent handoff; workflow and Slack re-review; bounded
multi-repository concurrency and valid context reuse. Deterministic Go gates retain
the supplied one-script-per-repository contract. The example workflow runs on a
different machine; local tests cannot establish its live end-to-end performance.

Threads UI check (Svelte and all TypeScript projects): passed. Full UI unit suite:
2,401 passed. Server draft authorization: passed. Policy coverage: 3 passed.
Browser acceptance: 24 passed across desktop Chromium, iPhone WebKit portrait and
landscape, and Android Chromium. Follow-up question/header polish: 4 passed.
These use a real isolated daemon with deterministic fake providers; no physical
phone, live provider account, or Tailscale network was exercised. Screenshots:

- [Phone questions, light](screenshots/questions-phone-light.png) / [dark](screenshots/questions-phone-dark.png)
- [Recovered draft, light](screenshots/draft-desktop-light.png) / [dark](screenshots/draft-desktop-dark.png)
- [Tool image preview, light](screenshots/tool-image-desktop-light.png) / [dark](screenshots/tool-image-desktop-dark.png)

## Review of review changes

Independent source reviews identified the following required corrections. All are
implemented, with the integrated review/workflow suites passing (66 and 79 tests):

- Context reuse must include resolved upstream input and skill/provider dependencies.
- Nested workflow review steps must support admission and prior-review lineage.
- Explicitly unverified claims must keep the review incomplete.
- Prior-finding identities must survive more than two iterations, including findings
  already marked fixed/not relevant and mutable finding rows later seen again.
- The final verifier must account for every input claim by stable identity; a valid
  response that silently omits another claim is incomplete.

No additional concrete authorization or pinned-fix-worktree defect was found in the
bounded independent passes. These were source traces, not executed exploit tests.

Review/workflow UI verification: four desktop Chromium checks passed, including
390×844 phone viewport checks for scope and fix-launch retry. No actual provider
was launched by these UI fixtures. Review fix-launch API regression passed for
workspace permissions, incomplete/mismatched review rejection and idempotent paused
goal reuse. Route policy coverage: three passed. Python workflow-upgrade checks:
two passed; the original gate prompt and graph are preserved.

- [Adaptive review, light](screenshots/adaptive-review-light.png) / [dark](screenshots/adaptive-review-dark.png) / [phone](screenshots/adaptive-review-phone.png)

## Goal graph extension

Additional user-authorized scope: enforced dependency graphs, editable suggested
loops for Angular, Go, general engineering, research, POC (backend/frontend/both),
rewrite and performance. The templates are informed by a selected recent Claude
and Codex history sample, with private provenance retained outside the repository.
Graph runtime: 42 focused Goal Loop tests passed before final helper extraction;
the full integration gate checks the extracted form. Graph API validation/save/load
and research report normalization passed against the actual router/state. Two
browser flows passed in light and dark, each including desktop and 390px phone
views, template replacement cancellation, cycle rejection, edited graph submission,
state display and reload. The browser fixtures mock the provider/API result boundary;
the separate API test verifies real persistence. No provider turn is launched.

Independent source review confirmed human verification cannot be a prerequisite
invalidated by downstream work; terminal human nodes remain supported. Task prompts
retain the concrete user goal. Failed final review reopens a task for repair, and
an optimistic reevaluation cannot pass that task without execution.

- [Graph editor, light](screenshots/goal-graph-editor-light.png) / [dark](screenshots/goal-graph-editor-dark.png)
- [Phone editor, light](screenshots/goal-graph-phone-light.png) / [dark](screenshots/goal-graph-phone-dark.png)
- [Phone task evidence, light](screenshots/goal-graph-state-phone-light.png) / [dark](screenshots/goal-graph-state-phone-dark.png)

Seven D2 graphs rendered and passed the diagram quality check. The offline HTML
guide was checked in Chromium for all seven selections, mobile layout, keyboard
zoom closure and absence of network requests. The sampled recent session research
covers both Claude and Codex; it establishes requested patterns, not verified
historical task outcomes.
