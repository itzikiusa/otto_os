import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

// architecture-1-13 (vault): the `otto.ui_vault_*` handlers live in lib/ and
// reach the Vault module's store only through `vaultPort`, which the Vault
// page binds while it is mounted. A handler navigates to the vault, then WAITS for the
// port (the Vault chunk may still be loading), and drives the bound store.

class UiCommandError extends Error {
  code: string;
  constructor(code: string, message: string) { super(message); this.code = code; }
}

function load() {
  const handlers: Record<string, (args: any, ctx: any) => Promise<any>> = {};
  const states: Record<string, () => any> = {};
  const uiCommands = {
    UiCommandError,
    whenMounted: async () => null,
    registerUiCommands: (_m: string, map: Record<string, any>) => { Object.assign(handlers, map); return () => {}; },
    registerUiState: (m: string, fn: () => any) => { states[m] = fn; return () => {}; },
  };
  const pagePort = loadSource(new URL('../src/lib/uiCommands/pagePort.ts', import.meta.url), {
    '../uiCommands': uiCommands,
    '../confirm.svelte': { confirmer: {} },
  });
  const vaultPort = loadSource(new URL('../src/lib/uiCommands/vaultPort.ts', import.meta.url), { './pagePort': pagePort });
  const router = { parts: ['agents'] as string[], go(m: string) { this.parts = [m]; } };
  loadSource(new URL('../src/lib/uiCommands/vault.ts', import.meta.url), {
    '../uiCommands': uiCommands,
    '../router.svelte': { router },
    '../api/vault': { vaultNote: async () => ({}) },
    '../api/client': { ApiError: class extends Error {} },
    '../toast.svelte': { toasts: { success() {} } },
    './pagePort': pagePort,
    './vaultPort': vaultPort,
  }, { CSS: { escape: (s: string) => s } });
  return { handlers, states, vaultPort, router };
}

function fakeStore() {
  const calls: string[] = [];
  const docs = { id: 7, name: 'Docs', root_path: '/d', okf: false, notes: 3 };
  return {
    calls,
    wsId: 'w1',
    vaults: [docs],
    loading: false,
    loadError: null,
    current: docs,
    tabs: [{ path: 'a.md' }],
    notePath: 'a.md',
    note: null,
    editing: false,
    dirty: false,
    draft: '',
    centerMode: 'note',
    leftMode: 'files',
    searchQuery: '',
    searchedQuery: '',
    searchError: null,
    searchHits: [{ path: 'a.md', title: 'A', snippet: 'hit' }],
    holdAutosave: false,
    async load() { calls.push('load'); },
    async select(id: number) { calls.push(`select:${id}`); },
    async open() {},
    async runSearch() { calls.push(`search:${this.searchQuery}`); this.searchedQuery = this.searchQuery; },
    async createNote() { return true; },
    onDraftChange() {},
    async saveNow() { return true; },
  };
}

/** Values built inside the vm realm, compared as plain data. */
const plain = (v: unknown) => JSON.parse(JSON.stringify(v));

const ctx = (signal = new AbortController().signal) => ({ signal, confirmWrite: async () => true });

test('a handler navigates to the vault and waits for the store to bind the port', async () => {
  const { handlers, vaultPort, router } = load();
  const store = fakeStore();
  const pending = handlers.vault_search({ query: 'okf' }, ctx());
  // The Vault chunk "loads" after the navigation: the port is bound later.
  await new Promise((r) => setTimeout(r, 120));
  assert.deepEqual(router.parts, ['vault'], 'navigated first');
  vaultPort.registerVaultPort(store);
  const out = await pending;
  assert.deepEqual(plain(out), { vault: 'Docs', query: 'okf', hits: [{ path: 'a.md', title: 'A', snippet: 'hit' }] });
  assert.deepEqual(store.calls, ['search:okf'], 'drives the bound store');
  assert.equal(store.leftMode, 'search');
});

test('an aborted command stops waiting for a store that never binds', async () => {
  const { handlers } = load();
  const ac = new AbortController();
  const pending = handlers.vault_list({}, ctx(ac.signal));
  ac.abort();
  await assert.rejects(pending, (e: UiCommandError) => e.code === 'cancelled_by_user');
});

test('ui_state reports nothing open before the store binds, then the bound store', () => {
  const { states, vaultPort } = load();
  assert.equal(states.vault().vault, null);
  assert.deepEqual(plain(states.vault().tabs), []);
  const unbind = vaultPort.registerVaultPort(fakeStore());
  assert.deepEqual(plain(states.vault().vault), { id: 7, name: 'Docs' });
  assert.deepEqual(plain(states.vault().tabs), ['a.md']);
  unbind();
  assert.equal(states.vault().vault, null, 'unbinding forgets the store');
});

test('lib/uiCommands/vault.ts no longer imports modules/ (structure)', () => {
  const src = readFileSync(new URL('../src/lib/uiCommands/vault.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /from '\.\.\/\.\.\/modules\//);
  const page = readFileSync(new URL('../src/modules/vault/VaultPage.svelte', import.meta.url), 'utf8'); // ui-guards: allow (structural pin)
  assert.match(page, /\$effect\(\(\) => registerVaultPort\(vault\)\);/, 'bound for the page’s lifetime (the effect returns the unbind)');
});
