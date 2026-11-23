import { test } from 'node:test';
import assert from 'node:assert/strict';
import { StepClock, barPct, clockText, etaLeft, firstPicture, nextEta, sceneSoFar, stepSeconds, stripState } from '../src/modules/design-hall/create/progress.ts';
import { glbJson, orderMoves, rigFromGltf, rigWords } from '../src/modules/design-hall/create/rig.ts';
import { agentPickLabel, parseCreateRoute, productionBody, steerExample } from '../src/modules/design-hall/create/plan.ts';

const item = (over: Record<string, unknown>) =>
  ({ id: 'x', kind: 'object', name: 'X', description: '', enabled: true, details: {}, status: 'working', progress_pct: 0, ...over }) as never;
const prod = (over: Record<string, unknown>) =>
  ({ id: 'p', status: 'running', plan: { items: [] }, revisions: [], created_at: '', updated_at: '', ...over }) as never;

test('the clock reads like a kitchen timer', () => {
  assert.equal(clockText(0), '0:00');
  assert.equal(clockText(7.9), '0:07');
  assert.equal(clockText(161), '2:41');
  assert.equal(clockText(3725), '1:02:05');
  assert.equal(clockText(-3), '0:00');
});

test('a long step keeps the bar moving, eases, and never reaches the end of the step', () => {
  const shape = item({ step: { n: 2, of: 3, label: 'Building the 3D shape' }, progress_pct: 40 });
  // It starts at the step's mark (the server's 40 % is past the 33 % mark), never lower.
  assert.equal(Math.round(barPct(shape, 0)), 40);
  let last = -1;
  const deltas: number[] = [];
  for (const t of [0, 10, 30, 60, 120, 180, 300, 600, 3600, 36000]) {
    const v = barPct(shape, t);
    assert.ok(v >= last, `monotonic at ${t}s: ${v} < ${last}`);
    assert.ok(v < 200 / 3, `never at the step's end (66.7 %) at ${t}s: ${v}`);
    if (last >= 0) deltas.push(v - last);
    last = v;
  }
  // It visibly moves during the first minutes of a three-minute step (not a frozen bar) …
  assert.ok(barPct(shape, 60) - barPct(shape, 0) > 4, 'a minute in, the bar has moved');
  assert.ok(barPct(shape, 180) - barPct(shape, 60) > 3, 'and keeps moving');
  // … and slows as it nears the mark (eases), stopping short of it.
  assert.ok(barPct(shape, 36000) <= 40 + (200 / 3 - 40) * 0.9 + 1e-9);
  // The next step starts past wherever this one got to.
  const ready = item({ step: { n: 3, of: 3, label: 'Getting it ready' }, progress_pct: 66 });
  assert.ok(barPct(ready, 0) >= barPct(shape, 36000));
  assert.ok(barPct(ready, 36000) < 100);
});

test('without step words the bar still eases from the server’s percent and stays under 100', () => {
  const old = item({ progress_pct: 90 });
  assert.equal(barPct(old, 0), 90);
  assert.ok(barPct(old, 99999) < 99.0001);
  assert.ok(barPct(old, 60) > 90);
});

test('each step’s usual length comes from its plain words', () => {
  assert.equal(stepSeconds(item({ kind: 'object', step: { n: 2, of: 3, label: 'Building the 3D shape' } })), 170);
  assert.equal(stepSeconds(item({ kind: 'character', step: { n: 4, of: 5, label: 'Adding a skeleton' } })), 300);
  assert.equal(stepSeconds(item({ kind: 'object', step: { n: 1, of: 1, label: 'Looking in the library' } })), 2);
  assert.equal(stepSeconds(item({ kind: 'character' })), 780);
});

