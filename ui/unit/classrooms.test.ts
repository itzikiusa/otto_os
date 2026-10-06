// Home "Classrooms" widget — the pure parts: sessions/workspaces → scene model
// (rooms, seats, status → visual, back-row grouping, ordering, permissions),
// the layout math, tooltip copy, the token colour parser, and the headmaster's
// kick-out / detention wiring (confirm first, the app's delete path, role gate).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  MAX_BACK,
  MAX_COLS,
  MAX_FRONT,
  MIN_COLS,
  SEAT_X,
  alongPath,
  buildClassrooms,
  campusColumns,
  columnsFor,
  doorPosition,
  exitPath,
  providerInitials,
  providerToken,
  roomSummary,
  seatPosition,
  shortPath,
  studentWorld,
  tooltipLines,
  visualFor,
  type Student,
} from '../src/modules/home/classrooms/model.ts';
import { kickOut, kickOutPrompt, sendToDetention, type KickDeps } from '../src/modules/home/classrooms/actions.ts';
import { parseCssColor } from '../src/lib/cssColor.ts';
import type { Session } from '../src/lib/api/types.ts';

let n = 0;
function mk(p: Partial<Session> = {}): Session {
  n++;
  return {
    id: `s${n}`,
    workspace_id: 'w1',
    kind: 'agent',
    provider: 'claude',
    title: `Session ${n}`,
    status: 'idle',
    cwd: '/Users/me/code/otto_os/ui',
    provider_session_id: 'psid',
    connection_id: null,
    created_by: 'u1',
    created_at: '2026-10-05T10:00:00Z',
    last_active_at: `2026-10-05T10:${String(n % 60).padStart(2, '0')}:00Z`,
    archived: false,
    meta: {},
    ...p,
  };
}

const WS = [
  { id: 'w1', name: 'Otto', my_role: 'admin' },
  { id: 'w2', name: 'Casino', my_role: 'viewer' },
  { id: 'w3', name: 'Empty', my_role: 'editor' },
];

test('workspaces become rooms — current first, then busiest; empty rooms stay', () => {
  const m = buildClassrooms({
    workspaces: WS,
    currentId: 'w3',
    sessions: [mk({ workspace_id: 'w1' }), mk({ workspace_id: 'w2', status: 'working' })],
  });
  assert.deepEqual(
    m.rooms.map((r) => r.name),
    ['Empty', 'Casino', 'Otto'],
  );
  assert.equal(m.rooms[0].current, true);
  assert.equal(m.rooms[0].counts.total, 0);
  assert.equal(m.total, 2);
});

test('archived workspaces, archived sessions, connections and unknown workspaces are skipped', () => {
  const m = buildClassrooms({
    workspaces: [...WS, { id: 'w9', name: 'Old', archived: true }],
    currentId: 'w1',
    sessions: [
      mk({ archived: true }),
      mk({ kind: 'connection' }),
      mk({ workspace_id: 'w9' }),
      mk({ workspace_id: 'nope' }),
      mk(),
    ],
  });
  assert.equal(m.total, 1);
  assert.ok(!m.rooms.some((r) => r.name === 'Old'));
});

test('status → visual: working animates, needs-you raises a hand, suspended is a ghost', () => {
  assert.equal(visualFor('working'), 'working');
  assert.equal(visualFor('running'), 'working');
  assert.equal(visualFor('needs-you'), 'needs-you');
  assert.equal(visualFor('idle'), 'idle');
  assert.equal(visualFor('suspended'), 'away');
  assert.equal(visualFor('ended'), 'away');
  assert.equal(visualFor('failed'), 'away');
  assert.equal(visualFor('stale'), 'stale');

  const a = mk({ status: 'idle' });
  const b = mk({ status: 'reconnectable' });
  const c = mk({ status: 'idle' });
  const m = buildClassrooms({
    workspaces: WS,
    currentId: 'w1',
    sessions: [a, b, c],
    // The live (event-fed) status wins over the row; needs-you wins over live.
    statusOf: (id) => (id === a.id ? 'working' : undefined),
    needsYou: (id) => id === c.id,
  });
  const v = Object.fromEntries(m.students.map((s) => [s.id, s.visual]));
  assert.equal(v[a.id], 'working');
  assert.equal(v[b.id], 'away');
  assert.equal(v[c.id], 'needs-you');
  // Front-row order: needs-you, working, …, away.
  assert.deepEqual(
    m.rooms[0].students.map((s) => s.id),
    [c.id, a.id, b.id],
  );
  assert.deepEqual(m.rooms[0].counts, { working: 1, needsYou: 1, idle: 0, away: 1, total: 3 });
});

