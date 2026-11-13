// Shot lists per chapter, on the beat grid (120 BPM: a beat is 15 frames). A
// chapter's shots must add up to its bars × 4 beats (script/chapters.json);
// layout() refuses anything else. Times inside a shot (camera keys `b`,
// callouts `b`/`until`) are beats from the shot's cut.
// Coordinates are normalized to the captured page (1600×900 CSS px @2x):
// x 0..1 left→right, y 0..1 top→bottom.
// Stills/clips come from scripts/capture.mjs → public/capture/ and the Rooms
// clips from scripts/capture-rooms.mjs (or scripts/reuse-rooms.mjs: the same
// footage, cropped clear of the published movie's caption band — hence the
// Rooms coordinates below are in that crop) → public/capture/rooms/.
import type { ChapterSpec } from './types';

const cap = (f: string) => `capture/${f}`;
const room = (f: string) => `capture/rooms/${f}`;

export const SCENES: Record<string, ChapterSpec> = {
  intro: { id: 'intro', group: 'Shell', title: 'Meet Otto', shots: [{ kind: 'custom', id: 'intro', beats: 16 }] },

  // 3 bars = 12 beats
  home: {
    id: 'home',
    group: 'Work',
    title: 'Home',
    kicker: 'Your desktop for every agent',
    shots: [
      {
        kind: 'screen',
        src: cap('home.jpg'),
        beats: 8,
        label: 'Live sessions, dashboards, usage and findings',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2.5, x: 0.45, y: 0.32, z: 1.35 },
        ],
        callouts: [
          { b: 0.75, until: 2.5, x: 0.07, y: 0.475, label: 'Grouped sidebar', side: 'right' },
          { b: 3, x: 0.25, y: 0.3, label: 'Live sessions', side: 'bottom' },
          { b: 4.5, x: 0.65, y: 0.32, label: 'Dashboards & usage', side: 'bottom' },
        ],
      },
      {
        kind: 'screen',
        src: cap('home.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Light, dark and right-to-left, one design system',
        wipeTo: { src: cap('home-light.jpg'), b0: 0.5, b1: 2.5 },
        cam: [{ b: 0, x: 0.5, y: 0.5, z: 1.06 }],
      },
    ],
  },

  // 4 bars = 16 beats
  'command-bar': {
    id: 'command-bar',
    group: 'Shell',
    title: 'The command bar',
    kicker: 'Type or speak. ⌘K from anywhere.',
    shots: [
      {
        kind: 'screen',
        src: cap('cmdk.mp4'),
        from: 0.6,
        rate: 2,
        beats: 5,
        label: 'Run any command from one bar',
        cam: [
          { b: 0, x: 0.5, y: 0.66, z: 1.3 },
          { b: 3.6, x: 0.5, y: 0.5, z: 1 },
        ],
        callouts: [
          { b: 0.3, until: 3.2, x: 0.33, y: 0.93, label: 'Press', keys: '⌘ K', side: 'top' },
          { b: 4, x: 0.5, y: 0.985, label: 'Docks into the status bar', side: 'top' },
        ],
      },
      {
        kind: 'screen',
        src: cap('cmdk.mp4'),
        from: 7,
        rate: 2.2,
        beats: 4,
        transition: 'push',
        label: 'Jump straight to any module or ask Otto',
        cam: [
          { b: 0, x: 0.5, y: 0.74, z: 1.35 },
          { b: 2.6, x: 0.55, y: 0.78, z: 1.45 },
        ],
      },
      { kind: 'custom', id: 'desktop', beats: 7, label: '⌥Space from any app, the menu bar, pop-out windows' },
    ],
  },

  // 3 bars = 12 beats
  agents: {
    id: 'agents',
    group: 'Work',
    title: 'Agents',
    kicker: 'Claude Code, Codex, Antigravity and shells as real sessions',
    shots: [
      {
        kind: 'screen',
        src: cap('agents-tab.jpg'),
        beats: 4,
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1, x: 0.4, y: 0.4, z: 1.25 },
        ],
        callouts: [{ b: 1, x: 0.07, y: 0.24, label: 'Every session, live', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('agents-tiled.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Tiled: see every agent at once',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.2 },
          { b: 1.5, x: 0.55, y: 0.5, z: 1 },
        ],
      },
      {
        kind: 'screen',
        src: cap('agents-broadcast.jpg'),
        beats: 4,
        transition: 'whip',
        label: 'Broadcast one prompt to many agents',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.1 },
          { b: 1.5, x: 0.5, y: 0.5, z: 1.38 },
        ],
      },
    ],
  },

  // 6 bars = 24 beats — Threads on the structured runtime (a Codex thread
  // driven by the capture's scripted app-server, scripts/lib/demo-codex.mjs;
  // the Claude it asks is scripts/lib/demo-claude.mjs)
  threads: {
    id: 'threads',
    group: 'Work',
    title: 'Threads',
    kicker: 'Agents on the structured runtime: plans, commands and diffs, live',
    shots: [
      {
        kind: 'screen',
        src: cap('threads-runtime.mp4'),
        from: 9,
        rate: 1.8,
        beats: 8,
        label: 'Start a Codex thread and watch it work',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.5 },
          { b: 2.5, x: 0.55, y: 0.3, z: 1.5 },
        ],
        callouts: [{ b: 3.5, until: 7.5, x: 0.31, y: 0.17, label: 'Plan · commands · edits, streamed', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('threads-approval.jpg'),
        beats: 4,
        transition: 'push',
        label: 'It asks before anything outward-facing',
        cam: [
          { b: 0, x: 0.6, y: 0.45, z: 1.2 },
          { b: 1, x: 0.5, y: 0.52, z: 1.65 },
        ],
        callouts: [{ b: 1.5, x: 0.335, y: 0.55, label: 'Approve inline', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: cap('threads-ask-chip.jpg'),
        beats: 4,
        transition: 'whip',
        label: 'Ask another agent from inside the thread',
        cam: [
          { b: 0, x: 0.6, y: 0.7, z: 1.2 },
          { b: 1, x: 0.45, y: 0.86, z: 1.7 },
        ],
        callouts: [{ b: 1.5, x: 0.27, y: 0.87, label: 'Ask Claude Opus', side: 'top' }],
      },
      {
        kind: 'screen',
        src: cap('threads-ask.jpg'),
        beats: 8,
        transition: 'push',
        label: 'Its answer arrives as a live card, and nests under the thread',
        cam: [
          { b: 0, x: 0.55, y: 0.5, z: 1.1 },
          { b: 1, x: 0.5, y: 0.5, z: 1.5 },
          { b: 4.5, x: 0.22, y: 0.2, z: 1.7 },
        ],
        callouts: [
          { b: 1.5, until: 4.5, x: 0.45, y: 0.566, label: 'A child thread on Claude', side: 'top' },
          { b: 5, x: 0.19, y: 0.187, label: 'Nested in the sidebar', side: 'right' },
        ],
      },
    ],
  },

  // 4 bars = 16 beats — Council: one prompt, two seats, then a judge
  council: {
    id: 'council',
    group: 'Work',
    title: 'Council',
    kicker: 'One prompt to several agents, then a judge',
    shots: [
      {
        kind: 'screen',
        src: cap('council-stage.jpg'),
        beats: 4,
        label: 'Ask several: Claude Opus and GPT side by side',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.6 },
          { b: 1, x: 0.5, y: 0.52, z: 2 },
        ],
        callouts: [{ b: 1.5, x: 0.43, y: 0.548, label: 'Two seats', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: cap('council.jpg'),
        beats: 6,
        transition: 'push',
        label: 'Each seat answers in its own column',
        cam: [
          { b: 0, x: 0.58, y: 0.4, z: 1.05 },
          { b: 1.5, x: 0.58, y: 0.3, z: 1.2 },
        ],
      },
      {
        kind: 'screen',
        src: cap('council-synthesis.jpg'),
        beats: 6,
        transition: 'whip',
        label: 'The judge: agreement, disagreement, a pick',
        cam: [
          { b: 0, x: 0.6, y: 0.6, z: 1.1 },
          { b: 1.5, x: 0.5, y: 0.73, z: 1.5 },
        ],
      },
    ],
  },

  // 4 bars = 16 beats — verse
  loops: {
    id: 'loops',
    group: 'Automate',
    title: 'Goal Loops',
    kicker: 'Plan, execute and evaluate until every acceptance check passes',
    shots: [
      {
        kind: 'screen',
        src: cap('loops.jpg'),
        beats: 16,
        label: 'Goal Loops: autonomous iterations with hard budgets',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 2.5, x: 0.45, y: 0.3, z: 1.45 },
          { b: 8, x: 0.32, y: 0.74, z: 1.45 },
        ],
        callouts: [
          { b: 1.5, until: 7, x: 0.53, y: 0.24, label: 'Plan → execute → evaluate', side: 'bottom' },
          { b: 9, x: 0.31, y: 0.71, label: 'Machine-checked acceptance criteria', side: 'right' },
        ],
      },
    ],
  },

  // 4 bars = 16 beats
  workflows: {
    id: 'workflows',
    group: 'Automate',
    title: 'Workflows and Scheduled Tasks',
    kicker: 'Pipelines and schedules in one place',
    shots: [
      {
        kind: 'screen',
        src: cap('workflows.jpg'),
        beats: 8,
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.42, y: 0.3, z: 1.3 },
        ],
        callouts: [{ b: 2, x: 0.31, y: 0.36, label: 'Paused for your approval', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('scheduled-tasks.jpg'),
        beats: 8,
        transition: 'push',
        label: 'Scheduled tasks run agents on a cadence and report back',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.08 },
          { b: 1.5, x: 0.45, y: 0.22, z: 1.45 },
        ],
      },
    ],
  },

  // 4 bars = 16 beats — a personal agent's Chat tab is a runtime thread
  'personal-agents': {
    id: 'personal-agents',
    group: 'Automate',
    title: 'Personal Agents',
    kicker: 'Agents with a soul, a schedule and a team, chatting on Threads',
    shots: [
      {
        kind: 'screen',
        src: cap('pa-chat.mp4'),
        from: 0.2,
        rate: 1.15,
        beats: 12,
        label: 'Chat with your agent: a thread on the structured runtime',
        cam: [
          { b: 0, x: 0.53, y: 0.78, z: 1.4 },
          { b: 2.5, x: 0.5, y: 0.45, z: 1.42 },
        ],
        callouts: [
          { b: 10.5, until: 12, x: 0.25, y: 0.165, label: 'Its work folds to one line', side: 'right' },
        ],
      },
      {
        kind: 'screen',
        src: cap('personal-agents.jpg'),
        beats: 4,
        transition: 'whip',
        label: 'Each agent keeps its routines and history',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.1 },
          { b: 1, x: 0.48, y: 0.22, z: 1.45 },
        ],
      },
    ],
  },

  // 4 bars = 16 beats — chorus 2
  git: {
    id: 'git',
    group: 'Build',
    title: 'Git and AI review',
    kicker: 'Commit graph, work in progress, PRs and multi-lens review',
    shots: [
      {
        kind: 'screen',
        src: cap('git.jpg'),
        beats: 6,
        label: 'A commit graph with your work in progress',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.4, y: 0.38, z: 1.3 },
        ],
        callouts: [
          { b: 1.5, x: 0.44, y: 0.18, label: 'Work in progress', side: 'right' },
          { b: 3.5, x: 0.21, y: 0.43, label: 'Branches, stashes and worktrees', side: 'right' },
        ],
      },
      {
        kind: 'screen',
        src: cap('git-wip.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Stage, commit and draft the PR description with AI',
        cam: [
          { b: 0, x: 0.7, y: 0.4, z: 1.15 },
          { b: 1, x: 0.76, y: 0.35, z: 1.45 },
        ],
      },
      {
        kind: 'screen',
        src: cap('git-review.jpg'),
        beats: 6,
        transition: 'whip',
        label: 'Multi-agent AI review with tracked findings and proof',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 1.5, x: 0.5, y: 0.35, z: 1.35 },
        ],
      },
    ],
  },

  // 5 bars = 20 beats — Design Hall, the one home for designs
  product: {
    id: 'product',
    group: 'Build',
    title: 'Product and Design Hall',
    kicker: 'Stories and designs, one home',
    shots: [
      {
        kind: 'screen',
        src: cap('product-design.jpg'),
        beats: 4,
        label: "A story's Design tab lists its designs",
        cam: [
          { b: 0, x: 0.55, y: 0.4, z: 1.08 },
          { b: 1, x: 0.52, y: 0.38, z: 1.4 },
        ],
      },
      {
        kind: 'screen',
        src: cap('design-lobby.jpg'),
        beats: 6,
        transition: 'push',
        label: 'Design Hall: one prompt, every design',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.45, y: 0.35, z: 1.25 },
        ],
        callouts: [
          { b: 1, until: 3, x: 0.45, y: 0.13, label: 'Describe it, Generate', side: 'bottom' },
          { b: 3, x: 0.255, y: 0.23, label: 'Grouped by story', side: 'bottom' },
        ],
      },
      {
        kind: 'screen',
        src: cap('design-versions.jpg'),
        beats: 5,
        transition: 'whip',
        label: 'Autosaved, with every version kept',
        cam: [
          { b: 0, x: 0.6, y: 0.4, z: 1.05 },
          { b: 1, x: 0.86, y: 0.25, z: 1.6 },
        ],
      },
      {
        kind: 'screen',
        src: cap('design-compare.jpg'),
        beats: 5,
        transition: 'push',
        label: 'Compare two versions, then restore or approve',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.05 },
          { b: 1.5, x: 0.5, y: 0.45, z: 1.25 },
        ],
      },
    ],
  },

  // 3 bars = 12 beats
  vault: {
    id: 'vault',
    group: 'Build',
    title: 'Vault and Skills Lab',
    kicker: 'Linked markdown docs, and one home for agent skills',
    shots: [
      {
        kind: 'screen',
        src: cap('vault.jpg'),
        beats: 6,
        label: 'Vault: linked notes, backlinks and live context',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.2 },
          { b: 1.5, x: 0.55, y: 0.3, z: 1.4 },
        ],
        callouts: [{ b: 2, x: 0.62, y: 0.31, label: 'Links and backlinks', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('skills-eval.jpg'),
        beats: 6,
        transition: 'push',
        label: 'Skills Lab: review, evaluate and edit your skills',
        cam: [
          { b: 0, x: 0.45, y: 0.45, z: 1.05 },
          { b: 1, x: 0.35, y: 0.32, z: 1.38 },
        ],
      },
    ],
  },

  // 4 bars = 16 beats — verse 2 (a throwaway local ClickHouse, capture.mjs)
  database: {
    id: 'database',
    group: 'Infrastructure',
    title: 'Database Explorer',
    kicker: 'MySQL · Postgres · MongoDB · Redis · ClickHouse',
    shots: [
      {
        kind: 'screen',
        src: cap('db-query.jpg'),
        beats: 5,
        label: 'Query and read the results in a grid',
        cam: [
          { b: 0, x: 0.55, y: 0.4, z: 1.1 },
          { b: 1.5, x: 0.6, y: 0.55, z: 1.35 },
        ],
      },
      {
        kind: 'screen',
        src: cap('db-readonly.jpg'),
        beats: 4,
        transition: 'push',
        label: 'A read-only connection refuses writes',
        cam: [
          { b: 0, x: 0.6, y: 0.4, z: 1.15 },
          { b: 1, x: 0.64, y: 0.45, z: 1.6 },
        ],
      },
      {
        kind: 'screen',
        src: cap('db-history.jpg'),
        beats: 4,
        transition: 'whip',
        label: 'History across every connection',
        cam: [
          { b: 0, x: 0.4, y: 0.3, z: 1.3 },
          { b: 1, x: 0.24, y: 0.2, z: 2 },
        ],
      },
      {
        kind: 'screen',
        src: cap('db-processes.jpg'),
        beats: 3,
        transition: 'push',
        label: 'What is running, in the process list',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.25 },
          { b: 1, x: 0.5, y: 0.5, z: 1.7 },
        ],
      },
    ],
  },

  // 4 bars = 16 beats
  infra: {
    id: 'infra',
    group: 'Infrastructure',
    title: 'Connections and cloud',
    kicker: 'SSH, AWS, Kubernetes, APIs and the web',
    shots: [
      { kind: 'screen', src: cap('connections.jpg'), beats: 3, card: { title: 'Connections', sub: 'SSH, databases, sections' }, cam: [{ b: 0, x: 0.2, y: 0.3, z: 1.9 }, { b: 1, x: 0.18, y: 0.26, z: 2.1 }] },
      { kind: 'screen', src: cap('aws.jpg'), beats: 2, transition: 'whip', card: { title: 'AWS', sub: 'S3 · SQS · EC2 · Athena · EKS' }, cam: [{ b: 0, x: 0.22, y: 0.2, z: 1.8 }] },
      { kind: 'screen', src: cap('kubernetes.jpg'), beats: 2, transition: 'whip', card: { title: 'Kubernetes', sub: 'Clusters, workloads, logs' }, cam: [{ b: 0, x: 0.3, y: 0.2, z: 1.45 }] },
      { kind: 'screen', src: cap('api.jpg'), beats: 2, transition: 'whip', card: { title: 'API client', sub: 'Requests, environments, history' }, cam: [{ b: 0, x: 0.5, y: 0.3, z: 1.2 }] },
      { kind: 'screen', src: cap('browser.jpg'), beats: 2, transition: 'whip', card: { title: 'Browser', sub: 'Reader and live tabs for agents' }, cam: [{ b: 0, x: 0.6, y: 0.4, z: 1.15 }] },
      {
        kind: 'screen',
        src: cap('mcp.jpg'),
        beats: 3,
        transition: 'push',
        label: 'MCP servers run native or governed',
        cam: [
          { b: 0, x: 0.5, y: 0.3, z: 1.1 },
          { b: 1, x: 0.3, y: 0.2, z: 1.9 },
        ],
      },
      {
        kind: 'screen',
        src: cap('mcp-activity.jpg'),
        beats: 2,
        transition: 'whip',
        label: 'A dangerous tool call waits for a person',
        cam: [{ b: 0, x: 0.45, y: 0.22, z: 1.6 }],
      },
    ],
  },

  // 4 bars = 16 beats — breakdown
  insights: {
    id: 'insights',
    group: 'Insight',
    title: 'Insights and Usage',
    kicker: 'Key findings, tokens, cost and plan usage left',
    shots: [
      {
        kind: 'screen',
        src: cap('insights.jpg'),
        beats: 6,
        label: 'Reports with key findings',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 2, x: 0.55, y: 0.35, z: 1.3 },
        ],
        callouts: [{ b: 2.5, x: 0.45, y: 0.24, label: 'Key findings', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('usage.jpg'),
        beats: 10,
        transition: 'wipe',
        label: 'Usage: what is left on each plan, per login',
        cam: [
          { b: 0, x: 0.55, y: 0.35, z: 1.1 },
          { b: 1.5, x: 0.55, y: 0.2, z: 1.5 },
          { b: 6.5, x: 0.6, y: 0.22, z: 1.35 },
        ],
        callouts: [{ b: 2, until: 6, x: 0.2, y: 0.1, label: 'Remaining, per login', side: 'bottom' }],
      },
    ],
  },

  // 8 bars = 32 beats — the final chorus
  rooms: {
    id: 'rooms',
    group: 'Work',
    title: 'Rooms',
    kicker: 'Collaborate on a live session, then recap it',
    shots: [
      {
        kind: 'screen',
        src: room('01-start.mp4'),
        from: 0.5,
        rate: 1.3,
        beats: 6,
        label: 'Start a room from a live session',
        cam: [
          { b: 0, x: 0.5, y: 0.558, z: 1.05 },
          { b: 2, x: 0.5, y: 0.558, z: 1.6 },
        ],
        callouts: [{ b: 3, x: 0.634, y: 0.547, label: 'The host admits who joins', side: 'right' }],
      },
      {
        kind: 'screen',
        src: room('04-screen.mp4'),
        from: 0.6,
        rate: 1.4,
        beats: 6,
        transition: 'whip',
        label: 'Share a live screen and talk it through',
        cam: [
          { b: 0, x: 0.444, y: 0.558, z: 1.1 },
          { b: 1.5, x: 0.31, y: 0.614, z: 1.75 },
        ],
        highlights: [{ b: 2.5, until: 5.5, x: 0.12, y: 0.424, w: 0.335, h: 0.424 }],
      },
      {
        kind: 'screen',
        src: room('05-draw.mp4'),
        from: 0.4,
        rate: 1.3,
        beats: 6,
        transition: 'push',
        label: 'Teammates draw on the shared screen, with permission',
        cam: [
          { b: 0, x: 0.332, y: 0.558, z: 1.4 },
          { b: 1, x: 0.243, y: 0.591, z: 1.9 },
        ],
      },
      {
        kind: 'screen',
        src: room('07-request-control.mp4'),
        from: 0.5,
        rate: 1.3,
        beats: 6,
        transition: 'whip',
        label: 'Hand over terminal control, take it back any time',
        cam: [
          { b: 0, x: 0.5, y: 0.558, z: 1 },
          { b: 2, x: 0.254, y: 0.893, z: 1.7 },
        ],
        callouts: [{ b: 3, x: 0.165, y: 0.926, label: 'A real test, in the shared terminal', side: 'right' }],
      },
      {
        kind: 'screen',
        src: room('11-summary.mp4'),
        from: 0.8,
        rate: 1.6,
        beats: 8,
        transition: 'push',
        label: 'A local recap, only after everyone consents',
        cam: [
          { b: 0, x: 0.5, y: 0.502, z: 1.05 },
          { b: 1.5, x: 0.5, y: 0.368, z: 1.45 },
        ],
        callouts: [{ b: 2.5, x: 0.344, y: 0.268, label: 'Decisions · actions · open questions', side: 'right' }],
      },
    ],
  },

  // 4 bars = 16 beats — Phone access over a FICTIONAL tailnet (scripted
  // Tailscale CLI + a serve-like proxy in scripts/lib/stack.mjs)
  phone: {
    id: 'phone',
    group: 'Everywhere',
    title: 'Phone access',
    kicker: 'Your tailnet only, paired with a QR code you approve',
    shots: [
      {
        kind: 'screen',
        src: cap('phone-pair.mp4'),
        from: 3.6,
        rate: 4,
        beats: 6,
        label: 'Continue on phone: scan, then approve the matching code',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.15 },
          { b: 1, x: 0.5, y: 0.5, z: 1.75 },
        ],
      },
      { kind: 'custom', id: 'phone-pair', beats: 5, label: 'The same thread, on your phone' },
      { kind: 'custom', id: 'phone-light', beats: 5, label: 'Or the light version: five tabs' },
    ],
  },

  outro: { id: 'outro', group: 'Everywhere', title: 'Everywhere', shots: [{ kind: 'custom', id: 'outro', beats: 24 }] },
};
