<script lang="ts">
  // Capability & Health Registry (B3) — one page answering "what can Otto do
  // right now, what's degraded, and how do I fix it?". Root-only: backed by
  // GET /capabilities (5 s cached on the server) and GET /support-bundle.
  import { capabilitiesApi, featureLabel, settingsRoute, statusLabel } from './capabilities';
  import type { ModuleCapability } from './capabilities';
  import { router } from '../../lib/router.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { CapabilityStatus } from './capabilities';

  /** Capability status → the shared status tone (green / amber / neutral). */
  function tone(s: CapabilityStatus): 'success' | 'warning' | 'neutral' {
    return s === 'ready' ? 'success' : s === 'degraded' ? 'warning' : 'neutral';
  }

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let caps: ModuleCapability[] = $state([]);
  let loading = $state(true);
  /** Last load failure — shown inline (with Retry) instead of a toast plus a
   *  generic "no capability data" page that hid the actual reason. */
  let loadErr = $state('');
  /** Which feature is expanded (showing dep breakdown). */
  let expanded = $state<Set<string>>(new Set());

  // ---------------------------------------------------------------------------
  // Load on mount
  // ---------------------------------------------------------------------------

  let loaded = false;
  $effect(() => {
    if (loaded) return;
    loaded = true;
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadErr = '';
    try {
      caps = await capabilitiesApi.list();
    } catch (e) {
      loadErr = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // UI helpers
  // ---------------------------------------------------------------------------

  function toggle(feature: string): void {
    const next = new Set(expanded);
    if (next.has(feature)) next.delete(feature);
    else next.add(feature);
    expanded = next;
  }

  /** Count of ready / degraded / missing_setup modules. */
  const summary = $derived({
    ready: caps.filter((c) => c.status === 'ready').length,
    degraded: caps.filter((c) => c.status === 'degraded').length,
    missing: caps.filter((c) => c.status === 'missing_setup').length,
  });

  /** Sort: degraded first, then missing_setup, then ready. */
  const sorted = $derived(
    [...caps].sort((a, b) => {
      const order = { degraded: 0, missing_setup: 1, ready: 2 };
      return (order[a.status] ?? 99) - (order[b.status] ?? 99);
    }),
  );
</script>

<!-- Body of Insights → Health; the page header (title, support-bundle
     action) lives in InsightsPage. -->
<div class="caps">

  {#if !loading && caps.length > 0}
    <!-- Summary: counts by state, worst first. -->
    <div class="summary-row" role="status">
      {#if summary.degraded > 0}
        <StatusBadge tone="warning" label="{summary.degraded} degraded" />
      {/if}
      {#if summary.missing > 0}
        <StatusBadge tone="neutral" label="{summary.missing} not set up" />
      {/if}
      {#if summary.ready > 0}
        <StatusBadge tone="success" label="{summary.ready} ready" />
      {/if}
    </div>
  {/if}

  <LoadState what="capabilities" variant="page" {loading} error={loadErr || null} empty={caps.length === 0} onretry={load} rows={5}>
    {#snippet emptyView()}
      <EmptyState
        icon="gauge"
        variant="page"
        title="No capability data"
        body="The daemon reported no capability information. The Health view needs the root account."
      />
    {/snippet}
    <div class="cap-list">
      {#each sorted as cap (cap.feature)}
        {@const open = expanded.has(cap.feature)}
        <div class="cap-card" class:has-issues={cap.status === 'degraded'}>
          <div class="cap-head">
            <button class="cap-toggle" aria-expanded={open} aria-controls="deps-{cap.feature}" onclick={() => toggle(cap.feature)}>
              <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
              <span class="feature-label" title={featureLabel(cap.feature)}>{featureLabel(cap.feature)}</span>
              <StatusBadge tone={tone(cap.status)} label={statusLabel(cap.status)} />
              {#if cap.deps.length > 0}
                <span class="dep-count dim">
                  {cap.deps.length} {cap.deps.length !== 1 ? 'checks' : 'check'}
                </span>
              {/if}
            </button>
            <!-- A fixed-width slot, so every row's badge lines up whether or not
                 it has a Fix link. -->
            <span class="fix-slot">
              {#if cap.status !== 'ready'}
                <!-- Quick link to the relevant settings surface -->
                <button class="btn small" title="Open the settings that fix {featureLabel(cap.feature)}"
                        onclick={() => router.go(settingsRoute(cap.feature))}>
                  <Icon name="gear" size={12} />
                  Fix in Settings
                </button>
              {/if}
            </span>
          </div>

          <!-- Issues (reasons + fixes) -->
          {#if cap.reasons.length > 0}
            <ul class="cap-issues">
              {#each cap.reasons as reason, i (reason)}
                <li class="issue-row">
                  <Icon name="warning" size={13} />
                  <div class="issue-text">
                    <span class="issue-reason">{reason}</span>
                    {#if cap.fixes[i]}
                      <span class="issue-fix dim">{cap.fixes[i]}</span>
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
          {/if}

          <!-- Expanded check breakdown -->
          {#if open && cap.deps.length > 0}
            <ul class="dep-list" id="deps-{cap.feature}">
              {#each cap.deps as dep (dep.name + dep.kind)}
                <li class="dep-row">
                  <span class="dep-ok" class:bad={!dep.ok} role="img" aria-label={dep.ok ? 'OK' : 'Not OK'} title={dep.ok ? 'OK' : 'Not OK'}>
                    <Icon name={dep.ok ? 'check' : 'x'} size={12} />
                  </span>
                  <span class="dep-kind dim">{dep.kind}</span>
                  <span class="dep-name">{dep.name}</span>
                  {#if dep.detail}
                    <span class="dep-detail dim" title={dep.detail}>{dep.detail}</span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/each}
    </div>
  </LoadState>
</div>

<style>
  /* summary */
  .summary-row { display: flex; gap: 8px; margin-bottom: 16px; flex-wrap: wrap; }

  /* capability list */
  .cap-list { display: flex; flex-direction: column; gap: 8px; }

  .cap-card {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .cap-card.has-issues { border-inline-start: 3px solid var(--warning); }

  .cap-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-inline-end: 12px;
  }
  .cap-head:hover { background: var(--hover); }
  .fix-slot { display: flex; justify-content: flex-end; min-width: 124px; flex: none; }
  @media (max-width: 640px) { .fix-slot { min-width: 0; } }
  .cap-toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    background: none;
    border: none;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .cap-toggle :global(svg) { color: var(--text-dim); flex-shrink: 0; }
  .cap-toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; border-radius: var(--radius-m); }

  .feature-label {
    font-weight: 500;
    font-size: var(--fs-m);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dep-count { font-size: var(--fs-s); flex-shrink: 0; }

  /* issues (reasons + fixes) */
  .cap-issues { list-style: none; margin: 0; padding-block: 0 12px; padding-inline: 36px 14px; display: flex; flex-direction: column; gap: 6px; }
  .issue-row  { display: flex; align-items: flex-start; gap: 8px; font-size: var(--fs-m); }
  .issue-row > :global(svg) { color: var(--warning); flex-shrink: 0; margin-block-start: 2px; }
  .issue-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .issue-reason { font-weight: 500; }
  .issue-fix  { font-size: var(--fs-s); }

  /* check breakdown */
  .dep-list { list-style: none; margin: 0; border-block-start: 1px solid var(--border); padding-block: 8px; padding-inline: 36px 14px; }
  .dep-row  {
    display: flex; align-items: center; gap: 8px;
    padding: 3px 0; font-size: var(--fs-s); min-width: 0;
  }
  .dep-ok  { display: flex; align-items: center; flex-shrink: 0; color: var(--success); }
  .dep-ok.bad { color: var(--danger); }
  .dep-kind  { text-transform: uppercase; font-size: var(--fs-xs); letter-spacing: .04em; width: 56px; flex-shrink: 0; }
  .dep-name  { font-weight: 500; flex-shrink: 0; }
  .dep-detail { font-size: var(--fs-xs); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .dim { color: var(--text-dim); }
</style>
