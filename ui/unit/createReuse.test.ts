import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { groupByProject, parseCreateRoute, productionBody, projectsOf, remainingSecs, reuseLine } from '../src/modules/design-hall/create/plan.ts';

const item = (over: Record<string, unknown>) =>
  ({ id: 'x', kind: 'object', name: 'X', description: '', enabled: true, details: {}, status: 'waiting', progress_pct: 0, ...over }) as never;
const reused = (id: string, kind = 'object') => item({ id, kind, details: { reuse: { artifact_id: `a-${id}`, from: 'The pier at night' } } });

test('the plan says honestly how much is reused and the time saved', () => {
  const items = [reused('cat', 'character'), reused('table'), item({ id: 'kettle' }), item({ id: 'line', kind: 'voice_line' })];
  assert.equal(reuseLine(items, 12 * 60, 70 * 60), '2 of 3 pieces reused · about 12 minutes instead of about an hour');
  assert.equal(reuseLine(items, 0, undefined), '2 of 3 pieces reused');
  assert.equal(reuseLine([item({ id: 'kettle' })], 300, undefined), null, 'nothing reused: nothing said');
  // A turned-off piece is not counted.
  assert.equal(reuseLine([reused('cat'), item({ id: 'rock', enabled: false })], 0, undefined), '1 of 1 pieces reused');
});

test('a reused piece costs no time; one that fell back to being made does', () => {
  const kettle = item({ id: 'kettle' });
  assert.equal(remainingSecs([reused('table'), kettle]), remainingSecs([kettle]));
  assert.ok(remainingSecs([item({ id: 'table' }), kettle]) > remainingSecs([kettle]));
});

test('a new scene names its project; none is a new project', () => {
  assert.equal(productionBody('w1', 'tea', null, 'p1').project_id, 'p1');
  assert.equal('project_id' in productionBody('w1', 'tea', null), false);
  assert.deepEqual(parseCreateRoute(['in', 'p1']), { view: 'home', project: 'p1' });
  assert.deepEqual(parseCreateRoute(['in']), { view: 'home' });
});

test('projects are listed once, newest first, named by their first creation', () => {
  const list = [
    { id: 's3', project_id: 's1', project_name: 'Breakfast', title: 'Later' },
    { id: 's2', project_id: 's2', title: 'A pier' },
    { id: 's1', project_id: 's1', title: 'Breakfast' },
  ];
  assert.deepEqual(projectsOf(list), [{ id: 's1', name: 'Breakfast' }, { id: 's2', name: 'A pier' }]);
});

test('My creations puts the scenes of one project together, after its name', () => {
  const projects = [{ id: 'p', name: 'Breakfast' }];
  const e = (key: string, project?: string) => ({ key, project });
  const rows = groupByProject([e('a', 'p'), e('m'), e('b', 'p'), e('c', 'q')], (x) => (x.project === 'p' ? projects[0] : x.project ? { id: x.project, name: 'Alone' } : null));
  assert.deepEqual(rows.map((r) => ('label' in r ? `${r.label}: ${r.items.map((x) => x.key).join(' ')}` : r.key)), ['Breakfast: a b', 'm', 'c']);
});

test('the plan screen switches a piece to one the project has, picked from them, and back to a new one', async () => {
  const sent: unknown[] = [];
  // Objects made inside the component's context: compared as plain data.
  const plain = (v: unknown) => JSON.parse(JSON.stringify(v));
  const asked: { message: string; options: { label: string; value: string }[] }[] = [];
  let answer: string | null = 'a-cat';
  const pieces = [
    { kind: 'character', name: 'Cat', description: 'A cat.', details: { reuse: { artifact_id: 'a-cat', version_id: 'v2', from: 'Breakfast' } } },
    { kind: 'object', name: 'Table', description: 'A table.', details: { reuse: { artifact_id: 'a-table', from: 'Breakfast' } } },
  ];
  const view = componentFunctions(new URL('../src/modules/design-hall/create/ProductionView.svelte', import.meta.url), ['switchPiece'], {
    id: 'p2',
    p: { project_pieces: pieces },
    theName: (n: string) => `the ${n.toLowerCase()}`,
    confirmer: { choose: async (message: string, o: { options: { label: string; value: string }[] }) => (asked.push({ message, options: o.options }), { value: answer, remember: false }) },
    papi: { updateProduction: (id: string, body: unknown) => (sent.push([id, body]), Promise.resolve({})) },
    act: async (run: () => Promise<unknown>) => void (await run()),
  });
  const kitty = { id: 'kitty', kind: 'character', name: 'Kitty', details: { prompt: 'grey cat', size_m: 0.4 } };
  await view.switchPiece(kitty, 'pick');
  assert.deepEqual(plain(asked[0].options), [{ label: 'Cat (from “Breakfast”)', value: 'a-cat' }], 'only the project’s pieces of its kind');
  assert.equal(asked[0].message, 'Which one is the kitty?');
  assert.deepEqual(plain(sent[0]), ['p2', { items: [{ id: 'kitty', details: { prompt: 'grey cat', size_m: 0.4, reuse: { artifact_id: 'a-cat', version_id: 'v2' } } }] }]);
  await view.switchPiece({ ...kitty, details: { prompt: 'grey cat', reuse: { artifact_id: 'a-cat' } } }, 'new');
  assert.deepEqual(plain(sent[1]), ['p2', { items: [{ id: 'kitty', details: { prompt: 'grey cat' } }] }], 'made new: no reuse sent');
  answer = null;
  await view.switchPiece(kitty, 'pick');
  assert.equal(sent.length, 2, 'Cancel changes nothing');
});
