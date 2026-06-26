# MCP Control Plane — Implementation Plan

Ordered, independently-verifiable tasks. Each: **Goal**, **Files**, **Verify**. Backend is
built bottom-up by the lead (security-critical); UI + E2E are delegated once the API contract
freezes. Design source of truth: `mcp-control-plane-design.md` (incl. §14 resolutions).

## Phase A — State layer (`otto-state`) + restricted token (`otto-rbac`)

### Task A1: Migration `0077_mcp_control_plane.sql`
- **Goal:** Additive ALTERs on `mcp_servers` (transport,url,description,headers_json,secret_ref,
  injection_risk,managed,default_tool_access,health_*,tools_count,tools_discovered_at) + new
  tables `mcp_tools`, `mcp_allowlist`, `mcp_policies`, `mcp_call_log`, `mcp_approvals` (with
  `args_hash`,`consumed_at` per F2) + indexes.
- **Verify:** `cargo test -p otto-state` (existing migration-run tests pass with the new file;
  legacy `mcp_servers` tests still green).

### Task A2: Repos
- **Goal:** `McpRegistryRepo` (rich server CRUD over augmented cols + new `McpServerDetail`
  type), `McpToolsRepo` (upsert/list/patch discovered tools), `McpAllowlistRepo`,
  `McpPolicyRepo`, `McpCallLogRepo` (insert + filtered list + per-tool stats agg),
  `McpApprovalRepo` (create/list/get/decide/consume with args_hash binding). Extend
  `McpAuditRepo` to also write `mcp_call_log` for inward calls.
- **Files:** `crates/otto-state/src/mcp_registry.rs`, `mcp_tools_catalog.rs`, `mcp_allowlist.rs`,
  `mcp_policies.rs`, `mcp_call_log.rs`, `mcp_approvals.rs`; export in `lib.rs`.
- **Verify:** `cargo test -p otto-state` — unit tests per repo (in-memory pool).

### Task A3: Restricted `mcp` token kind (`otto-rbac`)
- **Goal:** `issue_mcp_token(user_id,label)` (kind='mcp'); `authenticate` returns a scope marker
  `TokenScope::Mcp`; helper `scope_allows(scope, method, path)` permitting only
  `POST /mcp/otto-tools/invoke`, `GET /mcp/otto-server`, `POST /mcp/gateway/invoke`,
  `GET /mcp/gateway/tools`. (No new migration — reuse `auth_sessions.kind`.)
- **Verify:** `cargo test -p otto-rbac` — token round-trip + scope allow/deny table.

## Phase B — `otto-mcp` crate (client + governance)

### Task B1: Crate scaffold + types
- **Goal:** New crate, workspace-wired (root Cargo.toml members+deps). `types.rs`: wire DTOs
  (CreateServerReq incl. transport/url/secret_env/secret_headers, ServerDetail, ToolView,
  AllowlistEntry, Policy, ApprovalView, CallLogRow, ToolStats, InvokeReq/Resp, decision enums).
- **Verify:** `cargo build -p otto-mcp`.

### Task B2: Outbound MCP client (`client.rs`)
- **Goal:** `McpClient` with stdio (spawn cmd/args/env+secret_env, JSON-RPC line framing) and
  http (Streamable-HTTP: initialize→capture Mcp-Session-Id→op; pinned-IP via netguard;
  redirect_policy; SSE/json negotiation). Methods: `initialize`, `list_tools`, `call_tool`,
  `ping`. Caps: 20s/1MiB/500-row; `redact_json` results.
- **Verify:** `cargo test -p otto-mcp` — stdio client drives an in-test mock server (a Rust
  fixture binary or piped child) for initialize/list/call; http request-builder unit test;
  netguard pin unit test.

### Task B3: Risk labeling + policy engine (`risk.rs`, `policy.rs`)
- **Goal:** `label_tool(annotations, name, desc) -> (risk_label, injection_risk, mutating,
  supports_dry_run)` per §6 heuristics. `evaluate(rules, ctx) -> Effect` most-restrictive-wins
  (F3), matcher grammar per §14.