test('a dropped event socket marks working students stale (no animation claims)', () => {
  const m = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions: [mk({ status: 'working' })], stale: true });
  assert.equal(m.students[0].visual, 'stale');
});

test('background engine sessions sit in a capped back row, live ones only', () => {
  const fg = mk();
  const bgs = Array.from({ length: MAX_BACK + 3 }, () => mk({ status: 'working', meta: { source: 'workflow' } }));
  const exitedBg = mk({ status: 'exited', meta: { source: 'review' } });
  const m = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions: [fg, ...bgs, exitedBg] });
  const room = m.rooms[0];
  assert.deepEqual(room.students.map((s) => s.id), [fg.id]);
  assert.equal(room.backRow.length, MAX_BACK);
  assert.equal(room.overflow, 3);
  assert.ok(room.backRow.every((s) => s.background && s.source === 'workflow'));
  assert.ok(!m.students.some((s) => s.id === exitedBg.id), 'an exited engine step is history');
  // Back row seats are behind (−z) every front row seat.
  const minFront = Math.min(...room.students.map((s) => s.seat.z));
  assert.ok(room.backRow.every((s) => s.seat.z < minFront));
});

test('a crowded room caps its front rows and reports the overflow', () => {
  const many = Array.from({ length: MAX_FRONT + 4 }, () => mk());
  const m = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions: many });
  assert.equal(m.rooms[0].students.length, MAX_FRONT);
  assert.equal(m.rooms[0].overflow, 4);
  assert.equal(m.rooms[0].cols, MAX_COLS);
});

test('scratch sessions get a "No workspace" room only when there are any', () => {
  const none = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions: [] });
  assert.ok(!none.rooms.some((r) => r.scratch));
  const m = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions: [mk({ workspace_id: 'scratch' })] });
  const r = m.rooms.find((x) => x.scratch);
  assert.equal(r?.name, 'No workspace');
  assert.equal(r?.students[0].workspaceName, 'No workspace');
});

test('canManage follows the role in the session’s own workspace and Agents:Edit', () => {
  const sessions = [mk({ workspace_id: 'w1' }), mk({ workspace_id: 'w2' }), mk({ workspace_id: 'scratch' })];
  const m = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions });
  const can = Object.fromEntries(m.students.map((s) => [s.workspaceId, s.canManage]));
  assert.deepEqual(can, { w1: true, w2: false, scratch: true });
  const ro = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions, canEditAgents: false });
  assert.ok(ro.students.every((s) => !s.canManage));
});

test('provider identity: fixed category per built-in, stable hash for custom', () => {
  assert.equal(providerToken('claude'), '--cat-2');
  assert.equal(providerToken('codex'), '--cat-1');
  assert.equal(providerToken('agy'), '--cat-4');
  assert.equal(providerToken('shell'), '--cat-6');
  assert.equal(providerToken('grok'), providerToken('grok'));
  assert.ok(['--cat-3', '--cat-5'].includes(providerToken('grok')));
  assert.equal(providerInitials('claude'), 'CL');
  assert.equal(providerInitials('codex'), 'CX');
  assert.equal(providerInitials('grok-2'), 'GR');
  assert.equal(providerInitials(''), '?');
});

