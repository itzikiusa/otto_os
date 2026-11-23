import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import {
  WHERE,
  agentPickLabel,
  productionBody,
  changePrompt,
  currentStep,
  groupItems,
  readPlan,
  jobSteps,
  moveLabel,
  parseCreateRoute,
  pickVoiceId,
  PLAIN_VOICES,
  planSecs,
  savePath,
  secondsLeft,
  timeText,
} from '../src/modules/design-hall/create/plan.ts';

const item = (over: Record<string, unknown>) =>
  ({ id: 'x', kind: 'object', name: 'X', description: '', enabled: true, details: {}, status: 'waiting', progress_pct: 0, ...over }) as never;

test('where choices map to growing detail, and only triangles differ by budget', () => {
  assert.ok(WHERE.phone.triangles < WHERE.pc.triangles && WHERE.pc.triangles < WHERE.closeup.triangles);
  assert.equal(WHERE.pc.label, 'Computer game');
});

test('a character from words walks all five plain steps; a picture skips the drawing', () => {
  assert.deepEqual(jobSteps({ prompt: 'knight', then_stages: ['rig', 'animate'] }), ['draw', 'shape', 'ready', 'skeleton', 'moves']);
  assert.deepEqual(jobSteps({ prompt: null, then_stages: [] }), ['shape', 'ready']);
});

test('the current step follows the stage and the engine words', () => {
  const steps = jobSteps({ prompt: 'k', then_stages: ['rig', 'animate'] });
  const at = (pipeline_stage: string, stage: string, status = 'running') => currentStep({ pipeline_stage, stage, status } as never, steps);
  assert.equal(at('generate', 'Generating 3D mesh with Otto Forge (local)'), 'draw');
  assert.equal(at('generate', 'drawing the concept picture (FLUX.2 klein 4B)'), 'draw');
  assert.equal(at('generate', 'building the 3D model (Pixal3D)'), 'shape');
  assert.equal(at('generate', 'game-ready pass to 25000 triangles, 3 LODs, collision'), 'ready');
  assert.equal(at('rig', 'fitting a skeleton'), 'skeleton');
  assert.equal(at('animate', 'retargeting walk'), 'moves');
  assert.equal(at('generate', 'Queued', 'queued'), 'draw');
});

test('time left never drops below the steps still to come', () => {
  const steps = jobSteps({ prompt: 'k', then_stages: ['rig', 'animate'] });
  assert.equal(secondsLeft(steps, 'moves', 10_000), 10);
  assert.ok(secondsLeft(steps, 'draw', 0) > 300);
  assert.equal(timeText(30), 'Less than a minute');
  assert.equal(timeText(320), 'About 5 minutes');
  assert.equal(timeText(3600), 'About an hour');
  assert.equal(timeText(9000), 'About 2 and a half hours');
});

test('the plan groups in plain words and leaves sounds out of the time when no sound maker', () => {
  const items = [item({ id: 'm', kind: 'character' }), item({ id: 's', kind: 'sound' }), item({ id: 'v', kind: 'voice_line' })];
  assert.deepEqual(groupItems(items).map((g) => g.label), ['People', 'Voices', 'Sounds and music']);
  assert.ok(planSecs(items) < planSecs(items, true));
});

test('the plan lists only what will really be made, and says the rest once under Not yet', () => {
  const items = [item({ id: 'm', kind: 'character', name: 'The man' }), item({ id: 's', kind: 'sound', name: 'Sword clashes' }), item({ id: 'k', kind: 'object', name: 'Key', enabled: false })];
  const off = readPlan(items, ['I can’t make a pulling move yet.', 'The coins sit inside the chest.'], false);
  assert.deepEqual(off.made.map((i: { id: string }) => i.id), ['m']);
  assert.ok(off.notYet.some((n) => n.startsWith('Sounds and music aren’t ready yet (sword clashes)')));
  assert.ok(off.notYet.some((n) => n.includes('stand, walk and run')));
  assert.ok(off.notYet.includes('The coins sit inside the chest.'));
  const on = readPlan(items, [], true);
  assert.deepEqual(on.made.map((i: { id: string }) => i.id), ['m', 's']);
});

