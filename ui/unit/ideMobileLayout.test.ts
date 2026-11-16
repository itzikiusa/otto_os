// The IDE's phone layout: the stack/workbench switch is decided by the page's
// own size (a phone on its side is wide but short), a tap on a file in Files
// opens the editor, the bottom dock never fills a short column, and the
// stacked Tools pane lists the terminal first. Plus the viewport's "short"
// bucket that turns the shell's Navigator into a drawer on a phone on its side.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const LAYOUT = new URL('../src/modules/ide/shell/ideLayout.svelte.ts', import.meta.url);
const VIEWPORT = new URL('../src/lib/stores/viewport.svelte.ts', import.meta.url);

function memoryStorage(): Storage {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, String(v)),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
    key: (i: number) => [...m.keys()][i] ?? null,
    get length() { return m.size; },
  } as Storage;
}

function loadLayout(storage = memoryStorage()) {
  return loadSource(LAYOUT, {}, { sessionStorage: storage });
}

test('a phone, a phone on its side and a narrow window stack; a desktop and an iPad held wide do not', () => {
  const { isStackLayout } = loadLayout();
  assert.equal(isStackLayout(430, 640), true, 'phone portrait (narrow)');
  assert.equal(isStackLayout(814, 344), true, 'phone landscape (wide but short)');
  assert.equal(isStackLayout(614, 1150), true, 'iPad portrait beside the Navigator');
  assert.equal(isStackLayout(1138, 790), false, 'iPad landscape');
  assert.equal(isStackLayout(1280, 760), false, 'desktop');
  // Not measured yet: no phone-layout flash on a desktop first paint.
  assert.equal(isStackLayout(0, 0), false);
});

test('a tap on a file in Files shows the editor; other panes stay where they are', () => {
  const { paneAfterFileOpen } = loadLayout();
  assert.equal(paneAfterFileOpen('files'), 'editor');
  assert.equal(paneAfterFileOpen('editor'), 'editor');
  assert.equal(paneAfterFileOpen('tools'), 'tools');
  assert.equal(paneAfterFileOpen('agent'), 'agent');
});

test('the bottom dock leaves room for the editor in a short column', () => {
  const { dockMaxFor } = loadLayout();
  // A phone on its side: a 232 px column used to get a 300 px dock (no editor).
  assert.equal(dockMaxFor(232, 380, 120), 120);
  assert.equal(dockMaxFor(700, 900, 120), 560);
  // Unmeasured column: the old window-based cap.
  assert.equal(dockMaxFor(0, 900, 120), 765);
  assert.equal(dockMaxFor(0, 400, 120), 520);
});

test('the stacked Tools pane lists the terminal first and keeps the rest in order', () => {
  const { stackDockOrder } = loadLayout();
  const ids = Array.from(stackDockOrder([{ id: 'debug' }, { id: 'run' }, { id: 'git' }, { id: 'terminal' }]), (t: { id: string }) => t.id);
  assert.deepEqual(ids, ['terminal', 'debug', 'run', 'git']);
});

test('the stack pane is remembered per tab and an unknown stored value falls back to the editor', () => {
  const storage = memoryStorage();
  const first = loadLayout(storage);
  assert.equal(first.ideLayout.pane, 'editor');
  first.ideLayout.pane = 'tools';
  assert.equal(storage.getItem('otto.ide.stackPane'), 'tools');
  assert.equal(loadLayout(storage).ideLayout.pane, 'tools', 'a reload in the same tab keeps the pane');
  storage.setItem('otto.ide.stackPane', 'bogus');
  assert.equal(loadLayout(storage).ideLayout.pane, 'editor');
});

test('the stack follows the measured page size', () => {
  const { ideLayout } = loadLayout();
  assert.equal(ideLayout.stack, false);
  ideLayout.width = 814;
  ideLayout.height = 344;
  assert.equal(ideLayout.stack, true);
  ideLayout.height = 900;
  ideLayout.width = 1200;
  assert.equal(ideLayout.stack, false);
});

function loadViewport(matches: (q: string) => boolean) {
  const listeners: Array<() => void> = [];
  const window = {
    matchMedia: (q: string) => ({
      get matches() { return matches(q); },
      addEventListener: (_t: string, f: () => void) => listeners.push(f),
    }),
  };
  const mod = loadSource(VIEWPORT, {}, { window });
  return { viewport: mod.viewport, fire: () => listeners.forEach((f) => f()), mod };
}

test('a short touch screen at tablet width is a phone on its side: the Navigator becomes a drawer', () => {
  let w = 814;
  let h = 380;
  const matches = (q: string): boolean => {
    if (q === '(max-width: 640px)') return w <= 640;
    if (q === '(max-width: 1024px)') return w <= 1024;
    if (q.includes('max-height: 500px')) return h <= 500;
    return false;
  };
  const { viewport, fire } = loadViewport(matches);
  assert.equal(viewport.mode, 'tablet', 'module layouts keep the width bucket');
  assert.equal(viewport.isShort, true);
  assert.equal(viewport.navDrawer, true);
  // Back to portrait: a phone — still a drawer, no longer "short".
  w = 430; h = 740; fire();
  assert.equal(viewport.mode, 'phone');
  assert.equal(viewport.isShort, false);
  assert.equal(viewport.navDrawer, true);
  // A real tablet keeps its persistent Navigator.
  w = 834; h = 1194; fire();
  assert.equal(viewport.navDrawer, false);
  // A short DESKTOP-width window is left alone.
  w = 1280; h = 400; fire();
  assert.equal(viewport.isShort, false);
  assert.equal(viewport.navDrawer, false);
});
