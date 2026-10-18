# Iteration 09 — access (who may message a session)

Static pass. No Cargo. Messaging behavior was not changed.

| Lens | Score | This pass |
|---|---:|---|
| L17 messaging | 9.7 | Owner, workspace Admin, and agent-credential rules traced on the live send path. |
| L18 workflow cwd | 8.7 | Not this pass. |
| L19 native grants | 8.3 | Not this pass. |
| L20 authorization compatibility | 9.4 | Cross-user send stays behind owner-or-admin. |

## Case: a person messages their own agent

`POST /sessions/{id}/message` loads the session through `input_session`. `input_user` reloads the user, requires workspace Editor or Admin, then allows the send when the user is root, the session's `created_by`, or a workspace Admin. `agent_message_rule(None, …)` allows a person's credential. The text is recorded as a user message.

Root's iteration-03 note says authenticated messages to both named sibling sessions were delivered in that task. This pass did not repeat that run.

## Case: an agent credential messages a shell sibling

`agent_message_rule` (`crates/otto-sessions/src/http.rs`) allows another agent-kind session only when `provider != "shell"`. A shell or SSH target falls through to `agent_input_rule`, which allows the caller's own session or a worker it opened. A shell sibling stays forbidden. The personal-agent broadcast test still expects that 403. `agent_message_rule` was not tightened, so a non-shell sibling agent remains reachable.

## Case: an agent credential messages another user's agent

The rule itself returns success for any non-shell agent target when the caller has a session id. `input_user` still runs first. A non-root caller who is not `created_by` and not a workspace Admin is refused with "not the session owner or a workspace admin". Workspace Admin in that session's workspace can send; that is the existing membership role, not a new cross-user hole. Root is Admin of every workspace through `role_of`.

## Not changed

No new refusal for root, for a person messaging their own shell-provider session, or for sibling agent CLIs.
