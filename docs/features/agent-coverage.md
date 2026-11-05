# Agent coverage — what an agent can do in each module

How much of each Otto module an agent can operate itself, through the two
agent surfaces:

- **MCP tools**: the governed `otto.*` catalog
  (`crates/otto-mcp/src/outward/catalog.rs`; reads are on by default, writes
  are approval-gated and off by default) plus the session-only native tools of
  the stdio bridge (`crates/ottod/src/mcp_tools/catalog.rs`: DB explorer,
  canvas, workbench, browser, memory, usage).
- **UI control**: the `otto.ui_*` commands that drive the user's Otto window
  visibly (`docs/contracts/ui-commands.json`, see
  [agent-ui-control.md](agent-ui-control.md)).

**full**: the everyday read *and* write operations are covered. **partial**:
reads or a few writes only. **none**: nothing. Snapshot of 2026-11-05.

| Module | MCP tools | UI control | Notes |
|--------|-----------|------------|-------|
| agents (sessions) | full | none | open / message / wait / broadcast sessions, linked threads; no UI commands for the sessions page |
| api (REST client) | full | full | list / get / history / upsert / execute / automations; UI: drafts, curl import, env, send, save |
| aws | partial | partial | S3, SQS, EC2, Athena, EKS list, Logs; no RDS, no EC2 start/stop |
| brokers (Kafka) | full | full | clusters, topics, groups, consume, produce |
| browser | full | full | navigate, page, query, summarize, marks, login |
| canvas | partial | none | list / get / create / update scenes (native only); no delete, no UI commands |
| connections | partial | partial | connections listed and queried (Database); SSH / Redis sessions not drivable |
| database (DB Explorer) | full | full | schema tree, objects, read-only + approval-gated write queries; UI: tabs, run, page, explain, export |
| design-hall | partial | none | list / get / search / links, assist, link; no version approve (human-only by design), no UI commands |
| desktop | none | none | tray / floating bar |
| git | full | full | repos, status, PRs (create / comment / merge), review; UI: stage, commit, fetch, pull, push |
| help | none | none | |
| home | none | full | boxes and views via UI only |
| insights | none | none | |
| kubernetes | full | full | resources, describe, logs, top, health, actions, pod HTTP |
| loops (goal loops) | partial | partial | list + run; no create / edit |
| mcp (control plane) | none | none | governance stays human-only by design |
| panels | n/a | n/a | session side panels |
| personal-agents | full | full | **Agent admin** tools: templates, list / get / create (template + persona + schedule + primary) / update / delete, schedules add / update / remove, autonomy, Slack act-as-me (always a person's approval), run now, runs + report; plus the main agent's team tools and rooms. UI: `pa_*` |
| plugins | none | none | |
| product (stories) | partial | none | list / get story, context packet; no story create / analysis run |
| rooms (session rooms) | partial | none | agent rooms post / read / list |
| scheduled-tasks | full | partial | list / get / create / update / enable / run / delete / runs / report; UI: list, runs, run now, pause |
| settings | none | none | human-only by design (integrations, tokens, policy) |
| share | none | none | human-only by design |
| skills-eval / skills-lab | partial | none | bundled skill list only |
| snip | none | none | |
| threads | full | none | `thread_*` tools; no UI commands for the inbox |
| usage | partial | none | usage summary |
| vault | full | partial | list / read / search / graph / write / rename / delete; UI: list, open, search, write |
| workbench | full | none | list / get / write (native) |
| workflows | partial | partial | list / get / runs / run / cancel; no create or edit of a workflow graph |

Also covered: Jira / Confluence (search, read, create, edit, comment,
transition), Slack (as the bot or as the user), self-improvement, code-review
findings, memory search.

## Most valuable missing operations

- **Workflows**: create / edit a workflow graph (nodes, triggers) — today an
  agent can only run existing ones.
- **Goal loops**: create and edit a loop (goal, repo, budget), not just run it.
- **Product stories**: create / import a story and start its analysis,
  rewrite or test-case run.
- **Scheduled Tasks UI**: `sched_create` / `sched_edit` commands (the MCP
  tools exist; the visible form-filling path does not).
- **Sessions / Threads UI**: open, focus and type into a session or thread
  visibly (`agents`, `threads` have no UI commands).
- **Canvas**: delete a scene; UI commands to open and show a scene.
- **AWS**: RDS reads, EC2 start / stop / reboot (routes exist, Outward-tagged).
- **Rooms**: create a room and manage its members (agents can only post / read).
- **Insights / usage**: run an insights report and read it back.
- **Design Hall UI**: open an artifact and show a version diff.
