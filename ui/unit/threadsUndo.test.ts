// Undo instead of confirm (threadUndo.ts): one notice at a time, Undo runs
// the reverse, ⌘Z runs it outside text fields / editors / terminals only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function load(module = 'threads') {
  const shown: { id: number; title: string; ttl: number; run: () => unknown; onClose?: (r: string) => void }[] = [];
  let next = 1;
  const toasts = {
    push: (_l: string, title: string, _b: unknown, ttl: number, o: { action: { run: () => unknown }; onClose?: (r: string) => void }) => {
      shown.push({ id: next, title, ttl, run: o.action.run, onClose: o.onClose });
      return next++;
    },
    dismiss: (id: number) => {
      const i = shown.findIndex((t) => t.id === id);
      if (i >= 0) shown.splice(i, 1)[0].onClose?.('dismissed');
    },
    runAction: async (id: number) => {
      const t = shown.find((x) => x.id === id);
      toasts.dismiss(id);
      await t?.run();
    },
  };
  const mod = loadSource(new URL('../src/modules/threads/threadUndo.ts', import.meta.url), {
    '../../lib/router.svelte': { router: { module } },
    '../../lib/toast.svelte': { toasts },
  }) as unknown as typeof import('../src/modules/threads/threadUndo.ts');
  return { ...mod, shown };
}

const key = (k: string, mods: Partial<KeyboardEvent> = {}) => {
  let prevented = false;
  return { e: { key: k, metaKey: true, ctrlKey: false, shiftKey: false, altKey: false, ...mods, preventDefault: () => (prevented = true) }, prevented: () => prevented };
};
const el = (inside: boolean) => ({ closest: () => (inside ? {} : null) }) as unknown as Element;

test('a newer action replaces the notice; Undo runs the last reverse once', async () => {
  const { offerUndo, undoLast, shown } = load();
  const ran: string[] = [];
  offerUndo('Pinned', () => void ran.push('unpin'));
  offerUndo('Moved to History', () => void ran.push('restore'));
  assert.deepEqual(shown.map((t) => t.title), ['Moved to History']);
  assert.equal(shown[0].ttl, 5000);
  assert.equal(undoLast(), true);
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(ran, ['restore']);
  assert.equal(undoLast(), false);
});

test('⌘Z undoes outside text fields, never inside one or outside Threads', () => {
  const t = load();
  t.offerUndo('Pinned', () => {});
  const inField = key('z');
  assert.equal(t.onUndoKey(inField.e, el(true)), false);
  assert.equal(inField.prevented(), false);
  const shifted = key('z', { shiftKey: true });
  assert.equal(t.onUndoKey(shifted.e, el(false)), false);
  const plain = key('z');
  assert.equal(t.onUndoKey(plain.e, el(false)), true);
  assert.equal(plain.prevented(), true);
  const other = load('agents');
  other.offerUndo('Pinned', () => {});
  assert.equal(other.onUndoKey(key('z').e, el(false)), false);
});
