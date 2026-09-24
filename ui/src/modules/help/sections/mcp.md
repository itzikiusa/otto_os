---
id: mcp
title: MCP Control Plane
group: Infrastructure
route: mcp
summary: Govern every MCP tool call your agents make, and expose Otto's own tools to external MCP clients behind tokens, approvals and an audit log.
---

## What it's for

MCP (Model Context Protocol) is how coding agents call tools. The MCP Control Plane puts Otto in the path of those calls:

- **Otto server** — Otto's own built-in MCP server. Choose which `otto.*` tools exist, whether each risky tool asks a human before every call, and connect external clients (Claude Code elsewhere, Copilot, scripts) with scoped access tokens.
- **External servers** — register third-party MCP servers (GitHub, Linear, Blender…) so every call to them runs through one governance pipeline: allowlist → per-tool switch → policy → risk and approval → optional dry run → execute → audit.
- **Activity** — the approval queue, the audit log of every governed call, and per-tool stats.

The page opens on the Otto server section (`#/mcp`). The other sections live at `#/mcp/servers` and `#/mcp/activity`.

## Getting started

1. Open **MCP Control Plane** in the sidebar (Infrastructure). Pick a workspace in the sidebar first: session attachment and external servers are per workspace.
2. On **Otto server**, leave **Attach to sessions in _workspace_** on so new agent sessions there get Otto's built-in tools. It applies to sessions started from now on.
3. Review the **External tool catalog**. Tick the tools you want available; use **All** or **None** per category. Mutating tools are marked `mutating`.
4. To let an outside client use Otto, turn on **Expose to external clients**, open **Connect an external client**, choose **New token**, then copy the install snippet or the `claude mcp add --transport http …` command. The token is shown once.
5. To govern a third-party server, open **External servers** → **Add server**, fill in the form (or start from a template), then **Discover** its tools and turn it on.
6. Watch **Activity**: approve or deny waiting calls, and check the audit log.

## Everything it can do

**Otto server (built-in)**
- **Attach to sessions** — per workspace, gives new agent sessions Otto's tool server (`ottod mcp-tools`). Otto's sessions also get every catalog tool you enabled that the session server doesn't already serve natively (for example `otto_create_pr`), through the same gate.
- **Expose to external clients** — serves the enabled `otto.*` tools over stdio (`ottod mcp-server`) or HTTP (`/api/v1/mcp/http`). Off by default.
- **External tool catalog** — a filterable checklist grouped by feature (Workflows, Git, Issues, Swarm, Vault, Sessions, AWS, Kubernetes, API client, Scheduled tasks and more), with per-category **All** / **None**. A counter shows total, exposed and mutating tools, and how many run without asking.
- **Ask before each call** — under every enabled mutating tool. On (the default) means a human approves each call. Turning it off asks you to confirm; the tool then runs without a prompt but is still audited. Each category also has **Always ask** / **Don't ask** for all of its enabled mutating tools. Disabling a tool drops its exemption, so re-enabling it starts gated again.
- **What my sessions see** — whether the built-in tools are attached, plus the governed gateway tools from external servers (named `mcp__<server>__<tool>`). **Copy** copies the names; **Refresh** reloads them.
- **Connect an external client** — the `.mcp.json` install snippet, the loopback HTTP URL and a ready `claude mcp add` command, each with a copy button.
- **Network access (advanced)** — lets remote clients reach `https://<host>:<port>/api/v1/mcp/http` over TLS with a self-signed certificate. Pick a port other than the daemon's loopback port. Off by default; restart the daemon to apply.
- **Access tokens** — **New token** with an owner (you or another user), a label, an optional workspace pin, **Allow mutating (write) tools** (otherwise read-only) and **Restrict to specific tools** (otherwise every enabled tool). Each row shows the label, prefix, owner, scope and last use, with **Rotate** (replaces only that token, shown once) and **Revoke**.

**Cross-workspace references for agents**
- The list tools (`otto_list_workflows`, `otto_list_connections`, `otto_list_repos`, `otto_list_workspaces`, `otto_list_issue_accounts`…) span every workspace you can read, current one first.
- Wherever a tool takes an id, an agent can pass a name instead: a repo name, path or `owner/repo`, a connection, cluster, workflow, vault or room name, a Jira key, a Confluence URL. An ambiguous name is settled by your current workspace when possible; otherwise the error lists the candidates.
- A reference never resolves to anything you couldn't already list. A workspace-pinned token is re-checked against the target's real workspace.

