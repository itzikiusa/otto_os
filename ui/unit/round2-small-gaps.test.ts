// Small UI gaps closed together: machine-status source labels, the terminal
// find bar's per-session memo, the room chat's poll back-off and the shell
// dialogs that load on first open.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { sourceLabel } from '../src/modules/settings/machineStatusText.ts';
import { parkFind, takeFind } from '../src/lib/components/termFindMemo.ts';
import { CHAT_POLL_STEPS, ChatCadence } from '../src/modules/rooms/chatCadence.ts';
import { chainTo, staticClosure } from './staticClosure.ts';

const ROOT = join(import.meta.dirname, '..', '..');
const SRC = join(ROOT, 'ui', 'src');
const read = (p: string): string => readFileSync(join(ROOT, p), 'utf8');

test('every admission-gated background source has a plain-language label', () => {
  // The daemon's list (otto-sessions admission.rs); an unlabelled source would
  // show as its raw id with the separators blanked ("self improvement").
  const rust = read('crates/otto-sessions/src/admission.rs');
  const block = rust.slice(rust.indexOf('pub const ADMISSION_GATED_SOURCES'), rust.indexOf('];', rust.indexOf('pub const ADMISSION_GATED_SOURCES')));
  const sources = [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.ok(sources.includes('self_improvement') && sources.includes('product') && sources.includes('plugin'), 'parsed the gated list');
  for (const s of sources) assert.notEqual(sourceLabel(s), s.replace(/[_-]+/g, ' '), `no label for "${s}"`);
  assert.equal(sourceLabel('self_improvement'), 'Self-improvement review');
  assert.equal(sourceLabel('product'), 'Story watcher');
  assert.equal(sourceLabel('plugin'), 'Plugin agent');
});

test('the terminal find bar parks per session and comes back once', () => {
  parkFind('ssh-a', { open: true, query: 'error' });
  parkFind('ssh-b', { open: false, query: 'warn' });
  assert.deepEqual(takeFind('ssh-a'), { open: true, query: 'error' });
  assert.equal(takeFind('ssh-a'), null, 'taken once');
  assert.deepEqual(takeFind('ssh-b'), { open: false, query: 'warn' });
  // A closed, empty bar forgets an older memo for that session.
  parkFind('ssh-c', { open: true, query: 'x' });
  parkFind('ssh-c', { open: false, query: '' });
  assert.equal(takeFind('ssh-c'), null);
  // No session (never connected): nothing is kept.
  parkFind(null, { open: true, query: 'x' });
  assert.equal(takeFind(null), null);
});

test('the terminal find memo is bounded, oldest first', () => {
  for (let i = 0; i < 70; i++) parkFind(`s${i}`, { open: true, query: String(i) });
  assert.equal(takeFind('s0'), null);
  assert.equal(takeFind('s5'), null);
  assert.deepEqual(takeFind('s69'), { open: true, query: '69' });
  for (let i = 0; i < 70; i++) takeFind(`s${i}`);
});

test('the room chat backs off while quiet and snaps back on activity', () => {
  const c = new ChatCadence();
  assert.deepEqual([...CHAT_POLL_STEPS], [2500, 5000, 10_000]);
  assert.equal(c.ms, 2500);
  c.unchanged();
  assert.equal(c.ms, 5000);
  c.unchanged();
  c.unchanged();
  assert.equal(c.ms, 10_000, 'capped');
  c.active();
  assert.equal(c.ms, 2500);
});

test('menu-only shell dialogs load on first open (not in the shell chunk)', () => {
  const entry = join(SRC, 'shell/App.svelte');
  const closure = staticClosure(entry);
  for (const f of ['shell/RemovedWorkspaces.svelte', 'shell/MachineDetails.svelte']) {
    assert.ok(!closure.has(join(SRC, f)), chainTo(entry, join(SRC, f), SRC).join(' → '));
  }
  const chunks = read('ui/src/shell/lazyDialogs.ts');
  assert.match(chunks, /import\('\.\/RemovedWorkspaces\.svelte'\)/);
  assert.match(chunks, /import\('\.\/MachineDetails\.svelte'\)/);
});
