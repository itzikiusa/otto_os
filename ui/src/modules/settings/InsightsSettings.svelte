<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  import SectionIntro from './SectionIntro.svelte';
  import SettingToggle from './SettingToggle.svelte';
  // Settings → Insights: opt-in toggles for scheduled HTML insight reports.
  // All three are OFF by default. Runs are catch-up — if the app was closed at
  // the scheduled time, the report is generated the next time the app is open,
  // so a scheduled report is never silently missed.
  import { insightsApi } from '../../lib/api/insights';
  import type { InsightsConfig } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { contextApi } from '../../lib/api/context';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';

  let cfg: InsightsConfig | null = $state(null);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  // Quiet inline confirmation for the report-agent fields (the checkboxes
  // show their own state; toasting every change was noise).
  // One quiet "Saved" per section (a toggle or select applies immediately —
  // layout.md §4.4), next to the heading of the section that changed.
  let savedIn = $state<'schedule' | 'agent' | null>(null);
  let savedTimer: ReturnType<typeof setTimeout> | null = null;
  function flashSaved(where: 'schedule' | 'agent'): void {
    savedIn = where;
    if (savedTimer) clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (savedIn = null), 2500);
  }

  // Reports are written by the bundled `insights` skill: say whether it's
  // installed instead of only telling people it's required (a schedule with
  // the skill missing produced nothing, silently). Unknown (no access to the
  // bundled list) = say nothing extra.
  let skillState = $state<'installed' | 'missing' | null>(null);
  $effect(() => {
    void contextApi
      .listBundled()
      .then((list) => {
        const sk = list.find((x) => x.name === 'insights');
        skillState = sk ? (sk.state === 'not_installed' ? 'missing' : 'installed') : null;
      })
      .catch(() => (skillState = null));
  });
  const anyOn = $derived.by(() => {
    const c = cfg as InsightsConfig | null;
    return !!c && (c.daily || c.weekly || c.monthly);
  });
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
      flashSaved('agent');
    } catch (e) {
      cfg = prev;
      modelDraft = prev.model || '';
      toasts.error('Couldn’t save the report agent', e instanceof Error ? e.message : String(e));
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
      flashSaved('schedule');
    } catch (e) {
      cfg = prev; // revert
      toasts.error(`Couldn’t turn ${String(key)} reports ${next[key] ? 'on' : 'off'}`, e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('insights')} subtitle="Scheduled HTML reports about your Otto activity" />
  <PageBody width="readable">
  <SectionIntro>These are <strong>opt-in</strong> and <strong>off by default</strong> — turn on only the cadences you want. Runs are <strong>catch-up</strong>: if the app was closed at the scheduled time, the report is generated the next time the app is open, so a scheduled report is never missed.</SectionIntro>

  {#if !cfg}
    <LoadState what="insights settings" loading={loading || !loadError} error={loadError || null} empty rows={3} onretry={() => void load()} />
  {:else}
    {#if skillState === 'missing'}
      <p class="skill-warn" role={anyOn ? 'alert' : undefined}>
        <Icon name="warning" size={12} />
        <span>The <span class="mono">insights</span> skill isn't installed, so {anyOn ? 'scheduled reports can’t be generated' : 'reports can’t be generated yet'}.</span>
        <button class="btn small" onclick={() => router.go('settings/skills')}>Open Skills</button>
      </p>
    {/if}
    <h2 class="section-title">Schedule {#if savedIn === 'schedule'}<span class="saved" role="status">Saved</span>{/if}</h2>
    <div class="card toggles">
      <SettingToggle label="Daily" hint="Covers the previous day (UTC), generated once it has ended." checked={cfg.daily} disabled={saving} onchange={() => toggle('daily')} />
      <SettingToggle label="Weekly" hint="Runs on Monday (UTC), covering the previous Monday–Sunday." checked={cfg.weekly} disabled={saving} onchange={() => toggle('weekly')} />
      <SettingToggle label="Monthly" hint="Runs on the 1st (UTC), covering the previous month." checked={cfg.monthly} disabled={saving} onchange={() => toggle('monthly')} />
    </div>

    <h2 class="section-title">Report agent {#if savedIn === 'agent'}<span class="saved" role="status">Saved</span>{/if}</h2>
    <div class="card agent-row">
      <div class="field agent-fld">
        <label for="ins-provider">Provider</label>
        <select
          id="ins-provider"
          class="input"
          value={cfg.provider || ''}
          disabled={saving}
          onchange={(e) => {
            // A model belongs to its provider — drop it rather than send a
            // Claude model id to codex.
            modelDraft = '';
            void saveAgent({ provider: e.currentTarget.value, model: '' });
          }}
        >
          <option value="">Default ({defaultAgentProvider()})</option>
          {#each agentProviders() as p (p)}<option value={p}>{p}</option>{/each}
        </select>
      </div>
      <!-- Catalog-backed; hides itself when the provider has no model-flag
           template. Empty provider = default → resolve for the model list. -->
      <div class="agent-fld model">
        <ModelPicker
          provider={cfg.provider || defaultAgentProvider()}
          value={modelDraft}
          hint="Model the report agent runs with (blank = the provider's default)."
          onchange={onModelChange}
        />
      </div>
    </div>

    <p class="note">
      {#if skillState === 'installed'}
        Written by the installed <span class="mono">insights</span> skill.
      {:else if skillState === null}
        Requires the <span class="mono">insights</span> skill to be installed
        (<button class="link" onclick={() => router.go('settings/skills')}>Settings → Skills</button>).
      {/if}
      Generated reports appear in
      <button class="link" onclick={() => router.go('insights')}>Insights</button>, where you
      can also run a report on demand.
    </p>
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
  .section-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .saved {
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
    color: var(--success);
  }
  .toggles {
    display: flex;
    flex-direction: column;
    padding: 4px 16px;
    max-width: var(--settings-col);
  }
  /* Rows stacked in one card read as a list. */
  .toggles > :global(.st + .st) {
    border-top: 1px solid var(--border);
  }
  .agent-row {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    padding: 12px 16px;
    max-width: var(--settings-col);
    flex-wrap: wrap;
  }
  .agent-fld {
    margin: 0;
    min-width: 180px;
  }
  .agent-fld.model {
    flex: 1;
    min-width: 240px;
  }
  .note {
    margin: 14px 0 0;
    max-width: var(--settings-col);
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.6;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .skill-warn {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 14px;
    padding: 8px 12px;
    max-width: var(--settings-col);
    box-sizing: border-box;
    border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    font-size: var(--fs-s);
  }
  .skill-warn > :global(svg) {
    flex-shrink: 0;
    color: var(--warning);
  }
  .skill-warn > span {
    flex: 1;
    min-width: 0;
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
