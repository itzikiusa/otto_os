// Session pane header helpers (lib/paneHeader.ts): the width tiers that mirror
// SessionView's `@container pane` rules, the sweep of the retired Terminal ·
// Chat preference, and the details rows the compact header moved off the bar.
import * as transcriptTrim from '../src/lib/stores/transcriptTrim.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  PANE_TIER_MIN,
  cwdLabel,
  paneDetails,
  paneDetailsTitle,
  paneTier,
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

test('the Terminal · Chat view helpers are gone — a session pane is always its terminal', () => {
  assert.equal('parseSessionView' in paneHeader, false);
  assert.equal('otherSessionView' in paneHeader, false);
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
    usage: '2.1M tokens · $1.24',
  });
  assert.deepEqual(rows, [
    ['Agent', 'claude'],
    ['Name', 'Cristiano Ronaldo'],
    ['Idle', 'suspends in 12m'],
    ['Usage', '2.1M tokens · $1.24'],
    ['Tasks', '2/5 done'],
    ['Now', 'Run tests'],
    ['Handover', 'Preparing the brief…'],
    ['Folder', '/repo'],
  ]);
  assert.equal(paneDetailsTitle(rows.slice(0, 2)), 'Agent: claude\nName: Cristiano Ronaldo');
  assert.deepEqual(paneDetails({ tasks: { done: 0, total: 0 } }), [], 'an empty roll-up is not a fact');
});

test('the transcript store sweeps the retired Terminal · Chat / Split choices in every window namespace', () => {
  const storage = new Map<string, string>([
    ['otto_session_view:a', 'chat'],
    ['otto_session_split_frac:a', '0.62'],
    ['otto_win_w2::otto_session_view:b', 'split'],
    ['otto_conv_show_system', '1'],
    ['otto_session_viewer_note', 'keep'],
  ]);
  const localStorage = {
    get length() {
      return storage.size;
    },
    key: (i: number) => [...storage.keys()][i] ?? null,
    getItem: (k: string) => storage.get(k) ?? null,
    setItem: (k: string, v: string) => void storage.set(k, v),
    removeItem: (k: string) => void storage.delete(k),
  };
  const { transcript } = loadSource(
    new URL('../src/lib/stores/transcript.svelte.ts', import.meta.url),
    {
      './transcriptLifecycle': { TranscriptLifecycle }, './transcriptTrim': transcriptTrim,
      '../win': { winKey: (key: string) => key },
      '../api/client': { api: {}, isAbortError: () => false },
    },
    { localStorage },
  );
  assert.equal(typeof transcript.view, 'undefined', 'the per-session view API is gone');
  assert.equal(typeof transcript.setView, 'undefined');
  transcript.sweepRetiredViewKeys();
  assert.deepEqual([...storage.keys()].sort(), ['otto_conv_show_system', 'otto_session_viewer_note']);
});
