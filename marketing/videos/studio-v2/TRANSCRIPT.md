# Otto walkthroughs

Original illustrated demonstrations with fictional Atlas data. Narration is synthetic (Andrew, en-US); music and effects are original procedural compositions.

## Your workspace

Start an agent, find your bearings, and follow the work.

### Start a session

Source: `ui/src/modules/agents/NewSession.svelte` · [Feature guide](../../../docs/features/agent-sessions.md)

1. Open Agents and create a session. Choose your provider, model, and repository.
2. Give the agent a concrete task. Its terminal stays live while it works.
3. Split or tile sessions to compare work. Reopen a session when you need to continue.

### Follow the work

Source: `ui/src/modules/mission-control/MissionControlPage.svelte` · [Feature guide](../../../docs/features/mission-control.md)

1. Open Mission Control to see related work in one place.
2. Select a work item to inspect its status, links, and evidence.
3. Follow the linked session or review when you need to act.

### Make room to think

Source: `ui/src/lib/sidebar.ts` · [Feature guide](../../../docs/features/rtl-and-responsive.md)

1. Open Settings, then Appearance, to choose your theme and arrange the sidebar.
2. Keep the pages you use most within reach. Hide entries you rarely need.
3. Press Command K to find a command and move straight to the next task.

## From change to proof

Inspect a change, review it, and follow the evidence to approval.

### Inspect the change

Source: `ui/src/modules/git/GitPage.svelte` · [Feature guide](../../../docs/features/git.md)

1. Open Git and choose your repository. Inspect the working changes before staging.
2. Select a file to read its diff. Stage only the changes you intend to commit.
3. Use the branch graph and pull requests to keep the change in context.

### Bring different lenses

Source: `ui/src/modules/git/ReviewPanel.svelte` · [Feature guide](../../../docs/features/code-review.md)

1. Open Review for a pull request or your working tree. Choose review lenses and providers.
2. Open a reviewer session to inspect its work. Triage the collected findings.
3. Hand a finding to an agent, then verify the fix before resolving it.

### Read the evidence

Source: `ui/src/modules/proof/ProofPage.svelte` · [Feature guide](../../../docs/features/proof-packs.md)

1. Open the linked Proof Pack. Inspect test output, snapshots, and attached artifacts.
2. Check the derived status. Missing evidence or a failed check still needs attention.
3. Use the evidence to decide whether the work is ready for approval.

### Start from the task

Source: `ui/src/modules/run-with-otto/RunLauncher.svelte` · [Feature guide](../../../docs/features/run-with-otto.md)

1. Open Run with Otto. Choose a source item, repository, provider, and model.
2. Follow the run through implementation, evidence, and review. Inspect anything that needs attention.
3. At the approval gate, review the result before allowing the pull request draft.

## From idea to context

Refine a story, explore the web, and preserve what the team learns.

### Refine the story

Source: `ui/src/modules/product/ProductPage.svelte` · [Feature guide](../../../docs/features/product.md)

1. Import a Jira issue or Confluence page into Product. Open its analysis.
2. Resolve the questions, inspect the rewrite, and refine the test cases and plan.
3. Create a mockup when a screen helps. Inject the refined context into an agent.

### Make the idea visible

Source: `ui/src/modules/canvas/CanvasPage.svelte` · [Feature guide](../../../docs/features/canvas.md)

1. Open Canvas and create a scene. Choose Excalidraw, Mermaid, or D2.
2. Describe the flow to the agent. Watch the scene update as the source changes.
3. Refine the diagram in the conversation until it explains the idea clearly.

### Research with context

Source: `ui/src/modules/browser/BrowserView.svelte` · [Feature guide](../../../docs/features/browser.md)

1. Open Browser and navigate to a reference. Choose Reader, or Live in the desktop app.
2. Mark the passages or elements that matter. Add a note about what to investigate.
3. Send the marks into a session, or save the page and notes to Vault.

### Keep the knowledge

Source: `ui/src/modules/vault/VaultPage.svelte` · [Feature guide](../../../docs/features/vault.md)

