# Hermes Agent vs Otto — Self-Improvement Loop: Analysis & Recommendations

> Date: 2026-06-19. Source for Hermes: research pass over github.com/nousresearch/hermes-agent + its docs/blogs (see Sources at end). Source for Otto: `crates/otto-improve/*`, `crates/otto-core/src/event.rs`, `crates/otto-channels/*`.
> Trigger: user asked to study Hermes's self-learning loop and (a) improve Otto's loop, (b) surface self-improvement results **immediately** into Telegram/Slack (like the `💾 Self-improvement review: Patched … in skill …` line they shared).

---

## 1. How Hermes's self-improvement loop works (and what it does well)

| Dimension | Hermes |
|---|---|
| **Triggers** | Event-driven + periodic: after a complex task (5+ tool calls), after error-recovery, on user correction, on discovering a non-obvious working approach; a background *skill-review sub-agent* periodically reviews recent history; a **Curator** runs on a ~7-day cycle to maintain the library. |
| **Learns from** | Full transcripts (SQLite + FTS5), explicit feedback (👍/👎), implicit signals (clarification requests, corrections), tool success/failure, and usage metrics. |
| **Memory model** | Four layers: always-loaded *prompt memory* (small, cache-friendly), *session search* (episodic, FTS5), *skills* (procedural, **human-readable Markdown**, agentskills.io), *user model* (persistent preferences). |
| **How edits apply** | Skills are **patched** (targeted text replacements, not full rewrites). Default auto-applies on disk (visible next session); with `skills.write_approval: true`, every write is **staged** and survives restarts (`/skills pending`, `/skills diff`, `/skills approve|reject`). |
| **Maintenance (Curator)** | Grades every skill on **usage freq · success rate · exec time · satisfaction**; auto-transitions unused→stale (30d)→archived (90d); optional LLM consolidation of overlapping skills; writes human-readable `REPORT.md` + machine `run.json`. |
| **Safety** | Optional write-approval staging; a documented effort (#15204) to **restrict the background review agent to skill-management ops only** (it had leaked into terminal/messaging side-effects); 3-stage staging→review→active pipeline for regulated envs. |
| **How it surfaces learning** | **Weakly.** Optional background "voice" messages (unreliable scope), manual `hermes curator status` / report files, and on-demand `/skills pending`. **No always-on, proactive, structured notification** when it learns. |

**Hermes's best ideas worth stealing:**
1. **Patch-based skill/memory edits** (not full-file writes) — preserves working content, smaller blast radius. *Directly relevant to Otto's S6 fix: the audit flagged Otto writing the entire file on auto-apply as a poisoning vector.*
2. **Curator maintenance loop** — periodic grading + stale/archive lifecycle so the skill library doesn't rot or bloat.
3. **Multi-signal feedback** — 👍/👎 + implicit corrections + usage/success metrics, not a single signal.
4. **Token-efficient memory** — frozen prompt snapshot for cache reuse; async memory writes off the response path.
5. **Restrict the self-improvement agent's tools** to skill/memory ops only.

---

## 2. Gap analysis — Hermes vs Otto (`otto-improve`)

| Capability | Hermes | Otto today | Opportunity |
|---|---|---|---|
| Edit granularity | Patch | **Full-file write** on auto-apply (`engine.rs`) — now size-capped + injection-gated (S6 batch-2), but still whole-file | Move Memory/skill auto-apply to a **diff/patch** with context-match (safer, smaller) |
| Triggers | task-done / error / correction / periodic | scheduler + live + product-narrative | Add **error-recovery** and **explicit-correction** triggers; add a 👍/👎 signal |
| Library maintenance | Curator (grade + stale/archive + consolidate) | skill-eval is **on-demand only** | Add a periodic **curator** using `otto-usage` + skill-eval scores to grade/retire skills |
| Feedback signals | explicit + implicit + usage | session digests only | Capture 👍/👎 on proposals + reuse/success counts |
| Approval surfacing | manual (`/skills pending`) | queue + approve/reject/rollback API + `ImprovementApprovalPending` event | **Push** pending/applied to the user proactively (this is the channel feature) |
| Proactive notify | weak | events exist but only reach the **UI WS**; channels only see it incidentally | **Build it** — Otto can beat Hermes here |
| Memory safety | optional staging | off-by-default, allow-list, audit, rollback, **deterministic gate** (batch-2 S6) | Otto is already **stronger** than Hermes here — keep it |

**Net:** Otto's *safety scaffolding already exceeds Hermes* (off-by-default, workspace allow-list, audit+rollback, and now a deterministic memory gate). Otto's gaps are (1) **patch-based edits**, (2) a **curator/maintenance loop**, (3) **feedback signals**, and (4) **proactive surfacing** — which is exactly the user's immediate ask.

---

## 3. Recommendations for Otto's loop (ranked)

### Must / high-value
- **R1 — Proactive self-improvement notifications to Telegram/Slack** (the user's ask). Design in §4; implement now.
- **R2 — Patch-based Memory/skill edits.** Replace the whole-file `patch.after` write with a contextual diff apply (anchor + replace), keeping the existing deterministic gate. Lower blast radius than a full overwrite; mirrors Hermes. Files: `otto-improve/src/engine.rs` apply path + `classify.rs` gate (validate the patch, not just the result).

### Should
- **R3 — Curator/maintenance loop.** A periodic (e.g. weekly) job that grades installed skills using `otto-usage` (invocation counts) + skill-eval scores, flags unused/stale skills, and proposes archive/consolidate as normal queued proposals (never auto-delete). Reuse the existing scheduler + proposal/approval pipeline.
- **R4 — Feedback signals.** Add a 👍/👎 (and "this proposal was wrong") action on applied/queued improvements that feeds back into autonomy weighting and the curator's grade. Capture implicit signal: a user reverting/rolling back an edit is a strong negative.
- **R5 — Error-recovery & correction triggers.** Fire an improvement evaluation after a session shows repeated tool errors then recovery, or after an explicit user correction — Hermes's highest-signal triggers, currently absent.

### Could / nice
- **R6 — Restrict the self-improvement run's tool surface** to memory/skill files only (defense-in-depth; Hermes learned this the hard way in #15204). Otto already runs headless + path-guarded, so this is incremental.
- **R7 — Curator transparency artifact** — a human-readable "what I learned this week" report (like Hermes's `REPORT.md`), surfaced in Insights.

---

## 4. Feature design — immediate self-improvement → Telegram/Slack (R1)

**Why Otto can do better than Hermes:** Otto already emits structured events (`Event::ImprovementEditApplied`, `ImprovementApprovalPending`, `ImprovementRunFinished` in `otto-core/src/event.rs`) and already runs Slack/Telegram adapters (`otto-channels`, `Adapter::send(chat, thread, text)`). Today those improvement events only reach the **UI WebSocket** (`ws_events.rs`); the Slack line the user saw appears only because an improvement happened *inside* a channel-driven session and got mirrored. The feature is to **proactively push** a concise, human-readable line to the user's channel(s) the moment an edit is applied or queued.

### Mechanism
A small **notifier** that subscribes to the daemon event broadcast and, for the improvement events, formats a line and sends it to the configured channel target(s).

- **Hook events:** `ImprovementEditApplied` (✅ applied), `ImprovementApprovalPending` (⏳ needs approval), optionally `ImprovementRunFinished` (summary: N applied / M queued).
- **Message format** (mirror the screenshot):
  - applied: `💾 Self-improvement: patched \`<target>\` in <skill|memory> '<name>' (<n> change(s)) — applied`
  - pending: `📝 Self-improvement: proposed edit to <skill|memory> '<name>' — needs approval (/improve approve <id>)`
  - finished: `🧠 Self-improvement run: <applied> applied, <queued> queued in workspace '<ws>'`
- **Routing / target:** add a per-integration (or per-workspace) **notify chat id** in channel settings. MVP: reuse the channel integration's configured default chat (the same chat the bot already operates in); only fire for integrations whose workspace matches the event's `workspace_id`. Opt-in flag `notify_self_improvement` (default off) so it's not noisy.
- **Draft vs auto:** applied/finished → send directly (informational). Pending-approval → send directly too (it's an FYI, not an outbound customer reply), but the message is clearly an internal notice.

### File-by-file (implementation plan)
1. `crates/otto-channels/` — add a `notifier` (e.g. `improve_notify.rs`): given an `Event` + the adapter registry, format and `send` to the target chat(s). Add a `notify_self_improvement: bool` + optional `notify_chat` to the channel/integration settings struct.
2. Wiring — where the `ChannelManager` is constructed in `otto-server` (the composition root / `modules.rs`), give the notifier a subscription to the same `broadcast::Receiver<Event>` the WS uses, filtered to `Improvement*` variants, and resolve target adapters by `workspace_id`.
3. `crates/otto-core/src/event.rs` — no change needed (events already carry `workspace_id`, target/skill name, applied/queued counts). Confirm the payload has enough to render `<target>`/`<name>`/`<n>`; if not, enrich the `ImprovementEditApplied` payload.
4. Settings/migration — a column/flag for `notify_self_improvement` (+ optional chat id) on the channel integration row; surface a toggle in the channels settings UI.
5. Docs — `docs/contracts/ws.md` already lists the improvement events; add the channel-notify behavior to the channels settings docs.

### Effort / risk
~1 focused agent. Risk low: additive, off-by-default, reuses existing events + adapters; no change to the improvement engine's safety. The only judgment call is target-chat resolution (per-integration default chat is the simplest correct MVP).

---

## Sources
github.com/nousresearch/hermes-agent + docs (hermes-agent.nousresearch.com), Curator v0.12 release notes, GitHub issues #15204 (background-agent scope) and #17583 (skill tiers / feedback-driven updates), and secondary write-ups (yuv.ai, mranand.substack, mindstudio, glukhov.org, meshworld). Otto: `crates/otto-improve`, `crates/otto-core/src/event.rs`, `crates/otto-channels`.
