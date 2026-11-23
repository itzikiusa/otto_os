// Wave 1 (MUST) tests for Browser module:
// 1. Autofill scopes username lookup to pwd.form (never fills an unrelated header search bar)
// 2. Autofill invokes HTMLInputElement.prototype's native value setter (React controlled inputs)
// 3. createNativeEventHub deduplicates onNewTab / onUrlChange across multi-mounted BrowserViews
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildFillScript } from '../src/modules/browser/autofill.ts';
import { createNativeEventHub, type NativeEventSource } from '../src/modules/browser/liveWebviews.ts';

type MockField = {
  value: string;
  focused: boolean;
  events: string[];
  form?: { querySelector(sel: string): MockField | null };
  focus(): void;
  dispatchEvent(ev: { type: string }): void;
};

function makeField(form?: MockField['form']): MockField {
  return {
    value: '',
    focused: false,
    events: [],
    form,
    focus() { this.focused = true; },
    dispatchEvent(ev: { type: string }) { this.events.push(ev.type); },
  };
}

test('autofill prefers pwd.form over a document-level search input[type="text"]', () => {
  const headerSearch = makeField();
  const formUser = makeField();
  const form = {
    querySelector(sel: string) {
      if (sel.includes('type="text"')) return formUser;
      return null;
    },
  };
  const pwd = makeField(form);
  const document = {
    querySelector(sel: string) {
      if (sel.includes('password')) return pwd;
      if (sel.includes('type="text"')) return headerSearch;
      return null;
    },
  };
  class Event { type: string; constructor(type: string) { this.type = type; } }
  const js = buildFillScript('example.com', 'alice', 'hunter2');
  const fn = new Function('location', 'document', 'Event', `return ${js};`);
  const result = fn({ hostname: 'login.example.com', protocol: 'https:' }, document, Event) as string;

  assert.equal(result, 'filled');
  assert.equal(headerSearch.value, '', 'unrelated header search box must stay untouched');
  assert.equal(formUser.value, 'alice');
  assert.equal(pwd.value, 'hunter2');
  assert.deepEqual(formUser.events, ['input', 'change']);
  assert.deepEqual(pwd.events, ['input', 'change']);
});

test('autofill does not fall back to document-global input[type="text"] when pwd.form has no text field', () => {
  const headerSearch = makeField();
  const form = { querySelector: () => null };
  const pwd = makeField(form);
  const document = {
    querySelector(sel: string) {
      if (sel.includes('password')) return pwd;
      if (sel.includes('type="text"')) return headerSearch;
      return null;
    },
  };
  class Event { type: string; constructor(type: string) { this.type = type; } }
  const js = buildFillScript('example.com', 'alice', 'hunter2');
  const fn = new Function('location', 'document', 'Event', `return ${js};`);
  const result = fn({ hostname: 'login.example.com', protocol: 'https:' }, document, Event) as string;

  assert.equal(result, 'filled');
  assert.equal(headerSearch.value, '', 'must not leak username into outside search input');
  assert.equal(pwd.value, 'hunter2');
});

test('autofill uses HTMLInputElement.prototype native value setter for React controlled inputs', () => {
  const nativeWrites: Array<{ target: unknown; val: string }> = [];
  class FakeInput {
    _val = '';
    events: string[] = [];
    focus() {}
    dispatchEvent(ev: { type: string }) { this.events.push(ev.type); }
    get value() { return this._val; }
    set value(v: string) {
      nativeWrites.push({ target: this, val: v });
      this._val = v;
    }
  }
  const user = new FakeInput();
  const pwd = new FakeInput();
  // Simulate React overriding the instance `value` property so direct `el.value = ...`
  // silently fails to notify React's internal _valueTracker.
  let directInstanceSetterCalled = 0;
  for (const el of [user, pwd]) {
    Object.defineProperty(el, 'value', {
      configurable: true,
      get() { return el._val; },
      set() { directInstanceSetterCalled += 1; },
    });
  }
  const document = {
    querySelector(sel: string) {
      if (sel.includes('password')) return pwd;
      if (sel.includes('email')) return user;
      return null;
    },
  };
  class Event { type: string; constructor(type: string) { this.type = type; } }
  const js = buildFillScript('example.com', 'react@example.com', 'p@ss');
  const fn = new Function('location', 'document', 'Event', 'HTMLInputElement', `return ${js};`);
  const result = fn({ hostname: 'example.com', protocol: 'https:' }, document, Event, FakeInput) as string;

  assert.equal(result, 'filled');
  assert.equal(directInstanceSetterCalled, 0, 'should bypass instance setter via prototype descriptor');
  assert.equal(nativeWrites.length, 2);
  assert.equal(user._val, 'react@example.com');
  assert.equal(pwd._val, 'p@ss');
});

