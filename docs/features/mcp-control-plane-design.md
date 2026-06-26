# MCP Control Plane — Design

> Status: design (for review). Author: autonomous build. Target: `otto-mcp` crate +
> `otto-server` wiring + `ottod` outward MCP server + `ui/src/modules/mcp`.

## 1. Goal & framing

Otto already *supports* MCP in the thinnest possible way: it stores per-workspace MCP
server **config** (`mcp_servers`, migration 0036) and merges the *enabled* ones into a
workspace's `.mcp.json` so the agent CLI (claude/codex) launches them directly. Otto is
**not in the call path** — it cannot see, govern, health-check, or meter those servers. It
also exposes a tiny **read-only, per-session** MCP server (`ottod mcp-tools`, 5 GET-only
tools) to its *own* agents.

This feature turns that thin layer into a real **control plane**: Otto becomes the governed
path between agents and MCP tools, and additionally exposes itself **outward** as an MCP
server so external agents can use Otto capabilities. Two halves:

- **A. Control plane** for MCP servers/tools (the 13 requirements): registry, health,
  discovery, per-tool permissions, per-workspace allowlists, secret isolation, prompt-
  injection risk labels, audit of every call, approval before dangerous actions, dry-run,
  policy-as-code, and cost/latency/error stats.
- **B. Otto-as-MCP-server**: 8 `otto.*` tools (`search_codebase`, `get_context_packet`,
  `run_goal_loop`, `create_work_item`, `query_db_readonly`, `open_pr_draft`,
  `get_proof_pack`, `ask_human_approval`) served to external agents over stdio, PAT-
  authenticated, and themselves governed by the same control plane.

### Key design decisions

1. **Otto becomes an outbound MCP client.** A new `otto-mcp` crate connects *out* to each
   registered server (stdio: spawn `command/args/env`; remote: HTTP/Streamable-HTTP to a
   `url`), performing `initialize`/`tools/list` (discovery), `ping` (health), and
   `tools/call` (invoke). This is what makes the plane *in the path*.
2. **One governance pipeline (`invoke`)** sits in front of every `tools/call`. It is the
   single choke point: allowlist → policy → per-tool permission → risk gate → approval →
   dry-run → execute → audit → stats. The UI tool-tester, the live agent **gateway**, and
   the outward `otto.*` tools all funnel through it (or its sibling for otto-tools).
3. **Reuse, don't fork, the existing data.** `mcp_servers` is *augmented* (ALTER, additive
   columns) so the legacy `.mcp.json` merge keeps working unchanged; new governance lives in
   new tables. `McpServersRepo` does `SELECT *` and maps named columns, so added columns are
   safe.
4. **Two MCP servers, clearly separated.** `ottod mcp-tools` (existing) stays read-only,
   per-session, inward. The new `ottod mcp-server` is the **outward** server: PAT-auth, the 8
   `otto.*` tools (some mutating), governed + audited. Different trust models, different
   binaries-modes.
5. **Live-agent governance via a gateway.** When a workspace opts into governed MCP, Otto
   injects only the `otto` server into `.mcp.json`; that server surfaces the registered
   downstream tools (namespaced `mcp__<server>__<tool>`) and proxies each call through the
   `invoke` pipeline. The agent's every downstream MCP call is then audited/policed. (Phase
   B2 — see scope.)
6. **Fail closed.** Unknown routes Deny (RBAC). Unknown/unlabeled tool risk defaults to the
   stricter side. A policy `deny` always beats an `allow` at equal priority. Secrets never
   serialize.

## 2. Architecture

```
                ┌──────────────────────── ottod (daemon) ─────────────────────────┐
 external agent │  otto-server (Axum)                                              │
 (Claude/Copilot│    /mcp/* , /workspaces/{ws}/mcp/*   ── RBAC: Feature::Mcp       │
   elsewhere)   │        │                                                         │
      │ stdio   │        ▼                                                         │
      ▼         │   otto-mcp::McpService ── governance pipeline (invoke)           │
 ottod          │        │        │         allowlist→policy→perm→risk→approval    │
 mcp-server ───▶│  /mcp/otto-tools/invoke   →dry-run→execute→audit→stats           │
 (8 otto.* )    │        │        │                                                │
 PAT auth       │        ▼        ▼                                                │
                │  outbound MCP client          SQLite (0077):                     │
                │   stdio child / HTTP          mcp_servers(+cols) mcp_tools       │
                │     │                         mcp_allowlist mcp_policies          │
                └─────┼──────────────────────── mcp_call_log mcp_approvals ────────┘
                      ▼
        registered downstream MCP servers (Linear, GitHub, DB, web-fetch, …)

 Otto's own agents (inward, unchanged): .mcp.json → ottod mcp-tools (read-only, per-session)
   (+ optional gateway: downstream tools proxied through invoke — Phase B2)
```

