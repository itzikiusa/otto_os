import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fitToVisualViewport, isTextEntry, keyboardState, nextBaseline, onKeyboard, resetShellScroll, KEYBOARD_MIN_PX } from '../src/lib/viewportFit.ts';

test('the app follows the visible viewport and an iOS keyboard pan is undone', () => {
  const handlers: Record<string, () => void> = {};
  const vv = { height: 932, scale: 1, offsetTop: 0, addEventListener: (t: string, f: () => void) => (handlers[t] = f) };
  const scrolls: number[][] = [];
  const win = { visualViewport: vv as never, scrollY: 0, scrollTo: (x: number, y: number) => scrolls.push([x, y]) };
  const props: Record<string, string> = {};
  fitToVisualViewport(win, { style: { setProperty: (k, v) => (props[k] = v) } });
  assert.equal(props['--app-h'], '932px');
  assert.deepEqual(scrolls, []);
  // Keyboard up: the visible viewport shrinks and iOS pans the page up.
  Object.assign(vv, { height: 520.4, offsetTop: 300 });
  handlers.resize();
  assert.equal(props['--app-h'], '520px');
  assert.deepEqual(scrolls, [[0, 0]]);
  // A pan left behind after the keyboard closed is undone too.
  Object.assign(vv, { height: 932, offsetTop: 0 });
  win.scrollY = 120;
  handlers.scroll();
  assert.equal(props['--app-h'], '932px');
  assert.equal(scrolls.length, 2);
  // Pinch-zoom is the user's: left alone.
  Object.assign(vv, { scale: 2, height: 466 });
  handlers.resize();
  assert.equal(props['--app-h'], '932px');
  assert.equal(scrolls.length, 2);
});

// A focused text field, as `document.activeElement`.
const field = (tagName: string, extra: Record<string, unknown> = {}) => ({ tagName, type: 'text', ...extra }) as unknown as Element;

test('a text field raises the keyboard; buttons, checkboxes and read-only fields do not', () => {
  assert.equal(isTextEntry(field('TEXTAREA')), true);
  assert.equal(isTextEntry(field('INPUT')), true);
  assert.equal(isTextEntry(field('INPUT', { type: 'search' })), true);
  assert.equal(isTextEntry(field('INPUT', { type: 'checkbox' })), false);
  assert.equal(isTextEntry(field('INPUT', { readOnly: true })), false);
  assert.equal(isTextEntry(field('TEXTAREA', { disabled: true })), false);
  assert.equal(isTextEntry(field('BUTTON')), false);
  assert.equal(isTextEntry(field('DIV', { isContentEditable: true })), true);
  assert.equal(isTextEntry(null), false);
});

test('the keyboard is a big drop with a text field focused — never browser chrome or a hardware keyboard', () => {
  // iPhone portrait: 932 tall, the keyboard leaves 520.
  assert.deepEqual(keyboardState(932, 520, true), { open: true, visibleHeight: 520, inset: 412 });
  // Same drop, nothing focused (a split-screen resize): not the keyboard.
  assert.deepEqual(keyboardState(932, 520, false), { open: false, visibleHeight: 520, inset: 0 });
  // Safari's toolbars (≈ 80 px) with the composer focused: not the keyboard.
  assert.equal(keyboardState(932, 852, true).open, false);
  assert.equal(KEYBOARD_MIN_PX, 120);
  // Landscape: 430 tall, a 200 px keyboard leaves 230.
  assert.deepEqual(keyboardState(430, 230, true), { open: true, visibleHeight: 230, inset: 200 });
  // An iPad with a hardware keyboard: focused, no drop.
  assert.equal(keyboardState(1194, 1194, true).open, false);
});

test('the keyboard-free baseline: the tallest height at this width; a rotation starts over', () => {
  // iOS: the layout viewport keeps its height while the visual one shrinks.
  let b = nextBaseline(null, 430, 932, 932);
  b = nextBaseline(b, 430, 932, 520);
  assert.equal(b.height, 932);
  // Android resizes-content: both shrink; the earlier full height is kept.
  b = nextBaseline(b, 430, 520, 520);
  assert.equal(b.height, 932);
  // Rotation: a new width, a new baseline (no stale portrait height).
  b = nextBaseline(b, 932, 430, 430);
  assert.deepEqual(b, { width: 932, height: 430 });
});

