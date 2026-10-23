import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// A manual frame clock: announce() must clear, then set on the NEXT frame.
function fixture() {
  let frames: { id: number; fn: () => void }[] = [];
  let next = 1;
  const globals = {
    requestAnimationFrame: (fn: () => void) => { const id = next++; frames.push({ id, fn }); return id; },
    cancelAnimationFrame: (id: number) => { frames = frames.filter(f => f.id !== id); },
  };
  const mod = loadSource(new URL('../src/lib/announce.svelte.ts', import.meta.url), {}, globals);
  const flush = () => { const due = frames; frames = []; due.forEach(f => f.fn()); };
  const h: Record<string, any> = { ...mod, flush };
  return h;
}

test('the same text twice is announced twice (cleared, then set on the next frame)', () => {
  const h = fixture();
  const heard: string[] = [];
  h.announce('Copied');
  assert.equal(h.announcer.polite, '', 'text lands on the next frame, not synchronously');
  h.flush();
  heard.push(h.announcer.polite);
  h.announce('Copied');
  assert.equal(h.announcer.polite, '', 'a repeat clears the region first, so the reader sees a change');
  h.flush();
  heard.push(h.announcer.polite);
  assert.deepEqual(heard, ['Copied', 'Copied']);
});

test('the assertive channel is separate from the polite one', () => {
  const h = fixture();
  h.announce('Saved');
  h.announce('Save failed', { assertive: true });
  h.flush();
  assert.equal(h.announcer.polite, 'Saved');
  assert.equal(h.announcer.assertive, 'Save failed');
  h.announce('Saved again');
  h.flush();
  assert.equal(h.announcer.assertive, 'Save failed', 'a polite announce leaves the assertive region alone');
});

test('a second announce before the frame replaces the first', () => {
  const h = fixture();
  h.announce('Loading');
  h.announce('Loaded');
  h.flush();
  assert.equal(h.announcer.polite, 'Loaded');
});

test('empty text only clears the channel', () => {
  const h = fixture();
  h.announce('Hello');
  h.flush();
  h.announce('');
  h.flush();
  assert.equal(h.announcer.polite, '');
});