## 3. Data model — migration `0077_mcp_control_plane.sql`

Additive ALTERs on `mcp_servers` (legacy merge path untouched):

```sql
ALTER TABLE mcp_servers ADD COLUMN transport        TEXT NOT NULL DEFAULT 'stdio'; -- 'stdio'|'http'
ALTER TABLE mcp_servers ADD COLUMN url              TEXT;            -- http transport endpoint
ALTER TABLE mcp_servers ADD COLUMN description      TEXT;
ALTER TABLE mcp_servers ADD COLUMN headers_json     TEXT NOT NULL DEFAULT '{}'; -- http auth headers (values may be secret-masked)
ALTER TABLE mcp_servers ADD COLUMN secret_ref       TEXT;            -- keychain ref 'mcp-{id}' for sensitive env/headers
ALTER TABLE mcp_servers ADD COLUMN injection_risk   TEXT NOT NULL DEFAULT 'medium'; -- server default label
ALTER TABLE mcp_servers ADD COLUMN managed          INTEGER NOT NULL DEFAULT 1;     -- governed by the control plane
ALTER TABLE mcp_servers ADD COLUMN default_tool_access TEXT NOT NULL DEFAULT 'allow'; -- 'allow'|'deny' posture
ALTER TABLE mcp_servers ADD COLUMN health_status    TEXT NOT NULL DEFAULT 'unknown'; -- unknown|healthy|unhealthy|disabled
ALTER TABLE mcp_servers ADD COLUMN health_checked_at TEXT;
ALTER TABLE mcp_servers ADD COLUMN health_latency_ms INTEGER;
ALTER TABLE mcp_servers ADD COLUMN health_error     TEXT;
ALTER TABLE mcp_servers ADD COLUMN tools_count      INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mcp_servers ADD COLUMN tools_discovered_at TEXT;
```

New tables (full DDL in the migration):

- **`mcp_tools`** — discovered catalog + per-tool governance. Cols: `id, server_id(FK),
  name, title, description, input_schema_json, annotations_json, risk_label
  ('read'|'write'|'dangerous'|'unknown'), injection_risk ('low'|'medium'|'high'),
  mutating, supports_dry_run, enabled (per-tool permission), require_approval,
  risk_overridden (human pinned — survives rediscovery), created_at, updated_at`;
  `UNIQUE(server_id, name)`.
- **`mcp_allowlist`** — per-workspace allow/deny. Cols: `id, workspace_id(FK),
  server_id(FK), tool_name (NULL = whole server), mode ('allow'|'deny'), created_by,
  created_at`; `UNIQUE(workspace_id, server_id, tool_name)`.
- **`mcp_policies`** — policy-as-code rules. Cols: `id, workspace_id (NULL=global), name,
  enabled, priority (lower first), match_json, effect
  ('allow'|'deny'|'require_approval'|'require_dry_run'), reason, created_by, created_at,
  updated_at`. The full ruleset is importable/exportable as one JSON document.
- **`mcp_call_log`** — append-only audit of every governed call + the source of stats.
  Cols: `id, workspace_id, server_id, server_name, tool, direction
  ('outbound'|'inbound'), caller_user_id, caller_kind, args_redacted_json, decision
  ('allowed'|'denied'|'approved'|'dry_run'|'pending_approval'|'error'), decision_reason,
  risk_label, injection_risk, dry_run, ok, error, latency_ms, bytes, rows, approval_id,
  created_at`. Indexed on (tool, created_at), (workspace_id, created_at), (server_id).
- **`mcp_approvals`** — the approval queue, shared by *dangerous-action gating* and
  *`otto.ask_human_approval`*. Cols: `id, workspace_id, kind ('tool_call'|'human_ask'),
  server_id, server_name, tool, title, detail, args_redacted_json, risk_label, status
  ('pending'|'approved'|'denied'|'expired'|'cancelled'), requested_by, requested_by_kind,
  decided_by, decision_note, created_at, decided_at, expires_at`. Indexed on (status,
  created_at), (workspace_id, status).

Settings keys (`SettingsRepo`): `mcp_health_interval_secs` (default 300; 0=off),
`mcp_otto_server_enabled` (outward server opt-in; default false), `mcp_otto_server_tools`
(JSON array of enabled `otto.*` names; default = read-only subset:
`search_codebase, get_context_packet, query_db_readonly, get_proof_pack, ask_human_approval`),
`mcp_require_approval_dangerous` (default true).

