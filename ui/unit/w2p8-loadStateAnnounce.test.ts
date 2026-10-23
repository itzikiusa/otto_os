import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

// accessibility-responsive-6w1-01: LoadState's "Couldn’t refresh" bar is
// inserted together with its text, which VoiceOver doesn't read. The refresh
// failure is spoken through the announcer instead — once per distinct error.

const LOAD_STATE = new URL('../src/lib/components/LoadState.svelte', import.meta.url);

/** The real announce module on a manual frame clock; `spoken` is every text
 *  that reached the assertive region. */
function announcer() {
  let frames: (() => void)[] = [];
  const mod = loadSource(new URL('../src/lib/announce.svelte.ts', import.meta.url), {}, {
    requestAnimationFrame: (fn: () => void) => frames.push(fn),
    cancelAnimationFrame: () => {},
  });
  const spoken: string[] = [];
  const flush = () => {
    const due = frames;
    frames = [];
    due.forEach((f) => f());
    if (mod.announcer.assertive) spoken.push(mod.announcer.assertive);
    mod.announcer.assertive = '';
  };
  return { ...mod, spoken, flush } as Record<string, any>;
}

/** One render of LoadState's effect: the component's own message function fed
 *  to the component's own announceEdge tracker. */
function renderer() {
  const a = announcer();
  const { staleAnnouncement } = componentFunctions(LOAD_STATE, ['staleAnnouncement'], {});
  const sayStale = a.announceEdge();
  const render = (props: { what: string; error: string | null; empty: boolean }) => {
    sayStale(staleAnnouncement(props.what, props.error, props.empty), { assertive: true });
    a.flush();
  };
  return { render, spoken: a.spoken as string[] };
}

test('a refresh failure over stale data is announced once, assertively', () => {
  const { render, spoken } = renderer();
  render({ what: 'scheduled tasks', error: null, empty: false });
  render({ what: 'scheduled tasks', error: 'Daemon unreachable', empty: false });
  render({ what: 'scheduled tasks', error: 'Daemon unreachable', empty: false });
  render({ what: 'scheduled tasks', error: 'Daemon unreachable', empty: false });
  assert.deepEqual(spoken, ['Couldn’t refresh scheduled tasks. Daemon unreachable']);
});

test('a different error speaks again; a recovery re-arms the same one', () => {
  const { render, spoken } = renderer();
  render({ what: 'repos', error: 'Timed out', empty: false });
  render({ what: 'repos', error: 'Forbidden', empty: false });
  render({ what: 'repos', error: null, empty: false });
  render({ what: 'repos', error: 'Forbidden', empty: false });
  assert.deepEqual(spoken, ['Couldn’t refresh repos. Timed out', 'Couldn’t refresh repos. Forbidden', 'Couldn’t refresh repos. Forbidden']);
});

test('a first-load error (nothing to show) is the alert view, not the stale bar', () => {
  const { render, spoken } = renderer();
  render({ what: 'repos', error: 'Timed out', empty: true });
  assert.deepEqual(spoken, []);
});

test('the stale bar carries no role and the effect feeds the tracker (structure)', () => {
  const src = readFileSync(LOAD_STATE, 'utf8');
  assert.match(src, /<div class="ls-stale" data-testid="load-stale">/);
  assert.match(src, /\$effect\(\(\) => \{\s*sayStale\(staleAnnouncement\(what, error, empty\), \{ assertive: true \}\);/);
});