test('layout math: columns, seat positions, campus grid, no overlaps', () => {
  assert.equal(columnsFor(0), MIN_COLS);
  assert.equal(columnsFor(4), MIN_COLS);
  assert.equal(columnsFor(12), 5);
  assert.equal(columnsFor(500), MAX_COLS);
  // Seats are centred on the room and evenly pitched.
  const a = seatPosition(0, 0, 3, 2);
  const b = seatPosition(0, 2, 3, 2);
  assert.equal(a.x, -SEAT_X);
  assert.equal(b.x, SEAT_X);
  assert.ok(seatPosition(1, 0, 3, 2).z < a.z, 'row 1 is behind row 0');
  assert.equal(campusColumns(1), 1);
  assert.equal(campusColumns(4), 3);
  assert.ok(campusColumns(20) <= 6);

  // 20 rooms × 30 students: rooms never overlap and every seat is inside its room.
  const workspaces = Array.from({ length: 20 }, (_, i) => ({ id: `w${i}`, name: `WS ${i}`, my_role: 'editor' }));
  const sessions = workspaces.flatMap((w) => Array.from({ length: 30 }, () => mk({ workspace_id: w.id })));
  const m = buildClassrooms({ workspaces, currentId: 'w0', sessions });
  assert.equal(m.total, 600);
  for (let i = 0; i < m.rooms.length; i++) {
    const r = m.rooms[i];
    for (const s of [...r.students, ...r.backRow]) {
      assert.ok(Math.abs(s.seat.x) < r.width / 2 && Math.abs(s.seat.z) < r.depth / 2, `${s.id} inside ${r.name}`);
    }
    for (let j = i + 1; j < m.rooms.length; j++) {
      const o = m.rooms[j];
      const apart = Math.abs(r.x - o.x) >= (r.width + o.width) / 2 || Math.abs(r.z - o.z) >= (r.depth + o.depth) / 2;
      assert.ok(apart, `${r.name} and ${o.name} overlap`);
    }
  }
  const first = m.students[0];
  const w = studentWorld(m, first.id);
  assert.ok(w && w.room.id === 'w0');
});

test('kick-out path: from the desk, through the door, out of the room', () => {
  const room = { x: 10, z: 5, width: 6, depth: 8 };
  const door = doorPosition(room);
  assert.equal(door.z, 9);
  assert.ok(door.x < 13 && door.x > 10);
  const path = exitPath({ x: 8, z: 4 }, door);
  assert.deepEqual(alongPath(path, 0), { x: 8, z: 4 });
  const end = alongPath(path, 1);
  assert.ok(end.z > door.z, 'ends outside the front wall');
  const mid = alongPath(path, 0.5);
  assert.ok(mid.z > 4 && mid.z <= end.z);
  assert.deepEqual(alongPath([], 0.5), { x: 0, z: 0 });
});

test('room summary + tooltip copy', () => {
  assert.equal(roomSummary({ counts: { working: 0, needsYou: 0, idle: 0, away: 0, total: 0 } }), 'Empty');
  assert.equal(roomSummary({ counts: { working: 2, needsYou: 1, idle: 3, away: 0, total: 6 } }), '1 needs you · 2 working · 3 idle');
  assert.equal(roomSummary({ counts: { working: 0, needsYou: 2, idle: 0, away: 1, total: 3 } }), '2 need you · 1 away');
  assert.equal(shortPath('/Users/me/code/otto_os/ui'), '…/otto_os/ui');
  assert.equal(shortPath('/tmp'), '/tmp');

  const m = buildClassrooms({
    workspaces: WS,
    currentId: 'w1',
    sessions: [mk({ title: 'Fix the bug', provider: 'codex', status: 'working', meta: { branch: 'feat/x' } }), mk({ meta: { source: 'swarm' }, status: 'idle' })],
  });
  const fg = m.students.find((s) => !s.background)!;
  const t = tooltipLines(fg, '3m ago');
  assert.equal(t.title, 'Fix the bug');
  assert.deepEqual(t.lines, ['Codex · Working', 'Otto', 'Last active 3m ago', '…/otto_os/ui · ⎇ feat/x']);
  const bg = m.students.find((s) => s.background)!;
  assert.match(tooltipLines(bg, '').lines[1], /back row \(swarm\)/);
});

