# Iteration 07 — access (credential class)

Static pass. No Cargo. No behavior change.

| Lens | Score | This pass |
|---|---:|---|
| L17 messaging | 9.7 | Not this pass. |
| L18 workflow cwd | 8.7 | Credential class at the manual route, distinct from the path predicate. |
| L19 native grants | 8.3 | Not this pass. |
| L20 authorization compatibility | 9.4 | Share, MCP-only, and agent-write withholding stay as they are. |

## Case: root agent token

`run_workflow` reads the user row's `is_root` and `ui_bridge::is_human`. `is_human` is false when any of `managed_session_id`, `mcp_session_id`, `mcp_only`, or `scope` is set (`crates/otto-server/src/ui_bridge.rs`).

`POST /api/v1/workflows/{id}/run` is `Require(Workflows, Edit)`, not `RouteClass::Outward` (`crates/otto-server/src/policy.rs`). An agent write on that template also gets `root_withheld`. The host-folder flag does not consult `root_authority`. It fails closed from `is_human` even if a later change stopped withholding root on this route. The passing agent-token test is the executed evidence for this case. It was not re-run here.

## Case: share link of the root user

`otto-rbac` share authentication clones the owner and sets `is_root = false` (`crates/otto-rbac/src/tokens.rs`). The token also carries `scope`, so `is_human` is false. The feature guard's scope branch allows only the pinned session read and, for an Editor share, input to that session. `POST /workflows/{id}/run` is not on that list. A leaked share does not reach the location check and does not regain root.

## Case: MCP-only token

`mcp_only` is denied by the feature guard before `run_workflow`, except the MCP invoke and status routes. `is_human` is also false. An external MCP token of the root user cannot post a host directory to the workflow run route.

## Not changed

Workspace Admin remains a workspace role. It does not set the host-folder flag. No route was added to the agent write allow-list.