**External servers**
- **Add server** — name, transport (**stdio**: a local command with arguments and env; **http**: a Streamable HTTP URL with headers), description, and secret env or secret headers stored in the macOS Keychain and never shown again. **Advanced** sets the injection risk (low, medium, high) and default tool access (allow, deny). **Enable now** is off unless you tick it.
- **Templates** — start from a preset (Blender MCP). Templates never start enabled and default to deny.
- **Registry table** — name, endpoint, transport, health pill with latency, tool count, injection risk and an enabled switch. Row actions: **Access**, **Discover**, **Health**, **Delete**.
- **Tools** (click a server name to expand) — each tool's risk (read, write, dangerous, unknown), injection risk, **Enabled** switch, **Approval** switch and **Override risk**. A key icon marks a human-pinned risk label, which rediscovery never lowers.
- **Test a tool** — run any tool with JSON arguments through the full pipeline, with **Dry run (preview only)** on by default. Shows the decision, reason and result, or the approval id when it is waiting.
- **Automatic labelling** — risk and injection labels come from the tool's MCP annotations and name or description keywords. A tool with no signal is treated as `write`; one that reaches untrusted content is `high` injection risk.
- **Health sweep** — enabled servers are re-probed in the background every 5 minutes by default.

**Rules** (the **Rules** button on External servers)
- **Allowlists** — per-workspace allow or deny entries per server, optionally per tool (blank covers the whole server). A deny always wins; with no match the server's default tool access applies. **Add entry**, then **Save**.
- **Policies** — policy-as-code rules, global or per workspace, with an effect (allow, deny, require approval, require dry run), a priority, an enabled switch, a reason and a JSON match on `server_id`, `server_name`, `tool`, `tool_glob`, `risk_label`, `min_injection_risk`, `mutating`, `direction`, `caller_kind`, `workspace_id`. The most restrictive matching rule wins regardless of priority.
- **Export** / **Import** the whole ruleset as JSON (append, or **Replace all existing rules**).
- **Evaluate** — pick a server and a tool name, then **Preview decision** to see what the current rules would do.

**Activity**
- **Approvals** — dangerous tool calls and `otto.ask_human_approval` requests, with redacted arguments, risk, requester and expiry. Add an optional note, then **Approve** or **Deny**. **Show decided too** shows history. The queue refreshes every 5 seconds and when you return to the window; the tab badge counts pending items.
- **Audit** — every governed call (tester, gateway, external clients, `otto.*` tools) with time, server, tool, decision, direction, success, latency and bytes. **Filters** narrow by server, tool and decision.
- **By tool** — calls, errors, error rate, average and max latency, average and total bytes, and last call per tool.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| Esc | Close the **Rules** drawer |
| ↩ | Apply the tool filter in **Audit → Filters** |

There are no other shortcuts specific to this page. For app-wide keys, see [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- **Permissions.** Viewing needs the MCP feature. Changing the Otto server, tokens, network access and policies needs MCP admin. Only the owner can add an external server or change its command, URL, headers or secrets, because a stdio server runs a command as the Otto daemon. Per-server **Access** rules can delegate discover, invoke, configure, approve and access management, down to single tools.
- **Enabling is not approving.** Turning on a mutating `otto.*` tool makes it callable; each call still waits for approval until you turn **Ask before each call** off. If approvals are turned off globally (`mcp_require_approval_dangerous`), the page warns you and nothing asks.
- **Write tokens skip the prompt.** A token created with **Allow mutating (write) tools** counts as a deliberate grant, so its calls don't ask per call. They are still scope-checked and audited.
- **Approvals are single-use** and bound to the exact arguments; changed arguments need a new approval. They expire after 120 minutes. You can't approve your own direct request, but you can decide requests raised by your own agents.
- **Policies and allowlists govern external servers only.** They never add an approval to an `otto.*` tool.
- **Raw access.** An enabled external server is also written into session `.mcp.json` files, so an agent can call it directly outside the gateway. Keep a server disabled unless that is acceptable; the audit only covers calls routed through Otto.
- **Remote servers** must be public URLs; loopback, private and cloud-metadata addresses are refused. There is no OAuth; use header or token auth.
- **Dry run never calls the tool.** It validates and shows what would be called, so it can't reflect the tool's own validation.
- Results are redacted and capped at 500 rows. Cost in dollars isn't metered; bytes are the proxy.
- An MCP token can only reach the MCP endpoints, never the rest of Otto's API.
- **Settings → MCP Servers** is a separate, simpler list: per-workspace servers written into `.mcp.json` when a session starts, without governance. Its **Connections MCP** switch turns Otto's built-in tools on or off for every workspace at once, replacing the per-workspace attach choices on this page.

## Related

- [Settings](#/walkthroughs/settings)
- [Agents](#/walkthroughs/agents)
- [Database Explorer](#/walkthroughs/database)
- [API](#/walkthroughs/api)
- [Plugins](#/walkthroughs/plugins)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