test('a change rebuilds from the idea plus every note', () => {
  assert.equal(changePrompt('A knight.', [{ text: 'make the helmet red' }, { text: ' ' }, { text: 'longer sword' }]), 'A knight. Changes: make the helmet red; longer sword.');
  assert.equal(changePrompt('A knight', []), 'A knight');
});

test('save paths are tidy and never collide', () => {
  assert.equal(savePath('character', 'Brave Knight!', 'glb'), 'assets/characters/brave-knight.glb');
  assert.equal(savePath('object', '', 'glb', 2), 'assets/objects/object-2.glb');
});

test('plain voices pick a real voice the provider has', () => {
  const deep = PLAIN_VOICES[0];
  assert.equal(pickVoiceId(deep, [{ id: 'Samantha', name: 'Samantha' }, { id: 'Daniel', name: 'Daniel' }]), 'Daniel');
  assert.equal(pickVoiceId(deep, [{ id: 'keeper', name: 'keeper' }]), 'keeper');
  assert.equal(pickVoiceId(deep, []), null);
});

test('moves get plain names', () => {
  assert.equal(moveLabel('walk'), 'Walk');
  assert.equal(moveLabel('Running_A'), 'Run');
  assert.equal(moveLabel('idle'), 'Stand');
  assert.equal(moveLabel('1h-melee-attack-chop'), 'Swing');
  assert.equal(moveLabel('100style-angry-fw'), 'Angry walk');
  assert.equal(moveLabel('block'), 'Block');
});

test('create routes', () => {
  assert.deepEqual(parseCreateRoute([]), { view: 'home' });
  assert.deepEqual(parseCreateRoute(['p', 'abc']), { view: 'production', id: 'abc' });
  assert.deepEqual(parseCreateRoute(['a', 'x', 'compare', '3']), { view: 'compare', id: 'x', before: 3 });
  assert.deepEqual(parseCreateRoute(['new', 'robot']), { view: 'kinds' });
  assert.deepEqual(parseCreateRoute(['voice', 'k1']), { view: 'voice', who: 'k1' });
});

test('the planner chip says the agent and its model in plain words, the default as Default', () => {
  assert.equal(agentPickLabel('Claude', null, 'Default model'), 'Claude · Default');
  assert.equal(agentPickLabel('Claude', 'opus', 'Claude Opus'), 'Claude · Opus');
  assert.equal(agentPickLabel('Codex', 'gpt-5.4', 'GPT-5.4'), 'Codex · GPT-5.4');
});

test('a production is created with the picked agent and model; none picked sends neither', () => {
  assert.deepEqual(productionBody('w1', 'a rock', null), { workspace_id: 'w1', description: 'a rock', use: 'pc' });
  assert.deepEqual(productionBody('w1', 'a rock', { provider: 'codex', model: 'gpt-5.4' }), {
    workspace_id: 'w1', description: 'a rock', use: 'pc', provider: 'codex', model: 'gpt-5.4',
  });
  assert.deepEqual(productionBody('w1', 'a rock', { provider: 'codex', model: null }), {
    workspace_id: 'w1', description: 'a rock', use: 'pc', provider: 'codex',
  });
});

test('Create remembers the planner pick per workspace, through the start-pick store Threads uses', () => {
  const sp = loadSource(new URL('../src/lib/startPick.ts', import.meta.url), {});
  const mem = new Map<string, string>();
  const store = { getItem: (k: string) => mem.get(k) ?? null, setItem: (k: string, v: string) => void mem.set(k, v) };
  const ready = () => true;
  sp.writeStartPick(store, 'create:w1', { provider: 'codex', model: 'gpt-5.4', effort: null });
  assert.deepEqual({ ...sp.readStartPick(store, 'create:w1', ['claude', 'codex'], ready) }, { provider: 'codex', model: 'gpt-5.4', effort: null });
  assert.equal(sp.readStartPick(store, 'create:w2', ['claude', 'codex'], ready), null);
  assert.equal(sp.readStartPick(store, 'threads:w1', ['claude', 'codex'], ready), null);
});
