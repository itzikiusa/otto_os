# Sessions that survive a deploy — design note

Status: proposal (2026-10-05). First step shipped: `packaging/deploy.sh` counts
the live sessions a restart will end and asks before it starts (`--yes` skips).

## Problem

Each deploy restarts `ottod`. On shutdown, `SessionManager::shutdown_for_restart`
detaches only sessions that run in a **PTY holder**, a separate `ottod pty-holder`
process that the next daemon picks up again. `spawn_session_pty` puts a session
in a holder only when `is_user_started(session)` is true. That means a
foreground agent with `meta.work.origin` `"manual"` or absent
(`crates/otto-sessions/src/manager.rs`, `is_user_started` and
`spawn_session_pty`). Everything else is killed:

- **Shell terminals.** `is_foreground_agent()` is false for `kind = shell`.
- **Engine-owned agents.** These are workflow steps, swarm role agents,
  scheduled tasks, reviews, delegations and channel sessions. Their engine
  awaits their output in-process.

A deploy in the middle of a workflow run therefore fails the step, and an open
terminal loses its shell, history and running process.

## Proposal

1. **Shells first. Low risk, high value.** Spawn `kind = shell` sessions
   (excluding connection terminals, which own an SSH or DB channel) through
   holders when `session_persistence` is on. A shell has no engine waiting on
   it, so the existing re-adoption path (re-attach the PTY and replay the
   holder's scrollback ring) is all it needs. The change is in
   `spawn_session_pty`: replace the `is_user_started` check with a
   `holder_eligible(session)` that is true for user-started agents **or** plain
   shells.
2. **Engine-owned agents. Needs an engine contract.** A holder keeps the
   process alive, but the engine's in-memory await (the workflow step future
   and swarm coordinator) dies with the old daemon. Each engine needs:
   - a durable "waiting on session X" record. Workflow steps already persist
     `nodes_json` activity, and swarm runs persist their board.
   - a boot reconciler that re-adopts the held session and re-arms the wait
     through the existing transcript tail or `/sessions/{id}/wait` instead of
     failing or re-running the step. `workflow_engine` already re-enqueues
     queued runs at boot. Extend it to running steps whose session was
     re-adopted.
   Ship it engine by engine (workflow, then scheduled tasks, then swarm), each
   behind the holder-eligibility switch, with an integration test that restarts
   the daemon mid-step.
3. **Deploy UX.** Once (1) lands, the deploy prompt can say how many sessions
   will be *terminated* rather than how many are live. To do that, add a
   read-only `held: bool` to the session row (or `/sessions?held=`) so
   `deploy.sh` can subtract the sessions that survive.

## Limits and risks

- Holders cost one small process per session. Cap the number of held sessions
  and fall back to in-process spawn, which is logged, as today.
- A holder outlives its daemon by design. A daemon that never comes back
  leaves orphans, so the existing holder idle timeout must cover shells too.
- Changing which sessions survive changes the idle sweep's assumptions.
  `is_user_started` is also its guard, so split the two predicates instead of
  widening that one.
