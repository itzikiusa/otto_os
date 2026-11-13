// ⌘T on Threads starts a thread, and the terminal "New session" sheet points
// there: the ⌃⇧T browser-tab alias honours a page's ⌘T claim (it used to
// return before `pageChords`, so the API page's ⌘T claim never saw it), the
// sheet's one-line hint only navigates once the sheet really closed (the
// Threads ⌘T claim and `#/threads/new` are covered by
// e2e/desktop-new-session-kind.spec.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

// A DOM-free stand-in for the browser bits installKeyMap touches.
class FakeKeyboardEvent {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  defaultPrevented = false;
  constructor(_type: string, init: { key: string; metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean; altKey?: boolean }) {
    this.key = init.key;
    this.metaKey = !!init.metaKey;
    this.ctrlKey = !!init.ctrlKey;
    this.shiftKey = !!init.shiftKey;
    this.altKey = !!init.altKey;
  }
  preventDefault(): void {
    this.defaultPrevented = true;
  }
}
const handlers: Record<string, (e: unknown) => void> = {};
Object.assign(globalThis, {
  KeyboardEvent: FakeKeyboardEvent,
  window: {
    addEventListener: (t: string, h: (e: unknown) => void) => (handlers[t] = h),
    removeEventListener: () => {},
  },
  document: { activeElement: null },
});

const { installKeyMap, keyContext, pageClaimsAlias } = await import('../src/lib/keys.ts');

const actions: string[] = [];
installKeyMap((a) => void actions.push(a));
const press = (init: ConstructorParameters<typeof FakeKeyboardEvent>[1]) => {
  const e = new FakeKeyboardEvent('keydown', init);
  handlers.keydown(e);
  return e;
};

test('pageClaimsAlias hands the page a plain ⌘T, and is false with no claim', () => {
  keyContext.pageChords = null;
  assert.equal(pageClaimsAlias('t'), false);
  const seen: { key: string; metaKey: boolean; shiftKey: boolean; ctrlKey: boolean }[] = [];
  keyContext.pageChords = (e) => (seen.push({ key: e.key, metaKey: e.metaKey, shiftKey: e.shiftKey, ctrlKey: e.ctrlKey }), true);
  assert.equal(pageClaimsAlias('t'), true);
  assert.deepEqual(seen, [{ key: 't', metaKey: true, shiftKey: false, ctrlKey: false }]);
  keyContext.pageChords = null;
});

test('⌃⇧T: a page claim (Threads → new thread, API → new request) wins over New session', () => {
  actions.length = 0;
  let claimed = 0;
  keyContext.pageChords = (e) => {
    if (e.key !== 't' || e.shiftKey) return false;
    claimed++;
    return true;
  };
  const e = press({ key: 'T', ctrlKey: true, shiftKey: true });
  assert.equal(claimed, 1, 'the page saw the alias');
  assert.deepEqual(actions, [], 'New session did not open');
  assert.equal(e.defaultPrevented, true);
  keyContext.pageChords = null;
});

test('⌃⇧T / ⌃⇧W with no page claim keep their global actions', () => {
  actions.length = 0;
  keyContext.pageChords = () => false;
  press({ key: 'T', ctrlKey: true, shiftKey: true });
  press({ key: 'W', ctrlKey: true, shiftKey: true });
  keyContext.pageChords = null;
  press({ key: 't', ctrlKey: true, shiftKey: true });
  assert.deepEqual(actions, ['newSession', 'closeTab', 'newSession']);
});

test('"Start a thread" goes to the composer only when the sheet closed (a kept draft stays)', async () => {
  const went: string[] = [];
  const router = { go: (r: string) => went.push(r) };
  const kept = componentFunctions(new URL('../src/modules/agents/SessionKindHint.svelte', import.meta.url), ['startThread'], {
    router,
    onclose: async () => false,
  });
  await kept.startThread();
  assert.deepEqual(went, [], 'the person chose "Keep editing"');
  const closed = componentFunctions(new URL('../src/modules/agents/SessionKindHint.svelte', import.meta.url), ['startThread'], {
    router,
    onclose: () => true,
  });
  await closed.startThread();
  assert.deepEqual(went, ['threads/new']);
});

test('New session requestClose tells the hint whether the sheet closed', async () => {
  const run = async (typed: string, answer: boolean) => {
    let closes = 0;
    const f = componentFunctions(new URL('../src/modules/agents/NewSession.svelte', import.meta.url), ['requestClose'], {
      prompt: typed, title: '', extraDirs: [],
      confirmer: { ask: async () => answer },
      onclose: () => void closes++,
    });
    return { result: await f.requestClose(), closes };
  };
  assert.deepEqual(await run('', false), { result: true, closes: 1 }, 'nothing typed: closes');
  assert.deepEqual(await run('draft', false), { result: false, closes: 0 }, 'Keep editing: stays open');
  assert.deepEqual(await run('draft', true), { result: true, closes: 1 }, 'Discard: closes');
});
