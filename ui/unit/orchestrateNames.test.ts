// ⌘K plain-English engine: a session answers only to its WHOLE handle / title
// (or an explicit `@name` / `name:`), never to one word of a multi-word title;
// closes that end several sessions, a working one, or use an ambiguous verb
// ("stop messi") come back as a confirm step instead of archiving at once.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as commandParser from '../src/lib/commandParser.ts';
import * as pluralMod from '../src/lib/plural.ts';
import { loadSource } from './sourceHarness.ts';

type Call = { path: string; body: any };
// Values cross a vm realm — compare structurally.
const same = (a: unknown, b: unknown) => assert.deepEqual(JSON.parse(JSON.stringify(a)), b);

function engine() {
  const calls: Call[] = [];
  const api = {
    post: async (path: string, body: any) => {
      calls.push({ path, body });
      if (path.endsWith('/broadcast')) return { session_ids: body.session_ids };
      if (path.endsWith('/orchestrate')) return { plan: [{ action: 'broadcast', text: body.text }], optimized_text: null };
      return { results: [] };
    },
  };
  const mod = loadSource(new URL('../src/lib/orchestrate.ts', import.meta.url), {
    './api/client': { api },
    './commandParser': commandParser,
    './providers': { allProviders: () => [] },
    './plural': pluralMod,
  });
  return { mod, calls };
}

const sess = (id: string, title: string, handle?: string, status = 'idle') => ({
  id,
  title,
  kind: 'agent',
  provider: 'claude',
  archived: false,
  status,
  meta: handle ? { name_handle: handle, name_full: `${handle[0].toUpperCase()}${handle.slice(1)} Ferreira` } : {},
});

function ctx(sessions: any[], archived: string[] = []) {
  return {
    workspaceId: 'w1',
    focusedSessionId: null,
    sessions,
    nameable: sessions,
    order: sessions.map((s) => s.id),
    archive: async (id: string) => void archived.push(id),
    kill: async () => {},
    optimize: false,
    aiFallback: true,
    confirmDestructive: true,
  };
}

test('one word of a multi-word title never addresses the session', async () => {
  const { mod, calls } = engine();
  const s = [sess('a', 'Fix login tests')];
  const out = await mod.runEnglish('login is broken again', ctx(s));
  assert.equal(out.kind, 'plan', 'falls through to the AI planner (a confirm step)');
  assert.ok(!calls.some((c) => c.path.endsWith('/broadcast')));
});

test('the whole title, the handle, @handle and handle: still address', async () => {
  const { mod, calls } = engine();
  const s = [sess('a', 'Fix login tests'), sess('m', 'messi', 'messi')];
  let out = await mod.runEnglish('fix login tests: rerun please', ctx(s));
  assert.equal(out.kind, 'sent');
  same(calls.at(-1)!.body, { text: 'rerun please', session_ids: ['a'] });
  out = await mod.runEnglish('@messi hi', ctx(s));
  same(calls.at(-1)!.body, { text: 'hi', session_ids: ['m'] });
  out = await mod.runEnglish('messi: hi', ctx(s));
  same(calls.at(-1)!.body, { text: 'hi', session_ids: ['m'] });
  // A surname alone (one word of name_full) is not the session's name.
  out = await mod.runEnglish('ferreira: hi', ctx(s));
  assert.equal(out.kind, 'plan');
});

test('close by a single title word falls through instead of archiving', async () => {
  const { mod } = engine();
  const archived: string[] = [];
  const out = await mod.runEnglish('close login', ctx([sess('a', 'Fix login tests')], archived));
  assert.notEqual(out.kind, 'closed');
  assert.deepEqual(archived, []);
});

test('close <whole name> of one idle session archives at once', async () => {
  const { mod } = engine();
  const archived: string[] = [];
  const out = await mod.runEnglish('close messi', ctx([sess('m', 'messi', 'messi')], archived));
  assert.equal(out.kind, 'closed');
  assert.deepEqual(archived, ['m']);
});

test('close all / several / a working one / "stop <name>" ask first', async () => {
  const { mod } = engine();
  const archived: string[] = [];
  const two = [sess('m', 'messi', 'messi'), sess('r', 'ronaldo', 'ronaldo')];
  let out = await mod.runEnglish('close all claude sessions', ctx(two, archived));
  assert.equal(out.kind, 'confirm-close');
  assert.equal(out.permanent, false);
  same(out.ids, ['m', 'r']);
  out = await mod.runEnglish('close messi and ronaldo', ctx(two, archived));
  assert.equal(out.kind, 'confirm-close');
  out = await mod.runEnglish('close messi', ctx([sess('m', 'messi', 'messi', 'working')], archived));
  assert.equal(out.kind, 'confirm-close');
  assert.equal(out.working, 1);
  out = await mod.runEnglish('stop messi', ctx([sess('m', 'messi', 'messi')], archived));
  assert.equal(out.kind, 'confirm-close');
  assert.deepEqual(archived, [], 'nothing archived without the confirm');
});

test('a "working" plain shell is not mid-turn: close <name> archives at once', async () => {
  // `working` only means recent output — a shell's prompt redraw is not an
  // agent turn, so it must not force the confirm a working agent gets.
  const { mod } = engine();
  const archived: string[] = [];
  const shell = { ...sess('z', 'Zlatan', undefined, 'working'), provider: 'shell' };
  const out = await mod.runEnglish('please close zlatan', ctx([shell], archived));
  assert.equal(out.kind, 'closed');
  assert.deepEqual(archived, ['z']);
});
