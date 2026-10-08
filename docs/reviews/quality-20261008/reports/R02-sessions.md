# R02 — Sessions, PTY, transcripts and agent execution

Review snapshot: `a0bd718b9fbc008d72c164ce643a24ed78e6368c` (PR #94 merged); main parent `196048df5bbd55a691b093ea0a3fa456f0912674`. Review branch: `review/quality-20261008`. Source line references below identify the reviewed implementation, before coordinator repairs. Concurrent fixes and other reviewers’ changes are not incorporated into these scores. This report closes the coordinator-requested four-issue reliability investigation; it does **not** certify the entire 193-path R02 ownership manifest.

Correctness, performance, architecture and test-quality passes were separated. Repository UI design guidelines informed the ownership, activation and feedback assessment. At the review handoff, three issues were reproduced through production code and the fourth was hand-traced. The authorized repair pass below subsequently reproduced the fourth and records verification separately. All four faulty implementations already exist in the main parent: these are existing defects, not identified PR #94 regressions.

## Assessment

Scores are provisional engineering assessments of inspected paths, not measurements or a whole-domain grade.

| Vertical | Score / 10 | Evidence confidence | What prevents 9.8 |
|---|---:|---|---|
| Correctness / bugs | 7.6 | High for the three reproductions; medium for the fourth trace | Transcript generation, clipboard ownership and passive activation invariants fail. UTF-8 truncation has a reachable panic. Repairs require negative-path verification. |
| Performance | 8.0 | Medium-low; source cost model, no benchmark | Opening N dormant Chat tiles can activate up to N processes. Tail identity repair must avoid duplicate pollers. Representative transcript/fanout/terminal workloads were not measured. |
| Design | 8.2 | Medium | The view-only intent is carried to Terminal but lost at the Chat boundary. Async input does not consistently retain operation ownership. Existing lifecycle/epoch and bounded-tail abstractions are useful foundations, but not consistently applied. |
| UX / usability | 7.7 | Medium | A paste can reach the wrong visible session; viewing history starts a session; a recaptured transcript stops updating. Only desktop Chromium was exercised; mobile, keyboard/accessibility breadth, RTL and both themes remain unverified. |

No independent performance number is inferred from CI counts or test durations. No claim of 9.8 is supportable from this review slice, even after the proposed repairs, without the remaining verification below.

## Findings

### R02-01 — Major / high: keepalive retains an obsolete transcript identity

**Location:** `crates/otto-server/src/transcript_tail.rs:173–178` (`touch`); lifecycle seams `:158–160` (`Slot::drop`) and `:219` (`should_continue`). Caller: `crates/otto-server/src/routes/transcript.rs:499` and `:572`. Reachability: `crates/otto-sessions/src/manager.rs:4165–4255` (`capture_nested_agents`).

**Trigger and trace:** A persistent shell session captures a new nested CLI conversation, changing its transcript path and potentially provider while retaining the Otto session ID. The manager explicitly persists the new nested PID/provider/session/path. A subsequent transcript request resolves the new path and calls `touch`. The registry lookup uses only the Otto session ID: an existing slot has its timestamp refreshed and returns without comparing provider or path. Its task continues polling the old file. `live_of` does compare provider/path, so it rejects that task for the newly resolved transcript. Repeated keepalives keep the obsolete task alive.

**Consequence:** Initial history can load from the new file while subsequent new messages fail to appear live; an old file can continue consuming tail work. This breaks an ordinary supported shell-to-nested-agent lifecycle, not merely an artificial registry collision.

**Evidence:** Added and executed `transcript_tail::tests::r02_touch_replaces_a_changed_transcript_identity`, invoking production `touch` twice with the same session ID and different provider/path, backed by `ServerCtx::for_tests` and temporary transcript files. It failed at the assertion that `live_of` expose the new identity: `touch must switch the live tail to the newly captured transcript`. No real session or daemon was used.

**Correction:** Compare provider plus path under the registry lock and replace the tail generation when either changes. Retain generation identity (for example the `Arc<Live>` ownership token) in the task. Both continuation and drop cleanup must require that token to still own the registry slot; an old task must neither continue nor remove its replacement. Only comparing identities in `touch` leaves the old ID-only cleanup race unfixed.

**Regression strength:** The red test directly rejects the existing behavior. After repair, also cover an old task’s delayed drop after replacement and rapid A→B→A changes. Same-file append tests do not cover identity replacement.

### R02-02 — Major / high: opening a view-only Chat tile resumes the session

**Location:** `ui/src/modules/agents/TiledView.svelte:488`; `ui/src/modules/agents/SessionView.svelte:981` versus `:985`; `ui/src/lib/stores/transcript.svelte.ts:392–406`; `crates/otto-server/src/routes/transcript.rs:558–564`.

**Trigger and trace:** TiledView supplies `resumeOnOpen={false}`. SessionView forwards that setting to Terminal, but its ConversationView instance has no corresponding setting. Acquiring a conversation view performs transcript resync and touch. The touch endpoint calls `ensure_live` whenever the session is reconnectable and has a provider session ID. Therefore merely displaying the Chat tile resumes its dormant process.

**Consequence / cost:** The explicit passive-view contract depends on the selected renderer. Opening N dormant Chat tiles can start up to N CLI processes and provider-resume attempts without an explicit Resume/send action. This reproduction proves activation; it does not measure provider CPU, memory or monetary cost.

**Evidence:** A real UI/isolated daemon browser test seeded a fixture-owned reconnectable shell with a nested Claude transcript. It verified that recorded history rendered, then asserted that the session remained dormant. Expected `live=false`, observed `live=true`. The daemon also logged the nested resume attempt `claude --resume 'r02-nested'`. The temporary DB state setup is explicit in the test and never accesses the user DB. The test does not depend on a real provider successfully completing a turn.

**Correction:** Carry the passive-view intent across the Chat/store/HTTP boundary. Either introduce an explicit non-resuming touch mode, or separate liveness keepalive from activation and let explicit Resume/send own activation. Preserve the intended active full-chat behavior while making Terminal and Chat obey the same tile contract.

**Regression strength:** Existing `desktop-view-only-attach.spec.ts` exercises the terminal WebSocket contract and misses the Chat path. The added real UI test fails against the current implementation; verify both passive Chat and intentional activation after repair.

### R02-03 — Major / high: a delayed clipboard read pastes into a replacement session

**Location:** `ui/src/lib/components/Terminal.svelte:2048–2054` (Ctrl+Shift+V); `:2362–2390` (`switchEngine`). Compare the existing ownership checks in the image-upload path at `:770–794`.

**Trigger and trace:** Request clipboard text in terminal A while `navigator.clipboard.readText()` remains pending. Change the active session to B in the same component. `switchEngine` changes the mutable `term` reference. The clipboard callback later calls `term?.paste(t)` using B rather than the requesting engine. Its writable check also occurred before the asynchronous read.

**Consequence:** Text intended for one session reaches another session’s PTY. The user’s navigation changes the destination of an already-started input operation. The reproduction used a harmless literal sentinel and executed no pasted command.

**Evidence:** In desktop Chromium, the test deferred `readText`, initiated it with the actual Control+Shift+V shortcut in A, navigated to B, and then resolved it with `R02_CLIPBOARD_FOR_A`. B’s real `/screen` response contained that sentinel. The assertion that it must not reach B failed. Only the clipboard provider was controlled; the actual UI switch, terminal routing and daemon PTY were exercised.

**Correction:** Capture the requesting terminal/session/socket generation and recheck ownership and writable state after the read resolves. Discard stale results, with understandable retry feedback where appropriate; do not unexpectedly send them to a parked session either. Follow the existing image-upload ownership pattern.

**Regression strength:** The controlled pending promise makes the async ordering deterministic and the destination-screen assertion verifies the real consequence. After repair, also verify same-session paste still works and a read-only transition rejects pending input. No mutation-testing tool was run.

### R02-04 — Minor / medium: a long Unicode signature panics during repo-map context assembly

**Location:** `crates/otto-context/src/repomap.rs:149–158`, specifically `:154`; consumer `crates/otto-context/src/materialize.rs:546–550`; spawn handling in `crates/otto-sessions/src/manager.rs:3279` and restart handling around `:5824`.

**Trigger and trace:** Repo-map context is enabled and a parsed definition’s signature crosses byte 120 within a multibyte character. For example, valid Rust source produced by `format!("fn {}() {{}}", "é".repeat(59))` is 127 bytes and byte 120 splits an `é`. `collect` uses `&t[..120]` after checking byte length; that Rust string slice panics. `extract` and `build_repo_map` do not catch it. The enclosing spawn-blocking context assembly yields a join error; the session manager’s default-on-error handling loses the assembled injection rather than returning the useful context.

**Consequence:** A valid multilingual source definition can silently remove the session’s context injection when this optional feature is enabled. The process-spawn caller contains the panic; this is not a demonstrated daemon crash.

**Evidence:** Source trace and concrete byte-boundary input. Added test `r02_unicode_signature_truncation_does_not_panic` at `crates/otto-context/src/repomap.rs:433`, invoking the real parser/extractor. This reviewer did **not** execute it; the coordinator owns the queued Cargo run. Do not count it as a fourth executed red test.

**Correction:** Truncate at a valid UTF-8 boundary or by characters, preserving the intended size policy. Add long multilingual signatures to the regression set. A repo-map failure should not silently discard unrelated context assembly.

## Executed checks and durable evidence

Rust command, repository root:

```sh
cargo test -p otto-server --lib transcript_tail::tests::r02_touch_replaces_a_changed_transcript_identity -- --exact --nocapture
```

Result: exit 101, 0 passed / 1 failed / 935 filtered; build 33.60 s, test 0.21 s. Tool execution session `10536`; assertion message quoted under R02-01. Full stdout was captured in the tool transcript, not persisted as a standalone log file. Durations are test-run observations, not performance evidence.

Browser command, `ui/`:

```sh
OTTO_E2E_SLOT=32 OTTO_E2E_PORT=18032 OTTO_E2E_PW_PORT=15132 OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod OTTO_E2E_SWEEP_ORPHANS=0 npx playwright test e2e/desktop-r02-session-ownership.spec.ts --project=desktop-browser --workers=1
```

Result: exit 1, 2 failed / 0 skipped, both the expected assertions described above. Tool execution session `52706`. Test source: `ui/e2e/desktop-r02-session-ownership.spec.ts:19` and `:37`. Isolated daemon slot 32 used port 18032 and temporary data directory `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-e2e-oxSkUU` (spawned PID 26925); automatic teardown completed. Desktop Chromium, default 1280×800 viewport, service workers blocked, existing merged debug daemon; not native Tauri/WebKit. Local toolchain inventory reports Rust 1.99.0 and Node 22.22.3; CI’s Node 26.10.0 differs.

Copied evidence survives later Playwright output replacement:

- `/tmp/otto-r02-review-evidence/playwright-results.json`
- `/tmp/otto-r02-review-evidence/desktop-r02-session-owners-90939-nt-until-an-explicit-resume-desktop-browser/trace.zip`
- `/tmp/otto-r02-review-evidence/desktop-r02-session-owners-90939-nt-until-an-explicit-resume-desktop-browser/test-failed-1.png`
- `/tmp/otto-r02-review-evidence/desktop-r02-session-owners-90939-nt-until-an-explicit-resume-desktop-browser/error-context.md`
- `/tmp/otto-r02-review-evidence/desktop-r02-session-owners-6fc58--into-a-replacement-session-desktop-browser/trace.zip`
- `/tmp/otto-r02-review-evidence/desktop-r02-session-owners-6fc58--into-a-replacement-session-desktop-browser/test-failed-1.png`
- `/tmp/otto-r02-review-evidence/desktop-r02-session-owners-6fc58--into-a-replacement-session-desktop-browser/error-context.md`

Screenshots are failure evidence, not a light/dark/mobile visual-quality certification. Existing CI’s 5271 Rust / 1708 UI tests are baseline context only. The recent PTY ACK-before-EXITED implementation and tests were read independently; its previously reported green results were not rerun or treated as proof of these unrelated paths.

## Coverage and explicit gaps

The expanded ownership manifest assigns 193 tracked paths plus feature-owned shared seams. This was a substantial targeted read, not 193 completed file reviews. The coordinator narrowed final reporting to the four established issues. No finding outside those four is asserted here.

| Area | Inspected coverage | Remaining limits |
|---|---|---|
| Session lifecycle | Manager sections for nested capture, ensure-live, human/room authority, kill/suspend, restart, selected provider/account/MCP and Codex capture helpers | Not the entire large manager or all provider adapters/tests; no real provider/account lifecycle exercise |
| PTY / transport | Holder client/input/ACK-exit barrier and relevant tests; input authority; selected WebSocket attach, credit and input paths | Held transport and WebSocket modules sampled; no reconnect/flood/native stress run |
| Transcript engine | Tailer, transcript library/records; relevant live-tail structs, budgets, touch, run/step and tests; selected cache, routes, fanout and history-index paths | Provider adapters/folding/usage corpus not exhaustive; no high-cardinality fanout or transcript benchmark |
| Agent execution / context | Agent-run watcher/recovery first 440 lines; oracle incremental line-tail section; orchestrator PTY/parser sections; provisioner and repo-map extraction/cache/build, context materialization callers | Complete turn verdict/recovery combinations, context/config modules and all consumers not verified |
| Agents / shared UI | Terminal keyboard, socket/reconnect, engine switching and image-paste paths; SessionView composition; TiledView mounting; ConversationView windowing/follow/liveness; transcript store/lifecycle; composer send/upload/slash paths | Not every Terminal/SessionView branch; no complete accessibility, keyboard, RTL, mobile/tablet or theme audit |
| Panels / supporting UI | FilesPanel; FileTree loading; OutputsPanel initial paths; selected Handover and WorkQueue code | Remaining panel states/actions largely unreviewed; no panel browser coverage |
| Existing regression tests | View-only attach and transcript-cache-live specs; portions of session-recovery/header specs; holder ACK-order test assertions | These specs were read, not rerun. Full suite and the proposed three baseline browser specs were intentionally not run; coordinator prioritized actual failing reproductions |

Positive source evidence includes explicit PTY acknowledgement/exit ordering, identity-aware lookup in `live_of`, bounded tail budgets, and the image-upload generation checks. Those mechanisms reduce risk in their own paths; they do not invalidate the failures above. Contract/state/error and shared-component seams were sampled, not exhaustively audited.

The initial review phase made no production edits. Its authorized test-only additions were the transcript-identity test, repo-map Unicode test, and the named browser spec. The later explicitly authorized repair pass is recorded below; the coordinator retains ownership of transcript-tail and clipboard production fixes.

## Authorized repair pass (2026-10-08)

R02 implemented passive Chat activation and UTF-8 truncation repairs; the coordinator implemented transcript-tail generation and delayed clipboard ownership repairs. This section supersedes the earlier pending-execution statements while preserving the original findings and pre-repair scores as snapshot evidence.

- **Passive Chat:** Added backward-compatible `view` query to transcript touch. `view=true` retains viewer keepalive and live tail behavior without calling `ensure_live`; omitted/false preserves existing clients. The transcript store defaults all keepalive/resync to passive mode. `SessionView` forwards `resumeOnOpen`; only an active `ConversationView` mount requests `view=false`. The existing explicit Resume action is unchanged. The HTTP contract and TypeScript query type are updated together. No persistent shared resume flag can let one pane overwrite another pane’s intent.
- **UTF-8:** The 120-byte signature slice now uses `floor_char_boundary(120)`, preserving the existing byte budget and valid text. The actual parser test first failed with `end byte index 120 is not a char boundary; it is inside 'é' (bytes 119..121 of string)`, then passed. The assertion verifies the exact retained signature.
- **Coordinator tail repair review:** Arc ownership protects continuation and cleanup across replacement, including A→B→A. A first repair left a check-to-publish race; independent review identified that interleaving. The revised `while_current` helper serializes the final identity check and synchronous publication with registry replacement; no await or reverse registry acquisition was found in the fanout paths inspected. Initial-fold guards reject obsolete queued generations. Validation is recorded below when complete.

Executed repair checks:

| Command | Result | Log |
|---|---|---|
| `cargo test -p otto-context --lib r02_unicode_signature_truncation_does_not_panic -- --nocapture` | Expected red: 0 passed / 1 failed / 61 filtered, exit 101 | `/tmp/otto-r02-review-evidence/repomap-red.log` |
| `cargo test -p otto-context --lib` | 62 passed / 0 failed | `/tmp/otto-r02-review-evidence/context-green.log` |
| `node --test ui/unit/transcriptLifecycle.test.ts` | Red before repair; 30 passed after repair | `/tmp/otto-r02-review-evidence/transcript-store-red.log`, `transcript-store-green.log` |
| `npm run check` (in `ui/`) | Passed, 0 Svelte errors/warnings; guards and TypeScript gates passed | `/tmp/otto-r02-review-evidence/ui-check.log` |
| `node node_modules/typescript7/bin/tsc -p tsconfig.e2e.json` (in `ui/`) | Passed after adding active-Chat positive control | `/tmp/otto-r02-review-evidence/e2e-types.log` |
| `rustfmt --check --edition 2021 crates/otto-context/src/repomap.rs crates/otto-server/src/routes/transcript.rs` | Passed | `/tmp/otto-r02-review-evidence/rustfmt.log` |

The browser regression now covers passive initial display, repeated keepalive, visibility recovery, explicit Resume and active-pane auto-resume, alongside the coordinator’s clipboard stale-result and same-session positive controls. The final coordinated results follow. Original domain-wide coverage gaps remain; these targeted repairs alone do not establish 9.8 or justify replacing the bounded snapshot scores with a whole-domain certification.

Final repair verification:

| Command | Result | Log |
|---|---|---|
| `cargo test -p otto-server --lib transcript_tail::tests` | 19 passed / 0 failed / 918 filtered; includes identity replacement, old-slot cleanup, stale publication and A→B→A generation regressions | `/tmp/otto-r02-review-evidence/tail-green.log` |
| `cargo build -p ottod` | Passed; rebuilt daemon used in the browser check below | `/tmp/otto-r02-review-evidence/daemon-build.log` |
| Slot-32 Playwright command recorded above, same named spec/project/ports and `OTTO_E2E_SWEEP_ORPHANS=0` | **3 passed / 0 failed / 0 skipped**, 8.6 s | `/tmp/otto-r02-review-evidence/browser-green.log`, `/tmp/otto-r02-review-evidence/playwright-green-results.json` |

The three final browser cases are `a suspended Chat tile stays dormant until an explicit resume` (4.0 s), `a delayed clipboard read does not paste into a replacement session` (1.6 s), and `an active Chat pane resumes on open` (655 ms). The first also exercises passive repeated HTTP touch and hide/show recovery; the clipboard case includes the coordinator’s positive same-session paste control. Both previously failing browser assertions are now green through the actual UI and isolated daemon. No native/WebKit/mobile or broad performance claim is made.

All four reported defect paths now have focused green verification; the fourth has executed red→green evidence instead of trace-only evidence. The HTTP query remains backward compatible (`view` omitted means the prior active behavior), and periodic/read-recovery calls explicitly select passive behavior. The comprehensive coordinator integration/clippy gates remain outside this reviewer’s executed checks.

Both leases have been returned to the coordinator. Cargo build session `72602` and final Playwright session `15953` exited 0. No owned Cargo/browser/daemon workload remains running. No commits, pushes or deployments were performed. The original scores remain explicitly pre-repair; the targeted closure does not erase the documented whole-domain verification gaps.
