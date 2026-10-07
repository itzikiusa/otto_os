// Otto School — the pure halves: the school model (rooms / seats / lanes),
// the furnishing (nothing blocks a walkway, doors line up) and the ambient
// life director (who wanders, who never may, the headmaster's rounds,
// kick-out and detention walks). The 3D scene only draws what these decide.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ROOM_BACK_Z,
  buildSchool,
  characterFor,
  doorInside,
  lanePath,
  laneZ,
  poseFor,
  roomSummary,
  toWorld,
  type Room,
} from '../src/modules/home/school/model.ts';
import { deskGrid, furnishCorridor, furnishRoom, walkways } from '../src/modules/home/school/furnish.ts';
import { BORED_MS, Life, HEAD_CLIPS, KID_CLIPS } from '../src/modules/home/school/life.ts';

/** Life with a wall clock right after the seeded sessions were active. */
const FRESH = () => Date.UTC(2026, 0, 1, 0, 0);
const life0 = (seed: number, extra: { reducedMotion?: boolean } = {}) => new Life({ seed, now: FRESH, ...extra });
import type { Session } from '../src/lib/api/types';

let n = 0;
function sess(p: Partial<Session> & { workspace_id: string }): Session {
  n++;
  return {
    id: p.id ?? `s${n}`,
    kind: 'agent',
    provider: 'claude',
    title: `Session ${n}`,
    status: 'working',
    cwd: '/tmp',
    created_at: new Date(Date.UTC(2026, 0, 1, 0, n)).toISOString(),
    last_active_at: new Date(Date.UTC(2026, 0, 1, 0, n)).toISOString(),
    archived: false,
    meta: {},
    ...p,
  } as Session;
}

const WS = [
  { id: 'w1', name: 'Otto', my_role: 'admin' },
  { id: 'w2', name: 'Payments', my_role: 'viewer' },
];

function school(sessions: Session[], opts: { status?: Record<string, string>; needs?: string[]; archived?: Session[] } = {}) {
  return buildSchool({
    workspaces: WS,
    currentId: 'w1',
    sessions,
    archived: opts.archived,
    statusOf: (id) => opts.status?.[id],
    needsYou: (id) => opts.needs?.includes(id) ?? false,
  });
}

// ── Model ────────────────────────────────────────────────────────────────

test('every workspace is a classroom door, the current one first', () => {
  const s = school([sess({ workspace_id: 'w2' })]);
  assert.deepEqual(
    s.rooms.map((r) => r.id),
    ['w1', 'w2'],
  );
  assert.equal(s.rooms[0].current, true);
  assert.equal(s.rooms[1].doorX - s.rooms[0].doorX, 6);
  assert.ok(s.corridorLength > s.rooms[1].doorX);
});

test('status → pose: only real output is typing; an alive-but-quiet process sits idle', () => {
  assert.equal(poseFor('working'), 'working');
  assert.equal(poseFor('running'), 'idle');
  assert.equal(poseFor('idle'), 'idle');
  assert.equal(poseFor('needs-you'), 'needs-you');
  assert.equal(poseFor('suspended'), 'away');
  assert.equal(poseFor('ended'), 'away');
});

test('each provider gets its own kid; unknown providers the custom kid', () => {
  assert.equal(characterFor('claude'), 'claude');
  assert.equal(characterFor('codex'), 'codex');
  assert.equal(characterFor('Grok'), 'grok');
  assert.equal(characterFor('agy'), 'agy');
  assert.equal(characterFor('shell'), 'shell');
  assert.equal(characterFor('my-llm'), 'custom');
});

test('seating is stable: a status change never moves a kid to another desk', () => {
  const a = sess({ workspace_id: 'w1', id: 'a' });
  const b = sess({ workspace_id: 'w1', id: 'b' });
  const before = school([a, b], { status: { a: 'idle', b: 'idle' } });
  const after = school([a, b], { status: { a: 'idle', b: 'idle' }, needs: ['b'] });
  const seat = (sc: typeof before, id: string) => sc.kids.find((k) => k.id === id)!.seat;
  assert.deepEqual(seat(before, 'b'), seat(after, 'b'));
  assert.equal(after.kids.find((k) => k.id === 'b')!.pose, 'needs-you');
});