## 4. Outbound MCP client (`otto-mcp/src/client.rs`)

A transport-abstracted client speaking newline-delimited JSON-RPC 2.0 (mirrors
`mcp_tools.rs` framing on the *client* side):

- **stdio**: spawn `command` with `args`+`env` (+ secret env from keychain), pipe stdin/stdout,
  send `initialize` → `notifications/initialized` → operation → drop (process exits on stdin
  close). One short-lived child **per operation** (discovery/health/invoke) keeps it stateless
  and robust; discovery results are cached in `mcp_tools`. (Connection pooling = deferred.)
- **http (Streamable HTTP)**: `POST url` JSON-RPC with `Accept: application/json,
  text/event-stream`; parse a JSON body, or read SSE `data:` lines for a streamed result.
  **`otto_netguard::check_url(url)` guards every remote URL** (blocks loopback/private/metadata
  — SSRF). Auth headers (incl. secret-ref-resolved values) attached per call.
- **Caps** (reuse the inward server's constants): 20s timeout, 1 MiB body cap, 500-row cap,
  `redact_json` on every result. Methods: `initialize`, `list_tools`, `call_tool`, `ping`.

## 5. Governance pipeline (`McpService::invoke`)

`invoke(ctx, server_id, tool, args, { workspace_id, dry_run, caller })`:

1. **Resolve** server (must exist, `enabled`, `managed`) + tool (from `mcp_tools`).
2. **Allowlist** (per-workspace): evaluate `mcp_allowlist` for (ws, server, tool) — a `deny`
   match wins; else `allow` match; else fall back to `server.default_tool_access`. Blocked →
   `denied`.
3. **Per-tool permission**: `mcp_tools.enabled` must be true. Off → `denied`.
4. **Policy** (policy-as-code): evaluate `mcp_policies` (global + ws) by ascending priority;
   first matching rule's effect applies, `deny` beats `allow` at a tie. Effects: `deny`,
   `require_approval`, `require_dry_run`, `allow`.
5. **Risk gate**: if tool `risk_label='dangerous'` (or policy/per-tool `require_approval`, or
   the `mcp_require_approval_dangerous` setting) and not already approved → create an
   `mcp_approvals(kind='tool_call')`, audit `pending_approval`, return *pending* (the caller
   polls / the UI shows the queue). An approved call carries an `approval_id` that satisfies
   this gate.
6. **Dry-run**: if `dry_run` requested (or policy `require_dry_run`): do **not** execute the
   real call. If the tool advertises a dry-run/validate affordance, call that; otherwise
   return a **simulated preview** (validated args against `input_schema`, the resolved
   target, and a "would call X on server Y" description). Audited as `dry_run`.
7. **Execute** via the outbound client; measure latency + response bytes.
8. **Audit** one `mcp_call_log` row (redacted args, decision, ok/err, latency, bytes, rows).
   **Stats** are derived from this table — no separate counter to drift.

## 6. The 13 control-plane requirements → design

| # | Requirement | Design element |
|---|---|---|
| 1 | **MCP server registry** | `mcp_servers` (+cols, stdio & http transports) + `/workspaces/{ws}/mcp/servers` CRUD; legacy `/mcp-servers` kept |
| 2 | **Health checks** | `ping`/`initialize` probe → `health_*` cols; `POST /mcp/servers/{id}/health` on-demand + background sweep every `mcp_health_interval_secs` |
| 3 | **Tool discovery** | `initialize`+`tools/list` → upsert `mcp_tools` (schema, annotations); `POST /mcp/servers/{id}/discover`; auto on create/enable |
| 4 | **Per-tool permissions** | `mcp_tools.enabled` + `require_approval`; `PATCH /mcp/tools/{id}`; enforced in pipeline step 3/5 |
| 5 | **Per-workspace allowlists** | `mcp_allowlist` (allow/deny, server- or tool-scoped) + `default_tool_access`; `GET/PUT /workspaces/{ws}/mcp/allowlist`; pipeline step 2 |
| 6 | **Secret isolation** | `SecretStore` (Keychain) ref `mcp-{id}`; sensitive env/header values stored in keychain, DB keeps refs; never serialized (exposed as `has_secret`/masked) |
| 7 | **Prompt-injection risk labels** | `mcp_tools.injection_risk` (low/med/high) + `mcp_servers.injection_risk`; auto-labeled from MCP `annotations.openWorldHint` + name/desc heuristics; human override (`risk_overridden`) |
| 8 | **Audit log of every MCP call** | `mcp_call_log` — every `invoke` (UI/gateway/otto-tools) writes a redacted row; `GET /mcp/audit` (filters) |
| 9 | **Approval before dangerous actions** | `mcp_approvals` queue; pipeline step 5; `GET /mcp/approvals` + `POST /mcp/approvals/{id}/decide`; WS event `McpApprovalPending` |
| 10 | **Dry-run for mutating tools** | pipeline step 6; `invoke {dry_run:true}` and policy `require_dry_run`; preview without executing |
| 11 | **Policy-as-code for tools** | `mcp_policies` data rules (matchers→effect, priority); `GET/POST/PATCH/DELETE /mcp/policies`; `export`/`import` the full ruleset as one JSON doc; `POST /mcp/policies/evaluate` preview |
| 12 | **Cost/latency/error stats per tool** | derived from `mcp_call_log`; `GET /mcp/stats` → per-tool call_count, error_count, error_rate, p50/p95/avg latency, avg/total bytes, last_called_at |
| 13 | **(registry health/discovery/secrets…)** | — covered by 1–12 above |

### Risk-labeling heuristics (requirement 7)

From MCP `toolAnnotations` when present: `readOnlyHint=true`→`read`; `destructiveHint=true`
→`dangerous`; else if not read-only→`write`. `openWorldHint=true`→ injection_risk≥`high`
(tool reaches untrusted external content). Name/description keywords refine: `delete|drop|
remove|exec|run|deploy|write|send|email|payment`→`dangerous`/`write`; `fetch|browse|web|url|
http|search|read_url`→ injection_risk `medium`+. Unlabeled → `write` + `medium` (strict
default). A human override pins `risk_overridden=1` so rediscovery never lowers it.

## 7. Otto-as-MCP-server (8 `otto.*` tools)

New subcommand **`ottod mcp-server`** — a thin stdio↔HTTP bridge (shape of `mcp_tools.rs`):
auth via `OTTO_API_TOKEN` (a Settings-minted PAT, full user scope), `tools/list` = the static
8-tool catalog (filtered by `mcp_otto_server_tools`), `tools/call` →
`POST /api/v1/mcp/otto-tools/invoke {tool, arguments, wait_seconds?}` with the PAT. The single
daemon endpoint applies governance (enabled? allowlisted? policy? dangerous→approval?), audits
(`direction='inbound'`), then executes the capability and returns MCP content.

| Tool | Maps to | Mutating? | Notes |
|---|---|---|---|
| `otto.search_codebase` | **new** `GET /workspaces/{ws}/code-search?q=&path=&repo_id=&max=` | no | ripgrep/`git grep` over the workspace root (or a repo); returns file:line hits |
| `otto.get_context_packet` | **new** `POST /workspaces/{ws}/context-packet` | no | assembles workspace summary + `otto-memory` recall for a query + optional story `build_agent_context` |
| `otto.run_goal_loop` | existing create+start (`/workspaces/{ws}/goal-loops`, `/goal-loops/{id}/start`) | **yes (dangerous)** | composes create→start; approval-gated |
| `otto.create_work_item` | existing product-story or swarm-task create | **yes** | default = product story; approval per policy |
| `otto.query_db_readonly` | existing `POST /connections/{id}/db/query` `{confirm_write:false}` | no | read-only enforced by connection write-guard |
| `otto.open_pr_draft` | existing `POST /repos/{id}/pr/draft` | no | *drafts* title/desc from the diff; does **not** publish a PR |
| `otto.get_proof_pack` | **new** `GET /workspaces/{ws}/proof-pack?repo_id=&branch=&pr=&goal_loop_id=` | no | evidence bundle: git status/recent commits/diffstat + PR review state + goal-loop acceptance-criteria verification |
| `otto.ask_human_approval` | `mcp_approvals(kind='human_ask')` + poll | no (creates a request) | creates a pending approval, optionally waits `wait_seconds`, returns the decision or `pending`+id |

"Proof pack" is a **new** concept (none existed): a structured, redacted evidence bundle for
a target (branch/PR/goal-loop) used to *prove* a claim of done — commits, diffstat, review
verdicts, and machine-checked acceptance criteria.

## 8. API surface (otto-mcp `api_router`, under `/api/v1`)

```
# Registry + health + discovery
GET    /workspaces/{ws}/mcp/servers
POST   /workspaces/{ws}/mcp/servers
GET    /mcp/servers/{id}
PATCH  /mcp/servers/{id}
DELETE /mcp/servers/{id}
POST   /mcp/servers/{id}/health
POST   /mcp/servers/{id}/discover
# Tools + per-tool permission + the governance pipeline
GET    /mcp/servers/{id}/tools
PATCH  /mcp/tools/{tool_id}
POST   /mcp/servers/{id}/tools/{name}/invoke      # {arguments, dry_run?, workspace_id?}
# Allowlists
GET    /workspaces/{ws}/mcp/allowlist
PUT    /workspaces/{ws}/mcp/allowlist             # bulk set
# Policy-as-code
GET    /mcp/policies            POST /mcp/policies
PATCH  /mcp/policies/{id}       DELETE /mcp/policies/{id}
GET    /mcp/policies/export     POST /mcp/policies/import
POST   /mcp/policies/evaluate                     # preview a decision
# Approvals (queue)
GET    /mcp/approvals
POST   /mcp/approvals/{id}/decide                 # {approved, note}
# Audit + stats
GET    /mcp/audit                                 # filters: workspace,server,tool,decision,ok,from,to,limit,offset
GET    /mcp/stats                                 # per-tool aggregates
# Outward Otto MCP server admin + the governed invoke for otto.* tools
GET    /mcp/otto-server                           # status, catalog, enabled tools, PAT prefix
PATCH  /mcp/otto-server                           # enable/disable, per-tool allow, mint/rotate PAT  (Admin)
POST   /mcp/otto-tools/invoke                      # called by `ottod mcp-server`
# New capability endpoints (also used by the otto.* executor)
GET    /workspaces/{ws}/code-search
POST   /workspaces/{ws}/context-packet
GET    /workspaces/{ws}/proof-pack
```

## 9. RBAC & policy.rs

New `Feature::Mcp` ("mcp"). `policy.rs` rule (before the `Deny` arm), matching both
`/mcp/...` and `/workspaces/{wid}/mcp/...` (distinct from the legacy `/mcp-servers` prefix):

- `GET` and the non-mutating previews (`/policies/export`, `/policies/evaluate`,
  `/code-search`, `/proof-pack`) → `Require(Mcp, View)`.
- mutations (create/update/delete, `/discover`, `/health`, `/invoke`, `/allowlist` PUT,
  `/policies` writes, `/approvals/.../decide`, `/context-packet`, `/otto-tools/invoke`) →
  `Require(Mcp, Edit)`.
- security-posture changes (`PATCH /mcp/otto-server`, `POST /mcp/policies/import`) →
  `Require(Mcp, Admin)`.

The legacy `/mcp-servers` rule is unchanged. The `policy.rs` coverage test gets entries for
the new prefixes so nothing silently Denies.

## 10. UI (`ui/src/modules/mcp/`)

Nav: `{ id: 'mcp', icon: 'plug', label: 'MCP Control Plane', feature: 'mcp' }`. `McpPage.svelte`
with tabs:

- **Servers** — registry list (transport, health pill, tool count, injection badge), add/edit
  (stdio command/args/env **or** http url/headers; secret fields masked), Discover + Health
  buttons.
- **Tools** — per server: discovered tools with risk label + injection badge, enable/disable,
  require-approval, risk override; a **dry-run/invoke tester** (run a tool with JSON args,
  toggle dry-run, see governed result).
- **Allowlists** — per-workspace allow/deny grid.
- **Policies** — rule list/editor + import/export the policy-as-code JSON + an evaluate
  preview.
- **Approvals** — pending queue with approve/deny.
- **Audit** — filterable call log.
- **Stats** — per-tool latency/error/bytes table.
- **Otto Server** — outward server status, enable + per-tool allow, mint/rotate PAT, install
  snippet for an external agent's `.mcp.json`.

New API methods extend `ui/src/lib/api/mcp.ts`; types extend `ui/src/lib/api/types.ts`. WS
event `McpApprovalPending`/`McpServerHealth` refresh the relevant tab.

## 11. Security

- **SSRF**: every remote (`http`) server URL passes `otto_netguard::check_url` before connect
  and on redirects.
- **Secrets**: stdio `env` secret values and http auth headers go to Keychain (`mcp-{id}`); DB
  stores only refs; API responses mask them (`has_secret`, `***`). Redaction (`redact_json`)
  on every tool result + audited args.
- **Command exec risk**: stdio servers run arbitrary local commands — creating/enabling one is
  `Edit`; the act is audited; the UI warns. (Same trust as `.mcp.json` today, but now logged.)
- **Outward server**: opt-in (default off), PAT-scoped to its minting user's RBAC, mutating
  tools default-disabled, dangerous calls approval-gated, every call audited.
- **Fail closed** everywhere (RBAC default-Deny, strict default risk, deny-beats-allow).

## 12. Testing

- **Rust unit/integration** (in `otto-mcp`): risk-labeling heuristics; policy evaluation
  (priority, deny-beats-allow, require_approval/dry_run); allowlist resolution; pipeline
  decisions (deny/approve/dry-run); JSON-RPC client framing against an in-test stdio mock; the
  proof-pack/context-packet assemblers. `otto-server` route smoke (401 not 404; RBAC).
- **Playwright E2E** (`ui/e2e/mcp.spec.ts`): a bundled **mock stdio MCP server**
  (`ui/e2e/fixtures/mock-mcp-server.mjs`, Node, implements initialize/tools/list/tools/call)
  registered via the API; then the UI exercises: server appears → Discover lists tools with
  risk labels → Health goes healthy → set a tool require-approval → invoke (dry-run preview) →
  invoke real → audit row appears → stats populate → a dangerous tool creates an approval →
  approve → re-invoke succeeds → policy deny blocks a tool. Plus the outward server: enable,
  mint PAT, and a round-trip driving `ottod mcp-server` against the mock through the otto.*
  catalog.
- **Gates**: `cargo build/test --workspace`, `clippy -D warnings`, `npm run check`,
  `npm run build`, `npm run test:e2e`.

## 13. Scope & deferrals (honest)

- **In**: everything in §6 and §7, the outbound client (stdio fully; http Streamable-HTTP
  request/response + SSE read), background health sweep, full UI, E2E.
- **Deferred (noted in UI/docs, not silently dropped)**: long-lived
  stdio connection pooling (we spawn per-op); true per-vendor *cost* in USD (we meter
  latency/bytes/errors, the available signals, and leave `cost_usd` nullable); OAuth flows for
  remote servers (we support header/token auth now).

---

## 14. Post-review resolutions (architecture + security)

This section **overrides** earlier sections where it conflicts. It resolves every blocker/major
from the two adversarial reviews. Implementation follows *this* section.

### Gateway is IN scope (arch B1)
The live-agent gateway is included in v1 so requirements 8/9/4/5/10/11 hold for Otto's own
agents, not just the UI tester + outward tools. Mechanism:
- New per-workspace setting `mcp_gateway_enabled` (default off, opt-in).
- When on, `otto-sessions::mcp` stops merging governed downstream servers directly into
  `.mcp.json`; only the `otto` server is injected. `ottod mcp-tools` (inward) gains a
  **gateway**: it lists governed downstream tools namespaced `mcp__<server>__<tool>` (fetched
  from `GET /mcp/gateway/tools?workspace_id=&session_id=`) and proxies each `tools/call` via
  `POST /mcp/gateway/invoke` — which runs the **same `McpService::invoke` pipeline**. Every
  downstream call from the agent is then policed + audited + approval-gated.
- Tested at the protocol level by driving the `ottod mcp-tools` subprocess directly (no live
  CLI needed). The inward server's existing 5 read-only tools stay GET-only; the gateway proxy
  is governed by the pipeline (its read-only invariant becomes "the pipeline governs every
  call"). The inward per-session token may reach **only** `/mcp/gateway/invoke` +
  `/mcp/gateway/tools` (see F1 token scoping).

### Server ownership (arch M1)
Servers stay **workspace-scoped** (`mcp_servers.workspace_id NOT NULL`, unchanged). The
`mcp_allowlist` is the *workspace's* allow/deny policy over its **own** servers' tools;
handlers enforce `allowlist.workspace_id == server.workspace_id`. `mcp_tools.enabled` =
**global** per-tool permission (req 4); `mcp_allowlist` = **per-workspace** override (req 5) —
two distinct controls. The pipeline checks allowlist (deny-first) then `enabled`.

### Audit federation (arch M2, sec F12)
`GET /mcp/audit` + `/mcp/stats` read **`mcp_call_log`** as the single source. The inward
read-only server **also** writes one `mcp_call_log` row per call (`direction='inbound'`,
`caller_kind='agent_readonly'`) in addition to its legacy `mcp_tool_calls` row, so inward
calls are visible to the control plane. The control-plane audit write is a **guaranteed**
(non-swallowed) step on **every terminal path** — denial, error, pending, ok. **Fail closed:**
if the audit insert fails, the call does **not** execute (sec F12).

### F1 (BLOCKER) — restricted token for the outward server
`ottod mcp-server` authenticates with a **new restricted token kind `kind='mcp'`** (minted on
`PATCH /mcp/otto-server`), NOT a full PAT. Server-side, an `mcp`-scoped token is authorized for
**only** `POST /mcp/otto-tools/invoke` and `GET /mcp/otto-server` — every other route returns
403, enforced in the auth layer (modeled on the existing share-token scope mechanism), not by
convention. So even if the external agent reads the token from its `.mcp.json`, it can reach
only the governed choke point. The invoke **handler** (in-daemon, full `ServerCtx`) executes
each capability **as the token's user with an explicit per-tool RBAC re-check** (e.g.
`capability_of(user, Database)` + connection visibility for `query_db_readonly`;
`Product:Edit`+ws-role for `create_work_item`; `Agents`/ws-role for `run_goal_loop`) — direct
service calls, never self-HTTP with the restricted token (arch B4 / sec F1).

### F2 (BLOCKER) — approval gate integrity
`mcp_approvals` gains `args_hash` (sha256 of the **canonical full args**), `consumed_at`,
`requested_by`, `requested_by_kind`. The gate requires: `status='approved'` AND not expired AND
`args_hash == hash(current full args)` AND same `(server_id, tool, workspace_id)`, then marks
the row **consumed** in the same transaction as execution (single-use). `POST
/mcp/approvals/{id}/decide` → **`Require(Mcp, Admin)`** and the handler rejects
`decided_by == requested_by` (separation of duties). The approver UI shows the **redacted**
display copy (secret isolation) while the hash binds the **full** args (integrity).

### F3 (MAJOR) — policy: most-restrictive-wins, Admin-gated
Policy evaluation is **most-restrictive-wins**, independent of priority: if **any** matching
enabled rule (global or ws) has effect `deny` → deny; else any `require_approval` → approval;
else any `require_dry_run` → dry-run; else `allow`. Priority only orders display + tie-break of
the `reason` shown. All `/mcp/policies` mutations (create/update/delete/import) →
**`Require(Mcp, Admin)`**.

Policy matcher grammar (`match_json`, all fields optional, AND-combined):
`server_id`, `server_name`, `tool` (exact), `tool_glob` (simple `*`), `risk_label`,
`min_injection_risk` (`>=`), `mutating` (bool), `direction`, `caller_kind`, `workspace_id`.

### F4 (MAJOR) — dry-run is pure simulation
Dry-run **never** calls the downstream tool. It validates args against `input_schema`, resolves
the target, and returns `{ executed:false, mode:"preview", would_call:{server,tool,args} }`. The
"call the tool's own dry-run affordance" path is **dropped**. Approval gate (step 5) precedes
dry-run (step 6) and is never bypassed by `dry_run`.

### F5 (BLOCKER) — `otto.query_db_readonly` truly read-only
The executor classifies the statement itself via `otto_dbviewer::statement_is_write(engine,
stmt)` and **rejects any write/unknown/multi-statement regardless of the connection's
write-guard flag**; forces `confirm_write=false` server-side (ignores caller value); restricts
`connection_id` to connections the caller can see. Never relies on `is_write_guarded`.

### F9 (MAJOR) — SSRF rebinding + redirect
For `http` servers: `otto_netguard::check_url` validates, and the per-op `reqwest::Client` is
built with `.resolve(host, validated_addr)` (pin the vetted IP — defeats DNS rebinding +
auth-header exfiltration) **and** `otto_netguard::redirect_policy()`. Auth headers are sent only
to the pinned, vetted address.

### F10 (BLOCKER) — stdio registration is Admin
Creating/enabling a `transport='stdio'` server (arbitrary command spawned **by the daemon**)
requires **`Mcp:Admin`** (checked in-handler via `capability_of(user, Mcp) >= Admin`).
`transport='http'` requires `Mcp:Edit`. The UI warns that stdio = code execution as the daemon.

### F11 (MAJOR) — injection-safe capability endpoints
`code-search` is implemented in **pure Rust** (walk + literal/regex match, no subprocess) →
no flag injection; `path` is canonicalized and **confined to the workspace root** (absolute /
`..` rejected); results are `redact_json`'d and capped. `proof-pack` uses the **otto-git
service** (no shelling with user refs) and validates refs; `context-packet` touches repos +
memory only (no subprocess). All three live under `/workspaces/{wid}/mcp/...` (sec F11, arch m1).

### F13 (MAJOR) — per-workspace authz on flat by-id routes
Every flat `/mcp/{servers,tools,approvals}/{id}...` handler resolves the entity's
`workspace_id` and enforces the caller's `Mcp` capability **in that workspace** (+ ws-role where
apt). `GET /mcp/audit` and `/mcp/stats` **filter to the workspaces the caller can access**
(or `Mcp:Admin`/root for the global view). No cross-workspace governance via a single-workspace
grant.

