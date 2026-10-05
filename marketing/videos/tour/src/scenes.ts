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

  // 3 bars
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
        label: 'What needs you, what’s running, what’s next',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2.5, x: 0.45, y: 0.3, z: 1.4 },
        ],
        callouts: [
          { b: 0.75, until: 2.5, x: 0.07, y: 0.475, label: 'Grouped sidebar', side: 'right' },
          { b: 3, x: 0.21, y: 0.325, label: 'What needs you', side: 'bottom' },
          { b: 4, x: 0.45, y: 0.325, label: 'What’s running', side: 'bottom' },
          { b: 5, x: 0.67, y: 0.325, label: 'What’s next', side: 'bottom' },
        ],
      },
      {
        kind: 'screen',
        src: cap('home.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Light and dark, one design system',
        wipeTo: { src: cap('home-light.jpg'), b0: 0.5, b1: 2.5 },
        cam: [{ b: 0, x: 0.5, y: 0.5, z: 1.06 }],
      },
    ],
  },

  // 4 bars
  'command-bar': {
    id: 'command-bar',
    group: 'Shell',
    title: 'The command bar',
    kicker: 'Type or speak. ⌘K from anywhere.',
    shots: [
      {
        // cmdk.mp4: ⌘K at ~0.9 s, "go to mission" typed by ~2.9 s, Enter ~4.5 s, then it docks.
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
        // Reopened at ~7.1 s, "what needs me today?" by ~9.6 s, ⌃2 at ~11.1 s.
        kind: 'screen',
        src: cap('cmdk.mp4'),
        from: 7,
        rate: 2.4,
        beats: 4,
        transition: 'push',
        label: 'Ask, and answer in four spaces',
        cam: [
          { b: 0, x: 0.5, y: 0.74, z: 1.35 },
          { b: 2.6, x: 0.55, y: 0.78, z: 1.45 },
        ],
        callouts: [{ b: 3.2, x: 0.73, y: 0.935, label: 'Four spaces', keys: '⌃1 ⌃4', side: 'top' }],
      },
      { kind: 'custom', id: 'desktop', beats: 7, label: '⌥Space from any app, the menu bar, pop-out windows' },
    ],
  },

  // 3 bars
  assistant: {
    id: 'assistant',
    group: 'Work',
    title: 'Assistant',
    kicker: 'Your personal agent, asks before anything leaves your Mac',
    shots: [
      {
        kind: 'screen',
        src: cap('assistant.jpg'),
        beats: 7,
        label: 'Threads, tasks and approvals',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2, x: 0.5, y: 0.35, z: 1.25 },
        ],
        callouts: [
          { b: 1, until: 4, x: 0.17, y: 0.108, label: 'Threads in four spaces', side: 'bottom' },
          { b: 2.5, x: 0.61, y: 0.405, label: 'Otto asks, you decide', side: 'bottom' },
          { b: 4, x: 0.86, y: 0.105, label: 'Needs you', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('assistant-memory.jpg'),
        beats: 5,
        label: 'Memory you can read and edit',
        cam: [
          { b: 0, x: 0.4, y: 0.4, z: 1.15 },
          { b: 1, x: 0.4, y: 0.45, z: 1.35 },
        ],
        callouts: [{ b: 1.5, x: 0.33, y: 0.35, label: 'Remembers what matters', side: 'right' }],
      },
    ],
  },

  // 6 bars
  agents: {
    id: 'agents',
    group: 'Work',
    title: 'Agents',
    kicker: 'Claude Code, Codex and shells as real sessions',
    shots: [
      {
        kind: 'screen',
        src: cap('agents-tab.jpg'),
        beats: 4,
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.4, y: 0.4, z: 1.25 },
        ],
        callouts: [{ b: 1.5, x: 0.07, y: 0.24, label: 'Every session, live', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('agents-tiled.jpg'),
        beats: 5,
        label: 'Tiled: see every agent at once',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.2 },
          { b: 1, x: 0.55, y: 0.5, z: 1 },
        ],
      },
      {
        kind: 'screen',
        src: cap('agents-queue.jpg'),
        beats: 5,
        label: 'Work queue: what needs you first',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1, x: 0.6, y: 0.3, z: 1.35 },
        ],
      },
      {
        kind: 'screen',
        src: cap('agents-broadcast.jpg'),
        beats: 5,
        label: 'Broadcast one prompt to many agents',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.1 },
          { b: 1, x: 0.5, y: 0.5, z: 1.4 },
        ],
      },
      {
        kind: 'screen',
        src: cap('history.jpg'),
        beats: 5,
        label: 'History: every past conversation',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.7, y: 0.3, z: 1.3 },
        ],
        callouts: [{ b: 2, x: 0.905, y: 0.075, label: 'Resume any conversation', side: 'bottom' }],
      },
    ],
  },

  // 4 bars
  'mission-control': {
    id: 'mission-control',
    group: 'Work',
    title: 'Run with Otto',
    kicker: 'From a ticket to a reviewed branch',
    shots: [
      {
        kind: 'screen',
        src: cap('run-with-otto.jpg'),
        beats: 8,
        label: 'One click: source, branch, proof, review, PR',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 1.5, x: 0.55, y: 0.38, z: 1.3 },
          { b: 4.5, x: 0.7, y: 0.45, z: 1.4 },
        ],
        callouts: [
          { b: 1.5, until: 4.5, x: 0.38, y: 0.38, label: 'Source → branch → proof → review → PR', side: 'top' },
          { b: 5, x: 0.96, y: 0.48, label: 'Waits for your approval', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('mission-control.jpg'),
        beats: 8,
        transition: 'whip',
        label: 'Mission Control: the whole work graph',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2, x: 0.5, y: 0.32, z: 1.3 },
        ],
        callouts: [{ b: 2.5, x: 0.4, y: 0.085, label: 'Running · waiting · approvals · spend', side: 'bottom' }],
      },
    ],
  },

  // 4 bars — verse
  swarm: {
    id: 'swarm',
    group: 'Automate',
    title: 'Swarms and Goal Loops',
    kicker: 'Teams of agents with roles, a board and a goal',
    shots: [
      {
        kind: 'screen',
        src: cap('swarm.jpg'),
        beats: 5,
        label: 'Roles and an org chart',
        cam: [
          { b: 0, x: 0.45, y: 0.4, z: 1.1 },
          { b: 1.5, x: 0.35, y: 0.3, z: 1.5 },
        ],
        callouts: [{ b: 2, x: 0.325, y: 0.2, label: 'Coordinator and roles', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('swarm-board.jpg'),
        beats: 5,
        label: 'A shared board',
        cam: [
          { b: 0, x: 0.55, y: 0.4, z: 1.05 },
          { b: 1, x: 0.62, y: 0.35, z: 1.3 },
        ],
      },
      {
        kind: 'screen',
        src: cap('loops.jpg'),
        beats: 6,
        label: 'Goal Loops: plan, execute, evaluate',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.1 },
          { b: 1, x: 0.4, y: 0.25, z: 1.55 },
        ],
        callouts: [
          { b: 1.5, x: 0.2, y: 0.1, label: 'Plan → execute → evaluate', side: 'bottom' },
          { b: 3, x: 0.18, y: 0.26, label: 'Until the acceptance checks pass', side: 'right' },
        ],
      },
    ],
  },

  // 4 bars
  workflows: {
    id: 'workflows',
    group: 'Automate',
    title: 'Workflows',
    kicker: 'Pipelines of agents, approvals and tools',
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
        label: 'Scheduled tasks on a cadence',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.1 },
          { b: 1, x: 0.45, y: 0.22, z: 1.55 },
        ],
      },
      {
        kind: 'screen',
        src: cap('personal-agents.jpg'),
        beats: 5,
        label: 'Personal agents with their own soul and memory',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.1 },
          { b: 1, x: 0.48, y: 0.22, z: 1.55 },
        ],
      },
    ],
  },

  // 4 bars — chorus 2
  git: {
    id: 'git',
    group: 'Build',
    title: 'Git, review and proof',
    kicker: 'Graph, work in progress, PRs and AI review',
    shots: [
      {
        kind: 'screen',
        src: cap('git.jpg'),
        beats: 5,
        label: 'A commit graph with your work in progress',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1, x: 0.4, y: 0.38, z: 1.3 },
        ],
        callouts: [
          { b: 1.5, x: 0.3, y: 0.225, label: 'Work in progress', side: 'right' },
          { b: 3, x: 0.2, y: 0.315, label: 'Branches and worktrees', side: 'right' },
        ],
      },
      {
        kind: 'screen',
        src: cap('git-wip.jpg'),
        beats: 3,
        label: 'Stage, commit, draft the message with AI',
        cam: [
          { b: 0, x: 0.7, y: 0.4, z: 1.15 },
          { b: 0.5, x: 0.76, y: 0.35, z: 1.45 },
        ],
      },
      {
        kind: 'screen',
        src: cap('git-review.jpg'),
        beats: 4,
        label: 'Multi-agent AI review with findings',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 1, x: 0.5, y: 0.35, z: 1.35 },
        ],
      },
      {
        kind: 'screen',
        src: cap('proof.jpg'),
        beats: 4,
        label: 'Proof packs: the evidence behind every change',
        cam: [
          { b: 0, x: 0.6, y: 0.4, z: 1.1 },
          { b: 1, x: 0.7, y: 0.3, z: 1.4 },
        ],
      },
    ],
  },

  // 3 bars
  vault: {
    id: 'vault',
    group: 'Build',
    title: 'Product and Vault',
    kicker: 'Stories, analysis and a living knowledge graph',
    shots: [
      {
        kind: 'screen',
        src: cap('product.jpg'),
        beats: 5,
        label: 'Product: stories into analysis and drafts',
        cam: [
          { b: 0, x: 0.45, y: 0.4, z: 1.1 },
          { b: 1, x: 0.4, y: 0.3, z: 1.4 },
        ],
      },
      {
        kind: 'screen',
        src: cap('vault-graph.jpg'),
        beats: 7,
        transition: 'wipe',
        label: 'Vault: linked markdown and a live graph',
        cam: [
          { b: 0, x: 0.55, y: 0.5, z: 1 },
          { b: 2, x: 0.65, y: 0.5, z: 1.4 },
        ],
      },
    ],
  },

  // 5 bars
  design: {
    id: 'design',
    group: 'Build',
    title: 'Design Hall',
    kicker: 'Every studio, one shared library',
    shots: [
      {
        kind: 'screen',
        src: cap('design.jpg'),
        beats: 6,
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2, x: 0.45, y: 0.55, z: 1.25 },
        ],
        callouts: [
          { b: 1.5, until: 4, x: 0.35, y: 0.46, label: 'Frames · Graphics · Site · 3D · Brand', side: 'top' },
          { b: 4, x: 0.82, y: 0.12, label: 'What Otto learned', side: 'bottom' },
        ],
      },
      // Quick cuts through the studios, one every two beats.
      { kind: 'screen', src: cap('design-frame.jpg'), beats: 2, transition: 'whip', card: { title: 'Frames', sub: 'Versions you compare' }, cam: [{ b: 0, x: 0.4, y: 0.4, z: 1.25 }] },
      { kind: 'screen', src: cap('design-3d.jpg'), beats: 2, transition: 'whip', card: { title: '3D Studio' }, cam: [{ b: 0, x: 0.35, y: 0.45, z: 1.35 }] },
      { kind: 'screen', src: cap('design-site.jpg'), beats: 2, transition: 'whip', card: { title: 'Site Studio' }, cam: [{ b: 0, x: 0.35, y: 0.4, z: 1.35 }] },
      { kind: 'screen', src: cap('design-brand.jpg'), beats: 2, transition: 'whip', card: { title: 'Brand Kit' }, cam: [{ b: 0, x: 0.5, y: 0.3, z: 1.5 }] },
      {
        kind: 'screen',
        src: cap('skills-eval.jpg'),
        beats: 6,
        transition: 'push',
        label: 'Skills Lab evaluates and sharpens your agent skills',
        cam: [
          { b: 0, x: 0.45, y: 0.45, z: 1.05 },
          { b: 1, x: 0.35, y: 0.32, z: 1.4 },
        ],
      },
    ],
  },

  // 4 bars — verse 2
  database: {
    id: 'database',
    group: 'Infrastructure',
    title: 'Database Explorer',
    kicker: 'MySQL · Postgres · MongoDB · Redis · ClickHouse',
    shots: [
      {
        // db-builder.mp4 at 2.6×: tables join by ~3 s, aggregate ~7 s, HAVING/sort ~11 s, Run ~13.7 s.
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
        label: 'Results in a grid, or one record at a time',
        cam: [
          { b: 0, x: 0.6, y: 0.6, z: 1.2 },
          { b: 1, x: 0.55, y: 0.6, z: 1.45 },
        ],
        callouts: [{ b: 1, x: 0.55, y: 0.5, label: 'Grid · Vertical · JSON', side: 'top' }],
      },
    ],
  },

  // 4 bars — quick cuts across the infrastructure modules
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

  // 4 bars — breakdown: slower, wider moves
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

  // 8 bars — the final chorus. Genuine two-person room footage (1920×1080).
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
