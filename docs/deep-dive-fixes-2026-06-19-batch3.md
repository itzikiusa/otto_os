# Deep-Dive Fixes — Batch 3 (2026-06-19)

Driven by user review after batch 2. Addresses the "genuinely most important next" reliability/guardrail items below the critical line, plus the S6 defense-in-depth tail. Run as parallel Opus agents (disjoint files) → verifier → orchestrator final pass. Reports under `docs/fixes-2026-06-19/batch3-*.md`.

## Dispositions decided up front
- **D7 (workflow crash recovery) — ALREADY DONE in batch 1**, verified still present: `reap_orphaned_runs` (workflow_engine.rs:40) called at startup (main.rs:274) marks orphaned `pending`/`running` runs `error`; 30-min `RUN_WALL_CLOCK_TIMEOUT` (workflow_engine.rs:31) enforced in the run loop (:168) and applied (:245). Not re-done.
- **S1 (DNS-rebinding TOCTOU) — DEFERRED** per user ("Advanced; fine to defer"). The current SSRF guard validates resolved IPs + re-checks each redirect hop; full closure (resolve-once-and-pin-the-dial-IP) is a later hardening.

## Tasks
- [ ] **D3 — Swarm spend/run/time budget** (`b3-d3-swarm-budget`): add `max_total_runs` / `max_runtime_secs` / `max_cost_usd` (swarm config + migration 0032) enforced in `tick` (stop + pause with reason on exceed), and a per-task `max_attempts` ceiling so `route_result` stops re-queuing `in_progress`/unknown forever (→ `blocked` + notify after N).
- [ ] **Reliability — HTTP timeouts + Slack upload** (`b3-reliability`): connect+request timeouts on every Jira/Confluence/Slack/Telegram `reqwest::Client` (long-poll sized appropriately); replace deprecated Slack `files.upload` with `getUploadURLExternal`+`completeUploadExternal` (raw bytes); fix Telegram `from_utf8_lossy` binary corruption.
- [ ] **DB query cancel (server-side)** (`b3-db-cancel`): query-id/registry + `Driver::cancel` per engine (MySQL `KILL QUERY`, ClickHouse `KILL QUERY WHERE query_id=`, Mongo `killOp`), a cancel REST endpoint (Editor), and `abortQuery` wired to it; api.md updated.
- [ ] **D4 — Terminal scrollback on reattach** (`b3-d4-scrollback`): include scrollback history (honoring the client `lines` request) in the reattach snapshot so history above the viewport survives a reconnect, without double-rendering the visible screen.
- [ ] **D5 — TiledView terminal cap** (`b3-d5-tiledview`): stop resurrecting every suspended session — lazy-mount visible tiles (IntersectionObserver) or a live-tile cap with an explicit attach affordance; tear-down closes the WS to reclaim memory.
- [ ] **S6 secondary — atomic memory/skill apply** (`b3-s6-atomic`): make auto-apply atomic (tmp+rename) + conflict-checked (compare to `before_content`, queue on mismatch) + backup + canonicalize. Defense-in-depth + crash-safety; the batch-2 injection gate is unchanged.

Verifier + orchestrator final build/clippy/test/UI verification after all land.
