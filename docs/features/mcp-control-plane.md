# MCP Control Plane

A **governed path** between agents and MCP (Model Context Protocol) tools, plus a way
to expose **Otto itself** as an MCP server to external agents. Otto becomes an
**outbound MCP client** that connects out to each registered server (local `stdio`
command or remote `http`), discovers its tools, health-checks it, and routes every
`tools/call` through a single **governance pipeline** — allowlist → per-tool
permission → policy-as-code → risk/approval gate → dry-run → execute → fail-closed
audit → stats. The same daemon additionally runs an **outward** `ottod mcp-server`
that exposes Otto's own `otto.*` tools — spanning **every Otto feature** (workflows,
message brokers, git/PRs, Jira/Confluence issues, the swarm, the vault, sessions,
code-review findings, product stories, usage, channels, skills, self-improvement,
scheduled tasks, …) — to other agents (Claude Code, Copilot, …) over a **restricted
token**, itself governed by the same pipeline.

This is the definitive end-user + operator guide. It documents what the code in
`crates/otto-mcp/` (the engine), `crates/otto-server/src/mcp_outward.rs` +
`mcp_capabilities.rs` (the outward server, gateway, and capability endpoints),
`crates/ottod/src/mcp_server.rs` + `mcp_tools.rs` (the two stdio binaries),
`crates/otto-state/migrations/0077_mcp_control_plane.sql`, and `ui/src/modules/mcp/`
actually do — the real governance stages, the outward tool list, the server command,
the token kind, and the routes.

> **For the internals/threat-model**, see the internal design doc
> [`mcp-control-plane-design.md`](./mcp-control-plane-design.md) (and its companion
> [`mcp-control-plane-plan.md`](./mcp-control-plane-plan.md)) — including §14, the
> two adversarial-review resolutions this implementation follows. This guide is the
> **user-facing** reference; it links to the design for internals rather than
> duplicating them. Related: **[Scheduled Tasks](./scheduled-tasks.md)** (whose 7
> `otto.*` tools ride on the outward surface), the
> **[daemon HTTP API](./daemon-http-api.md)**, and `docs/contracts/api.md`
> (MCP Control Plane) — authoritative for the API shape.

---

## 1. Summary

| | |
|---|---|
| **What it is** | Otto governs MCP tool calls (in the call path) **and** exposes itself outward as an MCP server. |
| **Two halves** | (A) a **control plane** for registered MCP servers/tools; (B) **Otto-as-MCP-server** — Otto's own `otto.*` feature tools (read + write, every feature) served to external agents. |
| **Outbound client** | `otto-mcp` connects out to each server: `stdio` (spawn a command) or `http` (Streamable-HTTP, SSRF-pinned). |
| **Governance** | One pipeline (`McpService::invoke`): allowlist → per-tool permission → policy → risk/approval → dry-run → execute → guaranteed audit → stats. |
| **Outward server** | `ottod mcp-server` (stdio), authenticated by a **restricted `kind='mcp'` token** that can reach only the governed choke point. |
| **Gateway** | `ottod mcp-tools` surfaces governed downstream tools (namespaced `mcp__<server>__<tool>`) and proxies them through the same pipeline. |
| **Persistence** | SQLite (migration **0077**): augmented `mcp_servers` + `mcp_tools`, `mcp_allowlist`, `mcp_policies`, `mcp_call_log`, `mcp_approvals`. |
| **RBAC** | `Feature::Mcp` (`mcp`): reads = **View**, mutations/invoke = **Edit**, posture (policy writes/import, outward config, approvals) = **Admin**; stdio registration = **Admin** in-handler. |
| **UI** | The **MCP Control Plane** page (`{ id:'mcp', icon:'plug', feature:'mcp' }`) with **8 tabs**. |
| **Defaults** | Outward server **off**; dangerous-tool approval **on**; health sweep every **300 s**. |

---

## 2. Architecture — where it lives