test('a room hangs behind its corridor door: back wall on the corridor, door segment on its door', () => {
  const s = school([sess({ workspace_id: 'w1' })]);
  const r = s.rooms[0];
  assert.equal(r.cz + r.depth / 2, ROOM_BACK_Z);
  assert.equal(toWorld(r, doorInside(r)).x, r.doorX);
  assert.equal(r.width % 2, 0);
  assert.equal(r.depth % 2, 0);
});

test('engine sessions sit in the back row only while live; viewers cannot manage', () => {
  const live = sess({ workspace_id: 'w2', meta: { source: 'workflow' } });
  const dead = sess({ workspace_id: 'w2', meta: { source: 'workflow' }, status: 'exited' });
  const s = school([live, dead], { status: { [live.id]: 'working', [dead.id]: 'exited' } });
  const r = s.rooms.find((x) => x.id === 'w2')!;
  assert.deepEqual(
    r.kids.map((k) => k.id),
    [live.id],
  );
  assert.equal(r.kids[0].background, true);
  assert.equal(r.kids[0].row, r.rows, 'the back row comes after the front rows');
  assert.equal(r.kids[0].canManage, false);
});

test('archived sessions sit on the detention bench (newest first, max 3)', () => {
  const arch = [1, 2, 3, 4].map((i) => sess({ workspace_id: 'w1', archived: true, id: `x${i}`, last_active_at: `2026-02-0${i}T00:00:00Z` }));
  const s = school([], { archived: arch });
  const r = s.rooms[0];
  assert.deepEqual(
    r.bench.map((k) => k.id),
    ['x4', 'x3', 'x2'],
  );
  assert.ok(r.bench.every((k) => k.detention));
  assert.equal(r.counts.detention, 3);
});

test('complete membership keeps overflow sessions actionable while geometry stays capped', () => {
  const front = Array.from({ length: 38 }, (_, i) => sess({ workspace_id: 'w1', id: `front-${i}` }));
  const back = Array.from({ length: 8 }, (_, i) => sess({ workspace_id: 'w1', id: `engine-${i}`, meta: { source: 'workflow' } }));
  const archived = Array.from({ length: 5 }, (_, i) => sess({ workspace_id: 'w1', id: `archived-${i}`, archived: true }));
  const s = school([...front, ...back], { archived, needs: ['engine-7'] });
  const r = s.rooms[0];
  assert.equal(r.kids.length, 42);
  assert.equal(r.bench.length, 3);
  assert.equal(r.kids.length + r.bench.length + 1, 46, 'includes the headmaster');
  assert.equal(r.overflow, 4);
  assert.equal(s.kids.length, 51, 'the accessible membership includes every session, including archives');
  assert.equal(s.kids.find((k) => k.id === 'engine-7')?.pose, 'needs-you');
});

test('roomSummary reads like a door sign', () => {
  const s = school([sess({ workspace_id: 'w1', id: 'q' }), sess({ workspace_id: 'w1', id: 'w' })], { needs: ['q'], status: { w: 'working' } });
  assert.match(roomSummary(s.rooms[0]), /1 needs you/);
  assert.equal(roomSummary(s.rooms[1]), 'Empty classroom');
});

// ── Furnishing ───────────────────────────────────────────────────────────

function bigRoom(): Room {
  const ss = Array.from({ length: 20 }, () => sess({ workspace_id: 'w1' }));
  return school(ss).rooms[0];
}

test('a full grid of desks, each with a chair, and one door each side of the wall', () => {
  const r = bigRoom();
  const p = furnishRoom(r);
  assert.equal(p.filter((x) => x.node === 'Workstation').length, deskGrid(r).length);
  assert.equal(p.filter((x) => x.node === 'Chair').length, deskGrid(r).length);
  const door = p.find((x) => x.node === 'Wall_Door')!;
  const corridor = furnishCorridor([r], 18).find((x) => x.node === 'Corridor_Door')!;
  assert.equal(door.x, corridor.x, 'the classroom door lines up with its corridor door');
});

