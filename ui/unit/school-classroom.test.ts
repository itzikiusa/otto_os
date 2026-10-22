/// <reference lib="dom" />
// Otto School — the "at a glance" layer: real session state → what a kid
// does (types / raises a hand / dozes / cheers / slumps), the school-wide
// "needs you" queue and the busiest room, the hover copy, the status pins
// and what a monitor paints. Plus a headless play-test: a scripted school
// day where sessions start, finish, fail and wait, and the room must stay
// truthful the whole time.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ASLEEP_MS,
  attentionQueue,
  buildSchool,
  busiestRoom,
  isAsleep,
  kidActivity,
  roomSummary,
} from '../src/modules/home/school/model.ts';
import { CHEER_MS, HEAD_CLIPS, KID_CLIPS, Life, SIT_MS, STAND_MS, cheerClip, seatedClip } from '../src/modules/home/school/life.ts';
import { paintScreen, screenKey, statusText, SCREEN_W, type ScreenInfo } from '../src/modules/home/school/screens.ts';
import { pinFor, PIN_COLORS } from '../src/modules/home/school/scene.ts';
import type { Session } from '../src/lib/api/types';

const T0 = Date.UTC(2026, 0, 1, 12, 0);
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
    created_at: new Date(T0 - 3_600_000 + n * 1000).toISOString(),
    last_active_at: new Date(T0 - 1000).toISOString(),
    archived: false,
    meta: {},
    ...p,
  } as Session;
}
const WS = [
  { id: 'w1', name: 'Otto', my_role: 'admin' },
  { id: 'w2', name: 'Payments', my_role: 'admin' },
];
type Opts = { status?: Record<string, string>; needs?: string[]; exit?: Record<string, number>; archived?: Session[] };
const school = (ss: Session[], o: Opts = {}) =>
  buildSchool({
    workspaces: WS,
    currentId: 'w1',
    sessions: ss,
    archived: o.archived,
    statusOf: (id) => o.status?.[id],
    needsYou: (id) => o.needs?.includes(id) ?? false,
    exitCodeOf: (id) => o.exit?.[id],
  });
const run = (life: Life, ms: number, every = 50, onTick?: () => void) => {
  for (let t = 0; t < ms; t += every) {
    life.step(every);
    onTick?.();
  }
};

// ── Model ────────────────────────────────────────────────────────────────

test('ended sessions stay seated as done, non-zero exits as error, suspended ones leave the chair empty', () => {
  const ok = sess({ workspace_id: 'w1', id: 'ok', provider: 'shell', status: 'exited' });
  const bad = sess({ workspace_id: 'w1', id: 'bad', provider: 'shell', status: 'exited' });
  const meta = sess({ workspace_id: 'w1', id: 'meta', provider: 'shell', status: 'exited', meta: { exit_code: 2 } });
  const parked = sess({ workspace_id: 'w1', id: 'parked', status: 'exited', provider_session_id: 'abc' } as Partial<Session> & { workspace_id: string });
  const s = school([ok, bad, meta, parked], { exit: { bad: 137 } });
  const pose = (id: string) => s.kids.find((k) => k.id === id)!.pose;
  assert.equal(pose('ok'), 'done');
  assert.equal(pose('bad'), 'error');
  assert.equal(pose('meta'), 'error', 'an exit code carried in meta counts too');
  assert.equal(pose('parked'), 'away', 'a resumable agent is suspended, not done');
  assert.match(s.kids.find((k) => k.id === 'bad')!.stateLabel, /exit 137/);
  const r = s.rooms[0];
  assert.equal(r.counts.done, 1);
  assert.equal(r.counts.error, 2);
  assert.equal(roomSummary(r), '2 failed · 1 done · 1 away');
});

test('an engine that finished or failed leaves the back row (live engines only)', () => {
  const live = sess({ workspace_id: 'w2', id: 'live', meta: { source: 'workflow' } });
  const done = sess({ workspace_id: 'w2', id: 'done', status: 'exited', provider: 'shell', meta: { source: 'workflow' } });
  const failed = sess({ workspace_id: 'w2', id: 'failed', status: 'exited', provider: 'shell', meta: { source: 'swarm' } });
  const r = school([live, done, failed], { status: { live: 'working' }, exit: { failed: 1 } }).rooms.find((x) => x.id === 'w2')!;
  assert.deepEqual(
    r.kids.map((k) => k.id),
    ['live'],
  );
});