1. Open Vault and select a vault folder. Create or open a note.
2. Link related notes and use search to find the knowledge you need.
3. Keep decisions, references, and runbooks connected to the work they explain.

## Work that keeps moving

Choose the right kind of automation, with clear limits and visible progress.

### Build an agent team

Source: `ui/src/modules/swarm/SwarmPage.svelte` · [Feature guide](../../../docs/features/agent-swarm.md)

1. Open Swarm and create a team. Give each role a clear responsibility.
2. Start the project and follow assignments on the board and run view.
3. Open an agent session when work needs direction. Review the team result.

### Give iteration a boundary

Source: `ui/src/modules/loops/LoopsPage.svelte` · [Feature guide](../../../docs/features/goal-loops.md)

1. Create a Goal Loop with a concrete goal, acceptance criteria, and budget.
2. Follow each iteration as the agent plans, executes, and evaluates its result.
3. Inspect the evidence and stop or redirect the loop when the work needs judgment.

### Connect the steps

Source: `ui/src/modules/workflows/WorkflowsPage.svelte` · [Feature guide](../../../docs/features/workflows.md)

1. Open Workflows and build a graph from the steps your process needs.
2. Connect agent work, conditions, and approval nodes. Choose how the workflow starts.
3. Run it and inspect each node. Use the run view to investigate a failed step.

### Put a report on a schedule

Source: `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte` · [Feature guide](../../../docs/features/scheduled-tasks.md)

1. Create a Scheduled Task with a prompt, provider, and schedule.
2. Choose the timezone and report destination. Check when it will run next.
3. Inspect the generated report and execution history before relying on the routine.

### Meet your personal agent

Source: `ui/src/modules/personal-agents/PersonalAgentsPage.svelte` · [Feature guide](../../../docs/features/personal-agents.md)

1. Open Personal Agents and create a named persona. Pin its provider and model.
2. Add schedules and useful memory. Chat with the agent whenever you need it.
3. Inspect its reports and shared rooms to follow the conversations between agents.

## Follow the data

Connect to a system, inspect a record, and test the contract around it.

### Start with a connection

Source: `ui/src/modules/connections/ConnectionForm.svelte` · [Feature guide](../../../docs/features/connections-ssh-sftp.md)

1. Open Connections to find saved terminals, databases, and Kafka clusters.
2. Choose a profile and connect. Use the configured SSH tunnel when the system requires one.
3. For SSH, open the terminal or SFTP browser to inspect files on the host.

### Ask a precise question

Source: `ui/src/modules/database/DatabasePage.svelte` · [Feature guide](../../../docs/features/database-explorer.md)

1. Open a database from Connections. Select its database and inspect the schema.
2. Write a query, or use the assistant to draft one. Read it before running.
3. Inspect the results. Use the grid, JSON viewer, and export for a closer look.

### Follow the event

Source: `ui/src/modules/brokers/BrokersPage.svelte` · [Feature guide](../../../docs/features/message-brokers.md)

1. Open a Kafka cluster from Connections. Select a topic and inspect its partitions.
2. Choose where to start reading and fetch a bounded set of messages.
3. Inspect a message key, headers, and payload to follow the event through the system.

### Check the response

Source: `ui/src/modules/api/ApiPage.svelte` · [Feature guide](../../../docs/features/api-client.md)

1. Open API and choose or create a request. Set its method, URL, and environment.
2. Review headers, authentication, and body before sending the request.
3. Inspect the response, then save the request in a collection for next time.

## Operate the cloud

Move from an AWS account to a workload and the logs that explain it.

### Choose the right account

Source: `ui/src/modules/aws/AwsPage.svelte` · [Feature guide](../../../docs/features/aws-console.md)

1. Open AWS and select the account you intend to inspect. Complete sign in if required.
2. Choose the region and service. Browse buckets, queues, instances, or databases.
3. Keep the account and region visible as you move between resources.

### Read the resource signals

Source: `ui/src/modules/aws/MetricsPanel.svelte` · [Feature guide](../../../docs/features/aws-console.md)