- **Verify:** `cargo test -p otto-mcp` — labeling table (read/write/dangerous, openWorldHint→
  high); policy: deny-global-override, require_approval, dry_run, allow, glob, min_injection.

### Task B4: `McpService` pipeline (`service.rs`)
- **Goal:** registry/discover/health ops + `invoke(ctx)` = allowlist(deny-first)→enabled→policy→
  risk/approval gate(args_hash, single-use, approver≠requester, expiry)→dry-run(pure sim)→
  execute(client)→**guaranteed** audit(every terminal path, fail-closed)→stats source.
  Secret resolution via `SecretStore`. Background health sweep handle.
- **Verify:** `cargo test -p otto-mcp` — pipeline decision tests (deny/enabled-off/approval-
  pending/approved-consume/replay-rejected/args-swap-rejected/dry-run-no-exec/audit-on-deny).

## Phase C — `otto-server` wiring + outward server + gateway + capabilities

### Task C1: RBAC `Feature::Mcp`
- **Goal:** add `Mcp` to `Feature` enum (parse/as_str), `ALL_FEATURES` (grants.rs). UI feature
  `mcp` flows through capabilities automatically.
- **Verify:** `cargo test -p otto-core` + `-p otto-server`.

### Task C2: `policy.rs` rules + coverage test
- **Goal:** rules for `/mcp/...` and `/workspaces/{wid}/mcp/...` per §9+§14 (View reads/previews,
  Edit mutations/invoke, Admin for otto-server PATCH + policies writes/import + decide).
  stdio-Admin enforced in-handler (F10). Add coverage-test asserts; default-Deny preserved.
- **Verify:** `cargo test -p otto-server policy` — new asserts pass, unknown route still Deny.

### Task C3: `McpCtx` + ServerCtx field + router registration
- **Goal:** `McpCtx` trait; `impl McpCtx for ServerCtx`; `pub mcp: Arc<McpService>` in ServerCtx
  + construction; register `otto_mcp::api_router::<ServerCtx>()` in `module_routers`. Start the
  health sweep at boot.
- **Verify:** `cargo build -p ottod`; daemon boots; `GET /api/v1/mcp/stats` → 401 not 404.

### Task C4: Control-plane routes (`otto-mcp/src/http.rs`)
- **Goal:** all §8 endpoints with per-workspace authz on flat by-id routes (F13), stdio-Admin
  (F10), secret masking, WS events (`McpApprovalPending`, `McpServerHealth`).
- **Verify:** route smoke (401-not-404 for each), RBAC unit where feasible.

### Task C5: New capability endpoints (injection-safe)
- **Goal:** `GET /workspaces/{wid}/mcp/code-search` (pure-Rust, root-confined, redacted, capped),
  `POST /workspaces/{wid}/mcp/context-packet` (workspace+memory+optional story), `GET
  /workspaces/{wid}/mcp/proof-pack` (otto-git status/commits/diffstat + PR review + goal-loop
  acceptance verify).
- **Verify:** unit tests: path-traversal rejected; pattern not parsed as flags; redaction.

### Task C6: Otto-as-MCP-server (`ottod mcp-server`) + governed invoke
- **Goal:** new subcommand: PAT(`mcp` kind)-auth stdio bridge; 8-tool catalog filtered by
  `mcp_otto_server_tools`; `tools/call`→`POST /mcp/otto-tools/invoke`. The invoke **handler**:
  governance (enabled/allowlist/policy/approval) + per-tool RBAC re-check + execute (direct
  service calls; `query_db_readonly` statement-classified F5; PR-draft verified no publish F7)
  + audit. `GET/PATCH /mcp/otto-server` (status/enable/tools/mint-rotate `mcp` token; Admin).
- **Verify:** integration: drive `ottod mcp-server` over stdio → initialize/tools/list/tools/call
  for a read tool; restricted-token reaches only invoke (403 elsewhere).

### Task C7: Gateway (live-agent governance)
- **Goal:** `GET /mcp/gateway/tools` + `POST /mcp/gateway/invoke` (run pipeline). Extend `ottod
  mcp-tools` to list+proxy governed downstream tools when `mcp_gateway_enabled`. Update
  `otto-sessions::mcp` merge + `DbMcpServerProvider` (resolve secret_env; skip governed servers
  when gateway on).