function harness(opts: { innerHeight?: number; width?: number; vvHeight: number }) {
  const handlers: Record<string, () => void> = {};
  const winHandlers: Record<string, () => void> = {};
  const vv = { height: opts.vvHeight, scale: 1, offsetTop: 0, addEventListener: (t: string, f: () => void) => (handlers[t] = f) };
  const doc = { activeElement: null as Element | null };
  const win = {
    visualViewport: vv as never,
    scrollY: 0,
    scrollTo: () => {},
    innerHeight: opts.innerHeight ?? opts.vvHeight,
    innerWidth: opts.width ?? 430,
    document: doc,
    addEventListener: (t: string, f: () => void) => (winHandlers[t] = f),
  };
  const props: Record<string, string> = {};
  const dataset: Record<string, string | undefined> = {};
  fitToVisualViewport(win, { style: { setProperty: (k, v) => (props[k] = v) }, dataset });
  return { vv, win, doc, props, dataset, handlers, winHandlers };
}

test('iOS: the visual viewport shrinks under a focused composer → data-kb, --kb-h, listeners; no double subtraction', () => {
  const h = harness({ innerHeight: 932, vvHeight: 932 });
  const seen: boolean[] = [];
  const off = onKeyboard((s) => seen.push(s.open));
  h.doc.activeElement = field('TEXTAREA');
  h.winHandlers.focusin();
  assert.equal(h.dataset.kb, undefined, 'focus alone is not the keyboard');
  Object.assign(h.vv, { height: 520 });
  h.handlers.resize();
  assert.equal(h.dataset.kb, 'open');
  assert.equal(h.props['--app-h'], '520px', 'the app is the visible height, once');
  assert.equal(h.props['--kb-h'], '412px');
  // Keyboard down (focus can stay on iOS): closed again.
  Object.assign(h.vv, { height: 932 });
  h.handlers.resize();
  assert.equal(h.dataset.kb, undefined);
  assert.equal(h.props['--kb-h'], '0px');
  assert.deepEqual(seen.slice(-2), [true, false]);
  off();
});

test('Android resizes-content: layout and visual viewport shrink together — still the keyboard, --app-h is the visible height', () => {
  const h = harness({ innerHeight: 839, vvHeight: 839, width: 412 });
  h.doc.activeElement = field('INPUT');
  Object.assign(h.vv, { height: 461 });
  h.win.innerHeight = 461;
  h.handlers.resize();
  assert.equal(h.dataset.kb, 'open');
  assert.equal(h.props['--app-h'], '461px');
  assert.equal(h.props['--kb-h'], '378px');
  // The field loses focus while the keyboard animates away: closed at once.
  h.doc.activeElement = null;
  h.handlers.resize();
  assert.equal(h.dataset.kb, undefined);
});

test('Android resizes-visual: a visual-viewport pan the window cannot scroll back is followed (--app-top), and the field is brought back into view', async () => {
  const h = harness({ innerHeight: 778, vvHeight: 778, width: 412 });
  let revealed = 0;
  h.doc.activeElement = Object.assign(field('TEXTAREA'), { scrollIntoView: () => void revealed++ });
  // Chrome shrinks the visual viewport and pans it 195 px down to show the field.
  Object.assign(h.vv, { height: 471, offsetTop: 195 });
  h.handlers.resize();
  assert.equal(h.dataset.kb, 'open');
  assert.equal(h.props['--app-h'], '471px');
  assert.equal(h.props['--app-top'], '195px', 'the app moves onto the panned visible area, no blank band');
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(revealed, 1, 'the focused field is scrolled back into the shrunk app once');
  Object.assign(h.vv, { height: 778, offsetTop: 0 });
  h.handlers.resize();
  assert.equal(h.dataset.kb, undefined);
  assert.equal(h.props['--app-top'], '0px');
});

