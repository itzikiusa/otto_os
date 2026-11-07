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