test('the attention queue lists every kid that needs you (longest waiting first), then the failed ones — school-wide', () => {
  const a = sess({ workspace_id: 'w1', id: 'a', title: 'Fresh ask', last_active_at: new Date(T0 - 5_000).toISOString() });
  const b = sess({ workspace_id: 'w2', id: 'b', title: 'Old ask', last_active_at: new Date(T0 - 600_000).toISOString() });
  const c = sess({ workspace_id: 'w2', id: 'c', title: 'Crashed', status: 'exited', provider: 'shell' });
  const d = sess({ workspace_id: 'w1', id: 'd', title: 'Busy' });
  const arch = sess({ workspace_id: 'w1', id: 'arch', archived: true, status: 'exited' });
  const s = school([a, b, c, d], { needs: ['a', 'b', 'arch'], exit: { c: 1 }, status: { d: 'working' }, archived: [arch] });
  assert.deepEqual(
    attentionQueue(s).map((k) => k.id),
    ['b', 'a', 'c'],
  );
});

test('the busiest room is the one that needs you most, then failures, then work; none when the school is empty', () => {
  const w1 = [sess({ workspace_id: 'w1', id: 'x' }), sess({ workspace_id: 'w1', id: 'y' })];
  const w2 = [sess({ workspace_id: 'w2', id: 'z' })];
  assert.equal(busiestRoom(school([...w1, ...w2], { status: { x: 'working', y: 'working', z: 'idle' }, needs: ['z'] }))!.id, 'w2');
  assert.equal(busiestRoom(school([...w1, ...w2], { status: { x: 'working', y: 'idle', z: 'idle' } }))!.id, 'w1');
  assert.equal(busiestRoom(school([])), null);
});

test('an idle kid dozes after ASLEEP_MS; a busy or archived one never does', () => {
  const idle = { pose: 'idle' as const, lastActiveAt: new Date(T0 - ASLEEP_MS - 1).toISOString(), detention: false };
  assert.equal(isAsleep(idle, T0), true);
  assert.equal(isAsleep({ ...idle, lastActiveAt: new Date(T0 - 60_000).toISOString() }, T0), false);
  assert.equal(isAsleep({ ...idle, pose: 'working' }, T0), false);
  assert.equal(isAsleep({ ...idle, detention: true }, T0), false);
  assert.equal(seatedClip('idle', true), 'Sit_Slump');
  assert.equal(seatedClip('idle', false), 'Sit_Idle');
  assert.equal(seatedClip('error'), 'Sit_Slump');
  assert.equal(seatedClip('done'), 'Sit_Idle');
});

test('the hover line says state, last activity and branch', () => {
  assert.equal(kidActivity({ stateLabel: 'Working', branch: 'main' }, '2m'), 'Working · active 2m ago · ⎇ main');
  assert.equal(kidActivity({ stateLabel: 'Idle', branch: null }, '—', true), 'Idle — dozing');
});

test('status pins: one per state worth seeing, none for quiet / away / detention kids', () => {
  assert.equal(pinFor('working', false), 'working');
  assert.equal(pinFor('needs-you', false), 'needs-you');
  assert.equal(pinFor('error', false), 'error');
  assert.equal(pinFor('done', false), 'done');
  assert.equal(pinFor('idle', false), null);
  assert.equal(pinFor('away', false), null);
  assert.equal(pinFor('needs-you', true), null);
  assert.equal(new Set(Object.values(PIN_COLORS)).size, Object.keys(PIN_COLORS).length, 'every pin colour is distinct');
});

// ── Life: dozing, cheering, the headmaster calling you ──────────────────

test('a kid idle for a long time dozes at its desk; a recent one sits up', () => {
  const old = sess({ workspace_id: 'w1', id: 'old', last_active_at: new Date(T0 - ASLEEP_MS - 5000).toISOString() });
  const fresh = sess({ workspace_id: 'w1', id: 'fresh' });
  const life = new Life({ seed: 1, now: () => T0, reducedMotion: true });
  life.setRoom(school([old, fresh], { status: { old: 'idle', fresh: 'idle' } }).rooms[0]);
  life.step(50);
  assert.equal(life.kidView('old')!.clip, 'Sit_Slump');
  assert.equal(life.kidView('fresh')!.clip, 'Sit_Idle');
});

