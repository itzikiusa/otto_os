import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

// accessibility-responsive-6w2-01: status lines inserted with their text in an
// `{:else if}` / `{:else}` branch (or a line or two into an `{#if}`) were
// silent. The Versions panel's stale-refresh failure is now spoken by
// `use:announceShown` (assertive, with the error); Walkthroughs' search count
// lives in one always-mounted region whose text starts empty, so the first
// count after typing is a change the reader speaks.

const VERSION_STRIP = new URL('../src/modules/design-hall/VersionsPanel.svelte', import.meta.url);
const WALKTHROUGHS = new URL('../src/modules/help/Walkthroughs.svelte', import.meta.url);

/** The real announce module on a manual frame clock; `heard` is every
 *  [channel, text] that reached the shell's always-mounted regions. */
function announcer() {
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

test('VersionsPanel: a refresh failure is announced once, assertively, with the error', () => {
  // The panel hands every load error to LoadState, whose stale bar speaks a
  // refresh that failed over loaded rows (the strip's own line is gone).
  const src = readFileSync(VERSION_STRIP, 'utf8');
  assert.match(src, /<LoadState what="versions"[^>]*\berror=\{list\.error\}/);
  const a = announcer();
  const { staleAnnouncement } = componentFunctions(new URL('../src/lib/components/LoadState.svelte', import.meta.url), ['staleAnnouncement'], {});
  const act = a.announceShown({}, { text: staleAnnouncement('versions', 'Daemon unreachable', false), assertive: true });
  a.flush();
  act.update({ text: staleAnnouncement('versions', 'Daemon unreachable', false), assertive: true });
  a.flush();
  assert.deepEqual(a.heard, [['assertive', 'Couldn’t refresh versions. Daemon unreachable']]);
  // A different error while the list is still stale speaks again.
  act.update({ text: staleAnnouncement('versions', 'Timed out', false), assertive: true });
  a.flush();
  assert.deepEqual(a.heard.at(-1), ['assertive', 'Couldn’t refresh versions. Timed out']);
});

test('Walkthroughs: the count region is empty until a search, then says the first count', () => {
  const { searchCountText } = componentFunctions(WALKTHROUGHS, ['searchCountText'], {});
  // Mounted with the rail before any typing: nothing to say (a change later is spoken).
  assert.equal(searchCountText('', 12), '');
  assert.equal(searchCountText('   ', 12), '', 'whitespace is not a search');
  // The first keystroke changes the always-mounted region's text: that is the announcement.
  assert.equal(searchCountText('g', 7), '7 guides');
  assert.equal(searchCountText('git', 1), '1 guide');
  assert.equal(searchCountText(' zzz ', 0), 'No guides match “zzz”');
});

test('VariantsTray: progress is spoken at the run’s edges, not per card, and not for a finished run', () => {
  const a = announcer();
  const plural = (n: number, w: string) => `${n} ${n === 1 ? w : `${w}s`}`;
  const tray = () => {
    const { variantProgress } = componentFunctions(new URL('../src/modules/design-hall/assist/VariantsTray.svelte', import.meta.url), ['variantProgress'], { plural });
    const say = a.announceEdge();
    const seen = { drawing: false };
    return (drawing: number, drawn: number) => { say(variantProgress(drawing, drawn, seen)); a.flush(); };
  };
  const live = tray();
  live(3, 0);
  live(2, 1); // a card landed: still drawing, a new count
  live(0, 3);
  live(0, 3); // a re-render says nothing new
  assert.deepEqual(a.heard.map((h: string[]) => h[1]), ['Drawing 3 variants…', 'Drawing 2 variants…', '3 variants ready to compare']);
  const opened = tray();
  opened(0, 3);
  assert.equal(a.heard.length, 3, 'a run that opens already finished stays quiet');
});