1. Open an AWS resource and inspect its available metrics. Choose the time range.
2. Read the chart alongside the resource details to narrow the investigation.
3. Use the related service view when you need to inspect the underlying resource.

### Find the workload

Source: `ui/src/modules/kubernetes/ResourceDrawer.svelte` · [Feature guide](../../../docs/features/kubernetes-console.md)

1. Open Kubernetes and select a saved cluster, or import an EKS cluster from AWS.
2. Choose a namespace you can access. Find the workload and open its details.
3. Use the Pods tab to inspect the pods belonging to that workload.

### Read what happened

Source: `ui/src/modules/kubernetes/WorkloadPods.svelte` · [Feature guide](../../../docs/features/kubernetes-console.md)

1. Open the workload Logs tab to inspect output from its pods.
2. Follow new lines and narrow the output to the event you are investigating.
3. Use pod details, events, or an exec session when the next step needs deeper inspection.

## Improve the system

Tune agent tools and skills, then understand the work they produce.

### Choose the tools agents use

Source: `ui/src/modules/mcp/McpPage.svelte` · [Feature guide](../../../docs/features/mcp-control-plane.md)

1. Open MCP Control Plane to inspect connected servers and their tools.
2. Review which capabilities an agent can access and how policies govern calls.
3. Use the audit view to investigate a governed tool invocation.

### Make good practice reusable

Source: `ui/src/modules/settings/SkillsLibrary.svelte` · [Feature guide](../../../docs/features/skills-library.md)

1. Open the skill library to inspect installed skills and their versions.
2. Read the instructions and usage scope before choosing a skill for a task.
3. Use focused skills to make your expectations explicit and reusable.

### Evaluate before promoting

Source: `ui/src/modules/skills-lab/SkillsLabPage.svelte` · [Feature guide](../../../docs/features/skills-evaluator.md)

1. Open Skills Lab to browse, edit, evaluate, or review a skill.
2. Run a benchmark with a concrete task and inspect the validation results.
3. Compare runs and evidence before promoting the version you want to keep.

### Understand cost and progress

Source: `ui/src/modules/usage/UsagePage.svelte` · [Feature guide](../../../docs/features/usage-and-cost.md)

1. Open Usage and choose a period to inspect tokens and cost across providers.
2. Use the breakdowns to understand which models and sessions account for the usage.
3. Open Insights for a report, then inspect its evidence and suggested actions.

### Extend your workspace

Source: `ui/src/modules/settings/PluginsSettings.svelte` · [Feature guide](../../../docs/features/plugins.md)

1. Open Plugins to inspect installed extensions and their configuration.
2. Choose a plugin to review its capabilities and current status.
3. Open its page from the sidebar when it provides a workspace tool.

## Take Otto with you

Keep the conversation and context close, across windows and devices.

### Continue the conversation

Source: `ui/src/modules/settings/Channels.svelte` · [Feature guide](../../../docs/features/channels-slack-telegram.md)

1. Connect Slack or Telegram in Channels settings and configure the bridge.
2. Start from a thread and send the agent a clear request.
3. Follow the responses in the same conversation, or open the session in Otto.

### Bring a session with you

Source: `ui/src/modules/share/SharePage.svelte` · [Feature guide](../../../docs/features/remote-mobile-access.md)

1. Enable remote access when you want to use Otto from a phone or tablet.
2. For a guest, create a scoped session link and choose viewer or editor access.
3. Set an expiry and revoke the link when it is no longer needed.

### Give each task a window

Source: `docs/features/multi-window.md` · [Feature guide](../../../docs/features/multi-window.md)

1. Choose File, New Window to open another Otto window.
2. Keep a session in one window and the related review or data in another.
3. Arrange the windows for the task. Otto restores your windows on relaunch.

### Show exactly what you mean

Source: `docs/features/snipping-tool.md` · [Feature guide](../../../docs/features/snipping-tool.md)

1. Press Command Control Shift Two and drag to capture a region.
2. Annotate the detail you want the agent to notice. The image is on your clipboard.
3. Paste it into the session and explain what needs to change.