test('when a session stops working its kid stands, cheers and sits back down; the headmaster gives a thumbs-up', () => {
  const ss = [sess({ workspace_id: 'w1', id: 'k' }), sess({ workspace_id: 'w1', id: 'j' })];
  const life = new Life({ seed: 2, now: () => T0 });
  life.setRoom(school(ss, { status: { k: 'working', j: 'working' } }).rooms[0]);
  life.step(50);
  life.sync(school(ss, { status: { k: 'idle', j: 'working' }, needs: ['k'] }).rooms[0]);
  const v = life.kidView('k')!;
  assert.equal(v.mode, 'celebrating');
  assert.equal(v.seated, false, 'the chair slides back while it stands');
  assert.equal(life.kidView('j')!.mode, 'seated', 'a kid still working keeps typing');
  assert.equal(life.headView()!.clip, 'ThumbsUp');
  const clips: string[] = [];
  run(life, STAND_MS + CHEER_MS + SIT_MS + 200, 50, () => {
    const c = life.kidView('k')!.clip;
    if (clips[clips.length - 1] !== c) clips.push(c);
  });
  assert.deepEqual(clips.slice(0, 4), ['Stand_Up', 'Wave', 'Sit_Down', 'Sit_RaiseHand']);
  assert.equal(life.kidView('k')!.mode, 'seated');
  assert.equal(cheerClip(STAND_MS + CHEER_MS + SIT_MS), 'Stand_Up');
  assert.equal(cheerClip(1), 'Sit_Down');
});

test('a cheer is cut short when the agent goes straight back to work; reduced motion never cheers', () => {
  const ss = [sess({ workspace_id: 'w1', id: 'k' })];
  const life = new Life({ seed: 3, now: () => T0 });
  life.setRoom(school(ss, { status: { k: 'working' } }).rooms[0]);
  life.sync(school(ss, { status: { k: 'idle' } }).rooms[0]);
  life.step(100);
  life.sync(school(ss, { status: { k: 'working' } }).rooms[0]);
  run(life, SIT_MS + 100);
  assert.equal(life.kidView('k')!.mode, 'seated');
  assert.equal(life.kidView('k')!.clip, 'Sit_Type');

  const calm = new Life({ seed: 3, now: () => T0, reducedMotion: true });
  calm.setRoom(school(ss, { status: { k: 'working' } }).rooms[0]);
  calm.sync(school(ss, { status: { k: 'idle' } }).rooms[0]);
  assert.equal(calm.kidView('k')!.mode, 'seated');
  assert.equal(calm.kidView('k')!.clip, 'Sit_Idle');
});

test('a status that never was "working" does not cheer (no fake celebrations)', () => {
  const ss = [sess({ workspace_id: 'w1', id: 'k' })];
  const life = new Life({ seed: 4, now: () => T0 });
  life.setRoom(school(ss, { status: { k: 'idle' } }).rooms[0]);
  life.sync(school(ss, { status: { k: 'idle' }, needs: ['k'] }).rooms[0]);
  assert.notEqual(life.kidView('k')!.mode, 'celebrating');
});

test('the headmaster reads a needs-you screen, then turns to you and waves; failed kids come before working ones', () => {
  const ss = Array.from({ length: 6 }, (_, i) => sess({ workspace_id: 'w1', id: `k${i}` }));
  const status = Object.fromEntries(ss.map((s) => [s.id, 'working']));
  const life = new Life({ seed: 11, now: () => T0 });
  life.setRoom(school(ss, { status, needs: ['k4'] }).rooms[0]);
  let called = false;
  run(life, 60_000, 50, () => {
    const h = life.headView()!;
    if (h.mode === 'calling') {
      called = true;
      assert.equal(h.clip, 'Wave');
      assert.equal(life.headTarget(), 'k4');
    }
  });
  assert.equal(called, true, 'the headmaster called you over');

  const ss2 = [sess({ workspace_id: 'w1', id: 'busy' }), sess({ workspace_id: 'w1', id: 'crash', provider: 'shell', status: 'exited' })];
  const life2 = new Life({ seed: 5, now: () => T0 });
  life2.setRoom(school(ss2, { status: { busy: 'working' }, exit: { crash: 1 } }).rooms[0]);
  let first: string | null = null;
  run(life2, 60_000, 50, () => {
    if (!first && life2.headView()!.mode === 'inspecting') first = life2.headTarget();
  });
  assert.equal(first, 'crash');
});

// ── Monitors ─────────────────────────────────────────────────────────────

/** A recording 2D context: enough surface for paintScreen. */
function recorder() {
  const texts: string[] = [];
  const transforms: number[][] = [];
  const noop = () => undefined;
  const ctx = {
    texts,
    transforms,
    fillStyle: '',
    strokeStyle: '',
    font: '',
    textAlign: '',
    textBaseline: '',
    lineWidth: 1,
    save: noop,
    restore: noop,
    fillRect: noop,
    strokeRect: noop,
    setTransform: (...a: number[]) => transforms.push(a),
    measureText: (t: string) => ({ width: t.length * 8 }),
    fillText: (t: string) => texts.push(t),
    createLinearGradient: () => ({ addColorStop: noop }),
  };
  return ctx;
}

