// MCP OAuth sign-in: leaving the Servers tab stops the 5-minute status poll
// without a stale "Sign-in didn't finish" toast on another page.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

test('an aborted sign-in stops polling and stays quiet', { timeout: 3000 }, async () => {
  const ctl = new AbortController();
  let polls = 0;
  const toasts: string[] = [];
  const { signIn } = loadSource(new URL('../src/modules/mcp/mcpOAuth.ts', import.meta.url), {
    '../../lib/api/mcp': { mcpCpApi: {
      cpOAuthStart: async () => ({ authorization_url: 'https://auth.example/authorize', authorization_host: 'auth.example' }),
      cpOAuthStatus: async () => { if (++polls === 2) ctl.abort(); return { signed_in: false, pending: true }; },
    } },
    '../../lib/confirm.svelte': { confirmer: { ask: async () => true } },
    '../../lib/external': { openExternal: async () => {} },
    '../../lib/toast.svelte': { toasts: { success: (t: string) => toasts.push(t), warn: (t: string) => toasts.push(t) } },
    '../../lib/toastError': { toastError: (t: string) => toasts.push(t) },
  }, { setTimeout: (fn: () => void) => setImmediate(fn) });
  assert.equal(await signIn({ id: 's1', name: 'Remote', url: 'https://mcp.example' }, ctl.signal), false);
  assert.equal(polls, 2);
  assert.deepEqual(toasts, []);
});

test('leaving while the sign-in starts or its status answers stays quiet and asks nothing', { timeout: 3000 }, async () => {
  for (const at of ['start', 'status'] as const) {
    const ctl = new AbortController();
    const seen: string[] = [];
    const { signIn } = loadSource(new URL('../src/modules/mcp/mcpOAuth.ts', import.meta.url), {
      '../../lib/api/mcp': { mcpCpApi: {
        cpOAuthStart: async () => { if (at === 'start') ctl.abort(); return { authorization_url: 'https://auth.example/authorize', authorization_host: 'auth.example' }; },
        cpOAuthStatus: async () => { ctl.abort(); return { signed_in: true, pending: false }; },
      } },
      '../../lib/confirm.svelte': { confirmer: { ask: async () => { seen.push('ask'); return true; } } },
      '../../lib/external': { openExternal: async () => void seen.push('browser') },
      '../../lib/toast.svelte': { toasts: { success: (t: string) => seen.push(t), warn: (t: string) => seen.push(t) } },
      '../../lib/toastError': { toastError: (t: string) => seen.push(t) },
    }, { setTimeout: (fn: () => void) => setImmediate(fn) });
    assert.equal(await signIn({ id: 's1', name: 'Remote', url: 'https://mcp.example' }, ctl.signal), false, at);
    assert.deepEqual(seen, at === 'start' ? [] : ['ask', 'browser'], at);
  }
});
