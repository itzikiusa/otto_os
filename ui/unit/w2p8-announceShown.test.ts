import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// accessibility-responsive-6w1-01: a status line inserted with its state
// (`{#if failed}<p use:announceShown=…>`) carries no role; the action speaks
// it through the shell's always-mounted announcer when it appears and again
// only when its text changes.

function fixture() {
  let frames: (() => void)[] = [];
  const mod = loadSource(new URL('../src/lib/announce.svelte.ts', import.meta.url), {}, {
    requestAnimationFrame: (fn: () => void) => frames.push(fn),
    cancelAnimationFrame: () => {},
  });
  const heard: [string, string][] = [];
  const flush = () => {
    const due = frames;
    frames = [];
    due.forEach((f) => f());
    for (const ch of ['polite', 'assertive'] as const) {
      if (mod.announcer[ch]) heard.push([ch, mod.announcer[ch]]);
      mod.announcer[ch] = '';
    }
  };
  return { ...mod, heard, flush } as Record<string, any>;
}

test('the line is spoken when it appears, and not again for the same text', () => {
  const h = fixture();
  const act = h.announceShown({}, 'Saving…');
  h.flush();
  act.update('Saving…');
  h.flush();
  assert.deepEqual(h.heard, [['polite', 'Saving…']]);
});

test('a changed text speaks again; a failure goes to the assertive channel', () => {
  const h = fixture();
  const act = h.announceShown({}, 'Testing…');
  h.flush();
  act.update({ text: 'Couldn’t connect: 401', assertive: true });
  h.flush();
  assert.deepEqual(h.heard, [['polite', 'Testing…'], ['assertive', 'Couldn’t connect: 401']]);
});

test('each mounted line has its own edge (two banners both speak)', () => {
  const h = fixture();
  h.announceShown({}, { text: 'Over budget: workspace', assertive: true });
  h.flush();
  h.announceShown({}, { text: 'Over budget: workspace', assertive: true });
  h.flush();
  assert.equal(h.heard.length, 2, 'a re-inserted banner is a new edge');
});
