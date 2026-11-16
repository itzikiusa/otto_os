import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  supportsEffortSelection,
  supportsModelSelection,
} from '../src/lib/providerModels.ts';
import {
  isRuntimeProvider,
  runtimeBaselineCaps,
  RUNTIME_PROVIDERS,
} from '../src/lib/providerRegistry.ts';

test('RUNTIME_PROVIDERS enumerates all 7 structured runtime providers from canonical registry', () => {
  assert.deepEqual(RUNTIME_PROVIDERS, [
    'claude',
    'codex',
    'agy',
    'gemini',
    'grok',
    'opencode',
    'pi',
  ]);
  for (const p of RUNTIME_PROVIDERS) {
    assert.equal(isRuntimeProvider(p), true);
  }
  assert.equal(isRuntimeProvider('shell'), false);
});

test('runtimeBaselineCaps reads the daemon baseline, by family for a custom slug', () => {
  const claude = { steer: true, images: true, files: true, plan: true };
  const meta = {
    provider_models: {},
    provider_families: { claudework: 'claude' },
    runtime_capabilities: { claude },
  };
  assert.equal(runtimeBaselineCaps('claude', meta), claude);
  assert.equal(runtimeBaselineCaps('claudework', meta), claude);
  assert.equal(runtimeBaselineCaps('codex', meta), undefined);
  assert.equal(runtimeBaselineCaps('claude', null), undefined);
});

test('model and reasoning effort selection parity includes opencode and pi', () => {
  for (const p of ['claude', 'codex', 'agy', 'opencode', 'pi'] as const) {
    assert.equal(supportsModelSelection(p, null), true, `${p} supports model selection fallback`);
  }
  assert.equal(supportsModelSelection('shell', null), false);
  for (const p of ['claude', 'codex', 'agy', 'pi'] as const) {
    assert.equal(supportsEffortSelection(p), true, `${p} supports reasoning effort selection`);
  }
  for (const p of ['gemini', 'grok', 'opencode', 'shell'] as const) {
    assert.equal(supportsEffortSelection(p), false, `${p} does not expose effort selection`);
  }
});