test('a step’s clock starts when the piece started (first step) or when the step was first seen, and holds between polls', () => {
  const c = new StepClock();
  const since = '2026-11-20T20:00:00Z';
  const t0 = Date.parse(since);
  assert.equal(c.start({ id: 'm', step: { n: 1, of: 3, label: 'Drawing' }, working_since: since }, t0 + 50_000), t0);
  assert.equal(c.start({ id: 'm', step: { n: 2, of: 3, label: 'Shape' }, working_since: since }, t0 + 90_000), t0 + 90_000);
  assert.equal(c.start({ id: 'm', step: { n: 2, of: 3, label: 'Shape' }, working_since: since }, t0 + 95_000), t0 + 90_000);
});

test('one time left: counts down between server updates, drops at once, only goes up for a real slip', () => {
  let e = nextEta(null, 720, 0);
  assert.equal(etaLeft(e, 60_000), 660);
  // The server says the same 720 on every poll while the step lasts: still counting down.
  e = nextEta(e, 720, 60_000);
  assert.equal(etaLeft(e, 90_000), 630);
  // A step ended early: lower at once.
  e = nextEta(e, 500, 100_000);
  assert.equal(etaLeft(e, 100_000), 500);
  // A small bump up is ignored (no jumping numbers) …
  e = nextEta(e, 560, 110_000);
  assert.equal(etaLeft(e, 110_000), 490);
  // … a real slip (two minutes or more) is said.
  e = nextEta(e, 800, 120_000);
  assert.equal(etaLeft(e, 120_000), 800);
  assert.equal(etaLeft(e, 10_000_000), 0);
});

test('the strip says each creation’s state in a few words', () => {
  const items = [item({ status: 'done' }), item({ id: 'b', status: 'done' }), item({ id: 'c', status: 'working' }), item({ id: 's', kind: 'sound', status: 'waiting' })];
  assert.equal(stripState(prod({ status: 'planning' }), false), 'Planning');
  assert.equal(stripState(prod({ queue_position: 1, plan: { items } }), false), 'Waiting, 1 ahead');
  assert.equal(stripState(prod({ plan: { items } }), false), 'Making 3 of 3');
  assert.equal(stripState(prod({ plan: { items } }), true), 'Making 3 of 4');
  assert.equal(stripState(prod({ queue_position: 0, plan: { items } }), true), 'Making 3 of 4');
  assert.equal(stripState(prod({ status: 'done' }), false), 'Ready to look');
});

test('“See the scene so far” only while some pieces are made, some still coming, and a scene exists', () => {
  const made = item({ status: 'done', result: { artifact_id: 'a' } });
  const coming = item({ id: 'b', status: 'waiting' });
  assert.equal(sceneSoFar(prod({ scene_artifact_id: 's', plan: { items: [made, coming] } }), false), true);
  assert.equal(sceneSoFar(prod({ plan: { items: [made, coming] } }), false), false);
  assert.equal(sceneSoFar(prod({ scene_artifact_id: 's', plan: { items: [made] } }), false), false);
  assert.equal(sceneSoFar(prod({ scene_artifact_id: 's', plan: { items: [coming] } }), false), false);
  // A person already in the scene while its skeleton is still being added: look-at-able.
  const adding = item({ kind: 'character', status: 'working', result: { artifact_id: 'm' }, step: { n: 4, of: 5, label: 'Adding a skeleton' } });
  assert.equal(sceneSoFar(prod({ scene_artifact_id: 's', plan: { items: [adding] } }), false), true);
  // Sounds that can't be made today are not "still coming".
  assert.equal(sceneSoFar(prod({ scene_artifact_id: 's', plan: { items: [made, item({ id: 's', kind: 'music', status: 'waiting' })] } }), false), false);
});

/** A minimal GLB: header + JSON chunk (+ a stand-in BIN chunk that is never read). */
function glb(json: unknown): Uint8Array {
  let text = JSON.stringify(json);
  while (text.length % 4) text += ' ';
  const body = new TextEncoder().encode(text);
  const out = new Uint8Array(20 + body.length + 16);
  const v = new DataView(out.buffer);
  v.setUint32(0, 0x46546c67, true);
  v.setUint32(4, 2, true);
  v.setUint32(8, out.length, true);
  v.setUint32(12, body.length, true);
  v.setUint32(16, 0x4e4f534a, true);
  out.set(body, 20);
  return out;
}