- **Verify:** integration: drive `ottod mcp-tools` stdio with a gateway tool → call audited +
  policy-gated; legacy merge unchanged when gateway off.

## Phase D — UI (`ui/src/modules/mcp`) [delegate after C contract freeze]
Tabs: Servers, Tools, Allowlists, Policies, Approvals, Audit, Stats, Otto Server. Sidebar entry
`{id:'mcp',icon:'plug',label:'MCP Control Plane',feature:'mcp'}`; `App.svelte` route; extend
`api/mcp.ts` + `api/types.ts`. **Verify:** `npm run check` + `npm run build`.

## Phase E — E2E [delegate after D]
`ui/e2e/fixtures/mock-mcp-server.mjs` (Node stdio MCP server: initialize/tools/list/tools/call,
incl. a `delete_*` dangerous tool + a read tool). `ui/e2e/mcp.spec.ts`: register→discover(risk
labels)→health→per-tool perm→invoke dry-run→invoke real→audit row→stats→dangerous→approval→
approve→re-invoke→policy deny blocks. **Verify:** `npm run test:e2e` (mcp spec green).

## Phase F — Verify + merge + deploy
`cargo build/test --workspace`, `clippy -D`, `npm run check/build/test:e2e`; commit on branch;
merge → local main (no push); rebuild+sign+reinstall app, restart ottod.

---

## Requirements traceability (final-verification checklist)

| Req | Tasks | Test |
|---|---|---|
| 1 registry | A1,A2,C4 | E2E register; repo tests |
| 2 health | A1,B2,B4,C4 | E2E health→healthy; ping unit |
| 3 discovery | B2,B4,C4 | E2E discover lists tools; mock list_tools |
| 4 per-tool perms | A2,B4,C4 | pipeline enabled-off denies; E2E toggle |
| 5 ws allowlists | A2,B4,C4 | pipeline allowlist deny-first; E2E |
| 6 secret isolation | A1,A2,B2,B4,C4,C7 | secret_env→keychain, masked resp; merge overlay |
| 7 injection labels | B3,C4 | labeling table; E2E badges |
| 8 audit every call | A2,B4,C6,C7 | audit-on-every-path; fail-closed; E2E audit row |
| 9 approval | A2,B4,C4,C6 | args_hash/single-use/approver≠requester; E2E approve flow |
| 10 dry-run | B4,C4 | pure-sim no-exec test; E2E dry-run preview |
| 11 policy-as-code | A2,B3,B4,C4 | most-restrictive-wins; import/export; E2E deny |
| 12 stats | A2,B4,C4 | per-tool agg; E2E stats populate (latency/error/bytes; cost PARTIAL) |
| otto.search_codebase | C5,C6 | path-confine + flag-safe unit; tool call |
| otto.get_context_packet | C5,C6 | assembler unit; tool call |
| otto.run_goal_loop | C6 | RBAC re-check; create+start; approval-gated |
| otto.create_work_item | C6 | RBAC re-check; story/task created |
| otto.query_db_readonly | C6 | statement_is_write rejects DROP (F5) |
| otto.open_pr_draft | C6 | drafts only, no publish (F7) |
| otto.get_proof_pack | C5,C6 | evidence bundle assembled |
| otto.ask_human_approval | A2,C6 | creates approval, wait-capped, returns decision |
| F1 token scope | A3,C6 | mcp token 403 outside invoke |
| F13 IDOR | C4 | cross-ws by-id denied |
| F10 stdio Admin | C4 | Edit-user stdio create denied |

**Plan self-review:** every one of the 13 control-plane requirements + 8 outward tools + every
review blocker (F1,F2,F5,F10,F12 / arch B1,B2,B3,B4,M1,M2) maps to ≥1 task with a concrete test.
No requirement is unaddressed. Sequencing respects deps (A→B→C→D→E). Security-critical pieces
(token scope, approval integrity, statement classification, stdio gating) are lead-built, not
delegated.
