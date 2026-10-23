// ux-design-6w1-04: the MCP home's "Attach to sessions" row never shows a
// guessed ticked box. While the daemon hasn't reported the workspace's
// setting the row is a loading placeholder; a failed read is the inline error
// with Retry; only a reported value renders the checkbox, ticked as reported.
// accessibility-responsive-6w1-01 (mcp): field errors and the audit
// "load more" failure are spoken at the state edge, not by a role inserted
// with its text.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';
import { attachView } from '../src/modules/mcp/sessionAttach.ts';

const HOME = new URL('../src/modules/mcp/OttoServerHome.svelte', import.meta.url);
const src = (file: string) => readFileSync(new URL(`../src/modules/mcp/${file}`, import.meta.url), 'utf8');

/** The real `loadAttach`, with the daemon call held open until `answer`. */
function home(initial: unknown) {
  let answer: (v: unknown) => void = () => {};
  let fail: (e: Error) => void = () => {};
  const state = componentFunctions(HOME, ['loadAttach'], {
    wsId: 'w1',
    attach: initial,
    attachError: null,
    attachGeneration: 0,
    loadErrorText: (e: Error) => e.message,
    mcpCpExtraApi: {
      sessionAttach: () => new Promise((resolve, reject) => { answer = resolve; fail = reject; }),
    },
  });
  const view = () => attachView(state.wsId, state.attach, state.attachError);
  return { state, view, answer: (v: unknown) => answer(v), fail: (e: Error) => fail(e) };
}

test('the attach row loads (no guessed tick) until the daemon reports a detached workspace', async () => {
  // A previous workspace's reported value must not linger either.
  const h = home({ attached: true });
  const done = h.state.loadAttach('w1');
  assert.equal(h.view(), 'loading');
  assert.equal(h.state.attach, null);
  h.answer({ attached: false });
  await done;
  assert.equal(h.view(), 'ready');
  assert.equal((h.state.attach as { attached: boolean } | null)?.attached, false, 'ticked only as reported');
});

test('a failed read is the inline error, and a switched workspace drops a stale answer', async () => {
  const h = home(null);
  const done = h.state.loadAttach('w1');
  h.fail(new Error('daemon unreachable'));
  await done;
  assert.equal(h.view(), 'error');
  assert.equal(h.state.attachError, 'daemon unreachable');

  const late = home(null);
  const pending = late.state.loadAttach('w1');
  late.state.wsId = 'w2';
  late.answer({ attached: true });
  await pending;
  assert.equal(late.state.attach, null, 'w1’s answer never lands on w2');
});

test('attachView covers every state', () => {
  assert.equal(attachView(null, null, null), 'no-workspace');
  assert.equal(attachView('w', null, 'boom'), 'error');
  assert.equal(attachView('w', null, null), 'loading');
  assert.equal(attachView('w', { attached: false }, null), 'ready');
});

test('the template branches on attachView: LoadState while loading, the reported value when ready', () => {
  const s = readFileSync(HOME, 'utf8');
  const loading = s.indexOf("{:else if attachState === 'loading'}");
  const ready = s.indexOf("{:else if attachState === 'ready' && attach}");
  assert.ok(loading > 0 && ready > loading);
  assert.match(s.slice(loading, ready), /<LoadState what="the session-attach setting" variant="compact" loading/);
  assert.match(s.slice(ready, ready + 400), /checked=\{attach\.attached\}/);
  assert.doesNotMatch(s, /attach\?\.attached \?\? true\}/, 'no guessed checkbox value');
});

test('mcp field errors are plain text tied to their field, and the save edge announces', () => {
  for (const file of ['PolicyForm.svelte', 'ServerForm.svelte', 'AuditTab.svelte']) {
    const s = src(file);
    assert.doesNotMatch(s, /\{#if \w+\}<span[^>]*role="alert"/, `${file}: no inserted alert`);
    assert.match(s, /announce\(/, `${file}: announces at the edge`);
  }
  for (const id of ['mcp-server-name-err', 'mcp-server-command-err', 'mcp-server-url-err', 'mcp-server-env-err']) {
    assert.ok(src('ServerForm.svelte').includes(`aria-describedby={`) && src('ServerForm.svelte').includes(`'${id}'`), id);
  }
});
