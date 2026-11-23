// Wave 2 (SHOULD — S-40) tests for Browser module:
// 1. watchDevicePixelRatio continuously re-subscribes across Retina -> 1x -> Retina moves
// 2. overlay.js preserves pending marks across immediate ticks (no flicker) and removes ghost boxes on SPA DOM removal
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { watchDevicePixelRatio, type DprMediaQuery } from '../src/modules/browser/live/geometry.ts';

test('watchDevicePixelRatio tracks repeated display moves and cleans up on dispose', () => {
  let dpr = 2;
  const queries: string[] = [];
  const listeners = new Map<string, Set<() => void>>();

  const fakeMatchMedia = (q: string): DprMediaQuery => {
    queries.push(q);
    let set = listeners.get(q);
    if (!set) {
      set = new Set();
      listeners.set(q, set);
    }
    return {
      addEventListener(_t, cb) { set!.add(cb); },
      removeEventListener(_t, cb) { set!.delete(cb); },
    };
  };

  let calls = 0;
  const stop = watchDevicePixelRatio(() => { calls += 1; }, () => dpr, fakeMatchMedia);
  assert.deepEqual(queries, ['(resolution: 2dppx)']);

  // Move Retina (2) -> external display (1)
  dpr = 1;
  for (const cb of [...(listeners.get('(resolution: 2dppx)') ?? [])]) cb();
  assert.equal(calls, 1);
  assert.deepEqual(queries, ['(resolution: 2dppx)', '(resolution: 1dppx)']);

  // Move external (1) -> back to Retina (2)
  dpr = 2;
  for (const cb of [...(listeners.get('(resolution: 1dppx)') ?? [])]) cb();
  assert.equal(calls, 2);
  assert.deepEqual(queries, ['(resolution: 2dppx)', '(resolution: 1dppx)', '(resolution: 2dppx)']);

  // Dispose detaches active listener
  stop();
  for (const cb of [...(listeners.get('(resolution: 2dppx)') ?? [])]) cb();
  assert.equal(calls, 2);
});

test('overlay.js avoids click-tick flicker and removes/repositions SPA highlight boxes', () => {
  const src = readFileSync(new URL('../src/modules/browser/overlay.js', import.meta.url), 'utf8');

  type MockEl = {
    id?: string;
    tagName: string;
    className: string;
    style: Record<string, string>;
    textContent: string;
    outerHTML: string;
    rect: { left: number; top: number; width: number; height: number };
    removed: boolean;
    getAttribute(k: string): string | null;
    getBoundingClientRect(): { left: number; top: number; width: number; height: number };
    remove(): void;
  };

  const appended: MockEl[] = [];
  const docHandlers: Record<string, (e: unknown) => void> = {};
  const winHandlers: Record<string, (e: unknown) => void> = {};

  const target: MockEl = {
    id: 'cta-btn',
    tagName: 'BUTTON',
    className: 'cta',
    style: {},
    textContent: 'Buy',
    outerHTML: '<button id="cta-btn">Buy</button>',
    rect: { left: 10, top: 20, width: 80, height: 30 },
    removed: false,
    getAttribute(k) { return k === 'id' ? 'cta-btn' : null; },
    getBoundingClientRect() { return this.rect; },
    remove() { this.removed = true; },
  };
  let targetInDom = true;

  const document = {
    body: {},
    documentElement: {
      appendChild(el: MockEl) { appended.push(el); },
    },
    getElementById(id: string) {
      return appended.find((e) => e.id === id) ?? null;
    },
    createElement(tag: string): MockEl {
      return {
        tagName: tag.toUpperCase(),
        className: '',
        style: {},
        textContent: '',
        outerHTML: '',
        rect: { left: 0, top: 0, width: 0, height: 0 },
        removed: false,
        getAttribute() { return null; },
        getBoundingClientRect() { return this.rect; },
        remove() { this.removed = true; },
      };
    },
    querySelector(sel: string) {
      return targetInDom && sel === '#cta-btn' ? target : null;
    },
    addEventListener(ev: string, fn: (e: unknown) => void) { docHandlers[ev] = fn; },
  };

  const win: Record<string, unknown> = {
    addEventListener(ev: string, fn: (e: unknown) => void) { winHandlers[ev] = fn; },
  };

  const fn = new Function('window', 'document', 'CSS', src);
  fn(win, document, { escape: (s: string) => s });

  const api = win.__ottoOverlay as {
    setPicking(v: boolean): void;
    tick(json: string): Array<{ selector: string }>;
  };
  api.setPicking(true);

  // Click target -> instant highlight box drawn
  docHandlers.click({
    target,
    preventDefault() {},
    stopPropagation() {},
  });
  const box = appended.find((e) => e.className.includes('__otto_mark__'));
  assert.ok(box, 'instant mark box created');
  assert.equal(box.removed, false);

  // Immediate tick before server round-trip returns empty marks -> box MUST NOT flicker/remove
  const drained = api.tick('[]');
  assert.equal(drained.length, 1);
  assert.equal(drained[0].selector, '#cta-btn');
  assert.equal(box.removed, false, 'pending mark box stays visible during round-trip');

  // Target moves in SPA layout -> tick repositions existing box
  target.rect = { left: 100, top: 220, width: 80, height: 30 };
  api.tick(JSON.stringify([{ selector: '#cta-btn', color: 'yellow' }]));
  assert.equal(box.style.left, '100px');
  assert.equal(box.style.top, '220px');

  // Target removed by SPA route change -> tick removes ghost box
  targetInDom = false;
  api.tick(JSON.stringify([{ selector: '#cta-btn', color: 'yellow' }]));
  assert.equal(box.removed, true, 'ghost box removed when target leaves DOM');
});