test('parseCssColor reads what getComputedStyle serializes', () => {
  assert.deepEqual(parseCssColor('rgb(255, 0, 51)'), { r: 1, g: 0, b: 0.2, a: 1 });
  assert.deepEqual(parseCssColor('rgba(0, 0, 0, 0.5)'), { r: 0, g: 0, b: 0, a: 0.5 });
  assert.deepEqual(parseCssColor('rgb(255 255 255 / 25%)'), { r: 1, g: 1, b: 1, a: 0.25 });
  assert.deepEqual(parseCssColor('color(srgb 0.5 0.25 1 / 0.2)'), { r: 0.5, g: 0.25, b: 1, a: 0.2 });
  assert.deepEqual(parseCssColor('#f00'), { r: 1, g: 0, b: 0, a: 1 });
  assert.equal(parseCssColor('transparent')?.a, 0);
  assert.equal(parseCssColor('color(display-p3 1 0 0)'), null);
  assert.equal(parseCssColor('var(--x)'), null);
  assert.equal(parseCssColor(''), null);
});

// ── Headmaster actions ─────────────────────────────────────────────────────

function student(p: Partial<Student> = {}): Student {
  const m = buildClassrooms({ workspaces: WS, currentId: 'w1', sessions: [mk({ title: 'Refactor', status: 'idle' })] });
  return { ...m.students[0], ...p };
}

function fakes(confirmed: boolean, killErr?: Error) {
  const calls: string[] = [];
  let prompt = '';
  let opts: unknown = null;
  const deps: KickDeps = {
    ask: async (message, o) => {
      calls.push('confirm');
      prompt = message;
      opts = o;
      return confirmed;
    },
    kill: async (id) => {
      calls.push(`kill:${id}`);
      if (killErr) throw killErr;
    },
    animate: async (id) => {
      calls.push(`animate:${id}`);
    },
    restore: (id) => calls.push(`restore:${id}`),
    done: (title) => calls.push(`done:${title}`),
    failed: (title) => calls.push(`failed:${title}`),
  };
  return { deps, calls, prompt: () => prompt, opts: () => opts };
}

test('kick out asks first (danger) and deletes through the app’s delete path', async () => {
  const s = student();
  const f = fakes(true);
  assert.equal(await kickOut(s, f.deps), true);
  assert.equal(f.calls[0], 'confirm');
  assert.ok(f.calls.includes(`kill:${s.id}`));
  assert.ok(f.calls.includes(`animate:${s.id}`));
  assert.equal(f.calls.at(-1), 'done:Kicked out Refactor');
  assert.deepEqual(f.opts(), { title: 'Kick out student', confirmLabel: 'Kick out', danger: true });
  assert.match(f.prompt(), /“Refactor”/);
  assert.match(f.prompt(), /“Otto”/);
  assert.match(f.prompt(), /entire history/);
  assert.match(f.prompt(), /no Undo/);
});

test('kick out: cancel deletes nothing; a viewer is refused without a prompt', async () => {
  const f = fakes(false);
  assert.equal(await kickOut(student(), f.deps), false);
  assert.deepEqual(f.calls, ['confirm']);
  const g = fakes(true);
  assert.equal(await kickOut(student({ canManage: false }), g.deps), false);
  assert.deepEqual(g.calls, []);
});

test('kick out a working agent: mid-turn wording', () => {
  const p = kickOutPrompt(student({ visual: 'working' }));
  assert.match(p.message, /mid-turn/);
  assert.equal(p.opts.title, 'Kick out a working agent');
  assert.equal(p.opts.danger, true);
  assert.doesNotMatch(kickOutPrompt(student()).message, /mid-turn/);
});

test('kick out: a failed delete puts the student back and reports it', async () => {
  const s = student();
  const f = fakes(true, new Error('403'));
  assert.equal(await kickOut(s, f.deps), false);
  assert.ok(f.calls.includes(`restore:${s.id}`));
  assert.equal(f.calls.at(-1), 'failed:Couldn’t kick out “Refactor”');
  assert.ok(!f.calls.some((c) => c.startsWith('done:')));
});

