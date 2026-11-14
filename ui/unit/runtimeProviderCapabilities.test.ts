import assert from 'node:assert/strict';
import { test } from 'node:test';
import { attachmentSupport, takesFile } from '../src/modules/threads/runtime/attachmentSupport.ts';
import {
  providerSupportsSteer,
  RUNTIME_PROVIDERS,
} from '../src/modules/threads/runtime/capabilities.ts';

test('RUNTIME_PROVIDERS enumerates all 7 structured runtime providers', () => {
  assert.deepEqual(RUNTIME_PROVIDERS, [
    'claude',
    'codex',
    'pi',
    'agy',
    'gemini',
    'grok',
    'opencode',
  ]);
});

test('providerSupportsSteer enables mid-turn Steer for Claude, Codex, and pi, and not ACP v1 providers', () => {
  for (const p of ['claude', 'codex', 'pi'] as const) {
    assert.equal(providerSupportsSteer(p), true, `${p} supports mid-turn steer`);
  }
  for (const p of ['agy', 'gemini', 'grok', 'opencode'] as const) {
    assert.equal(providerSupportsSteer(p), false, `${p} (ACP v1) has no mid-turn steer RPC`);
  }
  for (const other of ['', 'shell', 'unknown', 'CLAUDE']) {
    assert.equal(providerSupportsSteer(other), false, `unexpected steer support for ${other}`);
  }
});

test('attachment parity across all 7 runtime providers matches driver capabilities', () => {
  for (const p of RUNTIME_PROVIDERS) {
    assert.equal(attachmentSupport(p).images, true, `${p} accepts images`);
    assert.equal(takesFile(p, { type: 'image/png' }), true, `${p} accepts image/png`);
    const expectsPdf = p !== 'codex' && p !== 'pi';
    assert.equal(attachmentSupport(p).files, expectsPdf, `${p} PDF support`);
    assert.equal(takesFile(p, { type: 'application/pdf' }), expectsPdf, `${p} application/pdf`);
  }
});
