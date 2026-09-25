<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Settings → Insights: opt-in toggles for scheduled HTML insight reports.
  // All three are OFF by default. Runs are catch-up — if the app was closed at
  // the scheduled time, the report is generated the next time the app is open,
  // so a scheduled report is never silently missed.
  import { insightsApi } from '../../lib/api/insights';
  import type { InsightsConfig } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';

  let cfg: InsightsConfig | null = $state(null);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  // A provider/model change made while another save is in flight is queued
  // (merged) and sent right after — it used to be dropped while the picker
  // kept showing it, so the reports ran on the old model.
  let queuedAgent: Partial<InsightsConfig> | null = null;
  // Local model draft — the picker's free-text path fires per keystroke, so we
  // debounce the PUT instead of racing saveAgent's `saving` guard.
  let modelDraft = $state('');
  let modelSaveTimer: ReturnType<typeof setTimeout> | null = null;
  function onModelChange(m: string): void {
    modelDraft = m;
    if (modelSaveTimer) clearTimeout(modelSaveTimer);
    modelSaveTimer = setTimeout(() => void saveAgent({ model: m }), 500);
  }

  // ---------------------------------------------------------------------------
  // Load on mount
  // ---------------------------------------------------------------------------

  $effect(() => {
    void load();
  });

  let loaded = false;
  async function load(): Promise<void> {
    if (loaded) return;
    loaded = true;
    loading = true;
    loadError = '';
    try {
      cfg = await insightsApi.getConfig();
      modelDraft = cfg.model || '';
    } catch (e) {
      loaded = false; // let Retry load again
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // PUT on every toggle change (optimistic + revert on failure)
  // ---------------------------------------------------------------------------

  /** Persist a provider/model change (which agent generates the reports). */
  async function saveAgent(patch: Partial<InsightsConfig>): Promise<void> {
    if (!cfg) return;
    if (saving) {
      queuedAgent = { ...(queuedAgent ?? {}), ...patch };
      return;
    }
    const next: InsightsConfig = { ...cfg, ...patch };
    const prev = cfg;
    cfg = next;
    saving = true;
    try {
      cfg = await insightsApi.putConfig(next);
      toasts.success(
        'Report agent saved',
        `${cfg.provider || `default (${defaultAgentProvider()})`}${cfg.model ? ` · ${cfg.model}` : ''}`,
      );
    } catch (e) {
      cfg = prev;
      modelDraft = prev.model || '';
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
    if (queuedAgent) {
      const q = queuedAgent;
      queuedAgent = null;
      await saveAgent(q);
    }
  }

  async function toggle(key: keyof InsightsConfig): Promise<void> {
    if (!cfg || saving) return;
    const next: InsightsConfig = { ...cfg, [key]: !cfg[key] };
    const prev = cfg;
    cfg = next; // optimistic
    saving = true;
    try {
      cfg = await insightsApi.putConfig(next);
      toasts.success(
        'Insights schedule updated',
        `${key[0].toUpperCase()}${key.slice(1)} reports ${next[key] ? 'on' : 'off'}`,
      );
    } catch (e) {
      cfg = prev; // revert
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('insights')} subtitle="Scheduled HTML reports about your Otto activity" />
  <PageBody width="readable">
  <p class="section-intro">These are <strong>opt-in</strong> and <strong>off by default</strong> — turn on only the cadences you want. Runs are <strong>catch-up</strong>: if the app was closed at the scheduled time, the report is generated the next time the app is open, so a scheduled report is never missed.</p>

  {#if loading && !cfg}
    <Skeleton rows={3} height={64} />
  {:else if !cfg}
    <LoadState what="insights settings" error={loadError} empty onretry={() => void load()} />
  {:else}
    <div class="card toggles">
      <label class="toggle-row">
        <div class="toggle-text">
          <span class="toggle-title">Daily</span>
          <span class="toggle-desc">Covers the previous day (UTC), generated once it has ended.</span>
        </div>
        <input
          type="checkbox"
          checked={cfg.daily}
          disabled={saving}
          onchange={() => toggle('daily')}
        />
      </label>

      <label class="toggle-row">
        <div class="toggle-text">
          <span class="toggle-title">Weekly</span>
          <span class="toggle-desc">Runs on Monday (UTC), covering the previous Monday–Sunday.</span>
        </div>
        <input
          type="checkbox"
          checked={cfg.weekly}
          disabled={saving}
          onchange={() => toggle('weekly')}
        />
      </label>

      <label class="toggle-row">
        <div class="toggle-text">
          <span class="toggle-title">Monthly</span>
          <span class="toggle-desc">Runs on the 1st (UTC), covering the previous month.</span>
        </div>
        <input
          type="checkbox"
          checked={cfg.monthly}
          disabled={saving}
          onchange={() => toggle('monthly')}
        />
      </label>
    </div>

    <div class="agent-row">
      <label class="agent-fld">
        <span class="toggle-title">Provider</span>
        <select
          value={cfg.provider || ''}
          disabled={saving}
          onchange={(e) => {
            // A model belongs to its provider — drop it rather than send a
            // Claude model id to codex.
            modelDraft = '';
            void saveAgent({ provider: e.currentTarget.value, model: '' });
          }}
        >
          <option value="">default ({defaultAgentProvider()})</option>
          {#each agentProviders() as p (p)}<option value={p}>{p}</option>{/each}
        </select>
      </label>
      <!-- Catalog-backed; hides itself when the provider has no model-flag
           template. Empty provider = default → resolve for the model list. -->
      <div class="agent-fld">
        <ModelPicker
          provider={cfg.provider || defaultAgentProvider()}
          value={modelDraft}
          hint="Model the report agent runs with (blank = the provider's default)."
          onchange={onModelChange}
        />
      </div>
    </div>

    <div class="note">
      Requires the <span class="mono">insights</span> skill to be installed
      (<button class="link" onclick={() => router.go('settings/skills')}>Settings → Skills</button>).
      Generated reports appear in the
      <button class="link" onclick={() => router.go('insights')}>Insights view</button>, where you
      can also run a report on demand.
    </div>
  {/if}
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .section-intro {
    margin: 0 0 14px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .section-intro :global(code) {
    font-family: var(--font-mono);
    font-size: 11px;
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: 3px;
  }
  .toggles {
    display: flex;
    flex-direction: column;
    padding: 4px 18px;
    max-width: 560px;
  }

  .toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 0;
    cursor: pointer;
  }
  .toggle-row + .toggle-row {
    border-top: 1px solid var(--border);
  }
  .agent-row {
    display: flex;
    gap: 20px;
    padding: 14px 18px 4px;
    max-width: 560px;
    flex-wrap: wrap;
  }
  .agent-fld {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 13px;
  }
  .agent-fld select {
    padding: 5px 8px;
  }


  .toggle-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .toggle-title {
    font-size: 13px;
    font-weight: 600;
  }
  .toggle-desc {
    font-size: 11.5px;
    color: var(--text-dim);
  }

  .toggle-row input {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    cursor: pointer;
  }

  .note {
    margin-top: 14px;
    max-width: 560px;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.6;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    cursor: pointer;
    text-decoration: underline;
  }
</style>
