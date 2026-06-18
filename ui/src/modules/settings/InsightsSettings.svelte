<script lang="ts">
  // Settings → Insights: opt-in toggles for scheduled HTML insight reports.
  // All three are OFF by default. Runs are catch-up — if the app was closed at
  // the scheduled time, the report is generated the next time the app is open,
  // so a scheduled report is never silently missed.
  import { insightsApi } from '../../lib/api/insights';
  import type { InsightsConfig } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  let cfg: InsightsConfig | null = $state(null);
  let loading = $state(true);
  let saving = $state(false);

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
    try {
      cfg = await insightsApi.getConfig();
    } catch (e) {
      toasts.error('Could not load insights settings', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // PUT on every toggle change (optimistic + revert on failure)
  // ---------------------------------------------------------------------------

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

<div class="page">
  <div class="page-header">
    <div>
      <h1>Insights</h1>
      <div class="sub">
        Scheduled HTML insight reports about your Otto activity. These are
        <strong>opt-in</strong> and <strong>off by default</strong> — turn on only the cadences you
        want. Runs are <strong>catch-up</strong>: if the app was closed at the scheduled time, the
        report is generated the next time the app is open, so a scheduled report is never missed.
      </div>
    </div>
  </div>

  {#if loading && !cfg}
    <Skeleton rows={3} height={64} />
  {:else if cfg}
    <div class="card toggles">
      <label class="toggle-row">
        <div class="toggle-text">
          <span class="toggle-title">Daily</span>
          <span class="toggle-desc">Covers the previous day, generated the next morning.</span>
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
          <span class="toggle-desc">Runs on Sunday, covering the previous week.</span>
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
          <span class="toggle-desc">Runs on the 1st, covering the previous month.</span>
        </div>
        <input
          type="checkbox"
          checked={cfg.monthly}
          disabled={saving}
          onchange={() => toggle('monthly')}
        />
      </label>
    </div>

    <div class="note">
      Requires the <span class="mono">insights</span> skill to be installed
      (<button class="link" onclick={() => router.go('settings/skills')}>Settings → Skills</button>).
      Generated reports appear in the
      <button class="link" onclick={() => router.go('insights')}>Insights view</button>, where you
      can also run a report on demand.
    </div>
  {/if}
</div>

<style>
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
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
  }
</style>
