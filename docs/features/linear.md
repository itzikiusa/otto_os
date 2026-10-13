# Linear Integration (Issue Trackers)

Linear sits behind the same issue interface as Jira. A Linear account is an
issue account with `provider = "linear"`, and every place that reads or writes
issues uses it unchanged:

- issue search and pickers
- Product import, publish and the story watcher
- agent sessions and the `otto.*` / `otto_*` issue tools
- workflows (`prepare_context`)
- runs and Finding → issue

The account you pick (or the only one you have) decides whether a call goes to
Jira or to Linear. Nothing asks you which tracker to use.

Code: `crates/otto-issues/src/linear.rs` is the GraphQL client.
`crates/otto-issues/src/tracker.rs` holds `Tracker`, the facade that picks a
client per account; every caller builds its client through
`Tracker::for_account`.

## 1. Connecting Linear

1. In Linear, open **Settings → Security & access → Personal API keys** and
   create a key. It starts with `lin_api_`.
2. In Otto, open **Settings → Integrations → Jira & Linear accounts**, click
   **Add account** and pick **Linear**.
3. Fill in the form:
   - **Label**: any name you like.
   - **API key**: the key from step 1.
   - **Workspace URL**: optional. It is only the link shown on the card; when
     left empty it is saved as `https://linear.app`.
   - Linear needs no email.
4. Click **Test**. It lists the teams the key can see ("Connected · 3 teams
   visible").

The key is stored in the macOS Keychain. The database keeps only an opaque
reference to it. A personal API key is sent as-is in `Authorization`. Any other
token is sent as `Bearer <token>`, so an OAuth access token works too.

## 2. How Jira concepts map

| Jira | Linear |
|---|---|
| Project (`key`) | Team (`key`, e.g. `ENG`) |
| Issue key `PROJ-12` | Identifier `ENG-12` |
| Transition | Workflow state. Moving there is `issueUpdate(stateId)`; the transition `id` is the state id, listed in board order |
| JQL | `IssueFilter` JSON object |
| Issue type | Label. Listings say `Issue`, or `Sub-issue` when the issue has a parent; creating with a type name applies the label of that name when one exists |
| Fix version | None (`fix_versions` is always empty) |
| Description / comments (ADF) | Markdown, stored as-is |
| Assignable users | Members of the issue's team (`"-1"` unassigns) |
| Dev status | PR attachments (GitHub / GitLab / Bitbucket); no branches or commits |

`get_issue_full` also returns team, project, cycle, milestone, due date, git
branch name, created/updated and attachment links under `fields`. Parent,
sub-issues and relations (`blocks` / `is blocked by`, `duplicates`,
`relates to`) appear under `links`. Status, assignee, priority and title
changes appear under `history`.

## 3. Searching

The search box and `GET /api/v1/issue/search` accept the same inputs as for
Jira:

- **Text or key** (`q`). Empty shows your recently updated assigned issues. An
  identifier, or a bare number when a team is selected, opens that issue.
  Anything else runs Linear full-text search, optionally inside one team
  (`project=<team key>`).
- **Linear filter** (`filter`, raw mode). Any
  [IssueFilter](https://linear.app/developers/filtering), newest activity
  first, for example:

  ```json
  {"team":{"key":{"eq":"ENG"}},"state":{"type":{"eq":"started"}}}
  {"assignee":{"isMe":{"eq":true}},"state":{"type":{"nin":["completed","canceled"]}}}
  {"cycle":{"isActive":{"eq":true}},"labels":{"name":{"eq":"Bug"}}}
  ```

  The **Examples…** menu fills the selected team in. The cheat sheet lists the
  fields, comparators, state types and relative dates (`"-P7D"`).
- `query` is a provider-neutral raw query. It is read in the account's own
  language: JQL for Jira, filter JSON for Linear.

Sending JQL to a Linear account, or a `filter` to a Jira account, gets **400**
with a message that names the account's tracker. No upstream call is made.

Agents pass the same arguments to `otto.search_issues` / `otto_search_issues`:
`filter` (an object, or the JSON as text), `query`, `project` (team key),
`limit` (≤ 100) and `start_at`.

Paging: Linear is cursor-paginated. The daemon keeps the offset-based
`start_at` contract and memoises the cursor at each offset for 10 minutes, so
"Load more" resumes instead of re-walking from the first page.

## 4. What does not apply to Linear

| Operation | Behaviour on a Linear account |
|---|---|
| Confluence routes (`/issue/confluence/...`), Confluence import / RFC publish, run sources | **400**: "… is a Linear account — Confluence needs a Jira (Atlassian) account". The Product dialogs hide Confluence for Linear accounts |
| Attachment download (`…/attachment/{id}`) | **400**. Linear attachments are links; their URLs are in the issue's `fields` |
| Generic field editor (`…/editmeta`, `PUT …/fields`) | `editmeta` returns `[]`; `PUT …/fields` is **400**. Status, assignee, title and description have their own controls and work |
| Plugin host Jira credentials (`/plugin-host/jira/...`) | Linear accounts are not listed and their credentials are not returned. Those routes hand out Atlassian `email:token` pairs only |

## 5. Troubleshooting

- **"authentication failed — check the account's API key"**: the key was
  revoked or mistyped. Edit the account and paste a new one.
- **"Entity not found"**: the identifier doesn't exist in that workspace, or
  the key's user can't see the team.
- **A filter is rejected with "Linear filter is a JSON IssueFilter object"**:
  the raw box holds JQL or text. Switch Match to **Text or key**, or write the
  filter as JSON.
- **The filter is valid JSON but Linear returns a validation error**: the
  field or comparator does not exist on `IssueFilter`. The error message names
  it. Check the cheat sheet or Linear's filtering reference.

## 6. Related docs

- [Jira & Confluence](./jira-confluence.md): the same interface for
  Atlassian, plus multi-account behaviour.
- [API contract](../contracts/api.md) → "Issue trackers".
