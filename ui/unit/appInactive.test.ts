// Animations pause while nobody is looking (lib/appInactive.ts + app.css).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

function fixture() {
  const classes = new Set<string>();
  const on = new Map<string, () => void>();
  const timers: (() => void)[] = [];
  const doc = {
    hidden: false, focus: true,
    hasFocus() { return this.focus; },
    documentElement: { classList: { toggle: (c: string, v: boolean) => (v ? classes.add(c) : classes.delete(c)) } },
    addEventListener: (t: string, fn: () => void) => on.set('doc:' + t, fn),
  };
  const win = { addEventListener: (t: string, fn: () => void) => on.set('win:' + t, fn) };
  const m = loadSource(new URL('../src/lib/appInactive.ts', import.meta.url), {}, {
    setTimeout: (fn: () => void) => (timers.push(fn), timers.length),
    clearTimeout: () => void timers.splice(0),
  });
  m.installAppInactive(doc, win);
  return { doc, on, timers, inactive: () => classes.has('app-inactive') };
}

test('a hidden window pauses animations at once; focus brings them back', () => {
  const f = fixture();
  f.doc.hidden = true; f.doc.focus = false; f.on.get('doc:visibilitychange')!();
  assert.equal(f.inactive(), true);
  f.doc.hidden = false; f.doc.focus = true; f.on.get('win:focus')!();
  assert.equal(f.inactive(), false);
});

test('a blurred window pauses them after a grace period, not on every glance away', () => {
  const f = fixture();
  f.doc.focus = false; f.on.get('win:blur')!();
  assert.equal(f.inactive(), false, 'not yet');
  f.timers.at(-1)!();
  assert.equal(f.inactive(), true);
});

test('the stylesheet pauses every animation under html.app-inactive, and main.ts installs it', () => {
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(css, /html\.app-inactive \*,[\s\S]*?animation-play-state: paused !important/);
  assert.match(readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8'), /installAppInactive\(\);/);
});
