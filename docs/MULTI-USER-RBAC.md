# Otto Multi-User RBAC — Operator & Deployment Guide

Otto can be deployed for a team: every user gets explicit, per-feature
permissions; each user's sessions and data are private to them; and an admin can
oversee and impersonate. This guide explains the model and how to operate it.

> Implemented on branch `feat/rbac-multiuser` (design: `docs/superpowers/specs/2026-06-19-rbac-multiuser-design.md`).
> Migrations `0038`–`0040`.

## The model in one minute

Authorization has **three independent axes**, and **root bypasses all of them**:

1. **Feature capability** (new) — per user, per feature: `None < View < Edit < Admin`.
   Stored in `user_feature_grants`. Default-deny (no grant = no access). Enforced by
   one central middleware (`feature_guard`) that maps every route → `(feature, capability)`.
2. **Workspace role** (existing) — `Viewer < Editor < Admin` per workspace. Answers
   "which workspace's data."
3. **Ownership** (new) — sessions, query history, saved queries, and (optionally)
   connections are private to their creator. Answers "whose data."

Effective access = the request passes the feature gate **and** the workspace gate
**and** (for per-user resources) the ownership gate. Root passes everything.

## Features & what each capability means

`Agents, Connections, Database, Git, Issues, Product, Swarm, ApiClient, Workflows,
Channels, SkillEval, Skills, Insights, Usage, SelfImprovement, Context, Settings, Users`

- **Database** — *View*: `SELECT`/`SHOW`/`EXPLAIN`, schema, read history/saved queries.
  *Edit*: run mutating SQL, manage saved queries/dashboards. *Admin*: manage connections.
- **Connections** — *View*: list. *Edit*: open/create/edit. *Admin*: manage all / global.
- **Users** — *Admin* only: user CRUD, grant management, the active-sessions overview,
  session terminate, and impersonation. (This is the "granted admin" capability.)
- Other features follow the same read/write/manage pattern; `Usage`, `Insights`,
  `Settings`, `Users` are management-only (their "Admin" ≈ the old root-only gate, now
  grantable to non-root users).

## How to create a feature-scoped user (e.g. "Database only")

1. As **root** (or a `Users:Admin`), open **Settings → Users**.
2. **Create user** (username + password).
3. In that user's **feature grant matrix**, set `Database → View` (or `Edit`), leave
   everything else `None`.
4. Add them to the relevant **workspace** with a role (e.g. Editor) so they can reach
   that workspace's connections.

Result: they log in and see **only Database** in the nav; every other feature's API
returns `403`; they see only their own query history and saved queries.

## Per-user data isolation (what's private)

- **Sessions** — a user sees, attaches to, and controls only their own sessions
  (`created_by`). Workspace-admins and root see all.
- **Live terminal** (`/ws/term`) — only owner/ws-admin/root may attach. Others `403`
  before the upgrade.
- **Event stream** (`/ws/events`) — session status/trail/task events reach only the
  owner/ws-admin/root.
- **Activity** trail/tasks/summary — owner-scoped (admins/root see the full roll-up).
- **DB query history & saved queries / dashboards** — private to the user who ran/saved
  them; **root** sees all.
- **`/app/kill-sessions`** — root only.

### Shared vs private connections
By default connections are **shared** (an admin provisions a company DB connection;
users with `Connections:Edit` open it; each user's *queries* stay private). To make
connections private to their creator, set the daemon setting
**`connections.owner_private = true`** (default `false`). When on, non-root users see
and open only their own connections.

## Admin: active-sessions overview & terminate

**Settings → Sessions** (root or `Users:Admin`) lists every active session across all
users — owner, workspace, provider, status, live, attached viewers — and lets you
**Terminate** any session. Terminate kills the PTY and **forcibly disconnects** all
attached viewers (including a shared mobile session). A user can also terminate their
own sessions.

## Impersonation ("view as")

From **Settings → Users**, an admin can **Impersonate** a non-root, non-admin user to
see exactly what they see. A sticky banner shows "Viewing as <user> — Exit". Guardrails:
you cannot impersonate root or another `Users:Admin`, cannot nest impersonation, and an
impersonation session cannot mint API tokens. Authorization runs as the target; **audit
records the real admin**. Impersonation tokens are short-lived; "Exit" revokes them.

## Audit

Sensitive actions are recorded in the **audit log** (root-only `/audit-log`):
`grant.changed`, `user.created`/`user.disabled`, `impersonate.start`/`impersonate.stop`,
`session.terminated`. Impersonation entries carry both the real and effective user.

## Operating notes

- The **first** user (created at onboarding) is `root` and bypasses all gates. There is
  no promote-to-root endpoint; delegate via `Users:Admin` grants instead.
- New users start with **no grants** (default-deny) — assign them in the grant matrix.
- Existing single-user installs are unaffected: root sees everything; nothing is hidden.
- For remote/company access over the internet, pair this with the secure-exposure +
  share-link work (`docs/superpowers/specs/2026-06-19-remote-mobile-access-design.md`):
  Cloudflare Tunnel, scoped share links, and the email-OTP gate.