```
                ┌──────────────────────── ottod (daemon) ─────────────────────────┐
 external agent │  otto-server (Axum)                                              │
 (Claude/Copilot│    /mcp/* , /workspaces/{wid}/mcp/*   ── RBAC: Feature::Mcp       │
  elsewhere)    │        │                                                         │
      │ stdio   │        ▼                                                         │
      ▼         │   otto-mcp::McpService.invoke ── governance pipeline             │
 ottod          │        │        │   allowlist→perm→policy→risk/approval→dry-run   │
 mcp-server ───▶│  POST /mcp/otto-tools/invoke   →execute→fail-closed audit→stats  │
 (otto.* tools) │        │        │                                                │
 kind='mcp' tok │        ▼        ▼                  SQLite (migration 0077):       │
                │  outbound MCP client              mcp_servers(+cols) mcp_tools    │
                │   stdio child / http(SSRF-pinned) mcp_allowlist mcp_policies      │
                └─────┼──────────────────────────── mcp_call_log mcp_approvals ─────┘
                      ▼
        registered downstream MCP servers (Linear, GitHub, DB, web-fetch, …)

 Otto's own agents (inward): .mcp.json → ottod mcp-tools
   ├─ first-party read-only tools: DB (otto_db_schema/_query), git PR review,
   │  product story, canvas, + feature reads (otto_list_workflows,
   │  otto_list_broker_clusters, otto_search_memory, otto_list_findings, …)
   └─ gateway: governed downstream tools via /mcp/gateway/tools + /mcp/gateway/invoke
```

| Layer | File | Responsibility |
|---|---|---|
| **Engine** | `crates/otto-mcp/src/service.rs` (`McpService`) | The `invoke` pipeline, discovery, health, the secret resolver. |
| **Outbound client** | `crates/otto-mcp/src/client.rs` | stdio + Streamable-HTTP JSON-RPC 2.0 with caps + SSRF-pinned remote. |
| **Risk labeling** | `crates/otto-mcp/src/risk.rs` | `read`/`write`/`dangerous` + `low`/`medium`/`high` injection from annotations + keywords. |
| **Policy engine** | `crates/otto-mcp/src/policy.rs` | Most-restrictive-wins matcher (`mcp_policies`). |
| **Control-plane HTTP** | `crates/otto-mcp/src/http.rs` (`api_router`) | The `/mcp/*` + `/workspaces/{wid}/mcp/*` governance routes. |
| **Outward server** | `crates/otto-server/src/mcp_outward.rs` | `/mcp/otto-tools/invoke`, `/mcp/otto-server`, the gateway, the categorised `otto.*` tool catalog (`otto_tool_specs` + the pure `route_for` map). |
| **Capability endpoints** | `crates/otto-server/src/mcp_capabilities.rs` | `code-search`, `context-packet`, `proof-pack` (injection-safe). |
| **stdio binaries** | `crates/ottod/src/{mcp_server,mcp_tools}.rs` | The outward (`ottod mcp-server`) + inward (`ottod mcp-tools`) servers. |
| **Persistence** | `crates/otto-state/migrations/0077_mcp_control_plane.sql` + `mcp_control.rs` | The six tables + repos. |
| **UI** | `ui/src/modules/mcp/` (`McpPage.svelte` + 8 tab components) | Servers, Tools, Allowlists, Policies, Approvals, Audit, Stats, Otto Server. |

The **MCP Control Plane** page is workspace-scoped: with no workspace selected it
shows *"No workspace selected — Select a workspace to manage its governed MCP servers
and tools."* Servers are registered **per workspace**.

> This is distinct from the **legacy** per-workspace `.mcp.json` config CRUD
> (`/mcp-servers`, migration 0036), which is unchanged and stays Exempt from
> `Feature::Mcp` — Otto was never in the call path there. The control plane lives
> under the separate `/mcp/*` and `/workspaces/{wid}/mcp/*` prefixes.

---

## 3. Setup

Open **MCP Control Plane** in the sidebar (it requires the `mcp` feature capability).

### 3.1 Register a server (control plane)

In the **Servers** tab → **Add server**. Choose a transport:

- **`stdio`** — a local command Otto spawns (`command`, `args`, `env`). **Registering
  a stdio server requires MCP Admin**, because it runs an arbitrary command *as the
  Otto daemon* (the UI warns of this). Changing a stdio server's command later is
  also Admin.
- **`http`** — a remote Streamable-HTTP endpoint (`url`, `headers`). Requires MCP
  Edit. Every remote URL is SSRF-checked (see §9).

Secret env values / auth headers go in the **`secret_env` / `secret_headers`** fields
— these are written to the macOS Keychain (`mcp-{id}`) and **never** stored in the DB
or returned by the API (responses list only `secret_env_keys` / mask them). The
server list shows columns **Name, Transport, Health, Tools, Injection, Enabled,
Actions**, with per-server **Discover**, **Health**, and **Delete**.

On create/enable Otto **discovers** the server's tools (`initialize` + `tools/list`),
labels each (§6), and upserts the catalog. A background **health sweep** re-probes
every managed, enabled server every `mcp_health_interval_secs` (default **300 s**;
`0` disables it).