### Minors — F6/F8/F15 + cost/secret framing
- Outward default tool set excludes DB + code-search (read-exfiltration): default enabled =
  `get_context_packet, get_proof_pack, ask_human_approval`; `query_db_readonly`,
  `search_codebase`, and all mutating tools are opt-in.
- `mcp_call_log.error` and tool error text are `redact_text`'d before store/return.
- `headers_json` stores header **names only**; secret header values live solely in keychain
  (`mcp-{id}`) — invariant, not "may". Same for stdio `secret_env`.
- `wait_seconds` (ask_human_approval / approval-gated invoke) is capped server-side (≤30s);
  beyond that the caller polls.
- Requirement 12 framing corrected: **latency/error/bytes COVERED; cost(USD) PARTIAL**
  (`cost_usd` nullable/estimated from bytes).
- `otto.open_pr_draft` is verified in impl to perform **zero** git-host API calls (drafts text
  only).

### Secret model (arch B2, sec F8)
A server's env splits: `env` (plaintext, non-secret — stays in `env_json`, legacy merge
unchanged) + `secret_env` (create/update only → keychain `mcp-{id}`, never in DB, never
returned; response lists `secret_env_keys`). Same for http `headers` (plaintext) vs
`secret_headers` (keychain). `DbMcpServerProvider` is updated to overlay resolved `secret_env`
at `.mcp.json` merge time — this **retracts** decision 3's "unchanged" claim for servers that
use `secret_env` (legacy rows with none are byte-for-byte unchanged).

