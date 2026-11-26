// Shot lists per chapter, on the beat grid (120 BPM: a beat is 15 frames). A
// chapter's shots must add up to its bars × 4 beats (script/chapters.json);
// layout() refuses anything else. Times inside a shot (camera keys `b`,
// callouts `b`/`until`) are beats from the shot's cut.
// Coordinates are normalized to the captured page (1600×900 CSS px @2x):
// x 0..1 left→right, y 0..1 top→bottom.
// Stills/clips come from scripts/capture.mjs → public/capture/, the agent-debug
// and DB Watch stills from scripts/capture-agent-debug.mjs → public/capture/dbg/,
// the Workflows chapter from scripts/capture-workflows.mjs → public/capture/wf/
// (the IDE opens beside the agent's session, so those shots frame the right
// half), and the Rooms clips from scripts/capture-rooms.mjs (or
// scripts/reuse-rooms.mjs: the same footage, cropped clear of the published
// movie's caption band — hence the Rooms coordinates below are in that crop)
// → public/capture/rooms/.
import type { ChapterSpec } from './types';

const cap = (f: string) => `capture/${f}`;
const dbg = (f: string) => `capture/dbg/${f}`;
const wf = (f: string) => `capture/wf/${f}`;
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

  // 6 bars = 24 beats — Agents is one workspace: the inbox beside the canvas,
  // a session in its Chat view, tiled with the terminals, broadcast
  agents: {
    id: 'agents',
    group: 'Work',
    title: 'Agents',
    kicker: 'One workspace: an inbox of sessions beside the canvas',
    shots: [
      {
        kind: 'screen',
        src: cap('agents-chat.jpg'),
        beats: 10,
        label: 'A session in its Chat view',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 2, x: 0.16, y: 0.36, z: 1.6 },
          { b: 5, x: 0.5, y: 0.38, z: 1.4 },
        ],
        callouts: [
          { b: 2.5, until: 4.75, x: 0.2, y: 0.17, label: 'Status in words, what needs you first', side: 'right' },
          { b: 5.5, x: 0.94, y: 0.07, label: 'Terminal or Chat', side: 'left' },
        ],
      },
      {
        kind: 'screen',
        src: cap('agents-tiled.jpg'),
        beats: 7,
        transition: 'push',
        label: 'Tile chat threads and terminals together',
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

  // 7 bars = 28 beats — Threads on the structured runtime: the picker lists
  // every structured agent (stand-ins in scripts/lib/stack.mjs), a Codex
  // thread driven by the capture's scripted app-server (scripts/lib/demo-codex.mjs)
  threads: {
    id: 'threads',
    group: 'Work',
    title: 'Threads',
    kicker: 'Agents on the structured runtime: plans, commands and diffs, live',
    shots: [
      {
        kind: 'screen',
        src: cap('threads-picker.jpg'),
        beats: 5,
        label: 'Every structured agent, one picker',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.15 },
          { b: 1, x: 0.42, y: 0.42, z: 1.7 },
        ],
        callouts: [{ b: 1.5, x: 0.255, y: 0.45, label: 'Claude, Codex, Antigravity, OpenCode, pi, Gemini, Grok, Muse', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('threads-runtime.jpg'),
        beats: 8,
        transition: 'push',
        label: 'A Codex thread: plan, commands and edits, streamed',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.05 },
          { b: 2, x: 0.5, y: 0.3, z: 1.55 },
        ],
        callouts: [{ b: 2.5, until: 7.5, x: 0.28, y: 0.27, label: 'Tasks, commands, the edit and the test run', side: 'right' }],
      },
      {
        kind: 'screen',
        src: cap('threads-approval.jpg'),
        beats: 7,
        transition: 'push',
        label: 'It asks before anything outward-facing',
        cam: [
          { b: 0, x: 0.6, y: 0.6, z: 1.15 },
          { b: 1, x: 0.75, y: 0.86, z: 1.8 },
        ],
        callouts: [{ b: 2, x: 0.86, y: 0.955, label: 'Allow once, inline', side: 'top' }],
      },
      {
        kind: 'screen',
        src: cap('council-stage.jpg'),
        beats: 8,
        transition: 'whip',
        label: 'Compare models: one prompt, several agents side by side',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.3 },
          { b: 1, x: 0.5, y: 0.5, z: 1.9 },
        ],
        callouts: [{ b: 2, x: 0.3, y: 0.535, label: 'Claude Opus and GPT', side: 'bottom' }],
      },
    ],
  },

  // 9 bars = 36 beats — verse: Workflows on the Threads runtime, captured by
  // scripts/capture-workflows.mjs (isolated e2e daemon, fake agents, real runs).
  workflows: {
    id: 'workflows',
    group: 'Automate',
    title: 'Workflows',
    kicker: 'Runs you can read at a glance, on your Mac and your phone',
    shots: [
      {
        kind: 'screen',
        src: wf('wf-inbox.jpg'),
        beats: 4,
        label: 'Runs: needs you, running, recent',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 1, x: 0.56, y: 0.3, z: 1.6 },
        ],
        callouts: [{ b: 1.5, x: 0.75, y: 0.115, label: 'Needs you: a question', side: 'left' }],
      },
      {
        kind: 'screen',
        src: wf('wf-fanout.mp4'),
        from: 0,
        rate: 1.6,
        beats: 6,
        transition: 'push',
        label: 'The run page: every step in order, the review fanned out',
        cam: [
          { b: 0, x: 0.45, y: 0.45, z: 1.1 },
          { b: 1.5, x: 0.32, y: 0.42, z: 1.35 },
        ],
        callouts: [{ b: 2, x: 0.19, y: 0.2, label: 'One tick per reviewer', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: wf('wf-reviewer.jpg'),
        beats: 4,
        transition: 'whip',
        label: "A reviewer's conversation, beside the list",
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.1 },
          { b: 1, x: 0.56, y: 0.35, z: 1.4 },
        ],
      },
      {
        kind: 'screen',
        src: wf('wf-failed.jpg'),
        beats: 4,
        transition: 'wipe',
        label: "A failed run leads with the step's own error",
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.2 },
          { b: 1, x: 0.42, y: 0.22, z: 1.75 },
        ],
        highlights: [{ b: 1.5, x: 0.16, y: 0.2, w: 0.26, h: 0.083 }],
        callouts: [{ b: 2, x: 0.912, y: 0.026, label: 'Retry step', side: 'bottom' }],
      },
      { kind: 'custom', id: 'wf-phone', beats: 4, label: 'The same run, on your phone' },
      {
        kind: 'screen',
        src: wf('wf-editor-model.jpg'),
        beats: 4,
        transition: 'push',
        label: "Each step's model and effort, as in Threads",
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.1 },
          { b: 1, x: 0.66, y: 0.38, z: 1.5 },
        ],
      },
      {
        kind: 'screen',
        src: wf('wf-goal-form.jpg'),
        beats: 3,
        transition: 'whip',
        label: 'A goal: what must be true, and how it is checked',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.15 },
          { b: 1, x: 0.5, y: 0.42, z: 1.45 },
        ],
      },
      {
        kind: 'screen',
        src: wf('wf-goal-run.jpg'),
        beats: 4,
        transition: 'push',
        label: 'It works until every criterion is met',
        cam: [
          { b: 0, x: 0.45, y: 0.35, z: 1.2 },
          { b: 1, x: 0.29, y: 0.33, z: 1.9 },
        ],
      },
      {
        kind: 'screen',
        src: wf('wf-optimize.jpg'),
        beats: 3,
        transition: 'wipe',
        label: 'Optimize drafts a faster version; you apply it',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.1 },
          { b: 1, x: 0.68, y: 0.36, z: 1.5 },
        ],
      },
    ],
  },

  // 3 bars = 12 beats — a personal agent's Chat is a runtime thread; Scheduled Tasks
  'personal-agents': {
    id: 'personal-agents',
    group: 'Automate',
    title: 'Personal Agents',
    kicker: 'Agents with a soul, a schedule and a team, chatting on Threads',
    shots: [
      {
        kind: 'screen',
        src: cap('pa-chat.mp4'),
        from: 1,
        rate: 1.6,
        beats: 8,
        label: 'Chat with your agent: a thread on the structured runtime',
        cam: [
          { b: 0, x: 0.45, y: 0.78, z: 1.4 },
          { b: 2.5, x: 0.4, y: 0.42, z: 1.42 },
        ],
      },
      {
        kind: 'screen',
        src: cap('scheduled-tasks.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Scheduled tasks run agents on a cadence and report back',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.08 },
          { b: 1.5, x: 0.5, y: 0.22, z: 1.45 },
        ],
      },
    ],
  },

  // 6 bars = 24 beats — chorus 2: IDE (Beta) on the fictional acme-ledger
  // project (scripts/lib/demo-ledger.mjs): projects in the order added, tabs
  // back after a restart, typing, a pytest run, a breakpoint hit, the agent dock.
  ide: {
    id: 'ide',
    group: 'Build',
    title: 'IDE (Beta)',
    kicker: 'Edit, run, debug and ask an agent, in one project',
    shots: [
      {
        kind: 'screen',
        src: cap('ide-projects.jpg'),
        beats: 4,
        label: 'Your projects, in the order you added them',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1, x: 0.21, y: 0.22, z: 2 },
        ],
      },
      {
        kind: 'screen',
        src: cap('ide-restored.jpg'),
        beats: 4,
        transition: 'push',
        label: 'After a restart, every tab comes back',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.2 },
          { b: 1, x: 0.4, y: 0.25, z: 1.6 },
        ],
        callouts: [{ b: 1.5, x: 0.36, y: 0.07, label: 'The same tabs, the same active file', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: cap('ide-typing.jpg'),
        beats: 4,
        transition: 'whip',
        label: 'Type with the language server beside you',
        cam: [
          { b: 0, x: 0.45, y: 0.4, z: 1.3 },
          { b: 1, x: 0.4, y: 0.33, z: 1.85 },
        ],
      },
      {
        kind: 'screen',
        src: cap('ide-run.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Run configurations: live output and a test tree',
        cam: [
          { b: 0, x: 0.55, y: 0.5, z: 1.05 },
          { b: 1, x: 0.4, y: 0.8, z: 1.5 },
        ],
      },
      {
        kind: 'screen',
        src: cap('ide-debug.jpg'),
        beats: 4,
        transition: 'whip',
        label: 'A breakpoint hit: frames, variables, evaluate',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.1 },
          { b: 1, x: 0.5, y: 0.72, z: 1.45 },
        ],
      },
      {
        kind: 'screen',
        src: cap('ide-agent.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Ask the agent about the selected code, then apply its diff',
        cam: [
          { b: 0, x: 0.7, y: 0.45, z: 1.2 },
          { b: 1, x: 0.86, y: 0.4, z: 1.75 },
        ],
      },
    ],
  },

  // 6 bars = 24 beats — an agent debugs a real Go service with real delve
  // and a real MariaDB (the e2e fixture ui/e2e/fixtures/go-orders); the IDE
  // opens beside the agent's session, so every shot frames the right half.
  'agent-debug': {
    id: 'agent-debug',
    group: 'Build',
    title: 'Debug with an agent',
    kicker: 'It drives the debugger; you watch, and you decide',
    shots: [
      {
        kind: 'screen',
        src: dbg('dbg-start-confirm.jpg'),
        beats: 5,
        label: 'Before anything runs: exactly what will start',
        cam: [
          { b: 0, x: 0.6, y: 0.5, z: 1.1 },
          { b: 1, x: 0.76, y: 0.48, z: 2.1 },
        ],
      },
      {
        kind: 'screen',
        src: dbg('dbg-running.jpg'),
        beats: 4,
        transition: 'push',
        label: 'The agent says what it is doing',
        cam: [
          { b: 0, x: 0.75, y: 0.4, z: 1.9 },
          { b: 1.5, x: 0.76, y: 0.2, z: 2.2 },
        ],
        callouts: [{ b: 1.5, x: 0.83, y: 0.08, label: 'Driving the IDE, in its own words', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: dbg('dbg-stop.jpg'),
        beats: 5,
        transition: 'whip',
        label: 'Stopped at its breakpoint in the handler',
        cam: [
          { b: 0, x: 0.75, y: 0.45, z: 1.9 },
          { b: 1.5, x: 0.72, y: 0.35, z: 2.2 },
        ],
        callouts: [{ b: 2, x: 0.535, y: 0.385, label: 'The agent’s breakpoint', side: 'right' }],
      },
      {
        kind: 'screen',
        src: dbg('dbg-step-added.jpg'),
        beats: 5,
        transition: 'push',
        label: 'At every stop it explains, and waits',
        cam: [
          { b: 0, x: 0.75, y: 0.4, z: 1.9 },
          { b: 1.5, x: 0.76, y: 0.6, z: 2 },
        ],
        callouts: [{ b: 2, x: 0.535, y: 0.7, label: 'DB: the row it just wrote', side: 'top' }],
      },
      {
        kind: 'screen',
        src: dbg('dbg-proceed.jpg'),
        beats: 5,
        transition: 'whip',
        label: 'Press Proceed, or take over the debugger yourself',
        cam: [
          { b: 0, x: 0.75, y: 0.4, z: 1.9 },
          { b: 1, x: 0.84, y: 0.2, z: 2.4 },
        ],
        callouts: [{ b: 1.5, x: 0.924, y: 0.075, label: 'Proceed', side: 'bottom' }],
      },
    ],
  },

  // 2 bars = 8 beats — DB Watch on the same run's MariaDB container
  dbwatch: {
    id: 'dbwatch',
    group: 'Build',
    title: 'DB Watch',
    kicker: 'The rows your code wrote, at every stop',
    shots: [
      {
        kind: 'screen',
        src: dbg('dbw-last.jpg'),
        beats: 4,
        label: 'Since the last stop: only the changed cell',
        cam: [
          { b: 0, x: 0.75, y: 0.6, z: 1.9 },
          { b: 1, x: 0.72, y: 0.7, z: 2.6 },
        ],
        callouts: [{ b: 1.5, x: 0.61, y: 0.728, label: 'status: new → paid', side: 'bottom' }],
      },
      {
        kind: 'screen',
        src: dbg('dbg-done.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Since start: the row the request added',
        cam: [
          { b: 0, x: 0.75, y: 0.5, z: 1.9 },
          { b: 1, x: 0.75, y: 0.62, z: 2.3 },
        ],
        callouts: [{ b: 1.5, x: 0.8, y: 0.075, label: 'Answered 201: done', side: 'bottom' }],
      },
    ],
  },

  // 4 bars = 16 beats — Git lands on History; uncommitted changes are its top row
  git: {
    id: 'git',
    group: 'Build',
    title: 'Git and AI review',
    kicker: 'History first, your changes on top, multi-lens review',
    shots: [
      {
        kind: 'screen',
        src: cap('git.jpg'),
        beats: 6,
        label: 'Git opens on History',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1 },
          { b: 1.5, x: 0.38, y: 0.3, z: 1.4 },
        ],
        callouts: [
          { b: 1.5, x: 0.42, y: 0.12, label: 'Uncommitted changes: the top row', side: 'right' },
          { b: 3.5, x: 0.08, y: 0.32, label: 'Branches, stashes and worktrees', side: 'right' },
        ],
      },
      {
        kind: 'screen',
        src: cap('git-wip.jpg'),
        beats: 4,
        transition: 'push',
        label: 'Stage, commit and draft the message with AI',
        cam: [
          { b: 0, x: 0.7, y: 0.4, z: 1.15 },
          { b: 1, x: 0.78, y: 0.3, z: 1.6 },
        ],
      },
      {
        kind: 'screen',
        src: cap('git-review.jpg'),
        beats: 6,
        transition: 'whip',
        label: 'Multi-agent AI review with tracked findings',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.05 },
          { b: 1.5, x: 0.55, y: 0.25, z: 1.5 },
        ],
      },
    ],
  },

  product: {
    id: 'product',
    group: 'Build',
    title: 'Product and Design Hall',
    kicker: 'Stories and designs, one home',
    shots: [
      {
        kind: 'screen',
        src: cap('product-design.jpg'),
        beats: 3,
        label: "A story's Design tab lists its designs",
        cam: [
          { b: 0, x: 0.55, y: 0.4, z: 1.08 },
          { b: 1, x: 0.52, y: 0.38, z: 1.4 },
        ],
      },
      {
        kind: 'screen',
        src: cap('design-lobby.jpg'),
        beats: 4,
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
        beats: 3,
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
        beats: 3,
        transition: 'push',
        label: 'Compare two versions, then restore or approve',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.05 },
          { b: 1.5, x: 0.5, y: 0.45, z: 1.25 },
        ],
      },
      {
        kind: 'screen',
        src: cap('design-comments.jpg'),
        beats: 3,
        transition: 'whip',
        label: 'Comments pin to the design',
        cam: [
          { b: 0, x: 0.6, y: 0.4, z: 1.1 },
          { b: 1, x: 0.65, y: 0.38, z: 1.3 },
        ],
        callouts: [{ b: 1, x: 0.357, y: 0.535, label: 'A pinned comment', side: 'bottom' }],
      },
    ],
  },

  // 2 bars = 8 beats
  vault: {
    id: 'vault',
    group: 'Build',
    title: 'Vault and Skills Lab',
    kicker: 'Linked markdown docs, and one home for agent skills',
    shots: [
      {
        kind: 'screen',
        src: cap('vault.jpg'),
        beats: 4,
        label: 'Vault: linked notes, backlinks and live context',
        cam: [
          { b: 0, x: 0.5, y: 0.4, z: 1.2 },
          { b: 1, x: 0.55, y: 0.3, z: 1.4 },
        ],
      },
      {
        kind: 'screen',
        src: cap('skills-eval.jpg'),
        beats: 4,
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
        beats: 6,
        label: 'Query and read the results in a grid',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.1 },
          { b: 1.5, x: 0.38, y: 0.28, z: 1.6 },
        ],
      },
      {
        kind: 'screen',
        src: cap('db-readonly.jpg'),
        beats: 5,
        transition: 'push',
        label: 'A read-only connection refuses writes',
        cam: [
          { b: 0, x: 0.55, y: 0.35, z: 1.2 },
          { b: 1, x: 0.6, y: 0.3, z: 1.75 },
        ],
      },
      {
        kind: 'screen',
        src: cap('db-history.jpg'),
        beats: 5,
        transition: 'whip',
        label: 'History across every connection',
        cam: [
          { b: 0, x: 0.4, y: 0.3, z: 1.3 },
          { b: 1, x: 0.2, y: 0.14, z: 2.3 },
        ],
      },
    ],
  },

  // 4 bars = 16 beats
  infra: {
    id: 'infra',
    group: 'Infrastructure',
    title: 'Connections and cloud',
    kicker: 'SSH, AWS, Kubernetes, APIs, the web and MCP',
    shots: [
      { kind: 'screen', src: cap('connections.jpg'), beats: 2, card: { title: 'Connections', sub: 'SSH, databases, brokers' }, cam: [{ b: 0, x: 0.3, y: 0.3, z: 1.6 }, { b: 1, x: 0.3, y: 0.28, z: 1.75 }] },
      { kind: 'screen', src: cap('aws.jpg'), beats: 2, transition: 'whip', card: { title: 'AWS', sub: 'S3 · SQS · EC2 · Athena · EKS' }, cam: [{ b: 0, x: 0.22, y: 0.2, z: 1.8 }] },
      { kind: 'screen', src: cap('kubernetes.jpg'), beats: 2, transition: 'whip', card: { title: 'Kubernetes', sub: 'Clusters, workloads, logs' }, cam: [{ b: 0, x: 0.3, y: 0.2, z: 1.45 }] },
      { kind: 'screen', src: cap('api.jpg'), beats: 2, transition: 'whip', card: { title: 'API client', sub: 'Requests, environments, history' }, cam: [{ b: 0, x: 0.5, y: 0.3, z: 1.2 }] },
      { kind: 'screen', src: cap('browser.jpg'), beats: 2, transition: 'whip', card: { title: 'Browser', sub: 'Reader and live tabs for agents' }, cam: [{ b: 0, x: 0.6, y: 0.4, z: 1.15 }] },
      {
        kind: 'screen',
        src: cap('mcp-scope.jpg'),
        beats: 3,
        transition: 'push',
        label: 'An MCP server, available in every workspace',
        cam: [
          { b: 0, x: 0.5, y: 0.45, z: 1.2 },
          { b: 1, x: 0.5, y: 0.47, z: 2 },
        ],
      },
      {
        kind: 'screen',
        src: cap('mcp-scope-shared.jpg'),
        beats: 3,
        transition: 'whip',
        label: 'Another workspace has it, with the same credentials',
        cam: [
          { b: 0, x: 0.4, y: 0.25, z: 1.5 },
          { b: 1, x: 0.3, y: 0.22, z: 2 },
        ],
        callouts: [{ b: 1.5, x: 0.27, y: 0.215, label: 'Shared from Acme Storefront', side: 'bottom' }],
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

  // 4 bars = 16 beats — Check for Updates; the desktop shell's update
  // commands are mocked as in ui/e2e/desktop-updates.spec.ts, the restart
  // impact is the capture daemon's real count.
  updates: {
    id: 'updates',
    group: 'Everywhere',
    title: 'Updates',
    kicker: 'Check for Updates…, then restart when you choose',
    shots: [
      {
        kind: 'screen',
        src: cap('update-checking.jpg'),
        beats: 4,
        label: 'Otto ▸ Check for Updates…',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.2 },
          { b: 1, x: 0.5, y: 0.5, z: 2.2 },
        ],
      },
      {
        kind: 'screen',
        src: cap('update-available.jpg'),
        beats: 6,
        transition: 'push',
        label: 'What is new, then download',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.6 },
          { b: 1, x: 0.5, y: 0.5, z: 2.4 },
        ],
      },
      {
        kind: 'screen',
        src: cap('update-staged.jpg'),
        beats: 6,
        transition: 'whip',
        label: 'Restart to finish: what keeps running comes first',
        cam: [
          { b: 0, x: 0.5, y: 0.5, z: 1.6 },
          { b: 1, x: 0.5, y: 0.48, z: 2.4 },
        ],
        callouts: [{ b: 2, x: 0.62, y: 0.65, label: 'Restart now', side: 'bottom' }],
      },
    ],
  },

  outro: { id: 'outro', group: 'Everywhere', title: 'Everywhere', shots: [{ kind: 'custom', id: 'outro', beats: 24 }] },
};
