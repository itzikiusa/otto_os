import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  canonicalModelId,
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
          {
            id: 'claude-haiku-4-5-20251001',
            label: 'Haiku 4.5',
            description: 'Fast',
            default_effort: null,
            supported_efforts: [],
          },
        ],
      },
    };
    const live = modelsForProvider('claude', liveMeta);
    assert.equal(live.length, 2);
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
    assert.deepEqual(effortsForModel('claude', 'claude-haiku-4-5-20251001', liveMeta), []);
    assert.equal(defaultEffortForModel('claude', 'claude-opus-5-5', liveMeta), 'medium');
  });

  it('unifies tier-suffixed Antigravity models into clean families with exact reasoning levels', () => {
    const legacyMeta = {
      provider_models: {
        agy: [
          {
            id: 'gemini-4-carbon-medium',
            label: 'Gemini 4 Carbon (Medium)',
            description: '',
            default_effort: 'medium',
            supported_efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
          },
          {
            id: 'gemini-4-carbon-high',
            label: 'Gemini 4 Carbon (High)',
            description: '',
            default_effort: 'high',
            supported_efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
          },
          {
            id: 'gemini-4-carbon-low',
            label: 'Gemini 4 Carbon (Low)',
            description: '',
            default_effort: 'low',
            supported_efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
          },
          {
            id: 'gemini-4-hydrogen-high',
            label: 'Gemini 4 Hydrogen (High)',
            description: '',
            default_effort: 'high',
            supported_efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
          },
          {
            id: 'gemini-4-hydrogen-xhigh',
            label: 'Gemini 4 Hydrogen (XHigh)',
            description: '',
            default_effort: 'xhigh',
            supported_efforts: ['low', 'medium', 'high', 'xhigh', 'max'],
          },
          {
            id: 'gemini-pro-orchestrator',
            label: 'Orchestrator (Carbon)',
            description: '',
            default_effort: null,
            supported_efforts: [],
          },
          {
            id: 'gpt-oss-120b-medium',
            label: 'GPT-OSS 120B (Medium)',
            description: '',
            default_effort: 'medium',
            supported_efforts: ['medium'],
          },
        ],
      },
    };

    const agy = modelsForProvider('agy', legacyMeta);
    assert.deepEqual(
      agy.map((m) => ({ id: m.id, label: m.label, efforts: m.supported_efforts })),
      [
        { id: 'gemini-4-carbon', label: 'Gemini 4 Carbon', efforts: ['low', 'medium', 'high'] },
        { id: 'gemini-4-hydrogen', label: 'Gemini 4 Hydrogen', efforts: ['high', 'xhigh'] },
        { id: 'gemini-pro-orchestrator', label: 'Orchestrator (Carbon)', efforts: [] },
        { id: 'gpt-oss-120b-medium', label: 'GPT-OSS 120B', efforts: ['medium'] },
      ],
    );

    // Saved legacy variant IDs seamlessly resolve to the unified family + clean label.
    assert.equal(canonicalModelId('agy', 'gemini-4-carbon-high', legacyMeta), 'gemini-4-carbon');
    assert.equal(formatModelLabel('agy', 'gemini-4-carbon-high', legacyMeta), 'Gemini 4 Carbon');
    assert.deepEqual(effortsForModel('agy', 'gemini-4-carbon-high', legacyMeta), [
      'low',
      'medium',
      'high',
    ]);
    assert.deepEqual(effortsForModel('agy', 'gemini-pro-orchestrator', legacyMeta), []);

    const legacySession = resolveSessionThreadConfig({
      provider: 'agy',
      meta: { model: 'gemini-4-carbon-high' },
    });
    assert.equal(legacySession.effort, 'high');
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