test('a keyboard that leaves a short strip (a phone on its side) marks data-kb-short; a tall one does not', () => {
  const h = harness({ innerHeight: 304, vvHeight: 304, width: 864 });
  h.doc.activeElement = field('TEXTAREA');
  Object.assign(h.vv, { height: 67 });
  h.handlers.resize();
  assert.equal(h.dataset.kb, 'open');
  assert.equal(h.dataset.kbShort, '', 'the shell chrome steps aside');
  Object.assign(h.vv, { height: 304 });
  h.handlers.resize();
  assert.equal(h.dataset.kbShort, undefined, 'and comes back when the keyboard closes');
  const tall = harness({ innerHeight: 784, vvHeight: 784, width: 412 });
  tall.doc.activeElement = field('TEXTAREA');
  Object.assign(tall.vv, { height: 472 });
  tall.handlers.resize();
  assert.equal(tall.dataset.kb, 'open');
  assert.equal(tall.dataset.kbShort, undefined, 'portrait with room keeps its chrome');
});

test('horizontal drift on window or shell containers is reset on sync and via resetShellScroll', () => {
  const handlers: Record<string, () => void> = {};
  const winHandlers: Record<string, () => void> = {};
  const vv = { height: 844, scale: 1, offsetTop: 0, offsetLeft: 160, addEventListener: (t: string, f: () => void) => (handlers[t] = f) };
  const scrolls: number[][] = [];
  const docEl = { scrollLeft: 180, scrollTop: 24 };
  const body = { scrollLeft: 90, scrollTop: 0 };
  const content = { scrollLeft: 210, scrollTop: 0 };
  const shell = { scrollLeft: 75, scrollTop: 12 };
  const win = {
    visualViewport: vv as never,
    scrollX: 160,
    scrollY: 0,
    scrollTo: (x: number, y: number) => {
      scrolls.push([x, y]);
      win.scrollX = x;
      win.scrollY = y;
    },
    innerHeight: 844,
    innerWidth: 390,
    document: {
      activeElement: null as Element | null,
      documentElement: docEl,
      body,
      getElementById: (id: string) => (id === 'app-content' ? content : null),
      querySelector: (sel: string) => (sel === '.shell' ? shell : null),
    },
    addEventListener: (t: string, f: () => void) => (winHandlers[t] = f),
  };
  const props: Record<string, string> = {};
  const dataset: Record<string, string | undefined> = {};
  fitToVisualViewport(win, { style: { setProperty: (k, v) => (props[k] = v) }, dataset });
  assert.deepEqual(scrolls, [[0, 0]]);
  assert.equal(docEl.scrollLeft, 0);
  assert.equal(docEl.scrollTop, 0);
  assert.equal(body.scrollLeft, 0);
  assert.equal(content.scrollLeft, 0);
  assert.equal(shell.scrollLeft, 0);
  // Explicit resetShellScroll undoes later drift (e.g. after navigation).
  content.scrollLeft = 140;
  vv.offsetLeft = 40;
  resetShellScroll(win);
  assert.equal(content.scrollLeft, 0);
  assert.equal(scrolls.length, 2);
  // Orientation change re-syncs --app-h immediately.
  Object.assign(vv, { height: 390, offsetLeft: 0 });
  win.innerWidth = 844;
  win.innerHeight = 390;
  winHandlers.orientationchange();
  assert.equal(props['--app-h'], '390px');
});

test('data-kb is never stuck open if focus leaves while zoomed', () => {
  const h = harness({ innerHeight: 844, vvHeight: 844, width: 390 });
  h.doc.activeElement = field('INPUT');
  Object.assign(h.vv, { height: 460 });
  h.handlers.resize();
  assert.equal(h.dataset.kb, 'open');
  // iOS auto-zoomed while typing, then user tapped outside / navigated away.
  Object.assign(h.vv, { scale: 1.25 });
  h.doc.activeElement = null;
  h.handlers.resize();
  assert.equal(h.dataset.kb, undefined, 'BottomNav must not stay hidden');
  assert.equal(h.props['--kb-h'], '0px');
});
