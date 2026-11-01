import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  defaultEffortForModel,
  effortsForModel,
  formatEffortLabel,
  formatModelLabel,
  modelsForProvider,
  nextInteractionModeToggle,
  resolveSessionThreadConfig,
  supportsModelSelection,
} from '../src/lib/providerModels.ts';

describe('providerModels', () => {
  it('prefers live MetaResp.provider_models and falls back cleanly when empty', () => {
    const fallback = modelsForProvider('claude', { provider_models: {} });
    assert.ok(fallback.length >= 3);
    assert.equal(fallback[0]?.id, 'opus');

    const liveMeta = {
      provider_models: {
        claude: [
          {
            id: 'claude-opus-5-5',
            label: 'Opus 5.5',
            description: 'Everyday tasks',
            default_effort: 'medium',
            supported_efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
          },
        ],
      },
    };
    const live = modelsForProvider('claude', liveMeta);
    assert.equal(live.length, 1);
    assert.equal(formatModelLabel('claude', 'claude-opus-5-5', liveMeta), 'Opus 5.5');
    assert.equal(formatModelLabel('claude', 'custom-id', liveMeta), 'custom-id');
    assert.equal(formatModelLabel('claude', null, liveMeta), 'Default model');
    assert.deepEqual(effortsForModel('claude', 'claude-opus-5-5', liveMeta), [
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ]);
    assert.equal(defaultEffortForModel('claude', 'claude-opus-5-5', liveMeta), 'medium');
  });

  it('resolves session thread config and toggles Plan <-> Full access', () => {
    const cfg = resolveSessionThreadConfig(
      { meta: { model: 'gpt-6.1-sol', effort: 'xhigh', interaction_mode: 'plan' } },
      'gpt-5.4',
    );
    assert.equal(cfg.pinnedModel, 'gpt-6.1-sol');
    assert.equal(cfg.effectiveModel, 'gpt-6.1-sol');
    assert.equal(cfg.effort, 'xhigh');
    assert.equal(cfg.interactionMode, 'plan');
    assert.equal(formatEffortLabel(cfg.effort), 'Extra High');
    assert.equal(nextInteractionModeToggle(cfg.interactionMode), 'full');
    assert.equal(nextInteractionModeToggle('full'), 'plan');

    const unpinned = resolveSessionThreadConfig({ meta: {} }, 'claude-opus-5-5');
    assert.equal(unpinned.pinnedModel, null);
    assert.equal(unpinned.effectiveModel, 'claude-opus-5-5');
    assert.equal(unpinned.interactionMode, 'full');
    assert.equal(supportsModelSelection('shell', null), false);
    assert.equal(supportsModelSelection('codex', null), true);
  });
});