test('monitors say needs-you / failed / done in words, and the focused one paints at 2×', () => {
  const base: ScreenInfo = { title: 'Fix it', provider: 'claude', character: 'claude', pose: 'working', feed: { live: true, lines: ['$ cargo test', 'test ok'] } };
  for (const [pose, words] of [
    ['needs-you', 'Waiting for you'],
    ['error', 'Exited with an error'],
    ['done', 'Finished'],
  ] as const) {
    const r = recorder();
    paintScreen(r as unknown as CanvasRenderingContext2D, { ...base, pose });
    assert.ok(r.texts.some((t) => t.includes(words)), `${pose} says "${words}"`);
    assert.ok(r.texts.includes(statusText(pose)));
  }
  const hi = recorder();
  paintScreen(hi as unknown as CanvasRenderingContext2D, base, 2);
  assert.deepEqual(hi.transforms[0], [2, 0, 0, 2, 0, 0]);
  assert.ok(hi.texts.includes('$ cargo test'));
  assert.notEqual(screenKey({ ...base, pose: 'done' }), screenKey(base), 'a state change repaints');
  assert.ok(SCREEN_W >= 512);
});

// ── Play-test: a school day ──────────────────────────────────────────────

test('play-test: a scripted school day stays truthful — cheers only follow real finishes, busy kids keep typing', () => {
  const ids = Array.from({ length: 12 }, (_, i) => `k${i}`);
  const ss = ids.map((id) => sess({ workspace_id: 'w1', id }));
  const status: Record<string, string> = Object.fromEntries(ids.map((id, i) => [id, i % 3 === 0 ? 'working' : 'idle']));
  const needs = new Set<string>();
  const exit: Record<string, number> = {};
  let clock = T0;
  const life = new Life({ seed: 21, now: () => clock });
  const build = () => school(ss, { status: { ...status }, needs: [...needs], exit: { ...exit } }).rooms[0];
  life.setRoom(build());
  const contract = new Set<string>([...KID_CLIPS, ...HEAD_CLIPS]);
  /** Kids allowed to be mid-cheer: those whose session left `working`. */
  const finished = new Set<string>();
  let cheers = 0;
  let calls = 0;
  const events: [number, () => void][] = [
    [20_000, () => { status.k0 = 'idle'; needs.add('k0'); finished.add('k0'); }],
    [40_000, () => { status.k1 = 'working'; }],
    [70_000, () => { status.k1 = 'idle'; finished.add('k1'); }],
    [90_000, () => { status.k3 = 'exited'; exit.k3 = 1; finished.add('k3'); }],
    [110_000, () => { status.k6 = 'exited'; finished.add('k6'); }],
    [130_000, () => { needs.delete('k0'); status.k0 = 'working'; }],
  ];
  const stepMs = 50;
  for (let t = 0; t < 180_000; t += stepMs) {
    clock += stepMs;
    while (events.length && events[0][0] <= t) {
      events.shift()![1]();
      life.sync(build());
    }
    life.step(stepMs);
    const room = build();
    for (const v of life.kidViews()) {
      const k = room.kids.find((x) => x.id === v.id)!;
      assert.ok(contract.has(v.clip), `unknown clip ${v.clip}`);
      if (v.mode === 'celebrating') {
        assert.ok(finished.has(v.id), `${v.id} cheered without finishing`);
        cheers++;
      }
      if (k.pose === 'working' && v.mode === 'seated') assert.equal(v.clip, 'Sit_Type', `${v.id} works without typing`);
      if (k.pose === 'error' && v.mode === 'seated') assert.equal(v.clip, 'Sit_Slump');
      if (k.pose === 'working') assert.ok(['seated', 'celebrating', 'returning', 'sitting', 'standing', 'walking'].includes(v.mode), `${v.id} wanders while working (${v.mode})`);
    }
    if (life.headView()!.mode === 'calling') calls++;
  }
  assert.ok(cheers > 0, 'somebody cheered');
  assert.ok(calls > 0, 'the headmaster called you over at least once');
  assert.equal(life.kidView('k3')!.clip, 'Sit_Slump', 'the failed kid slumps');
  assert.equal(life.kidView('k6')!.visible, true, 'a done kid stays in its chair');
});
