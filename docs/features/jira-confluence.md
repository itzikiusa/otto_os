# Jira & Confluence Integration (Issue Trackers)

Connect Otto to Atlassian Cloud so you can search a project for a Jira issue (or a
space for a Confluence page), import it, read it as Markdown, and post comments
back — all without leaving the app. This document is the end-user and operator
reference for **connecting accounts** and the **import / search / read / comment**
surface. The product-owner workflow that runs *on top of* this integration
(analysis lenses, rewrites, test-case generation, multi-agent planning, publishing
stories to Jira and acceptance criteria to Confluence) is a separate feature — see
[`./product.md`](./product.md).

> **TL;DR auth.** Otto authenticates to Atlassian using **Atlassian Cloud API
> tokens** with **HTTP Basic auth** — `email:api_token`, Base64-encoded into an
> `Authorization: Basic …` header. You provide three things per account: a
> **base URL** (`https://your-domain.atlassian.net`), the **email** of the
> Atlassian account, and an **API token** created at
> `https://id.atlassian.com/manage-profile/security/api-tokens`. There is **no
> OAuth flow** and **no separate Confluence credential** — the same
> email + token pair is reused for both Jira (`/rest/api/3/…`) and Confluence
> (`/wiki/rest/api/…`) on the same site. Jira Server / Data Center **Personal
> Access Tokens (bearer PATs) are not supported**; only the Basic
> `email:token` scheme is sent (see [§10 Cloud vs Server](#10-cloud-vs-server-pats-2fasso)).

---

## 1. Concept: issues and pages as importable artifacts

Otto treats two kinds of Atlassian content as **importable artifacts**:

| Artifact | Source | Identified by | Imported as |
|---|---|---|---|
| **Jira issue** | `https://your-domain.atlassian.net/rest/api/3` | Issue key (`PROJ-123`) | Summary, status, type, assignee, description (ADF → Markdown), plus a full view: comments, changelog, attachments, links, story-point/time estimate, all non-empty custom fields |
| **Confluence page** | `https://your-domain.atlassian.net/wiki/rest/api` | Numeric page id | Title, space key, body (storage XHTML → Markdown), version, browser URL |

You find an artifact by **searching its container** — a Jira **project** or a
Confluence **space** — so you never have to type a key prefix by hand. Otto's
search builds the JQL/CQL for you (numeric input is treated as a ticket number or
page id; free text is matched against summary/title). Once imported, content is
rendered as **Markdown**: Jira's Atlassian Document Format (ADF) and Confluence's
storage-format XHTML are both converted by Otto's built-in converters, including
tables, code blocks, info/note/warning panels, links, lists, and images.

The integration is also a **two-way bridge**: Otto can **write comments** back to a
Jira issue or a Confluence page (and the product workflow can additionally create
issues, transition them, assign them, and update descriptions/pages — see §7–§8).

---

## 2. Overview & where it lives

| Concern | Location |
|---|---|
| Add / edit / delete accounts (UI) | **Settings → Jira Accounts** (`ui/src/modules/settings/IssueAccounts.svelte`) |
| Daemon crate | `crates/otto-issues/` — Axum router (`http.rs`), Jira client (`jira.rs`), Confluence client (`confluence.rs`), ADF↔Markdown (`adf.rs`), storage-XHTML↔Markdown (in `confluence.rs`) |
| Domain types | `crates/otto-core/src/domain.rs` (`IssueAccount`, `IssueProviderKind`, `IssueProject`, `IssueSummary`, `IssueDetail`) |
| Request types | `crates/otto-core/src/api.rs` (`CreateIssueAccountReq`, `UpdateIssueAccountReq`) |
| Persistence | SQLite `issue_accounts` table (columns: `username` = email, `api_base_url` = base URL, `token_ref` = Keychain item name); managed by `IssuesRepo` in `otto-state` |
| Secret storage | macOS **Keychain** via `otto-keychain` (`SecretStore`); the DB stores only the opaque `token_ref` |
| API contract | `docs/contracts/api.md` → **## Issue trackers (Jira / Confluence)** (lines 641–664) |
| Mount prefix | All paths below are relative to **`/api/v1`** (e.g. `/api/v1/issue/accounts`) |

**Per-user, owner-scoped.** Each issue account belongs to the user who created it.
Listing returns only your own accounts, and every read/write that resolves an
`account_id` is gated by an ownership check (`authorize_account` → `authorize_owner`):
only the account owner — or **root** — may act through that account's Atlassian
identity. The token is **never** returned by the API (the `token_ref` field is
`#[serde(skip_serializing)]`; the UI shows `token ••••••`).

---

## 3. Connecting Jira / Confluence (API token setup)

> One account connects **both** Jira and Confluence on the **same Atlassian
> site**. You do not create a separate Confluence account — the same
> email + API token authenticates `your-domain.atlassian.net` (Jira) and
> `your-domain.atlassian.net/wiki` (Confluence).

### Step 1 — Create an Atlassian API token

1. Go to **<https://id.atlassian.com/manage-profile/security/api-tokens>**
   (in Otto's "Add" dialog this is shown as *id.atlassian.com → Security → API
   tokens*). Sign in with the **same Atlassian account** you use for Jira/Confluence.
2. Click **Create API token** (for a basic token) — or **Create API token with
   scopes** if your org requires scoped tokens.
3. Give it a recognizable **Label** (e.g. `otto-desktop`) and, if prompted, an
   **expiry date**. Atlassian Cloud tokens may be capped at up to ~1 year.
4. Click **Create**, then **Copy** the token. **It is shown only once** — copy it
   immediately. If you lose it you must create a new one.

The token represents **your** Atlassian user; Otto acts in Jira/Confluence as you,
inheriting exactly your permissions (see [§9 Security](#9-security) and
[§5 permissions](#5-permissions--scopes-the-token-needs)).

### Step 2 — Add the account in Otto

1. Open **Settings → Jira Accounts** and click **Add Account**.
2. Fill the four fields:

   | Field | What to enter | Notes |
   |---|---|---|
   | **Label** | A friendly name, e.g. `work jira` | Local display name only; required. |
   | **Base URL** | `https://your-domain.atlassian.net` | Your Atlassian **site root**. **Do not** add `/jira` or `/wiki` — Otto derives both REST roots from this. A trailing `/` (and a trailing `/wiki`, for Confluence) is stripped automatically. Required. |
   | **Email** | The email of your Atlassian account | This is the username half of Basic auth — it **must** be the email tied to the token, not a display name. Required. |
   | **API Token** | The token you copied in Step 1 | Write-only; never displayed again after saving. Required when adding. |
   | **Token expiry** *(optional)* | The token's expiry date (yyyy-mm-dd) | Purely a reminder: Otto badges the account **"expires soon"** within 14 days and **"expired"** past the date so you rotate before access lapses. It does not enforce anything. |

3. Click **Add Account**. Otto stores the token in the macOS Keychain under an
   opaque reference (`issueacct-<uuid>`) and persists only that reference plus the
   label, email, base URL, and optional expiry in SQLite.

### Step 3 — Test the connection

There is no dedicated "Test" button on the account form. Verify the connection by
exercising a read:

- **Jira:** open the issue picker (in Product or anywhere the issue search is
  surfaced), select this account, choose a project, and confirm the project list
  and issue results load. Behind the scenes this calls `GET /issue/projects` and
  `GET /issue/search` for the account.
- **Confluence:** open the Confluence picker, select this account, and confirm
  the space list (`GET /issue/confluence/spaces`) populates.

A 401/403 here means the token, email, or base URL is wrong — see
[§10 Troubleshooting](#10-troubleshooting).

### Editing & rotating

Right-click an account (or use the pencil icon) → **Edit**. You can change the
label, email, base URL, and expiry. The **API Token** field is **blank on edit** —
*leave it blank to keep the current token*; type a new value only to **rotate**.
On rotation Otto writes the new secret to the Keychain and deletes the old one.
**Delete** removes the account and its Keychain token.

---

## 4. Importing issues / pages (search by project / space)

You never type a key prefix. Pick a container, type whatever you remember, and Otto
builds the query.

### Jira — search by project

`GET /api/v1/issue/projects?account_id=<id>` lists the projects available to your
token (ordered by name). `GET /api/v1/issue/search` then runs a JQL query Otto
constructs from your input:

| Query string `q` (with `project=KEY`) | Interpreted as | JQL built |
|---|---|---|
| *(empty)* | Recency default | `project = "KEY" AND assignee = currentUser() ORDER BY updated DESC` |
| All digits (e.g. `5218`) | Ticket number | `project = "KEY" AND (key = "KEY-5218" OR summary ~ "5218*" OR text ~ "5218") ORDER BY updated DESC` |
| A full key (e.g. `KEY-5218`) | Exact key | `project = "KEY" AND key = "KEY-5218" ORDER BY updated DESC` |
| Free text (e.g. `login bug`) | Title/text search | `project = "KEY" AND (summary ~ "login bug*" OR text ~ "login bug") ORDER BY updated DESC` |

Without a `project`, search falls back to an unrestricted key-or-text JQL. Results
are paginated 25 at a time via `start_at` (cursor-style "load more"). Each result
carries `key`, `summary`, `status`, `issue_type`, and a `url`
(`<base>/browse/<KEY>`).

**Search endpoint resilience.** Otto calls the newer
`GET /rest/api/3/search/jql` first and **transparently falls back** to the classic
`GET /rest/api/3/search` if it is unavailable — so it works across Cloud rollouts.
The project listing likewise tries `GET /rest/api/3/project/search` (paginated) and
falls back to `GET /rest/api/3/project`.

### Confluence — search by space

`GET /api/v1/issue/confluence/spaces?account_id=<id>` lists current spaces (up to
200). `GET /api/v1/issue/confluence/search` then runs a CQL query Otto builds:

| Query string `q` (with optional `space=KEY`) | Interpreted as | CQL built |
|---|---|---|
| All digits (e.g. `12345`) | Page id lookup | `type=page and id=12345` |
| Free text (e.g. `onboarding guide`) | Title contains-match | `type=page and space="KEY" and title ~ "onboarding guide"` (the `space` clause is omitted when no space is given) |

Results (up to 25) carry `id`, `title`, `space_key`, and a browser `url`.

### What fields are pulled on import

- **Jira issue (`GET /issue/{account_id}/{key}`):** `summary`, `status`,
  `issue_type`, `assignee` (display name), and `description` — the ADF description
  is converted to **Markdown**. Plus the browse `url`.
- **Jira issue, full view (`GET /issue/{account_id}/{key}/full`):** everything
  above as Markdown, **plus** assignee/reporter objects (with avatar), priority,
  labels, **comments** (ADF→Markdown), **changelog** history, **attachments**
  (filename/mime/size/author), **links** (blocks/is-blocked-by, parent, subtasks,
  epic), an **estimate** (story points, else original time estimate), and **every
  non-empty field including custom fields** with their human display names
  (sourced from Jira's `expand=names`). Fetched with
  `?expand=changelog,names,renderedFields&fields=*all`.
- **Confluence page (engine-level `get_page`):** title, space key, body
  (storage-format XHTML → Markdown), current version number (needed for edits),
  and a browser URL. Fetched with `?expand=body.storage,version,space`.

---

## 5. Reading & commenting

### Reading

Imported content is presented as Markdown. The converters handle the common cases
faithfully:

- **ADF → Markdown** (Jira descriptions & comments): headings, paragraphs, bold,
  italic, inline code, links, bullet/ordered lists, code blocks (with language),
  hard breaks. Unknown nodes are recursed into best-effort.
- **Storage XHTML → Markdown** (Confluence bodies & comments): headings,
  paragraphs, `<br>`, `<pre>`, blockquotes, inline marks, links, nested lists,
  **tables** (→ GFM pipe tables, with `|` escaped), **`<ac:structured-macro>`**
  panels (`code` → fenced block; `info`/`note`/`warning`/`tip` → labeled
  blockquote; `status` → inline badge), and **images**
  (`<ac:image>`/`<ri:attachment>` → `![file](file)`). Unknown macros/tags are
  dropped silently, keeping their text.

**Attachments** are proxied through the daemon so your token never reaches the
browser: `GET /issue/{account_id}/{key}/attachment/{attachment_id}` streams the
bytes from Jira (`/rest/api/3/attachment/content/{id}`) and forwards the upstream
`Content-Type`, so images/PDFs render inline.

### Commenting back

The integration can **write comments**:

- **Jira:** `POST /api/v1/issue/{account_id}/{key}/comment` with `{"body": "…"}`.
  Your text is converted to **ADF** before posting (plain text → paragraphs;
  `- ` lines → bullet lists). Returns a `CommentRef` (`id` + optional `url`).
  Calls `POST /rest/api/3/issue/{key}/comment`.
- **Confluence** (engine-level): footer comments are posted by converting Markdown
  to storage XHTML and `POST`ing a `comment`-type content to
  `/wiki/rest/api/content`.

Typical uses for write-back are posting **open questions** or **generated test
cases** onto the artifact for the team to see — these are driven by the product
workflow in [`./product.md`](./product.md), which uses these same endpoints.

### Other write/admin operations (Jira)

Beyond comments, the Jira client also exposes status transitions, assignment, and
issue-type discovery (used by the product workflow):

| Operation | Endpoint (Otto) | Jira REST call |
|---|---|---|
| List transitions | `GET /issue/{account_id}/{key}/transitions` | `GET /rest/api/3/issue/{key}/transitions` |
| Apply transition | `POST /issue/{account_id}/{key}/transitions` `{"transition_id":"21"}` | `POST …/transitions` `{"transition":{"id":…}}` |
| List assignable users | `GET /issue/{account_id}/{key}/assignable` | `GET /rest/api/3/user/assignable/search?issueKey=…` |
| Assign / unassign | `PUT /issue/{account_id}/{key}/assignee` `{"account_id":"…"}` (`"-1"` unassigns) | `PUT /rest/api/3/issue/{key}/assignee` `{"accountId":…}` |
| Project issue types | `GET /issue/{account_id}/{project_key}/issue-types` | `GET /rest/api/3/project/{key}` (reads non-subtask `issueTypes[].name`) |

Issue creation and description updates exist at the engine level
(`POST /rest/api/3/issue`, `PUT /rest/api/3/issue/{key}`) and are driven by the
product workflow rather than this settings surface.

---

## 6. Multiple accounts / sites

You can connect **as many accounts as you like**. Common cases:

- **Multiple sites** — e.g. a work `https://acme.atlassian.net` and a personal
  `https://me.atlassian.net`, each with its own email + token.
- **Same site, different identities** — multiple tokens for the same site if you
  need to act as different Atlassian users.

Every read/write takes an explicit `account_id` (as a query param for the
list/search endpoints, or in the path for per-issue endpoints), so there is no
ambiguity about which site/identity a call uses. Each account is owner-scoped;
other users on a multi-user Otto install never see or use your accounts (root is
the only exception). The Jira and Confluence HTTP clients keep a process-wide
connection pool keyed by `base_url + auth_header`, so multiple accounts coexist
without cross-talk.

> **Provider note.** The stored `provider` enum currently has a single variant,
> `jira` (the UI sends `provider: "jira"` and the section is titled "Jira
> Accounts"). Confluence is **not** a separate account type — it is reached through
> the same Jira/Atlassian account's site, since Atlassian Cloud co-locates Jira and
> Confluence under one domain and one credential.

---

## 7. API / contract reference

All paths are relative to **`/api/v1`**; auth = **member** unless noted; mutating
account endpoints require the **owner**. Authoritative source: `docs/contracts/api.md`,
section **## Issue trackers (Jira / Confluence)**.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `GET /issue/accounts` | member | — | `IssueAccount[]` (own; token never present) |
| `POST /issue/accounts` | member | `CreateIssueAccountReq` | `IssueAccount` |
| `PATCH /issue/accounts/{id}` | member (owner) | `UpdateIssueAccountReq` | `IssueAccount` |
| `DELETE /issue/accounts/{id}` | member (owner) | — | `204` (Keychain token removed) |
| `GET /issue/projects?account_id=` | member | — | `IssueProject[]` |
| `GET /issue/search?account_id=&q=&project=&start_at=` | member | — | `IssueSummary[]` (JQL) |
| `GET /issue/confluence/spaces?account_id=` | member | — | `ConfluenceSpace[]` |
| `GET /issue/confluence/search?account_id=&q=&space=` | member | — | `ConfluencePageSummary[]` (CQL) |
| `GET /issue/{account_id}/{key}` | member | — | `IssueDetail` |
| `GET /issue/{account_id}/{key}/full` | member | — | `IssueFull` (description, comments, changelog, attachments, links, fields, estimate) |
| `GET /issue/{account_id}/{key}/transitions` | member | — | `JiraTransition[]` |
| `POST /issue/{account_id}/{key}/transitions` | member | `{"transition_id":"…"}` | `204` |
| `GET /issue/{account_id}/{key}/assignable` | member | — | `JiraUser[]` |
| `PUT /issue/{account_id}/{key}/assignee` | member | `{"account_id":"…"}` | `204` |
| `GET /issue/{account_id}/{key}/attachment/{attachment_id}` | member | — | attachment bytes (forwarded `Content-Type`) |
| `POST /issue/{account_id}/{key}/comment` | member | `{"body":"…"}` | `CommentRef` |
| `GET /issue/{account_id}/{project_key}/issue-types` | member | — | `string[]` (non-subtask type names) |

**Request bodies (`otto-core`):**

```jsonc
// CreateIssueAccountReq  (POST /issue/accounts)
{
  "provider": "jira",                       // only "jira" today
  "label": "work jira",
  "email": "you@company.com",               // Basic-auth username
  "base_url": "https://your-domain.atlassian.net",
  "token": "<atlassian API token>",         // write-only → Keychain
  "token_expires_at": "2026-12-31T00:00:00Z" // optional reminder; null = none
}

// UpdateIssueAccountReq  (PATCH /issue/accounts/{id}) — all fields optional
{
  "label": "…", "email": "…", "base_url": "…",
  "token": "…",            // non-empty → rotate; empty/absent → keep
  "token_expires_at": "…"  // absent → keep current
}
```

**Upstream Atlassian REST calls Otto makes:**

- **Jira REST v3** (`{base_url}/rest/api/3/…`): `project/search` (→ `project`
  fallback), `search/jql` (→ `search` fallback), `issue/{key}`,
  `issue/{key}/comment`, `issue/{key}/transitions`, `user/assignable/search`,
  `issue/{key}/assignee`, `attachment/content/{id}`, `project/{key}`, `issue`
  (create), with `?expand=changelog,names,renderedFields&fields=*all` on the full
  fetch.
- **Confluence REST v1** (`{base_url}/wiki/rest/api/…`): `space`,
  `content/search` (CQL), `content/{id}`, `content` (create page / comment),
  `content/{id}/child/comment`.

Every upstream request carries `Authorization: Basic base64(email:token)` and
`Accept: application/json`. Connection timeout is 10s; total per-request timeout
is 30s.

---

## 8. Capabilities & limitations

**You can:**

- Connect multiple Atlassian **Cloud** sites/identities, each with its own API
  token; rotate or delete a token at any time.
- Search a **Jira project** for issues by number, key, or free text — no key
  prefix required — with "load more" pagination.
- Search a **Confluence space** for pages by id or title.
- Import a Jira issue (summary/status/type/assignee/description) and a **full**
  issue view (comments, changelog, attachments, links, estimate, all custom
  fields) rendered as Markdown.
- Import a Confluence page body (storage XHTML → Markdown, incl. tables, panels,
  code, images).
- View attachments inline (proxied server-side so the token stays on the daemon).
- **Post comments** back to a Jira issue (text → ADF) and to a Confluence page
  (Markdown → storage XHTML).
- List/apply Jira **status transitions**, list **assignable users**, and
  **assign/unassign** an issue; discover a project's issue types.

**You cannot:**

- Use **OAuth** or **Jira Server / Data Center Personal Access Tokens (bearer
  PATs)** — only Atlassian **Cloud API tokens via Basic auth** are sent. (A
  self-hosted instance that accepts `email:token` Basic auth on `/rest/api/3`
  might work, but it is not a supported configuration.)
- Connect a **separate Confluence-only** credential — Confluence rides on the same
  site's account.
- Edit arbitrary Jira fields from the import surface (only comment, transition,
  assign here; create/update-description live in the product workflow).
- Have Otto **discover your site** for you — you must supply the base URL.
- Rely on **token-expiry enforcement** — the expiry field is a reminder only.
- See another user's accounts (owner-scoped; root excepted).

---

## 9. Security

- **Tokens live in the macOS Keychain.** On create/rotate, Otto writes the token
  to the Keychain under an opaque item name (`issueacct-<uuid>`) via the
  `SecretStore` (`otto-keychain`). The SQLite `issue_accounts` row stores **only**
  that `token_ref` plus the email, base URL, label, and optional expiry — never
  the token. On delete (or rotation) the old Keychain item is removed; on delete
  the orphaned secret is also cleaned up if account creation fails.
- **The token is never returned.** `IssueAccount.token_ref` is
  `#[serde(skip_serializing)]`, so the API and UI never echo it (the UI renders
  `token ••••••`).
- **Server-side only.** All Atlassian calls are made by the daemon. The
  attachment proxy keeps the `Authorization` header on the daemon side so token
  bytes never reach the webview.
- **Least privilege.** Because Otto acts as **you** via your token, scope access
  by the Atlassian account the token belongs to. Prefer a dedicated, clearly
  labeled token; if your org supports **scoped tokens**, grant only what §5
  requires; set an **expiry** and rotate. Revoke a leaked token at
  <https://id.atlassian.com/manage-profile/security/api-tokens> and rotate it in
  Otto's Edit dialog.
- **Owner isolation.** Ownership is enforced on every account-resolving handler;
  cross-user access is rejected with `403 Forbidden`.

---

### 5. Permissions / scopes the token needs

An Atlassian API token inherits **your user's permissions** — it cannot do more
than you can in the web UI. For the operations Otto performs, your Atlassian
account needs:

- **Jira — read:** *Browse Projects* on each project you search/import; *View
  Development Tools* is not required. To read attachments, permission to view them.
- **Jira — write (optional, for the product workflow / comments):** *Add Comments*;
  *Transition Issues* for the transitions you apply; *Assign Issues* / *Assignable
  User*; *Create Issues* and *Edit Issues* if you create/update issues.
- **Confluence — read:** *View* on the spaces you search/import.
- **Confluence — write (optional):** *Add Comments* / *Create* permission on the
  space if you post comments or create/update pages.

If you create a **scoped** token, the equivalent granular scopes are roughly
`read:jira-work` / `write:jira-work` and `read:confluence-content.all` /
`write:confluence-content` — but the simplest path is a classic (unscoped) token
on an account that already has the project/space access you need.

---

## 10. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| **401 Unauthorized** on first search | Wrong email, wrong/expired token, or token created for a different Atlassian account | Confirm the **email** matches the token owner exactly; regenerate the token at id.atlassian.com and re-enter it (Edit → paste new token). Otto Basic-auths `email:token`, so a username (not email) fails. |
| **403 Forbidden** from Atlassian | Token is valid but the user lacks project/space permission (or scoped token missing a scope) | Grant *Browse Projects* / space *View* (or the write permission you need) to the account; if scoped, add the scope. See §5. |
| **403 Forbidden** from Otto (not Atlassian) | You are not the account owner | Use your own account; only the owner (or root) may act through it. |
| **404 / empty project or space list** | Wrong base URL (e.g. you used a project URL or added `/jira` / `/wiki`) | Use the bare site root `https://your-domain.atlassian.net`. Otto strips trailing `/` and `/wiki`; it does not strip a path like `/jira`. |
| **Search returns nothing for a known ticket** | Typed text that doesn't match summary/text, or wrong project selected | Try the exact key (`PROJ-123`) or the ticket number with the right project selected; empty query returns your recently-updated issues in that project. |
| **`502 Bad Gateway` / "jira … request" errors** | Network/timeout reaching Atlassian, or Atlassian outage | Check connectivity; requests time out after 30s. Retry. |
| **"expired" / "expires soon" badge** | The optional expiry you entered has passed/approaches | Rotate the token and update (or clear) the expiry. Note this is a reminder only — an actually-expired token surfaces as 401. |
| **Captcha / 2FA / SSO prompt fails** | You tried to authenticate interactively, or used an SSO-only password | API tokens bypass interactive 2FA/captcha — always use an **API token**, never your account password. SSO does not change this; the token is the credential. |

### Cloud vs Server (PATs, 2FA/SSO)

Otto's clients are built for **Atlassian Cloud** and send **Basic
`email:token`** to Jira `/rest/api/3` and Confluence `/wiki/rest/api`. They do
**not** send a **bearer Personal Access Token**, so Jira **Server / Data Center**
PATs (which use `Authorization: Bearer <PAT>`) are **not supported**. If you are on
Server/DC, there is no supported path today. For Cloud, an **API token** is the
correct credential and it sidesteps interactive **2FA** and **SSO/captcha** flows
entirely — those gate browser logins, not token-based REST.

---

## 11. Related docs

- **[`./product.md`](./product.md)** — the Product (Jira/Confluence) **workflow**
  that imports a story/page through this integration and then runs analysis lenses,
  rewrites, test-case generation, multi-agent planning, and publishing (Jira
  stories, Confluence acceptance criteria). That workflow consumes the endpoints
  documented here.
- `docs/contracts/api.md` — authoritative API contract (section *## Issue trackers
  (Jira / Confluence)*).
- `README.md` — feature tour and crate map.