test('createNativeEventHub subscribes once across multiple mounted views and cleans up on last release', async () => {
  let urlListeners = 0;
  let tabListeners = 0;
  // Held in a box: TS narrows a closure-assigned `let x = null` to `never` at the call site.
  const emit: { url: ((id: string, url: string) => void) | null; newTab: ((url: string) => void) | null } = { url: null, newTab: null };

  const source: NativeEventSource = {
    async onUrlChange(cb) {
      urlListeners += 1;
      emit.url = cb;
      return () => {
        urlListeners -= 1;
        emit.url = null;
      };
    },
    async onNewTab(cb) {
      tabListeners += 1;
      emit.newTab = cb;
      return () => {
        tabListeners -= 1;
        emit.newTab = null;
      };
    },
  };

  const hub = createNativeEventHub();
  const opened: string[] = [];
  const tracked: Array<[string, string]> = [];
  const localA: Array<[string, string]> = [];
  const localB: Array<[string, string]> = [];

  const relA = hub.attach(source, {
    onUrlChange: (id, u) => tracked.push([id, u]),
    onNewTab: (u) => opened.push(u),
    onLocalUrlChange: (id, u) => localA.push([id, u]),
  });
  const relB = hub.attach(source, {
    onUrlChange: (id, u) => tracked.push([id, u]),
    onNewTab: (u) => opened.push(u),
    onLocalUrlChange: (id, u) => localB.push([id, u]),
  });
  await Promise.resolve();

  assert.equal(urlListeners, 1, 'only one underlying onUrlChange listener');
  assert.equal(tabListeners, 1, 'only one underlying onNewTab listener');

  emit.newTab?.('https://example.com/pop');
  assert.deepEqual(opened, ['https://example.com/pop'], 'window.open must create exactly one tab');

  emit.url?.('t1', 'https://example.com/next');
  assert.deepEqual(tracked, [['t1', 'https://example.com/next']], 'shared navigation tracks once');
  assert.deepEqual(localA, [['t1', 'https://example.com/next']], 'view A updates its local address bar');
  assert.deepEqual(localB, [['t1', 'https://example.com/next']], 'view B updates its local address bar');

  relA();
  assert.equal(urlListeners, 1, 'still subscribed while view B is mounted');
  assert.equal(tabListeners, 1);

  relB();
  assert.equal(urlListeners, 0, 'unsubscribed after last view unmounts');
  assert.equal(tabListeners, 0);
});

test('createNativeEventHub unsubscribes immediately if released before async subscription resolves', async () => {
  let active = 0;
  let resolveUrl!: (un: () => void) => void;
  let resolveTab!: (un: () => void) => void;
  const source: NativeEventSource = {
    onUrlChange: () => new Promise((r) => { active += 1; resolveUrl = r; }),
    onNewTab: () => new Promise((r) => { active += 1; resolveTab = r; }),
  };

  const hub = createNativeEventHub();
  const rel = hub.attach(source, { onUrlChange: () => {}, onNewTab: () => {} });
  rel();
  resolveUrl(() => { active -= 1; });
  resolveTab(() => { active -= 1; });
  await Promise.resolve();

  assert.equal(active, 0, 'late-resolving listeners must be immediately torn down');
});