test('detention archives (no confirm — it is resumable) and respects the role gate', async () => {
  const archived: string[] = [];
  const failed: string[] = [];
  const deps = { archive: async (id: string) => void archived.push(id), failed: (t: string) => void failed.push(t) };
  const s = student();
  assert.equal(await sendToDetention(s, deps), true);
  assert.deepEqual(archived, [s.id]);
  assert.equal(await sendToDetention(student({ canManage: false }), deps), false);
  assert.equal(archived.length, 1);
  const bad = { archive: async () => Promise.reject(new Error('x')), failed: (t: string) => void failed.push(t) };
  assert.equal(await sendToDetention(s, bad), false);
  assert.equal(failed[0], 'Couldn’t send “Refactor” to detention');
});

test('detention: a cancelled working-guard confirm is not a detention', async () => {
  const failed: string[] = [];
  const deps = { archive: async () => false, failed: (t: string) => void failed.push(t) };
  assert.equal(await sendToDetention(student({ visual: 'working' }), deps), false);
  assert.deepEqual(failed, []);
});

test('kick-out of a back-row engine session says which run loses it', () => {
  const p = kickOutPrompt(student({ background: true, source: 'workflow' }));
  assert.match(p.message, /running workflow session/);
  assert.match(p.message, /workflow run that owns it loses it/);
  assert.doesNotMatch(kickOutPrompt(student()).message, /run that owns it/);
});

test('detention hands the archive guard what the scene showed (S14-301)', async () => {
  const hints: unknown[] = [];
  const deps = { archive: async (_id: string, h: unknown) => { hints.push(h); return true; }, failed() {} };
  await sendToDetention(student({ visual: 'working', background: true, source: 'pr_review' }), deps);
  await sendToDetention(student(), deps);
  assert.equal(JSON.stringify(hints[0]), JSON.stringify({ working: true, title: 'Refactor', engine: 'pr review' }));
  assert.equal(JSON.stringify(hints[1]), JSON.stringify({ working: false, title: 'Refactor', engine: null }));
});

test('engine copy shows a readable source label, not the raw id (S14-305)', () => {
  const p = kickOutPrompt(student({ background: true, source: 'pr_review' }));
  assert.match(p.message, /running pr review session/);
  assert.doesNotMatch(p.message, /pr_review/);
});

test('the box wires kick-out to ws.killSession and detention to the guarded ws.requestArchive', () => {
  const src = readFileSync(join(import.meta.dirname, '..', 'src/modules/home/boxes/ClassroomsBox.svelte'), 'utf8');
  assert.match(src, /kill: \(id\) => ws\.killSession\(id\)/);
  assert.match(src, /archive: \(id, hint\) => ws\.requestArchive\(id, hint\)/);
  assert.doesNotMatch(src, /ws\.archiveSession\(/, 'detention must go through the working-guard');
  assert.match(src, /ask: \(message, opts\) => confirmer\.ask\(message, opts\)/);
  assert.doesNotMatch(src, /[^.\w]confirm\(/, 'never the native confirm()');
  // Destructive rows only for students the caller can manage.
  assert.match(src, /s\.canManage\s*\n?\s*\?/);
});

test('a kicked row is held in the model until its walk-out finishes (S14-304)', () => {
  const src = readFileSync(join(import.meta.dirname, '..', 'src/modules/home/boxes/ClassroomsBox.svelte'), 'utf8');
  // The held rows join the merged list unless a fresher source still has them…
  assert.match(src, /for \(const \[id, row\] of walking\) if \(!m\.has\(id\)\) m\.set\(id, row\);\s*for \(const id of removed\) m\.delete\(id\);/);
  // …and are released only when the scene's walk-out promise settles.
  assert.match(src, /walking = new Map\(\[\.\.\.walking, \[id, row\]\]\);\s*return handle\.kickOut\(id\)\.finally\(/);
});
