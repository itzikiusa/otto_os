// Quality iteration 6, wave 3 (P5): the rooms module's failure and retry paths
// keep their focused control mounted and say what failed, and (structure) no
// status region in modules/rooms or lib/rooms is inserted together with its text.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { componentFunctions } from './componentFunctions.ts';
import { loadErrorText } from '../src/lib/loadError.ts';

const rooms = (name: string) => new URL(`../src/modules/rooms/${name}`, import.meta.url);

test('a failed room window is a toast whose action opens it again', async () => {
  const toasts: { level: string; title: string; body?: string; action?: { label: string; run: () => Promise<void> } }[] = [];
  let attempts = 0;
  const lobby = componentFunctions(rooms('RoomLobby.svelte'), ['openRoom'], {
    opening: '', loadErrorText,
    toasts: { push: (level: string, title: string, body: string, _ttl: number, opts: { action: { label: string; run: () => Promise<void> } }) => toasts.push({ level, title, body, action: opts.action }) },
    openHostedRoom: async () => { attempts++; if (attempts === 1) throw { status: 500, message: 'Native window refused.' }; },
  });
  await lobby.openRoom('r1');
  assert.equal(lobby.opening, '', 'the Open button is usable again');
  assert.deepEqual(toasts.map(t => [t.level, t.title, t.body, t.action?.label]), [['error', 'Couldn’t open the room', 'Native window refused.', 'Open again']]);
  await toasts[0].action!.run();
  assert.equal(attempts, 2, 'the action retries the same room');
  assert.equal(toasts.length, 1, 'a successful retry adds nothing');
});

test('a failed screen sample stays (with its Retry) until the retry answers', async () => {
  let fail = true;
  const seen: string[] = [];
  const image = componentFunctions(rooms('RecapImage.svelte'), ['load'], {
    recapId: 'r1', imageId: 'i1', url: '', busy: false, error: '', live: true, loadErrorText,
    URL: { createObjectURL: () => 'blob:sample' },
    recapBlob: async () => { seen.push(image.error); if (fail) throw { status: 503, message: 'Sample store offline' }; return {}; },
  });
  await image.load();
  assert.equal(image.error, 'Sample store offline');
  assert.equal(image.busy, false);
  fail = false;
  await image.load();
  assert.deepEqual(seen, ['', 'Sample store offline'], 'the error is kept while the retry is in flight');
  assert.equal(image.error, '');
  assert.equal(image.url, 'blob:sample');
});

function prepare(request: () => Promise<unknown>) {
  const said: [string, boolean][] = [];
  const state = componentFunctions(rooms('RoomRecap.svelte'), ['openPrepare', 'fail'], {
    partial: true, prepare: false, busy: false, error: null, capability: null, loadErrorText,
    recapRequest: request,
    announce: (text: string, opts?: { assertive?: boolean }) => said.push([text, !!opts?.assertive]),
  });
  return { state, said };
}

test('a failed engine check is LoadState’s alert (not announced twice); Retry keeps it until the answer', async () => {
  let answer: () => Promise<unknown> = async () => { throw { status: 503, message: 'whisper missing' }; };
  const { state, said } = prepare(() => answer());
  await state.openPrepare();
  assert.deepEqual({ ...state.error }, { what: 'Couldn’t check the local recap engines.', detail: 'whisper missing', check: true });
  assert.deepEqual(said, [], 'the LoadState alert speaks the check failure');
  let during: unknown = 'unset';
  answer = async () => { during = state.error; return { speech_ready: true }; };
  await state.openPrepare(true);
  assert.equal((during as { check: boolean }).check, true, 'Retry keeps the error view (and its focus) mounted');
  assert.equal(state.error, null);
  assert.deepEqual(state.capability, { speech_ready: true });
  // A fresh open starts clean; a non-check failure is announced at its edge.
  state.error = { what: 'Couldn’t prepare the recap.', detail: 'x', check: false };
  answer = async () => { during = state.error; return { speech_ready: false }; };
  await state.openPrepare();
  assert.equal(during, null);
  assert.deepEqual(said, [['Speech recognition is not ready. You can continue without a spoken transcript.', false]]);
  state.fail('Couldn’t prepare the recap.', { status: 409, message: 'busy' }, false);
  assert.deepEqual(said.at(-1), ['Couldn’t prepare the recap. busy', true]);
});

test('Reconnect does nothing while an attempt runs, so the banner keeps its focused button', () => {
  let connects = 0;
  const page = componentFunctions(rooms('RoomPage.svelte'), ['reconnect'], { connection: 'connecting', client: { connect: () => connects++ } });
  page.reconnect();
  assert.equal(connects, 0);
  page.connection = 'disconnected';
  page.reconnect();
  assert.equal(connects, 1);
});

// (structure) The ui-guards detectors, run on the rooms sources only: this
// module carries no inserted live region, double announcement, guessed state
// word or hand-rolled Retry (scripts/ui-guards.mjs is the source of the rules).
test('(structure) modules/rooms and lib/rooms have no inserted live region or hand-rolled Retry', () => {
  const ui = fileURLToPath(new URL('..', import.meta.url));
  const src = readFileSync(join(ui, 'scripts/ui-guards.mjs'), 'utf8');
  const block = src.slice(src.indexOf('// p04-detectors:start'), src.indexOf('// p04-detectors:end'));
  const tagEndSrc = src.slice(src.indexOf('function tagEnd('), src.indexOf('\n}\n', src.indexOf('function tagEnd(')) + 3);
  const d = new Function(`${tagEndSrc}\n${block}\nreturn { detectInsertedLiveRegion, detectAnnounceInRegion, detectGuessedDerived };`)() as Record<string, (...a: string[]) => { index: number; kind?: string }[]>;
  const blank = (m: string) => m.replace(/[^\n]/g, ' ');
  const strip = (t: string) => t.replace(/\/\*[\s\S]*?\*\//g, blank).replace(/<!--[\s\S]*?-->/g, blank).replace(/(^|[\s;{}(,])\/\/[^\n]*/g, (m, l: string) => l + blank(m.slice(l.length)));
  const walk = (dir: string, out: string[] = []): string[] => {
    if (!existsSync(dir)) return out;
    for (const n of readdirSync(dir)) { const p = join(dir, n); if (statSync(p).isDirectory()) walk(p, out); else if (p.endsWith('.svelte')) out.push(p); }
    return out;
  };
  const hits: string[] = [];
  for (const file of [...walk(join(ui, 'src/modules/rooms')), ...walk(join(ui, 'src/lib/rooms'))]) {
    const text = readFileSync(file, 'utf8');
    const code = strip(text);
    const markup = code.replace(/<(script|style)\b[\s\S]*?<\/\1\s*>/gi, blank);
    const at = (i: number) => `${file.slice(ui.length)}:${text.slice(0, i).split('\n').length}`;
    for (const h of d.detectInsertedLiveRegion(markup)) if (h.kind !== 'role="alert"') hits.push(`inserted-live-region ${at(h.index)}`);
    for (const h of d.detectAnnounceInRegion(markup)) hits.push(`announce-in-region ${at(h.index)}`);
    for (const h of d.detectGuessedDerived(code, markup)) hits.push(`guessed-derived ${at(h.index)}`);
    for (const m of markup.matchAll(/\b(?:Retry|Try again)\b[^<>]*<\/button>/g)) hits.push(`inline-retry ${at(m.index!)}`);
  }
  assert.deepEqual(hits, []);
});
