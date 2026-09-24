// Shot lists per chapter. Coordinates are normalized to the captured page
// (1600×900 CSS px @2x): x 0..1 left→right, y 0..1 top→bottom.
// Stills/clips come from scripts/capture.mjs → public/capture/.
import type { ChapterSpec } from './types';

const cap = (f: string) => `capture/${f}`;

export const SCENES: Record<string, ChapterSpec> = {
  intro: { id: 'intro', group: 'Shell', title: 'Meet Otto', shots: [{ kind: 'custom', id: 'desktop' }] },

  home: {
    id: 'home',
    group: 'Work',
    title: 'Home',
    kicker: 'A macOS-native desktop for your agents',
    shots: [
      {
        kind: 'screen',
        src: cap('home.jpg'),
        weight: 1.6,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 0.42, x: 0.5, y: 0.5, z: 1 },
          { t: 0.95, x: 0.58, y: 0.3, z: 1.45 },
        ],
        callouts: [
          { t0: 0.32, t1: 0.6, x: 0.07, y: 0.475, label: 'Grouped sidebar', side: 'right' },
          { t0: 0.4, t1: 0.6, x: 0.13, y: 0.027, label: 'One header per page', side: 'bottom' },
          { t0: 0.66, x: 0.21, y: 0.175, label: 'What needs you', side: 'bottom' },
          { t0: 0.76, x: 0.45, y: 0.175, label: 'What’s running', side: 'bottom' },
          { t0: 0.86, x: 0.67, y: 0.175, label: 'What’s next', side: 'bottom' },
        ],
      },
      {
        kind: 'screen',
        src: cap('home.jpg'),
        wipeTo: cap('home-light.jpg'),
        weight: 0.8,
        chip: 'Light and dark, one design system',
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1.04 },
          { t: 1, x: 0.5, y: 0.5, z: 1 },
        ],
      },
    ],
  },

  'command-bar': {
    id: 'command-bar',
    group: 'Shell',
    title: 'The command bar',
    kicker: 'Type or speak. ⌘K from anywhere.',
    shots: [
      {
        kind: 'screen',
        src: cap('cmdk.mp4'),
        from: 1.2,
        rate: 1.75,
        weight: 2.2,
        cam: [
          { t: 0, x: 0.5, y: 0.62, z: 1.25 },
          { t: 0.3, x: 0.5, y: 0.7, z: 1.4 },
          { t: 0.42, x: 0.5, y: 0.5, z: 1 },
          { t: 0.55, x: 0.5, y: 0.8, z: 1.35 },
          { t: 1, x: 0.5, y: 0.8, z: 1.35 },
        ],
        callouts: [
          { t0: 0.04, t1: 0.3, x: 0.33, y: 0.93, label: 'Press', keys: '⌘ K', side: 'top' },
          { t0: 0.32, t1: 0.47, x: 0.5, y: 0.985, label: 'Docks into the status bar', side: 'top' },
          { t0: 0.8, t1: 0.99, x: 0.73, y: 0.935, label: 'Four spaces', keys: '⌃1 ⌃4', side: 'top' },
        ],
      },
      { kind: 'custom', id: 'desktop', weight: 1.1 },
    ],
  },

  assistant: {
    id: 'assistant',
    group: 'Work',
    title: 'Assistant',
    kicker: 'Your personal agent — it asks before anything leaves your Mac',
    shots: [
      {
        kind: 'screen',
        src: cap('assistant.jpg'),
        weight: 1.3,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 1, x: 0.55, y: 0.35, z: 1.3 },
        ],
        callouts: [
          { t0: 0.2, x: 0.17, y: 0.108, label: 'Threads in four spaces', side: 'bottom' },
          { t0: 0.45, x: 0.61, y: 0.405, label: 'Otto asks, you decide', side: 'bottom' },
          { t0: 0.65, x: 0.86, y: 0.105, label: 'Needs you', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('assistant-memory.jpg'),
        weight: 0.8,
        cam: [
          { t: 0, x: 0.4, y: 0.4, z: 1.2 },
          { t: 1, x: 0.4, y: 0.5, z: 1.35 },
        ],
        callouts: [{ t0: 0.2, x: 0.33, y: 0.35, label: 'Remembers what matters', side: 'right' }],
      },
    ],
  },

  agents: {
    id: 'agents',
    group: 'Work',
    title: 'Agents',
    kicker: 'Claude Code, Codex and shells as real sessions',
    shots: [
      {
        kind: 'screen',
        src: cap('agents-tab.jpg'),
        weight: 0.8,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 1, x: 0.4, y: 0.4, z: 1.2 },
        ],
        callouts: [{ t0: 0.25, x: 0.07, y: 0.24, label: 'Every session, live', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('agents-tiled.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1.15 },
          { t: 1, x: 0.55, y: 0.5, z: 1 },
        ],
        chip: 'Tiled — see every agent at once',
      },
      {
        kind: 'screen',
        src: cap('agents-queue.jpg'),
        weight: 0.9,
        cam: [
          { t: 0, x: 0.6, y: 0.3, z: 1.35 },
          { t: 1, x: 0.6, y: 0.3, z: 1.2 },
        ],
        chip: 'Work queue — what needs you first',
      },
      {
        kind: 'screen',
        src: cap('agents-broadcast.jpg'),
        weight: 0.8,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1.25 },
          { t: 1, x: 0.5, y: 0.5, z: 1.4 },
        ],
        chip: 'Broadcast one prompt to many agents',
      },
      {
        kind: 'screen',
        src: cap('history.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 1, x: 0.7, y: 0.3, z: 1.3 },
        ],
        callouts: [{ t0: 0.3, x: 0.905, y: 0.075, label: 'Resume any conversation', side: 'bottom' }],
        chip: 'History',
      },
    ],
  },

  'mission-control': {
    id: 'mission-control',
    group: 'Work',
    title: 'Run with Otto',
    kicker: 'From a ticket to a reviewed branch — then see all the work',
    shots: [
      {
        kind: 'screen',
        src: cap('run-with-otto.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.55, y: 0.35, z: 1.25 },
          { t: 1, x: 0.55, y: 0.4, z: 1.4 },
        ],
        callouts: [
          { t0: 0.18, x: 0.38, y: 0.38, label: 'Source → branch → proof → review → PR', side: 'top' },
          { t0: 0.55, x: 0.96, y: 0.48, label: 'Waits for your approval', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('mission-control.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 1, x: 0.55, y: 0.3, z: 1.3 },
        ],
        callouts: [{ t0: 0.25, x: 0.4, y: 0.085, label: 'Running · waiting · approvals · spend', side: 'bottom' }],
        chip: 'Mission Control',
      },
    ],
  },

  swarm: {
    id: 'swarm',
    group: 'Automate',
    title: 'Swarms and Goal Loops',
    kicker: 'Teams of agents with roles, a board and a goal',
    shots: [
      {
        kind: 'screen',
        src: cap('swarm.jpg'),
        weight: 0.9,
        cam: [
          { t: 0, x: 0.35, y: 0.3, z: 1.5 },
          { t: 1, x: 0.4, y: 0.3, z: 1.35 },
        ],
        callouts: [{ t0: 0.25, x: 0.325, y: 0.2, label: 'Roles and an org chart', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('swarm-board.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.6, y: 0.35, z: 1.3 },
          { t: 1, x: 0.65, y: 0.35, z: 1.2 },
        ],
        chip: 'A shared board',
      },
      {
        kind: 'screen',
        src: cap('loops.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.4, y: 0.25, z: 1.6 },
          { t: 1, x: 0.4, y: 0.25, z: 1.45 },
        ],
        callouts: [
          { t0: 0.15, x: 0.2, y: 0.1, label: 'Plan → execute → evaluate', side: 'bottom' },
          { t0: 0.45, x: 0.18, y: 0.26, label: 'Acceptance checks', side: 'right' },
        ],
        chip: 'Goal Loops',
      },
    ],
  },

  workflows: {
    id: 'workflows',
    group: 'Automate',
    title: 'Workflows and schedules',
    kicker: 'Pipelines of agents, approvals and tools',
    shots: [
      {
        kind: 'screen',
        src: cap('workflows.jpg'),
        weight: 1.3,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 1, x: 0.45, y: 0.3, z: 1.3 },
        ],
        callouts: [{ t0: 0.3, x: 0.3, y: 0.072, label: 'Paused for your approval', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: cap('scheduled-tasks.jpg'),
        weight: 0.9,
        cam: [
          { t: 0, x: 0.45, y: 0.2, z: 1.6 },
          { t: 1, x: 0.4, y: 0.2, z: 1.5 },
        ],
        chip: 'Scheduled Tasks',
      },
      {
        kind: 'screen',
        src: cap('personal-agents.jpg'),
        weight: 0.9,
        cam: [
          { t: 0, x: 0.5, y: 0.2, z: 1.6 },
          { t: 1, x: 0.45, y: 0.2, z: 1.5 },
        ],
        chip: 'Personal Agents',
      },
    ],
  },

  git: {
    id: 'git',
    group: 'Build',
    title: 'Git, reviews and proof',
    kicker: 'Graph, work in progress, PRs and AI review',
    shots: [
      {
        kind: 'screen',
        src: cap('git.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 1, x: 0.4, y: 0.4, z: 1.3 },
        ],
        callouts: [
          { t0: 0.2, x: 0.3, y: 0.225, label: 'Work in progress', side: 'right' },
          { t0: 0.45, x: 0.2, y: 0.315, label: 'Branches and worktrees', side: 'right' },
        ],
      },
      {
        kind: 'screen',
        src: cap('git-wip.jpg'),
        weight: 0.8,
        cam: [
          { t: 0, x: 0.75, y: 0.35, z: 1.35 },
          { t: 1, x: 0.78, y: 0.35, z: 1.45 },
        ],
        chip: 'Stage, commit, draft with AI',
      },
      {
        kind: 'screen',
        src: cap('git-review.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.5, y: 0.4, z: 1.2 },
          { t: 1, x: 0.5, y: 0.35, z: 1.35 },
        ],
        chip: 'Multi-agent AI review',
      },
      {
        kind: 'screen',
        src: cap('proof.jpg'),
        weight: 0.9,
        cam: [
          { t: 0, x: 0.7, y: 0.3, z: 1.3 },
          { t: 1, x: 0.72, y: 0.3, z: 1.4 },
        ],
        chip: 'Proof packs — the evidence behind every change',
      },
    ],
  },

  vault: {
    id: 'vault',
    group: 'Build',
    title: 'Product and Vault',
    kicker: 'Stories, analysis and a living knowledge graph',
    shots: [
      {
        kind: 'screen',
        src: cap('product.jpg'),
        weight: 0.9,
        cam: [
          { t: 0, x: 0.4, y: 0.3, z: 1.4 },
          { t: 1, x: 0.4, y: 0.3, z: 1.3 },
        ],
        chip: 'Product',
      },
      {
        kind: 'screen',
        src: cap('vault-graph.jpg'),
        weight: 1.1,
        cam: [
          { t: 0, x: 0.6, y: 0.55, z: 1.1 },
          { t: 1, x: 0.65, y: 0.5, z: 1.35 },
        ],
        chip: 'Vault — linked markdown + graph',
      },
    ],
  },

  design: {
    id: 'design',
    group: 'Build',
    title: 'Design Hall',
    kicker: 'Every studio, one shared library',
    shots: [
      {
        kind: 'screen',
        src: cap('design.jpg'),
        weight: 1.2,
        cam: [
          { t: 0, x: 0.5, y: 0.5, z: 1 },
          { t: 1, x: 0.45, y: 0.55, z: 1.25 },
        ],
        callouts: [
          { t0: 0.15, x: 0.35, y: 0.46, label: 'Frames · Graphics · Site · 3D · Brand', side: 'top' },
          { t0: 0.55, x: 0.82, y: 0.12, label: 'What Otto learned', side: 'bottom' },
        ],
      },
      { kind: 'screen', src: cap('design-frame.jpg'), weight: 0.75, cam: [{ t: 0, x: 0.4, y: 0.4, z: 1.2 }, { t: 1, x: 0.38, y: 0.35, z: 1.35 }], chip: 'Frames — versions you compare' },
      { kind: 'screen', src: cap('design-3d.jpg'), weight: 0.7, cam: [{ t: 0, x: 0.35, y: 0.45, z: 1.3 }, { t: 1, x: 0.35, y: 0.45, z: 1.45 }], chip: '3D Studio' },
      { kind: 'screen', src: cap('design-site.jpg'), weight: 0.7, cam: [{ t: 0, x: 0.35, y: 0.4, z: 1.3 }, { t: 1, x: 0.35, y: 0.4, z: 1.4 }], chip: 'Site Studio' },
      { kind: 'screen', src: cap('design-brand.jpg'), weight: 0.65, cam: [{ t: 0, x: 0.5, y: 0.25, z: 1.5 }, { t: 1, x: 0.5, y: 0.25, z: 1.6 }], chip: 'Brand Kit' },
      { kind: 'screen', src: cap('skills-eval.jpg'), weight: 0.8, cam: [{ t: 0, x: 0.35, y: 0.35, z: 1.3 }, { t: 1, x: 0.35, y: 0.3, z: 1.4 }], chip: 'Skills Lab' },
    ],
  },

  database: {
    id: 'database',
    group: 'Infrastructure',
    title: 'Database Explorer',
    kicker: 'MySQL · Postgres · MongoDB · Redis · ClickHouse',
    shots: [
      {
        kind: 'screen',
        src: cap('db-builder.mp4'),
        from: 0.4,
        rate: 1.55,
        weight: 2.3,
        cam: [
          { t: 0, x: 0.45, y: 0.3, z: 1.35 },
          { t: 0.35, x: 0.45, y: 0.35, z: 1.3 },
          { t: 0.55, x: 0.45, y: 0.7, z: 1.4 },
          { t: 0.8, x: 0.55, y: 0.7, z: 1.4 },
          { t: 0.9, x: 0.6, y: 0.4, z: 1.1 },
          { t: 1, x: 0.6, y: 0.4, z: 1.1 },
        ],
        callouts: [
          { t0: 0.1, t1: 0.36, x: 0.46, y: 0.2, label: 'Joins follow foreign keys', side: 'bottom' },
          { t0: 0.5, t1: 0.78, x: 0.34, y: 0.86, label: 'GROUP BY · HAVING · ORDER BY', side: 'right' },
          { t0: 0.6, t1: 0.86, x: 0.72, y: 0.62, label: 'Live SQL', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('db-mongo.jpg'),
        weight: 0.9,
        cam: [
          { t: 0, x: 0.55, y: 0.5, z: 1.15 },
          { t: 1, x: 0.6, y: 0.5, z: 1.3 },
        ],
        chip: 'MongoDB in the vertical view',
      },
    ],
  },

  infra: {
    id: 'infra',
    group: 'Infrastructure',
    title: 'Connections and cloud',
    kicker: 'SSH, Kafka, AWS, Kubernetes, APIs, the web — governed',
    shots: [
      {
        kind: 'montage',
        weight: 1,
        items: [
          { src: cap('connections.jpg'), label: 'Connections', sub: 'SSH hosts, SFTP, sections', x: 0.2, y: 0.25, z: 1.35 },
          { src: cap('brokers.jpg'), label: 'Message brokers', sub: 'Kafka topics and messages', x: 0.5, y: 0.35, z: 1.2 },
          { src: cap('aws.jpg'), label: 'AWS', sub: 'S3 · SQS · EC2 · Athena · EKS', x: 0.35, y: 0.25, z: 1.3 },
          { src: cap('kubernetes.jpg'), label: 'Kubernetes', sub: 'Clusters, workloads, logs', x: 0.3, y: 0.15, z: 1.5 },
          { src: cap('api.jpg'), label: 'API client', sub: 'Requests, environments, history', x: 0.5, y: 0.3, z: 1.2 },
          { src: cap('browser.jpg'), label: 'Browser', sub: 'Reader and live tabs for agents', x: 0.6, y: 0.4, z: 1.15 },
          { src: cap('mcp-activity.jpg'), label: 'MCP control plane', sub: 'Every tool call approved and audited', x: 0.55, y: 0.35, z: 1.2 },
        ],
      },
    ],
  },

  insights: {
    id: 'insights',
    group: 'Insight',
    title: 'Insights and Usage',
    kicker: 'Reports with key findings; tokens, cost and load',
    shots: [
      {
        kind: 'screen',
        src: cap('insights.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.55, y: 0.35, z: 1.2 },
          { t: 1, x: 0.6, y: 0.35, z: 1.35 },
        ],
        callouts: [{ t0: 0.25, x: 0.45, y: 0.24, label: 'Key findings', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('usage.jpg'),
        weight: 1,
        cam: [
          { t: 0, x: 0.5, y: 0.35, z: 1.25 },
          { t: 1, x: 0.55, y: 0.4, z: 1.35 },
        ],
        chip: 'Usage',
      },
    ],
  },

  outro: { id: 'outro', group: 'Everywhere', title: 'Everywhere', shots: [{ kind: 'custom', id: 'phone' }] },
};