test('no furniture footprint lands on a walkway', () => {
  const r = bigRoom();
  const ways = walkways(r);
  const solid = furnishRoom(r).filter((x) => ['Workstation', 'TeacherDesk', 'Bookshelf', 'Bench', 'Plant'].includes(x.node));
  const half: Record<string, [number, number]> = { Workstation: [0.6, 0.3], TeacherDesk: [0.8, 0.4], Bookshelf: [0.6, 0.2], Bench: [0.8, 0.2], Plant: [0.25, 0.25] };
  for (const s of solid) {
    let [hx, hz] = half[s.node];
    if (Math.abs(Math.abs(s.rotY) - Math.PI / 2) < 1e-6) [hx, hz] = [hz, hx];
    for (const w of ways) {
      const overlap = s.x - hx < w.x1 && s.x + hx > w.x0 && s.z - hz < w.z1 && s.z + hz > w.z0;
      assert.equal(overlap, false, `${s.node} at (${s.x.toFixed(2)}, ${s.z.toFixed(2)}) blocks a walkway`);
    }
  }
});

test('lanePath only walks axis-aligned along lanes and aisles', () => {
  const r = bigRoom();
  const a = r.kids[0];
  const b = r.kids[r.kids.length - 1];
  const p = lanePath(r, a.seat, laneZ(r, a.row), b.seat, laneZ(r, b.row));
  for (let i = 1; i < p.length; i++) assert.ok(p[i].x === p[i - 1].x || p[i].z === p[i - 1].z, 'segment is axis-aligned');
  assert.deepEqual(p[0], a.seat);
  assert.deepEqual(p[p.length - 1], b.seat);
});

// ── Life ─────────────────────────────────────────────────────────────────

function run(life: Life, ms: number, every = 50, onTick?: () => void): void {
  for (let t = 0; t < ms; t += every) {
    life.step(every);
    onTick?.();
  }
}

test('busy kids never leave their desks; idle kids do wander, and come back', () => {
  const ss = Array.from({ length: 12 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}` }));
  const status: Record<string, string> = {};
  ss.forEach((s, i) => (status[s.id] = i < 4 ? 'working' : 'idle'));
  const r = school(ss, { status, needs: ['k0'] }).rooms[0];
  const life = life0(7);
  life.setRoom(r);
  const wandered = new Set<string>();
  let maxUp = 0;
  run(life, 90_000, 50, () => {
    let up = 0;
    for (const v of life.kidViews()) {
      const k = r.kids.find((x) => x.id === v.id)!;
      if (!v.seated) {
        up++;
        wandered.add(v.id);
        assert.equal(k.pose, 'idle', `${v.id} (${k.pose}) left its desk`);
      }
    }
    maxUp = Math.max(maxUp, up);
  });
  assert.ok(wandered.size >= 3, `only ${wandered.size} idle kids wandered in 90 s`);
  assert.ok(maxUp <= Math.max(1, Math.floor(8 / 3)), `${maxUp} kids up at once (cap ⌊8/3⌋)`);
  for (const id of ['k0', 'k1', 'k2', 'k3']) assert.equal(life.kidView(id)!.clip, id === 'k0' ? 'Sit_RaiseHand' : 'Sit_Type');
});

test('a wandering kid hurries back when its session wakes up', () => {
  const ss = Array.from({ length: 6 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}` }));
  const idle = Object.fromEntries(ss.map((s) => [s.id, 'idle']));
  const life = life0(3);
  life.setRoom(school(ss, { status: idle }).rooms[0]);
  let who: string | null = null;
  for (let t = 0; t < 60_000 && !who; t += 50) {
    life.step(50);
    who = life.kidViews().find((v) => v.mode === 'visiting')?.id ?? null;
  }
  assert.ok(who, 'someone reached a wander spot');
  life.sync(school(ss, { status: { ...idle, [who!]: 'working' } }).rooms[0]);
  assert.equal(life.kidView(who!)!.mode, 'returning');
  run(life, 20_000);
  const v = life.kidView(who!)!;
  assert.equal(v.mode, 'seated');
  assert.equal(v.clip, 'Sit_Type');
});

