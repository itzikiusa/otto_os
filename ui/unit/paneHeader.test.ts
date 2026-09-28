// Session pane header helpers (lib/paneHeader.ts): the width tiers that mirror
// SessionView's `@container pane` rules, the retired-Split preference
// migration, and the details rows the compact header moved off the bar.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  PANE_TIER_MIN,
  cwdLabel,
  otherSessionView,
  paneDetails,
  paneDetailsTitle,
  paneTier,
  parseSessionView,
  tierAtMost,
} from '../src/lib/paneHeader.ts';
import * as paneHeader from '../src/lib/paneHeader.ts';
import { loadSource } from './sourceHarness.ts';
import { TranscriptLifecycle } from '../src/lib/stores/transcriptLifecycle.ts';

test('paneTier maps widths onto full / compact / minimal / micro at the CSS breakpoints', () => {
  assert.equal(paneTier(1400), 'full');
  assert.equal(paneTier(PANE_TIER_MIN.full), 'full', '720 is still full (CSS: width < 720px is compact)');
  assert.equal(paneTier(PANE_TIER_MIN.full - 1), 'compact');
  assert.equal(paneTier(PANE_TIER_MIN.compact), 'compact');
  assert.equal(paneTier(PANE_TIER_MIN.compact - 0.5), 'minimal');
  assert.equal(paneTier(PANE_TIER_MIN.minimal), 'minimal');
  assert.equal(paneTier(PANE_TIER_MIN.minimal - 1), 'micro');
  assert.equal(paneTier(85), 'micro', 'a 15-tile column');
});

test('an unmeasured pane is full, never a flash of the minimal header', () => {
  assert.equal(paneTier(0), 'full');
  assert.equal(paneTier(-5), 'full');
  assert.equal(paneTier(Number.NaN), 'full');
});

test('tierAtMost orders the tiers widest → narrowest', () => {
  assert.equal(tierAtMost('micro', 'minimal'), true);
  assert.equal(tierAtMost('minimal', 'minimal'), true);
  assert.equal(tierAtMost('compact', 'minimal'), false);
  assert.equal(tierAtMost('full', 'full'), true);
  assert.equal(tierAtMost('full', 'micro'), false);
});

test('a stored Split preference reads as Chat; unknown values fall back to the default', () => {
  assert.equal(parseSessionView('split'), 'chat');
  assert.equal(parseSessionView('chat'), 'chat');
  assert.equal(parseSessionView('terminal'), 'terminal');
  assert.equal(parseSessionView(null), null);
  assert.equal(parseSessionView(undefined), null);
  assert.equal(parseSessionView(''), null);
  assert.equal(parseSessionView('Split'), null, 'only the exact persisted token migrates');
});

test('the view toggle flips between the two remaining views', () => {
  assert.equal(otherSessionView('terminal'), 'chat');
  assert.equal(otherSessionView('chat'), 'terminal');
});

test('cwdLabel shows the last folder of a path', () => {
  assert.equal(cwdLabel('/Users/me/src/otto'), 'otto');
  assert.equal(cwdLabel('/Users/me/src/otto/'), 'otto');
  assert.equal(cwdLabel('/tmp'), 'tmp');
  assert.equal(cwdLabel('/'), '/');
  assert.equal(cwdLabel('C:\\work\\repo'), 'repo');
  assert.equal(cwdLabel(''), '');
  assert.equal(cwdLabel(null), '');
});

test('paneDetails keeps only the facts that exist, in a stable order', () => {
  const rows = paneDetails({
    provider: 'claude',
    nameFull: 'Cristiano Ronaldo',
    account: '  ',
    state: null,
    idle: 'suspends in 12m',
    tasks: { done: 2, total: 5 },
    now: 'Run tests',
    handoverFrom: null,
    handoverPending: true,
    cwd: '/repo',
  });
  assert.deepEqual(rows, [
    ['Agent', 'claude'],
    ['Name', 'Cristiano Ronaldo'],
    ['Idle', 'suspends in 12m'],
    ['Tasks', '2/5 done'],
    ['Now', 'Run tests'],
    ['Handover', 'Preparing the brief…'],
    ['Folder', '/repo'],
  ]);
  assert.equal(paneDetailsTitle(rows.slice(0, 2)), 'Agent: claude\nName: Cristiano Ronaldo');
  assert.deepEqual(paneDetails({ tasks: { done: 0, total: 0 } }), [], 'an empty roll-up is not a fact');
});

test('the transcript store migrates a stored Split to Chat once and drops the split fraction', () => {
  const storage = new Map<string, string>([
    ['otto_session_view:a', 'split'],
    ['otto_session_split_frac:a', '0.62'],
    ['otto_session_view:b', 'terminal'],
    ['otto_session_view:c', 'bogus'],
  ]);
  const localStorage = {
    getItem: (k: string) => storage.get(k) ?? null,
    setItem: (k: string, v: string) => void storage.set(k, v),
    removeItem: (k: string) => void storage.delete(k),
  };
  const { transcript } = loadSource(
    new URL('../src/lib/stores/transcript.svelte.ts', import.meta.url),
    {
      './transcriptLifecycle': { TranscriptLifecycle },
      '../paneHeader': paneHeader,
      '../win': { winKey: (key: string) => key },
      '../api/client': { api: {}, isAbortError: () => false },
    },
    { localStorage },
  );
  assert.equal(transcript.view('a'), 'chat');
  assert.equal(storage.get('otto_session_view:a'), 'chat', 'rewritten on read');
  assert.equal(storage.has('otto_session_split_frac:a'), false, 'orphaned fraction removed');
  assert.equal(transcript.view('b'), 'terminal');
  assert.equal(transcript.view('c'), null, 'unknown → default view');
  assert.equal(transcript.view('missing'), null);
  transcript.setView('b', 'chat');
  assert.equal(transcript.view('b'), 'chat');
  assert.equal(storage.get('otto_session_view:b'), 'chat');
  assert.equal(typeof transcript.splitFrac, 'undefined', 'the Split fraction API is gone');
});