### Streamable-HTTP transport (sec/arch B3)
One "operation" = a short POST sequence sharing one `Mcp-Session-Id`: `POST initialize`
(capture `Mcp-Session-Id` response header) → `POST <op>` (carry the header) → done. `Accept:
application/json, text/event-stream`; if the response is `text/event-stream`, read `data:`
lines and extract the JSON-RPC message; else parse the JSON body. stdio is the
fully-E2E-tested transport; http gets request-builder unit tests + the pinned-IP guard.
```

---

## 15. Implementation status (as shipped)

**Delivered (all 13 control-plane requirements + 8 outward tools):**
- `otto-mcp` crate: outbound MCP client (stdio + Streamable-HTTP w/ pinned-IP SSRF guard),
  risk-labeling, policy engine (most-restrictive-wins), and the `invoke` governance pipeline
  (allowlist → per-tool permission → policy → hash-bound single-use approval → pure-sim
  dry-run → execute → guaranteed fail-closed audit → stats). Repos in
  `otto-state/mcp_control.rs`; migration `0077`.
- Control-plane HTTP surface (`otto-mcp/http.rs`) with per-workspace authz on flat by-id
  routes (F13) and stdio-Admin gating (F10); `Feature::Mcp` + `policy.rs` rules + coverage test.
- Outward "Otto as MCP server" (`ottod mcp-server`) over a **restricted `kind='mcp'` token**
  (F1 — feature-guard limits it to the invoke choke point); the 8 `otto.*` tools executed via
  ephemeral self-calls that reuse each endpoint's native RBAC; `query_db_readonly` statement-
  classified read-only (F5); injection-safe `code-search`/`context-packet`/`proof-pack` (F11).
- Live-agent **gateway**: `/mcp/gateway/*` runs the same pipeline (`caller_kind='gateway'`),
  and `ottod mcp-tools` surfaces + proxies governed downstream tools through it — **opt-in via
  the per-workspace `mcp_gateway_enabled` setting (default off)**, so existing agent sessions
  are unaffected until a workspace opts in. This is what makes requirement #8 ("audit every
  MCP call") and #9 (approval) hold for Otto's own agents.
- Full Svelte UI module (`ui/src/modules/mcp`, 8 tabs) + the otto-mcp pipeline E2E
  (`crates/otto-mcp/tests/pipeline_e2e.rs`, real mock stdio server) + Playwright spec.

**Secret model (B2) as shipped:** secret env/header *values* live only in the macOS Keychain
(`mcp-{id}`), never in the DB or in API responses (requirement 6 holds). Otto's **governed**
outbound client resolves them at call time. The legacy direct `.mcp.json` merge writes only
the plaintext `env` — secret values are intentionally not written to `.mcp.json` (they stay
isolated); for a stdio server that needs a secret, use it through the control plane / gateway.

**Deferred (honest):** per-tool *cost in USD* (we meter latency/bytes/errors, `cost_usd`
nullable); long-lived stdio connection pooling (spawn-per-op); OAuth for remote servers
(header/token auth supported now); auto-flipping `.mcp.json` to gateway-only on enable (today
the gateway surfaces downstream tools additively when its setting is on).
