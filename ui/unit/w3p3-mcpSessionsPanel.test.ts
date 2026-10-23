// ux-design-6w2-01: the MCP home's "What my sessions see" panel never shows a
// guessed value. The "Built-in tools" line follows the attach row's state
// (checking… / attached / not attached / unknown), and the gateway list is a
// loading placeholder on its first load — not "…: 0" — and an inline error
// with Retry when the read fails. A failed attach save puts the box back to
// the value the daemon reported, never to a guessed tick.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';
import { attachView, builtInToolsLine, gatewayView } from '../src/modules/mcp/sessionAttach.ts';

const HOME = new URL('../src/modules/mcp/OttoServerHome.svelte', import.meta.url);

test('the Built-in tools line follows every attach state', () => {
  assert.equal(builtInToolsLine(attachView('w', null, null), null), 'Built-in tools: checking…');
  assert.equal(builtInToolsLine(attachView('w', null, 'boom'), null), 'Built-in tools: unknown (see above)');
  assert.equal(
    builtInToolsLine(attachView('w', { attached: true }, null), { attached: true }),
    'Built-in tools: attached',
  );
  assert.equal(
    builtInToolsLine(attachView('w', { attached: false }, null), { attached: false }),
    'Built-in tools: not attached',
  );
  assert.equal(builtInToolsLine(attachView(null, null, null), null), 'Built-in tools: select a workspace');
  for (const view of ['loading', 'error', 'no-workspace'] as const) {
    assert.doesNotMatch(builtInToolsLine(view, null), /\battached\b/, `${view}: no guessed "attached"`);
  }
});

/** The real `loadGatewayTools`, with the daemon call held open until `answer`. */
function gateway() {
  let answer: (v: unknown) => void = () => {};
  let fail: (e: Error) => void = () => {};
  const state = componentFunctions(HOME, ['loadGatewayTools'], {
    wsId: 'w1',
    gatewayTools: [],
    gatewayLoading: false,
    gatewayError: null,
    loadErrorText: (e: Error) => e.message,
    mcpCpExtraApi: {
      gatewayTools: () => new Promise((resolve, reject) => { answer = resolve; fail = reject; }),
    },
  });
  const view = () => gatewayView(state.wsId, state.gatewayLoading, state.gatewayError, state.gatewayTools.length);
  return { state, view, answer: (v: unknown) => answer(v), fail: (e: Error) => fail(e) };
}

test('the gateway list loads (no "0" count), then shows the reported tools', async () => {
  const g = gateway();
  const done = g.state.loadGatewayTools('w1');
  assert.equal(g.view(), 'loading', 'a first load is a placeholder, not a 0 count');
  g.answer({ tools: [{ name: 'gh.list_prs' }] });
  await done;
  assert.equal(g.view(), 'ready');
  assert.equal(g.state.gatewayTools.length, 1);

  // A refresh keeps the list it already has on screen.
  const again = g.state.loadGatewayTools('w1');
  assert.equal(g.state.gatewayLoading, true);
  assert.equal(g.view(), 'ready');
  g.answer({ tools: [] });
  await again;
  assert.equal(g.view(), 'ready', 'an empty answer is a real 0');
});

test('a failed gateway read is the inline error, not an empty list', async () => {
  const g = gateway();
  const done = g.state.loadGatewayTools('w1');
  g.fail(new Error('daemon unreachable'));
  await done;
  assert.equal(g.view(), 'error');
  assert.equal(g.state.gatewayError, 'daemon unreachable');
  assert.equal(g.state.gatewayLoading, false);
  assert.equal(gatewayView(null, false, null, 0), 'no-workspace');
});

test('a failed attach save puts the box back to the reported value', async () => {
  for (const reported of [true, false]) {
    const input = { checked: !reported };
    const errors: string[] = [];
    const state = componentFunctions(HOME, ['updateAttach'], {
      wsId: 'w1',
      attach: { attached: reported },
      attachBusy: false,
      attachError: null,
      attachGeneration: 0,
      toasts: { success: () => {} },
      toastError: (title: string) => errors.push(title),
      mcpCpExtraApi: { setSessionAttach: () => Promise.reject(new Error('nope')) },
    });
    await state.updateAttach(!reported, input);
    assert.equal(input.checked, reported, `reported ${reported}: no guessed tick`);
    assert.equal(errors.length, 1);
    assert.equal(state.attachBusy, false);
  }
});

test('(structure) the panel renders from the derived views, with no guessed fallbacks', () => {
  const s = readFileSync(HOME, 'utf8');
  assert.match(s, /\{builtInToolsLine\(attachState, attach\)\}/);
  const loading = s.indexOf("{:else if gatewayState === 'loading'}");
  assert.ok(loading > 0, 'a gateway loading branch');
  assert.match(s.slice(loading, loading + 200), /<LoadState what="the gateway tools" variant="compact" loading empty rows=\{1\}/);
  assert.ok(loading < s.indexOf('Gateway tools from governed external servers'), 'loading branch before the count');
  assert.doesNotMatch(s, /attached \?\? true/, 'no guessed attach value');
});
