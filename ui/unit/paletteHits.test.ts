// ⌘K palette search hits: every action shown has a handler (review 06 F4 + Feature Edge).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  hitActions,
  hitContext,
  hitRoute,
  normalizeHitAction,
  resolveHitKeyAction,
} from '../src/lib/floatingBar.ts';
import { loadSource } from './sourceHarness.ts';

test('open is first, power verbs rerun/review are surfaced for supported kinds, unknown actions stay hidden', () => {
  assert.deepEqual(
    hitActions({ kind: 'workflow', actions: ['rerun', 'unknown-verb', 'send-to-agent', 'open'] }),
    ['open', 'rerun', 'send-to-agent'],
  );
  assert.deepEqual(
    hitActions({ kind: 'api_request', actions: ['open', 'rerun', 'send-to-agent'] }),
    ['open', 'rerun', 'send-to-agent'],
  );
  assert.deepEqual(
    hitActions({ kind: 'repo', actions: ['review', 'open'] }),
    ['open', 'review'],
  );
  // Kind-mismatched power verbs are suppressed so no dead button renders.
  assert.deepEqual(
    hitActions({ kind: 'story', actions: ['open', 'rerun', 'review', 'copy-context'] }),
    ['open', 'copy-context'],
  );
  assert.equal(normalizeHitAction('bogus'), null);
});

test('resolveHitKeyAction routes ⏎ / ⌘⏎ / ⌥⏎ / ⇧⏎ to the matching hit verb', () => {
  const wfActs = hitActions({ kind: 'workflow', actions: ['open', 'rerun', 'send-to-agent', 'copy-context'] });
  assert.equal(resolveHitKeyAction(wfActs), 'open');
  assert.equal(resolveHitKeyAction(wfActs, { metaKey: true }), 'rerun');
  assert.equal(resolveHitKeyAction(wfActs, { ctrlKey: true }), 'rerun');
  assert.equal(resolveHitKeyAction(wfActs, { altKey: true }), 'copy-context');
  assert.equal(resolveHitKeyAction(wfActs, { shiftKey: true }), 'send-to-agent');

  const repoActs = hitActions({ kind: 'repo', actions: ['open', 'review'] });
  assert.equal(resolveHitKeyAction(repoActs, { metaKey: true }), 'review');
  assert.equal(resolveHitKeyAction(repoActs, { altKey: true }), 'open');
});

test("the canvas hit's legacy 'Open in Canvas' label still opens", () => {
  assert.equal(normalizeHitAction('Open in Canvas'), 'open');
  assert.deepEqual(hitActions({ actions: ['Open in Canvas'] }), ['open']);
  // …and opens the board itself (`#/canvas/<id>` forwards to its Hall mirror).
  assert.equal(hitRoute({ kind: 'canvas', id: 'c1' }), 'canvas/c1');
});

test('a repo hit routes to its repo', () => {
  assert.equal(hitRoute({ kind: 'repo', id: 'r9' }), 'git/r9');
});

test('context names the kind, title, subtitle and id', () => {
  assert.equal(
    hitContext({ kind: 'api_request', id: 'a1', title: 'Get user', subtitle: 'GET /users/1' }),
    'api request: Get user\nGET /users/1\n(api_request id a1)',
  );
  assert.equal(hitContext({ kind: 'repo', id: 'r', title: 'otto', subtitle: null }), 'repo: otto\n(repo id r)');
});

test('runHitPowerAction handles repo review, workflow rerun confirmation, and api_request rerun', async () => {
  const routes: string[] = [];
  const posts: Array<{ path: string; body: unknown }> = [];
  const toasts: string[] = [];
  let confirmReply = true;
  let apiExecuted = 0;

  const mod = loadSource(
    new URL('../src/lib/searchHitOpen.ts', import.meta.url),
    {
      './api/client': {
        api: {
          post: async (path: string, body: unknown) => {
            posts.push({ path, body: JSON.parse(JSON.stringify(body)) });
            return { id: 'run-42' };
          },
        },
      },
      './confirm.svelte': {
        confirmer: {
          ask: async () => confirmReply,
        },
      },
      './router.svelte': {
        router: {
          parts: [],
          go: (r: string) => routes.push(r),
        },
      },
      './floatingBar': { hitRoute, hitVaultNote: () => null },
      './stores/apiClient.svelte': {
        apiClient: {
          draft: { kind: 'http' },
          ensureLoaded: async () => {},
          openRequest: async () => true,
          execute: async () => { apiExecuted++; return null; },
          executeGrpc: async () => null,
        },
      },
      './toast.svelte': {
        toasts: {
          success: (t: string) => toasts.push(`ok:${t}`),
          warn: (t: string) => toasts.push(`warn:${t}`),
        },
      },
      './uiCommands/vaultPort': { vaultPort: { peek: () => null } },
      './uiCommands/pagePort': { waitFor: async () => null },
      './toastError': { toastError: (t: string) => toasts.push(`err:${t}`) },
    },
  );

  await mod.runHitPowerAction({ kind: 'repo', id: 'r 9', title: 'otto' }, 'review');
  assert.deepEqual(routes, ['git/r%209/review']);

  // Cancelled workflow rerun must NOT post.
  confirmReply = false;
  await mod.runHitPowerAction({ kind: 'workflow', id: 'wf-1', title: 'Deploy' }, 'rerun');
  assert.equal(posts.length, 0);

  // Confirmed workflow rerun posts and routes straight to the created run.
  confirmReply = true;
  await mod.runHitPowerAction({ kind: 'workflow', id: 'wf-1', title: 'Deploy' }, 'rerun');
  assert.deepEqual(posts, [{ path: '/workflows/wf-1/run', body: {} }]);
  assert.equal(routes[routes.length - 1], 'workflows/wf-1/runs/run-42');
  assert.ok(toasts.includes('ok:Workflow started'));

  // Confirmed api_request rerun navigates, opens, and executes.
  await mod.runHitPowerAction({ kind: 'api_request', id: 'req-7', title: 'Health', subtitle: 'GET /health' }, 'rerun');
  assert.equal(routes[routes.length - 1], 'api/req-7');
  assert.equal(apiExecuted, 1);
});
