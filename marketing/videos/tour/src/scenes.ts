// Shot lists per chapter, on the beat grid (120 BPM: a beat is 15 frames). A
// chapter's shots must add up to its bars × 4 beats (script/chapters.json);
// layout() refuses anything else. Times inside a shot (camera keys `b`,
// callouts `b`/`until`) are beats from the shot's cut.
// Coordinates are normalized to the captured page (1600×900 CSS px @2x):
// x 0..1 left→right, y 0..1 top→bottom.
// Stills/clips come from scripts/capture.mjs → public/capture/ and the Rooms
// clips from scripts/capture-rooms.mjs → public/capture/rooms/.
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

  // 5 bars = 20 beats
  agents: {
    id: 'agents',
    group: 'Work',
    title: 'Agents',
    kicker: 'Claude Code, Codex, Antigravity and shells as real sessions',
    shots: [
      {
        kind: 'screen',
        src: cap('agents-tab.jpg'),
        beats: 6,
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.4, y: 0.4, z: 1.25 },
        ],
        callouts: [{ b: 1.5, x: 0.07, y: 0.24, label: 'Every session, live', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('agents-tiled.jpg'),
        beats: 7,
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
        beats: 7,
        transition: 'whip',
        label: 'Broadcast one prompt to many agents',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.1 },
          { b: 1.5, x: 0.5, y: 0.5, z: 1.38 },
        ],
      },
    ],
  },

  // 5 bars = 20 beats
  threads: {
    id: 'threads',
    group: 'Work',
    title: 'Threads',
    kicker: 'Inbox-style conversations, multi-session splits and workspace files',
    shots: [
      {
        kind: 'screen',
        src: cap('threads-stage.jpg'),
        beats: 6,
        label: 'Pick Claude, Codex or Antigravity and start a thread',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.5, y: 0.44, z: 1.28 },
        ],
        callouts: [
          { b: 1.2, until: 4.5, x: 0.16, y: 0.26, label: 'Pins · Active · Projects · History', side: 'right' },
          { b: 2.5, x: 0.5, y: 0.36, label: 'Claude · Codex · Antigravity', side: 'bottom' },
        ],
      },
      {
        kind: 'screen',
        src: cap('threads-split.jpg'),
        beats: 8,
        transition: 'push',
        label: 'Split live threads beside files, diffs and connections',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.02 },
          { b: 2.5, x: 0.56, y: 0.42, z: 1.28 },
        ],
        callouts: [
          { b: 1.5, until: 5, x: 0.45, y: 0.18, label: 'Multi-session split', side: 'bottom' },
          { b: 3.5, x: 0.84, y: 0.24, label: 'Files · Changes · Connections · Peers', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('threads-peers.jpg'),
        beats: 6,
        transition: 'whip',
        label: 'Direct session-to-session relay and peer timeline',
        cam: [
          { b: 0, x: 0.62, y: 0.45, z: 1.1 },
          { b: 1.5, x: 0.76, y: 0.38, z: 1.4 },
        ],
      },
    ],
  },

  // 3 bars = 12 beats — verse
  loops: {
    id: 'loops',
    group: 'Automate',
    title: 'Goal Loops',
    kicker: 'Plan, execute and evaluate until every acceptance check passes',
    shots: [
      {
        kind: 'screen',
        src: cap('loops.jpg'),
        beats: 12,
        label: 'Goal Loops: autonomous iterations with hard budgets',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 3, x: 0.42, y: 0.28, z: 1.45 },
          { b: 8, x: 0.55, y: 0.42, z: 1.2 },
        ],
        callouts: [
          { b: 1.5, until: 6, x: 0.2, y: 0.12, label: 'Plan → execute → evaluate', side: 'bottom' },
          { b: 5, x: 0.24, y: 0.3, label: 'Machine-checked acceptance criteria', side: 'right' },
        ],
      },
    ],
  },

  // 4 bars = 16 beats
  workflows: {
    id: 'workflows',
    group: 'Automate',
    title: 'Workflows and Scheduled Tasks',
    kicker: 'Pipelines, schedules and personal agents in one place',
    shots: [
      {
        kind: 'screen',
        src: cap('workflows.jpg'),
        beats: 6,
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.42, y: 0.3, z: 1.3 },
        ],
        callouts: [{ b: 2, x: 0.3, y: 0.072, label: 'Paused for your approval', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: cap('scheduled-tasks.jpg'),
        beats: 5,
        transition: 'push',
        label: 'Unified Scheduled Tasks: Tasks · Agents · Agent channels',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.08 },
          { b: 1, x: 0.45, y: 0.22, z: 1.45 },
        ],
      },
      {
        kind: 'screen',
        src: cap('personal-agents.jpg'),
        beats: 5,
        transition: 'whip',
        label: 'Personal agents with their own soul, memory and rooms',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.1 },
          { b: 1, x: 0.48, y: 0.22, z: 1.5 },
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
          { b: 1.5, x: 0.3, y: 0.225, label: 'Work in progress', side: 'right' },
          { b: 3.5, x: 0.2, y: 0.315, label: 'Branches and worktrees', side: 'right' },
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

  // 5 bars = 20 beats
  product: {
    id: 'product',
    group: 'Build',
    title: 'Product and Design Hall',
    kicker: 'Stories, designs and learnings in one unified hub',
    shots: [
      {
        kind: 'screen',
        src: cap('product.jpg'),
        beats: 6,
        label: 'Product: Stories · Designs · Learnings',
        cam: [
          { b: 0, x: 0.45, y: 0.4, z: 1.08 },
          { b: 1.5, x: 0.4, y: 0.28, z: 1.38 },
        ],
      },
      {
        kind: 'screen',
        src: cap('design.jpg'),
        beats: 6,
        transition: 'push',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2, x: 0.45, y: 0.55, z: 1.25 },
        ],
        callouts: [
          { b: 1.5, until: 4, x: 0.35, y: 0.46, label: 'Frames · Graphics · Site · 3D · Brand', side: 'top' },
        ],
      },
      { kind: 'screen', src: cap('design-frame.jpg'), beats: 2, transition: 'whip', card: { title: 'Frames', sub: 'Versions you compare' }, cam: [{ b: 0, x: 0.4, y: 0.4, z: 1.25 }] },
      { kind: 'screen', src: cap('design-3d.jpg'), beats: 2, transition: 'whip', card: { title: '3D Studio' }, cam: [{ b: 0, x: 0.35, y: 0.45, z: 1.35 }] },
      { kind: 'screen', src: cap('design-site.jpg'), beats: 2, transition: 'whip', card: { title: 'Site Studio' }, cam: [{ b: 0, x: 0.35, y: 0.4, z: 1.35 }] },
      { kind: 'screen', src: cap('design-brand.jpg'), beats: 2, transition: 'whip', card: { title: 'Brand Kit' }, cam: [{ b: 0, x: 0.5, y: 0.3, z: 1.5 }] },
    ],
  },

  // 3 bars = 12 beats
  vault: {
    id: 'vault',
    group: 'Build',
    title: 'Vault and Skills Lab',
    kicker: 'Linked markdown docs and evaluated agent skills',
    shots: [
      {
        kind: 'screen',
        src: cap('vault.jpg'),
        beats: 4,
        label: 'Vault: Obsidian-parity docs in your repo',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.06 },
          { b: 1, x: 0.48, y: 0.36, z: 1.28 },
        ],
      },
      {
        kind: 'screen',
        src: cap('vault-graph.jpg'),
        beats: 4,
        transition: 'wipe',
        label: 'A live knowledge graph over your notes',
        cam: [
          { b: 0, x: 0.55, y: 0.5, z: 1 },
          { b: 1.5, x: 0.65, y: 0.5, z: 1.38 },
        ],
      },
      {
        kind: 'screen',
        src: cap('skills-eval.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Skills Lab evaluates and sharpens your agent skills',
        cam: [
          { b: 0, x: 0.45, y: 0.45, z: 1.05 },
          { b: 1, x: 0.35, y: 0.32, z: 1.38 },
        ],
      },
    ],
  },

  // 4 bars = 16 beats — verse 2
  database: {
    id: 'database',
    group: 'Infrastructure',
    title: 'Database Explorer',
    kicker: 'MySQL · Postgres · MongoDB · Redis · ClickHouse',
    shots: [
      {
        kind: 'screen',
        src: cap('db-builder.mp4'),
        from: 1,
        rate: 2.6,
        beats: 12,
        label: 'Build a query visually',
        cam: [
          { b: 0, x: 0.45, y: 0.32, z: 1.35 },
          { b: 5, x: 0.45, y: 0.7, z: 1.4 },
          { b: 9.5, x: 0.6, y: 0.5, z: 1.05 },
        ],
        callouts: [
          { b: 0.5, until: 4.5, x: 0.46, y: 0.2, label: 'Joins follow foreign keys', side: 'bottom' },
          { b: 5.5, until: 9, x: 0.34, y: 0.86, label: 'GROUP BY · HAVING · ORDER BY', side: 'right' },
          { b: 7, until: 9.5, x: 0.72, y: 0.62, label: 'Live SQL', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('db-results.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Results in a grid, plus live agent MCP tabs',
        cam: [
          { b: 0, x: 0.6, y: 0.6, z: 1.2 },
          { b: 1, x: 0.55, y: 0.6, z: 1.45 },
        ],
        callouts: [{ b: 1, x: 0.55, y: 0.5, label: 'Grid · Vertical · JSON', side: 'top' }],
      },
    ],
  },

  // 4 bars = 16 beats
  infra: {
    id: 'infra',
    group: 'Infrastructure',
    title: 'Connections and cloud',
    kicker: 'SSH, Kafka, AWS, Kubernetes, APIs and the web',
    shots: [
      { kind: 'screen', src: cap('connections.jpg'), beats: 4, card: { title: 'Connections', sub: 'SSH terminals, SFTP, sections' }, cam: [{ b: 0, x: 0.3, y: 0.35, z: 1.1 }, { b: 1, x: 0.2, y: 0.25, z: 1.35 }] },
      { kind: 'screen', src: cap('brokers.jpg'), beats: 2, transition: 'whip', card: { title: 'Message brokers', sub: 'Kafka topics and messages' }, cam: [{ b: 0, x: 0.5, y: 0.35, z: 1.2 }] },
      { kind: 'screen', src: cap('aws.jpg'), beats: 2, transition: 'whip', card: { title: 'AWS', sub: 'S3 · SQS · EC2 · Athena · EKS' }, cam: [{ b: 0, x: 0.35, y: 0.25, z: 1.3 }] },
      { kind: 'screen', src: cap('kubernetes.jpg'), beats: 2, transition: 'whip', card: { title: 'Kubernetes', sub: 'Clusters, workloads, logs' }, cam: [{ b: 0, x: 0.3, y: 0.2, z: 1.45 }] },
      { kind: 'screen', src: cap('api.jpg'), beats: 2, transition: 'whip', card: { title: 'API client', sub: 'Requests, environments, history' }, cam: [{ b: 0, x: 0.5, y: 0.3, z: 1.2 }] },
      { kind: 'screen', src: cap('browser.jpg'), beats: 2, transition: 'whip', card: { title: 'Browser', sub: 'Reader and live tabs for agents' }, cam: [{ b: 0, x: 0.6, y: 0.4, z: 1.15 }] },
      { kind: 'screen', src: cap('mcp-activity.jpg'), beats: 2, transition: 'whip', card: { title: 'MCP control plane', sub: 'Every tool call approved and audited' }, cam: [{ b: 0, x: 0.55, y: 0.35, z: 1.2 }] },
    ],
  },

  // 4 bars = 16 beats — breakdown
  insights: {
    id: 'insights',
    group: 'Insight',
    title: 'Insights and Usage',
    kicker: 'Key findings, tokens, cost and load',
    shots: [
      {
        kind: 'screen',
        src: cap('insights.jpg'),
        beats: 8,
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
        beats: 8,
        transition: 'wipe',
        label: 'Usage: tokens, cost and machine load',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.05 },
          { b: 2, x: 0.55, y: 0.4, z: 1.35 },
        ],
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
          { b: 0, x: 0.5, y: 0.5, z: 1.05 },
          { b: 2, x: 0.5, y: 0.5, z: 1.6 },
        ],
        callouts: [{ b: 3, x: 0.62, y: 0.49, label: 'The host admits who joins', side: 'right' }],
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
          { b: 0, x: 0.45, y: 0.5, z: 1.1 },
          { b: 1.5, x: 0.33, y: 0.55, z: 1.75 },
        ],
        highlights: [{ b: 2.5, until: 5.5, x: 0.16, y: 0.38, w: 0.3, h: 0.38 }],
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
          { b: 0, x: 0.35, y: 0.5, z: 1.4 },
          { b: 1, x: 0.27, y: 0.53, z: 1.9 },
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
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2, x: 0.28, y: 0.8, z: 1.7 },
        ],
        callouts: [{ b: 3, x: 0.2, y: 0.83, label: 'A real test, in the shared terminal', side: 'right' }],
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
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 1.5, x: 0.5, y: 0.33, z: 1.45 },
        ],
        callouts: [{ b: 2.5, x: 0.36, y: 0.24, label: 'Decisions · actions · open questions', side: 'right' }],
      },
    ],
  },

  outro: { id: 'outro', group: 'Everywhere', title: 'Everywhere', shots: [{ kind: 'custom', id: 'outro', beats: 24 }] },
};
