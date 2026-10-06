# Sessions that survive a deploy — design note

Status: proposal (2026-10-05, corrected 2026-10-06). First step shipped:
`packaging/deploy.sh` counts the live sessions a restart will end and asks
before it starts (`--yes` skips).

## Problem

Each deploy restarts `ottod`. On shutdown, `SessionManager::shutdown_for_restart`
detaches only sessions that run in a **PTY holder**, a separate `ottod pty-holder`
process that the next daemon picks up again. `spawn_session_pty` puts a session
in a holder only when `is_user_started(session)` is true
(`crates/otto-sessions/src/manager.rs`): `Session::is_foreground_agent()` —
`kind = agent` and `meta.source` not in `BACKGROUND_SESSION_SOURCES`
(`crates/otto-core/src/domain.rs`) — and `meta.work.origin` `"manual"` or
absent.

`SessionKind` is only `agent | connection`. A **shell terminal is
`kind = agent, provider = shell`** with no `meta.source` and no
`meta.work.origin`, so it already passes `is_user_started` and already
survives a deploy in a holder (with `session_persistence` on, the default).
What is killed:

- **Connection terminals** (`kind = connection`: SSH, DB shells, k8s exec).
  `is_foreground_agent()` is false for them, and each owns a live channel
  (an SSH session, a DB connection, a kubectl exec stream) that a holder
  could not keep meaningful across a restart anyway.
- **Engine-owned agents.** Workflow steps, swarm role agents, scheduled
  tasks, reviews, delegations and channel sessions — a background
  `meta.source` or a non-manual `meta.work.origin`. Their engine awaits their
  output in-process.

A deploy in the middle of a workflow run therefore fails the step, and an open
SSH/DB/k8s terminal loses its channel.

## Proposal

1. **Connection terminals: reconnect, don't hold.** Shells need no change —
   they are already holder-eligible. For connection terminals the useful
   behaviour is a clean, visible end plus a one-click reconnect (the
   connection profile is persisted), not a holder: the remote side of the
   channel does not survive the daemon either way. Keep `is_user_started`
   as the holder predicate.
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
3. **Deploy UX.** Shipped: the deploy prompt classifies live sessions with
   the same predicate (`classify_live_sessions` in `packaging/deploy.sh`;
   its background-source list is pinned to `BACKGROUND_SESSION_SOURCES` by
   `packaging/tests/test_deploy.py`) and reports survivors (user agents and
   shells in holders) separately from what is terminated (connection
   terminals, background/workflow agents). It assumes persistence is on; a
   read-only `held: bool` on the session row would make the count exact when
   a user has turned `session_persistence` off or a holder spawn fell back
   in-process.

## Limits and risks

- Holders cost one small process per session. Cap the number of held sessions
  and fall back to in-process spawn, which is logged, as today.
- A holder outlives its daemon by design. A daemon that never comes back
  leaves orphans; the existing holder idle timeout covers them (shells
  included — they are already held).
- Changing which sessions survive changes the idle sweep's assumptions.
  `is_user_started` is also its guard, so split the two predicates instead of
  widening that one.