### 3.2 Enable the outward Otto MCP server

In the **Otto Server** tab: toggle **Enable**, pick which `otto.*` tools to expose
(a **filterable checklist grouped by feature category**, with per-group **All/None**),
and **Mint token** / **Rotate token**. The minted token is a restricted `kind='mcp'`
token shown **once** (*"New token — shown once. Copy it now."*); only its 12-char
prefix is shown thereafter. The tab generates an **install snippet** to paste into an
external agent's `.mcp.json`:

```json
{
  "mcpServers": {
    "otto": {
      "command": "ottod",
      "args": ["mcp-server"],
      "env": { "OTTO_API_TOKEN": "<token>" }
    }
  }
}
```

`ottod mcp-server` reads `OTTO_API_TOKEN` (the restricted token) and optionally
`OTTO_MCP_BASE` (defaults to `http://127.0.0.1:<port>`). It is **off by default**
(`mcp_otto_server_enabled = false`) — nothing is exposed until an admin enables it.

### 3.3 The outward tool catalog — every Otto feature

`otto_tool_specs()` is the authoritative catalog (each tool carries `name`,
`description`, `mutating`, `category`, `inputSchema`). Each tool wraps an **existing**
daemon REST endpoint via a self-call executed **as the token's user**, so it reuses
that endpoint's native RBAC — no new privilege path. The `(tool, args) → (method,
path, body)` binding is a **pure function** (`route_for`) covered by unit tests.

Coverage by category (✅ = read tools, ⚠ = mutating tools, approval-gated):

| Category | Read tools | Mutating tools (DANGEROUS) |
|---|---|---|
| **Workflows** | ✅ list / get / list_runs / get_run | ⚠ run, cancel_run |
| **Message Brokers** | ✅ list_clusters / list_topics / get_topic / list_consumer_groups / *consume*¹ | ⚠ produce |
| **Git** | ✅ list_repos / status / list_prs / get_pr / *open_pr_draft*¹ | ⚠ create_pr, comment_pr, start_pr_review |
| **Database** | ✅ list_connections / *query_db_readonly*¹ | — |
| **Issues** | ✅ search_issues / get_issue / search_confluence | ⚠ comment_issue, transition_issue |
| **Swarm** | ✅ list / get / list_runs / get_board / create_work_item² | ⚠ post_swarm_board |
| **Memory/Vault** | ✅ list_memory / *search_memory*¹ | — |
| **Sessions** | ✅ list / get | ⚠ broadcast_message |
| **Code Review** | ✅ list_findings / get_finding | ⚠ start_pr_review |
| **Product** | ✅ list_stories / get_story | — |
| **Channels** | ✅ list_integrations | ⚠ test_integration |
| **Usage / Skills** | ✅ get_usage_summary / list_bundled_skills | — |
| **Self-Improvement** | ✅ get_config / list_runs / get_run / list_edits | ⚠ run, approve_edit, reject_edit, rollback_edit |
| **Scheduled Tasks** | ✅ list / list_runs | ⚠ create / update / set_enabled / run / delete |
| **Code & Context / Agents / Approvals** | ✅ *search_codebase*¹ / get_context_packet / get_proof_pack / ask_human_approval | ⚠ run_goal_loop |

¹ Off by default (opt-in): non-mutating tools that stream large/sensitive *content*
(messages, recalled knowledge, code, rows) are defined but not in `DEFAULT_ENABLED`.
² `create_work_item` is the Swarm-task create (mutating).

**Classification rule.** Every read tool is in exactly one of `DEFAULT_ENABLED`
(surfaced when the server is on) or the opt-in set; every mutating tool is in
`DANGEROUS` (off by default, approval-gated). A unit test enforces this invariant.

The same feature **reads** (no writes) are also injected into Otto's *own* agent
sessions through the inward `ottod mcp-tools` server (§5) as `otto_list_workflows`,
`otto_list_broker_clusters`, `otto_search_memory`, `otto_list_findings`,
`otto_list_improvement_edits`, … so an Otto session can inspect every feature while
keeping that server's strict read-only invariant.

---

## 4. The governance pipeline (`McpService::invoke`)

Every governed `tools/call` — from the UI tester, the live-agent gateway, and the
outward `otto.*` tools — funnels through one choke point. The stages run in this
order; the **first** decisive stage wins:

1. **Resolve** the server (must exist) and the tool. An **undiscovered/unknown tool
   fails closed** — treated as `dangerous` + `high` injection + disabled.
2. **Server gate** — the server must be `enabled` **and** `managed`.
3. **Allowlist** (per workspace) — evaluate `mcp_allowlist` for `(ws, server, tool)`:
   a `deny` match wins; else an `allow` match; else fall back to the server's
   `default_tool_access` (`allow`/`deny`). A tool-less entry covers the whole server.
4. **Per-tool permission** — `mcp_tools.enabled` must be true (a **global** per-tool
   switch, distinct from the per-workspace allowlist).
5. **Policy-as-code** — evaluate `mcp_policies` (global + this workspace),
   **most-restrictive-wins** independent of priority: any matching `deny` → deny;
   else any `require_approval`; else any `require_dry_run`; else `allow` (§7).
6. **Risk / approval gate** — if the tool is `dangerous` (with
   `mcp_require_approval_dangerous` on, the default), or policy/per-tool requires
   approval, the call needs a **human approval** bound to the exact args (a SHA-256
   `args_hash`). With no usable approval it creates a **pending** `mcp_approvals` row
   and returns `pending_approval`. An existing approval is **consumed single-use**
   (atomically) and must match `(server, tool, workspace, args_hash)`, be unexpired,
   and have been decided by **someone other than the requester**. A `dry_run` request
   skips approval *creation* (a preview executes nothing).
7. **Dry-run** — if requested (or policy `require_dry_run`): **pure simulation** — it
   validates args, resolves the target, and returns
   `{ executed:false, mode:"preview", would_call:{server,tool,arguments} }`. It
   **never** calls the downstream tool (the gate in step 6 precedes it and is never
   bypassed).
8. **Execute** via the outbound client; measure latency + response bytes; **cap every
   result array to 500 rows** and `redact_json` the content.
9. **Audit + stats** — exactly **one** `mcp_call_log` row per terminal path (denied /
   pending / dry-run / ok / error). The audit insert is a **fail-closed** step before
   execution: if it fails, the call does **not** run. Stats are derived from this
   table — no separate counter to drift.

The terminal decision is one of `allowed` · `approved` · `denied` · `dry_run` ·
`pending_approval` · `error`.

---

## 5. The outward `otto.*` tools — core set + full feature catalog

`ottod mcp-server` exposes a static catalog (`otto_tool_specs`), filtered to the
admin-enabled set. `tools/list` reflects `GET /mcp/otto-server`; `tools/call`
forwards to `POST /mcp/otto-tools/invoke`, which governs (enabled? allowlisted?
dangerous→approval?), audits (`direction='inbound'`), then executes the capability
**as the token's user** via a short-lived **ephemeral self-call** so each tool reuses
its endpoint's native RBAC (no privilege escalation).

The catalog spans **every Otto feature** — see the category table in **§3.3** for the
complete list and `otto_tool_specs()` / `route_for` for the authoritative source. The
**core/original** tools are detailed here:

| Tool | Mutating | Default-enabled | What it does |
|---|---|---|---|
| `otto.search_codebase` | no | no | Literal code search confined to the workspace root (`code-search`). |
| `otto.get_context_packet` | no | **yes** | Workspace metadata + the most-relevant code excerpts for a query. |
| `otto.run_goal_loop` | **yes (dangerous)** | no | Create **and start** a bounded goal loop. |
| `otto.create_work_item` | **yes (dangerous)** | no | Create a Swarm task under a project. |
| `otto.query_db_readonly` | no | no | A **read-only** SQL query against a connection (statement-classified; writes/DDL/multi-statement rejected). |
| `otto.open_pr_draft` | no | no | Draft a PR title + description from a diff — **does not** publish a PR. |
| `otto.get_proof_pack` | no | **yes** | An evidence bundle: git status/recent-commits/diffstat + a goal loop's machine-checked acceptance criteria. |
| `otto.ask_human_approval` | no (creates a request) | **yes** | Create a pending human-approval item and optionally wait (`wait_seconds ≤ 30`) for the decision. |

The **default-enabled** set (`mcp_otto_server_tools`) is the safe read subset —
`get_context_packet`, `get_proof_pack`, `ask_human_approval` — **plus** the two
scheduled-task reads (`list_scheduled_tasks`, `list_scheduled_task_runs`). DB +
code-search are read-exfiltration vectors and are **opt-in**; all mutating tools are
off by default and `DANGEROUS` (approval-gated). In addition to the core tools above,
the outward surface carries **read + write tools for every Otto feature** (workflows,
brokers, git/PRs, issues, swarm, vault, sessions, code-review, product, usage,
channels, skills, self-improvement) and the **seven Scheduled-Tasks tools** — the full
category table is in **§3.3**, and see the
[Scheduled Tasks guide](./scheduled-tasks.md#9-mcp-surface-the-7-otto-tools).

`otto.query_db_readonly` is enforced read-only **server-side**: the executor
classifies the statement itself (only `SELECT/SHOW/DESCRIBE/EXPLAIN/WITH`, single
statement) and forces `confirm_write=false`, **regardless** of the connection's
write-guard flag.

---

## 6. Tool discovery & risk labeling

On **Discover**, Otto runs `initialize` + `tools/list` and labels each tool
(`risk.rs`). The **Tools** tab shows columns **Tool, Risk, Injection, Enabled,
Approval, Override risk**, plus a **dry-run/invoke tester** (run a tool with JSON
args, toggle **Dry run**, see the governed decision).

- **Risk label** (`read` / `write` / `dangerous`) — from MCP `toolAnnotations` when
  present (`destructiveHint=true`→`dangerous`; `readOnlyHint=true`→`read`), refined by
  name/description **keywords** (`delete|drop|remove|exec|deploy|payment|…`→dangerous;
  `create|update|write|send|publish|merge|…`→write). **No signal → `write` (fail
  closed).**
- **Injection risk** (`low` / `medium` / `high`) — `openWorldHint=true` (the tool
  reaches untrusted external content — the classic prompt-injection vector) → **high**;
  keywords like `fetch|browse|web|url|http|search|email|…` → at least **medium**;
  unlabeled → **medium**.
- **`supports_dry_run` is always false** — Otto never trusts a server's claim of a
  dry-run affordance (a lie would let a "dry-run" execute a mutation). Dry-run is
  always pure local simulation.
- A human can **pin** labels (`risk_overridden`); rediscovery never lowers a pinned
  label.

---

## 7. Policy-as-code

The **Policies** tab manages `mcp_policies` rules (global + per-workspace), with
**Export** / **Import** of the full ruleset as one JSON document and a **Preview
decision** evaluator. Columns: **Name, Scope, Prio, Effect, Match, On**. Each rule is
a `match` object + an `effect` (`allow` / `deny` / `require_approval` /
`require_dry_run`).

Evaluation is **most-restrictive-wins**, *independent of priority* — among all
matching enabled rules, `deny` beats `require_approval` beats `require_dry_run` beats
`allow`. Priority only orders display + tie-breaks the displayed reason.

The matcher grammar (all fields optional, **AND**-combined; empty match = matches
everything): `server_id`, `server_name`, `tool` (exact), `tool_glob` (simple `*`),
`risk_label`, `min_injection_risk` (`>=`), `mutating` (bool), `direction`,
`caller_kind`, `workspace_id`.

All policy **mutations** (create/update/delete/import) require **MCP Admin**; export
and evaluate are read-only (View).

---

## 8. Approvals, audit & stats

- **Approvals** (`mcp_approvals`) — the **Approvals** tab is the queue for
  dangerous-tool calls and `otto.ask_human_approval` requests. Each card shows the
  **redacted** args (never full/secret values), a risk pill, and **Approve** / **Deny**
  with an optional note; a *"Show decided too"* toggle reveals history. A decision
  requires **MCP Admin**, and the repo enforces **approver ≠ requester** (separation
  of duties). Stale approvals expire (default TTL 120 min for tool-calls).
- **Audit** (`mcp_call_log`) — the **Audit** tab is the ledger of **every** governed
  call (UI tester, gateway, inbound `otto.*`, outbound downstream), filterable by
  server / tool / decision. Columns: **Time, Server, Tool, Decision, Dir, OK, Latency,
  Bytes**. Args are stored redacted; error text is redacted.
- **Stats** (derived from the audit table) — the **Stats** tab shows per-tool **Calls,
  Errors, Err rate, Avg/Max latency, Avg/Total bytes, Last called**. Cost in USD is a
  **partial** signal — Otto meters latency/bytes/errors (the available signals) and
  leaves true per-vendor cost out.

Both **Audit** and **Stats** are **filtered to the workspaces the caller can access**
(or the global view for MCP Admin / root) — no cross-workspace governance via a
single-workspace grant.

### 8.1 Live signals

There is **no dedicated MCP WebSocket event** in the contract. When a **gateway**
call lands in `pending_approval`, the daemon broadcasts a generic
`notice` event (*"MCP approval needed"*) so a connected client is nudged toward the
Approvals queue; otherwise the UI refreshes on demand. The background health sweep
updates server health rows without a WS push.

> **Drift note:** the design doc proposed `McpApprovalPending` / `McpServerHealth` WS
> events; the shipped code uses the generic `notice` event for the gateway approval
> nudge and polls for the rest.

---

## 9. The two stdio binaries & the gateway

**Outward — `ottod mcp-server`** (`mcp_server.rs`): the external-facing server. Reads
`OTTO_API_TOKEN` (the restricted `kind='mcp'` token) + optional `OTTO_MCP_BASE`. Its
`tools/list` is the enabled `otto.*` catalog (from `GET /mcp/otto-server`); its
`tools/call` forwards to `POST /mcp/otto-tools/invoke`. Those are the **only two
routes** the restricted token may reach (see §10).

**Inward — `ottod mcp-tools`** (`mcp_tools.rs`): the per-session server Otto injects
into its own agents' `.mcp.json` (read-only by construction — GETs or read-only-enforced
viewer POSTs only; 20 s timeout, 1 MiB body cap, 500-row cap, redacted, audited). It
serves the first-party **read-only** tools: the DB connection tools
(`otto_list_connections`, `otto_db_schema`/`_children`/`_object`, `otto_db_query`),
`otto_git_pr_review`, `otto_product_story`, `canvas_list_scenes`/`canvas_get_scene`,
**plus per-feature reads** — `otto_list_workflows`, `otto_get_workflow_run`,
`otto_list_broker_clusters`/`_topics`, `otto_search_issues`, `otto_list_swarms`,
`otto_search_memory`, `otto_list_repos`, `otto_list_sessions`,
`otto_list_product_stories`, `otto_list_findings`, `otto_usage_summary`,
`otto_list_improvement_runs`/`_edits` (the pure `read_route` map, unit-tested).

**The gateway.** The inward server *also* surfaces the workspace's **governed
downstream tools** — fetched from `GET /mcp/gateway/tools?workspace_id=` and
namespaced `mcp__<server>__<tool>` — and proxies each such call through
`POST /mcp/gateway/invoke`, which runs the **same `McpService::invoke` pipeline**
(`caller_kind='gateway'`). This is what puts the control plane in the path of a live
agent's every downstream MCP call, so the audit/approval/policy guarantees hold for
Otto's own agents, not just the UI tester and the outward tools. (In the shipped
code the gateway tools are surfaced **additively**; there is no separate
`mcp_gateway_enabled` toggle — a drift from the design doc, which proposed one.)

---

## 10. Security & permissions

- **Restricted outward token.** The outward server authenticates with a **`kind='mcp'`
  token**, not a full PAT. Server-side, an `mcp`-scoped token sets an `mcp_only` flag,
  and the auth/feature guard authorizes it for **only** `POST /api/v1/mcp/otto-tools/invoke`
  and `GET /api/v1/mcp/otto-server` — **every other route returns 403** (enforced in
  the guard, before the general policy table). So even if the external agent reads the
  token from its `.mcp.json`, it can reach only the governed choke point. The token is
  long-lived (10 years) and not slid; **Rotate token** revokes it immediately.
- **Two-axis + IDOR.** Routes are gated by `Feature::Mcp` (reads = View, mutations =
  Edit, posture = Admin) **and** every flat `/mcp/{servers,tools,approvals}/{id}`
  handler resolves the entity's workspace and re-checks the caller's role there.
- **stdio = Admin.** Registering or re-commanding a `stdio` server (a command run as
  the daemon) requires MCP Admin in the handler — `http` requires Edit.
- **SSRF for remote servers.** Every `http` URL passes `otto_netguard::check_url`, and
  the per-op client **pins the vetted IP** (`.resolve`) so a DNS rebind can't redirect
  the connect — or the auth header — to loopback/metadata; redirects use the netguard
  policy. Auth headers go only to the pinned address.
- **Secrets.** `secret_env` / `secret_headers` values live solely in the Keychain
  (`mcp-{id}`); the DB stores only key names; responses mask them. Otto's governed
  outbound client resolves them at call time. The legacy direct `.mcp.json` merge
  writes only the plaintext `env` — secret values are intentionally not written there.
- **Redaction + caps.** Every tool result is `redact_json`'d and capped at 500 rows /
  1 MiB; audited args + error text are redacted.
- **Injection-safe capability endpoints.** `code-search` is **pure Rust** (no
  subprocess → no flag injection) and confines `path` to the workspace root (absolute
  / `..` rejected); `proof-pack` shells `git` only with a fixed argv + validated ref;
  `context-packet` reuses the confined search. All three live under
  `/workspaces/{wid}/mcp/...`.
- **Fail closed everywhere** — unknown route → Deny (RBAC default); unknown tool →
  dangerous + disabled; audit-insert failure → no execution; `deny` beats `allow`.

---

## 11. REST API reference

All routes are under `/api/v1`; authoritative contract: `docs/contracts/api.md`
(MCP Control Plane). RBAC is `Feature::Mcp` unless noted; flat by-id routes also check
the workspace role.

**Control plane — registry, tools, governance**

| Method & path | Purpose | RBAC |
|---|---|---|
| `GET /workspaces/{wid}/mcp/servers` | List a workspace's servers (`McpServerDetail[]`) | View |
| `POST /workspaces/{wid}/mcp/servers` | Register a server | Edit (stdio → **Admin**) |
| `GET /mcp/servers/{id}` | Server + its tools | View |
| `PATCH /mcp/servers/{id}` | Update server config | Edit (stdio command → Admin) |
| `DELETE /mcp/servers/{id}` | Delete a server (+ keychain secret) | Edit |
| `POST /mcp/servers/{id}/health` | Probe health now | Edit |
| `POST /mcp/servers/{id}/discover` | Re-discover the tool catalog | Edit |
| `GET /mcp/servers/{id}/tools` | List discovered tools (`McpTool[]`) | View |
| `PATCH /mcp/tools/{tool_id}` | Per-tool: enable / require-approval / risk override | Edit |
| `POST /mcp/servers/{id}/tools/{name}/invoke` | Governed invoke (`{arguments, dry_run?}` → `InvokeResp`) | Edit |
| `GET /workspaces/{wid}/mcp/allowlist` | Get allow/deny entries | View |
| `PUT /workspaces/{wid}/mcp/allowlist` | Bulk-set the allowlist | Edit |
| `GET /mcp/policies` (`?workspace_id=`) | List policies | View |
| `POST /mcp/policies` | Create a policy | **Admin** |
| `PATCH /mcp/policies/{id}` · `DELETE …` | Update / delete a policy | **Admin** |
| `GET /mcp/policies/export` | Export the ruleset as JSON | View |
| `POST /mcp/policies/import` | Import (`{policies, replace?}`) | **Admin** |
| `POST /mcp/policies/evaluate` | Preview the decision for a tool | View |
| `GET /mcp/approvals` (`?status=`) | The approval queue (ws-filtered) | View |
| `POST /mcp/approvals/{id}/decide` | Approve/deny (`{approved, note?}`) | **Admin** (approver ≠ requester) |
| `GET /mcp/audit` (filters) | The call-log ledger (ws-filtered) | View |
| `GET /mcp/stats` | Per-tool aggregates (ws-filtered) | View |

**Outward server, gateway & capabilities**

| Method & path | Purpose | RBAC |
|---|---|---|
| `GET /mcp/otto-server` | Outward status + tool catalog + token prefix | View (or `mcp` token) |
| `PATCH /mcp/otto-server` | Enable/disable, per-tool allow, mint/rotate token | **Admin** |
| `POST /mcp/otto-tools/invoke` | The governed choke point for the `otto.*` tools | Edit (or `mcp` token) |
| `GET /mcp/gateway/tools` (`?workspace_id=`) | Namespaced governed downstream tools | View |
| `POST /mcp/gateway/invoke` | Proxy a downstream call through the pipeline | Edit |
| `GET /workspaces/{wid}/mcp/code-search` (`?q=&path=&max=`) | Pure-Rust confined code search | View |
| `POST /workspaces/{wid}/mcp/context-packet` | Assemble a context packet | Edit |
| `GET /workspaces/{wid}/mcp/proof-pack` (`?repo_id=&branch=&goal_loop_id=`) | Evidence bundle | View |

**Settings keys** (`settings` table, JSON): `mcp_otto_server_enabled` (default
`false`), `mcp_otto_server_tools` (default = read subset + the two scheduled-task
reads), `mcp_require_approval_dangerous` (default `true`), `mcp_health_interval_secs`
(default `300`; `0` = off).

---

## 12. Capabilities & limitations

**Capabilities**

- Otto sits **in the call path**: every governed MCP call is allowlisted, policed
  (policy-as-code), risk-gated, optionally dry-run, executed, and **audited** with
  derived per-tool stats.
- Two transports — local `stdio` (spawn-per-op) and remote `http`
  (Streamable-HTTP with SSE), the latter SSRF-pinned.
- Automatic risk + injection labeling (with human override that survives
  rediscovery), background health sweeps, and a single-use, hash-bound, approver≠requester
  approval queue.
- Otto-as-MCP-server: `otto.*` tools spanning **every Otto feature** (read + write)
  over a restricted token that can reach only the governed choke point; reads
  default-on, mutating tools off by default and approval-gated.
- A **gateway** that brings Otto's own agents' downstream MCP calls under the same
  pipeline.
- Per-workspace governance with strict per-workspace authorization on flat routes.

**Limitations**

- **No long-lived stdio connection pooling** — Otto spawns one child **per operation**
  (discovery / health / invoke). Robust and stateless, but not the lowest-latency
  design.
- **Per-tool cost in USD is partial** — latency / bytes / errors are metered; true
  per-vendor cost is not (`cost_usd` is nullable/estimated).
- **No OAuth flows** for remote servers — header/token auth only.
- **Dry-run is a pure simulation** — it never calls the downstream tool, so it cannot
  reflect a tool's own validation.
- **No `mcp_gateway_enabled` toggle** in the shipped code — the gateway surfaces
  governed downstream tools additively (a drift from the design doc).
- **No dedicated MCP WS event** — the gateway uses a generic `notice` nudge; the rest
  is poll/refresh.
- **`stdio` registration runs arbitrary local commands** as the daemon — gated to
  MCP Admin, logged, and UI-warned, but inherently powerful.

---

## 13. Troubleshooting

**Discover finds no tools / Health is unhealthy.** For `stdio`, confirm the `command`
is on the daemon's `PATH` and exits cleanly on stdin close; check the **Audit** /
daemon logs for the spawn error. For `http`, the URL must be **public** — the SSRF
guard refuses loopback/private/metadata and a host that resolves only to blocked
addresses. Discovery and health each spawn a short-lived connection; a 20 s timeout
applies.

**A tool call returns `pending_approval`.** It hit the risk/approval gate (a
`dangerous` tool with approval on, a `require_approval` policy, or a per-tool
require-approval). Approve it in the **Approvals** tab (MCP Admin, and not the same
user who requested it), then re-invoke — the approval is **single-use** and bound to
the exact arguments, so changing the args invalidates it.

**A tool is `denied`.** Walk the pipeline: is the server enabled + managed? Is there a
workspace **deny** in the Allowlist (deny wins)? Is the per-tool **Enabled** switch
off? Does a **policy** deny it? The **Audit** row's `decision_reason` names the stage.

**The external agent gets 403 on everything but two routes.** Expected — the
`kind='mcp'` token is restricted to `POST /mcp/otto-tools/invoke` and
`GET /mcp/otto-server`. Drive all `otto.*` tools through `tools/call` (which the
`ottod mcp-server` forwards to the invoke route), not by calling other API routes
directly.

**The outward server "isn't there".** It is **off by default** — enable it in **Otto
Server**, mint a token, paste the snippet into the external agent's `.mcp.json`, and
confirm `OTTO_API_TOKEN` is set and `OTTO_MCP_BASE` (if used) points at the daemon. A
disabled server returns `denied` to every invoke.

**`otto.query_db_readonly` rejected my statement.** It permits a **single read-only**
statement (`SELECT/SHOW/DESCRIBE/EXPLAIN/WITH`); writes, DDL, and multi-statement
input are rejected server-side regardless of the connection's write-guard.

**Stats show no cost.** By design — Otto meters latency/bytes/errors; per-vendor USD
cost is not tracked.

---

## 14. Related docs

- **[Scheduled Tasks](./scheduled-tasks.md)** — its 7 `otto.*` tools ride on the
  outward surface and are governed by this pipeline.
- **[Daemon HTTP API](./daemon-http-api.md)** — auth, tokens, and calling `/mcp/*`.
- **Design (internal):** [`mcp-control-plane-design.md`](./mcp-control-plane-design.md)
  (threat model, §14 review resolutions) and
  [`mcp-control-plane-plan.md`](./mcp-control-plane-plan.md).
- **Contracts (authoritative):** `docs/contracts/api.md` (MCP Control Plane).
- **Source:** `crates/otto-mcp/`, `crates/otto-server/src/{mcp_outward,mcp_capabilities}.rs`,
  `crates/ottod/src/{mcp_server,mcp_tools}.rs`,
  `crates/otto-state/migrations/0077_mcp_control_plane.sql`, `ui/src/modules/mcp/`.