test('the moves come from the clips the model file carries, in one plain order', () => {
  const bytes = glb({ asset: { version: '2.0' }, skins: [{ joints: [0, 1] }], animations: [{ name: 'idle' }, { name: 'run' }, { name: 'walk' }] });
  // Not enough bytes yet: wait for more (the download is read only this far).
  assert.equal(glbJson(bytes.subarray(0, 30)), null);
  const rig = rigFromGltf(glbJson(bytes) as never);
  assert.deepEqual(rig, { skeleton: true, clips: ['idle', 'run', 'walk'] });
  assert.equal(rigWords(rig), 'Skeleton added · 3 moves');
  const ordered = orderMoves(rig.clips.map((name, index) => ({ name, index })));
  assert.deepEqual(ordered.map((c) => c.index), [0, 2, 1], 'Stand, Walk, Run');
  assert.deepEqual(orderMoves([{ name: 'wave' }, { name: 'walk' }]).map((c) => c.name), ['walk', 'wave']);
  // A skeleton without moves yet, and a statue.
  assert.equal(rigWords(rigFromGltf({ skins: [{}] })), 'Skeleton added');
  assert.equal(rigWords(rigFromGltf({})), null);
  assert.equal(glbJson(new TextEncoder().encode('not a model at all, just text')), null);
});

test('the planner chip names the effort in plain words, and the create body carries it', () => {
  assert.equal(agentPickLabel('Claude Code', 'opus', 'Opus 5.5', 'High'), 'Claude Code · Opus 5.5 · High');
  assert.equal(agentPickLabel('Claude Code', null, '', 'Low'), 'Claude Code · Default · Low');
  assert.equal(agentPickLabel('Claude Code', 'opus', 'Opus 5.5'), 'Claude Code · Opus 5.5');
  assert.deepEqual(productionBody('w1', 'a rock', { provider: 'claude', model: 'opus', effort: 'high' }), {
    workspace_id: 'w1', description: 'a rock', use: 'pc', provider: 'claude', model: 'opus', effort: 'high',
  });
  assert.deepEqual(productionBody('w1', 'a rock', { provider: 'claude', model: null, effort: null }), {
    workspace_id: 'w1', description: 'a rock', use: 'pc', provider: 'claude',
  });
});

test('the change box’s example comes from this scene, else it is neutral', () => {
  const robot = item({ kind: 'character', name: 'Robot lighthouse keeper' });
  const crate = item({ id: 'c', kind: 'object', name: 'The crate' });
  assert.equal(steerExample([robot, crate]), 'Make the robot lighthouse keeper bigger. Move the crate…');
  assert.equal(steerExample([crate]), 'Move the crate…');
  assert.equal(steerExample([item({ kind: 'voice_line', name: 'Hello' })]), 'Tell me what to change…');
});

test('the scene so far is its own step (Back returns to the list), and a creation’s picture is its first drawn piece', () => {
  assert.deepEqual(parseCreateRoute(['p', 'abc', 'scene']), { view: 'production', id: 'abc', scene: true });
  assert.deepEqual(parseCreateRoute(['p', 'abc']), { view: 'production', id: 'abc' });
  const voice = item({ id: 'v', kind: 'voice_line', result: { clip_id: 'c' } });
  const drawn = item({ id: 'm', kind: 'character', has_preview: true, preview_rev: 2 });
  assert.deepEqual(firstPicture(prod({ id: 'p1', plan: { items: [voice, drawn] } })), { artifact: null, preview: 'p1/m/2' });
  assert.equal(firstPicture(prod({ plan: { items: [voice] } })), null);
});