test('the headmaster checks on the kid that needs you first', () => {
  const ss = Array.from({ length: 9 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}` }));
  const status = Object.fromEntries(ss.map((s) => [s.id, 'working']));
  const life = life0(11);
  life.setRoom(school(ss, { status, needs: ['k7'] }).rooms[0]);
  let inspected: string | null = null;
  for (let t = 0; t < 60_000 && !inspected; t += 50) {
    life.step(50);
    if (life.headView()!.mode === 'inspecting') inspected = life.headTarget();
  }
  assert.equal(inspected, 'k7');
  assert.equal(life.headView()!.clip, 'Inspect_Screen');
});

test('kick-out walks the kid out of the door; detention walks it to the bench', () => {
  const ss = Array.from({ length: 4 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}` }));
  const status = Object.fromEntries(ss.map((s) => [s.id, 'working']));
  const r = school(ss, { status }).rooms[0];
  const life = life0(1);
  life.setRoom(r);
  const ms = life.kick('k1');
  assert.ok(ms > 1000);
  life.step(100);
  assert.equal(life.kidView('k1')!.clip, 'Stand_Up');
  assert.equal(life.headView()!.clip, 'Scold');
  run(life, ms + 500);
  assert.equal(life.isGone('k1'), true);
  assert.equal(life.kidView('k1')!.visible, false);

  const dm = life.detention('k2');
  run(life, dm + 500);
  const v = life.kidView('k2')!;
  assert.equal(v.mode, 'bench');
  assert.equal(v.clip, 'Sit_Slump');
  assert.ok(v.z < -r.depth / 2 + 1.5, 'on the bench at the front, by the board');
  // The archive lands: the model now has it on the bench — it stays put.
  life.sync(school(ss.filter((s) => s.id !== 'k2' && s.id !== 'k1'), { status, archived: [{ ...ss[2], archived: true }] }).rooms[0]);
  run(life, 3000);
  assert.equal(life.kidView('k2')!.mode, 'bench');
});

test('restore puts a kicked kid back at its desk (the delete failed)', () => {
  const ss = [sess({ workspace_id: 'w1', id: 'k' })];
  const life = life0(2);
  life.setRoom(school(ss, { status: { k: 'working' } }).rooms[0]);
  life.kick('k');
  run(life, 1500);
  life.restore('k');
  const v = life.kidView('k')!;
  assert.equal(v.mode, 'seated');
  assert.equal(v.clip, 'Sit_Type');
});

test('reduced motion: nobody wanders, the headmaster stays at the board, kicks are instant', () => {
  const ss = Array.from({ length: 6 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}` }));
  const idle = Object.fromEntries(ss.map((s) => [s.id, 'idle']));
  const life = life0(5, { reducedMotion: true });
  life.setRoom(school(ss, { status: idle }).rooms[0]);
  run(life, 60_000, 100, () => {
    for (const v of life.kidViews()) assert.equal(v.seated, true);
    assert.equal(life.headView()!.mode, 'lectern');
  });
  assert.equal(life.kick('k0'), 0);
  assert.equal(life.isGone('k0'), true);
});

test('every clip life asks for is in the asset contract', () => {
  const contract = new Set([...KID_CLIPS, ...HEAD_CLIPS]);
  const ss = Array.from({ length: 10 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}` }));
  const status = Object.fromEntries(ss.map((s, i) => [s.id, i % 2 ? 'idle' : 'working']));
  const life = life0(9);
  life.setRoom(school(ss, { status, needs: ['k0'] }).rooms[0]);
  life.kick('k1');
  life.detention('k3');
  run(life, 120_000, 50, () => {
    for (const v of [...life.kidViews(), life.headView()!]) assert.ok(contract.has(v.clip as never), `unknown clip ${v.clip}`);
  });
});

test('an unanswered "needs you" frees the kid to wander after BORED_MS; a fresh one keeps the hand up', () => {
  const ss = Array.from({ length: 4 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}`, last_active_at: '2026-01-01T00:00:00Z' }));
  const room = school(ss, { needs: ss.map((s) => s.id) }).rooms[0];
  const fresh = new Life({ seed: 4, now: () => Date.parse('2026-01-01T00:00:10Z') });
  fresh.setRoom(room);
  run(fresh, 60_000, 100, () => {
    for (const v of fresh.kidViews()) assert.equal(v.seated, true, 'a fresh needs-you kid stays at its desk');
  });
  const bored = new Life({ seed: 4, now: () => Date.parse('2026-01-01T00:00:00Z') + BORED_MS + 1000 });
  bored.setRoom(room);
  let up = false;
  run(bored, 60_000, 100, () => {
    if (bored.kidViews().some((v) => !v.seated)) up = true;
  });
  assert.equal(up, true, 'a long-waiting kid gets up');
});
