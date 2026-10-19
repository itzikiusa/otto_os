# Pele UX reviewers

Root counted 6 earlier IDs: 4 engineering and 2 UX. Pele’s 10 UX are those 2 plus the 8 below. The `01a14ffc` ten were started before that count and are extra read-only reviews. They are not a second set of 10.

No launch was refused. No extra engineering reviewers were started. Root’s 3 native plus these 4 plus root’s 13 reach 20 engineering.

## Counted (10)

| # | Scope | Reviewer | Report |
|---|---|---|---|
| 1 | Scheduled tasks, goal loops, workflow UI | `01a14ff8-76af-77e0-9d76-7730654b009c` | early notes in `iteration-01-automation-grok.md` |
| 2 | Swarm, personal agents, assistant UI | `01a14ff8-76af-77e0-9d76-774de2177cb5` | `/tmp/pw-rev-ui-agents.md` |
| 3 | Vault | `01a14ffd-280a-7bd1-80ef-bc6c5204bc7e` | `11-vault.md` (8.0) |
| 4 | Design Hall and Canvas | `01a14ffd-280a-7bd1-80ef-bc73e6b24f8d` | `12-design-hall-canvas.md` (Hall 8.4, Canvas 8.6) |
| 5 | Product | `01a14ffd-280a-7bd1-80ef-bc86f5ac6d16` | `13-product-story.md` (8.8) |
| 6 | Review, proof, Run with Otto | `01a14ffd-280a-7bd1-80ef-bc9a8a738fd6` | `14-review-proof-run.md` (8.6) |
| 7 | API client and browser | `01a14ffd-280a-7bd1-80ef-bca3e63a208d` | `15-api-browser.md` (API 8.9, browser 8.6) |
| 8 | Usage and insights (usage edits stay with Ronaldo) | `01a14ffd-280a-7bd1-80ef-bcb925e56b1d` | `16-usage-insights.md` (usage 9.0, insights 9.1) |
| 9 | Skills and plugins | `01a14ffd-280a-7bd1-80ef-bcc24f6f501f` | `17-skills-plugins.md` |
| 10 | Mobile and accessibility on Pele-owned modules | `01a14ffd-280a-7bd1-80ef-bcda592d52b0` | `18-mobile-a11y-owned.md` |

Reports 11–16 are written. Skills/plugins (`17`) and mobile accessibility (`18`) were still open when this roster was updated.

## Extra read-only ten (not the counted 10)

| Scope | Reviewer | Report | Score |
|---|---|---|---|
| Workflow editor | `01a14ffc-0019-7e61-a559-581c8082f16d` | `01-workflow-editor.md` | 8.4 |
| Scheduled tasks and goal loops | `01a14ffc-0019-7e61-a559-582cfbb54092` | `02-scheduled-tasks-goal-loops.md` | tasks 8.5, loops 8.0 |
| Personal agents, assistant, messaging | `01a14ffc-0019-7e61-a559-583c2a728c08` | `03-personal-agents-assistant-messaging.md` | 8.8 |
| Swarm and Mission Control | `01a14ffc-0019-7e61-a559-5844f4c9ea8e` | `04-swarm-mission-control.md` | swarm 8.4, mission control 8.6 |
| Vault, search, docs | `01a14ffc-0019-7e61-a559-58590255e02d` | `05-vault-search-docs.md` | 8.0 |
| Design Hall and Canvas | `01a14ffc-0019-7e61-a559-5869c43a21ff` | `06-design-hall-canvas.md` | hall 8.6, canvas 8.2 |
| Product stories | `01a14ffc-0019-7e61-a559-587f773237c4` | `07-product-story-refinement.md` | 7.1 |
| Review, proof, Run with Otto | `01a14ffc-0019-7e61-a559-588e1bbe5e3b` | `08-review-proof-run-with-otto.md` | 9.3 |
| API client, browser, integrations | `01a14ffc-0019-7e61-a559-589d06948312` | `09-api-browser-integrations.md` | 8.3 |
| Usage, insights, skills, plugins | `01a14ffc-0019-7e61-a559-58ab8608e53b` | `10-usage-insights-skills-plugins.md` | 8.8 |

Ronaldo’s 10 UX reviewers are confirmed by root. Their reports stay in `design-agy/`.

## Fixes applied from the counted UI reviews

Source only. No Cargo, no browser pass.

- Scheduled tasks: webhook error names `http://` and `https://`. A paused task still shows its last run. Proof and workflow-run chips open the pack and the run.
- Goal loops: a finished agent dot is a filled `--success`; a running one is a ring. An empty timeline says “Waiting on an agent.” when that is the phase. Launch requires a shell command when verification is a command. Define rows wrap at 640px.
- Workflow runs: the sub-agent chip counts failed tasks. Connection hover uses the accent. A running-list name can use the row width.
- Swarm: stopping goal verification asks first and uses Stop….
- Agent channels: removing a member asks, and the control meets the 36px target on a phone.
- Assistant: the disabled microphone is not rendered.
- Share: revoking one link asks. The role reads “View only” or “Can type”.
- Handover: archiving the source is named on the button and confirmed.
- Composer: Send and remove-image grow to 36px on a phone or coarse pointer.
- Canvas: the page title is the open scene’s name.
- Insights: a run that ends without a report is titled as unpublished, and opens its session. The generating banner wraps so Stop stays on a phone. The period control grows to 36px on a coarse pointer.

Usage, settings, shell, home, git, and Mission Control were not edited.
