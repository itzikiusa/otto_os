<script lang="ts">
  // Capability & Health Registry (B3) — one page answering "what can Otto do
  // right now, what's degraded, and how do I fix it?". Root-only: backed by
  // GET /capabilities (5 s cached on the server) and GET /support-bundle.
  import { capabilitiesApi, featureLabel, settingsRoute, statusClass, statusLabel } from './capabilities';
  import type { ModuleCapability, SupportBundle } from './capabilities';
  import { router } from '../../lib/router.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { downloadJson } from '../../lib/components/exporters';
  import { toasts } from '../../lib/toast.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let caps: ModuleCapability[] = $state([]);
  let loading = $state(true);
  let bundleLoading = $state(false);
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
    try {
      caps = await capabilitiesApi.list();
    } catch (e) {
      toasts.error('Could not load capabilities', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Support bundle download
  // ---------------------------------------------------------------------------

  async function downloadBundle(): Promise<void> {
    if (bundleLoading) return;
    bundleLoading = true;
    try {
      const bundle: SupportBundle = await capabilitiesApi.bundle();
      const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
      downloadJson(bundle, `otto-support-bundle-${ts}.json`);
      toasts.success(
        'Support bundle downloaded',
        `${bundle.redaction_hits} secret value${bundle.redaction_hits !== 1 ? 's' : ''} redacted.`,
      );
    } catch (e) {
      toasts.error('Bundle download failed', e instanceof Error ? e.message : String(e));
    } finally {
      bundleLoading = false;
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

<div class="page">
  <div class="page-header head-row">
    <div>
      <h1>Capability & Health</h1>
      <div class="sub">
        What Otto can do right now — aggregated from config, PATH detection, and
        stored accounts. Results are cached for a few seconds; reload to refresh.
      </div>
    </div>
    <button class="btn" onclick={downloadBundle} disabled={bundleLoading} title="Download a redacted support bundle (secrets stripped)">
      <Icon name="fetch" size={13} />
      {bundleLoading ? 'Preparing…' : 'Download support bundle'}
    </button>
  </div>

  {#if !loading && caps.length > 0}
    <!-- Summary chips -->
    <div class="summary-row">
      {#if summary.ready > 0}
        <span class="chip chip-green">{summary.ready} ready</span>
      {/if}
      {#if summary.degraded > 0}
        <span class="chip chip-yellow">{summary.degraded} degraded</span>
      {/if}
      {#if summary.missing > 0}
        <span class="chip chip-gray">{summary.missing} not set up</span>
      {/if}
    </div>
  {/if}

  {#if loading}
    <Skeleton rows={5} height={68} />
  {:else if caps.length === 0}
    <EmptyState
      icon="gauge"
      title="No capability data"
      body="Could not load capability information. Make sure you are logged in as root."
      actionLabel="Retry"
      onaction={load}
    />
  {:else}
    <div class="cap-list">
      {#each sorted as cap (cap.feature)}
        {@const open = expanded.has(cap.feature)}
        {@const cls = statusClass(cap.status)}
        <div class="cap-card card" class:has-issues={cap.status !== 'ready'}>
          <!-- Header row -->
          <div class="cap-head" role="button" tabindex="0"
               onclick={() => toggle(cap.feature)}
               onkeydown={(e) => e.key === 'Enter' && toggle(cap.feature)}>
            <span class="status-dot {cls}" title={statusLabel(cap.status)}></span>
            <span class="feature-label">{featureLabel(cap.feature)}</span>
            <span class="status-badge badge-{cls}">{statusLabel(cap.status)}</span>
            <span class="dep-count dim">
              {cap.deps.length} dep{cap.deps.length !== 1 ? 's' : ''}
            </span>
            {#if cap.status !== 'ready'}
              <!-- Quick link to the relevant settings surface -->
              <button class="btn-sm" title="Open settings for this feature"
                      onclick={(e) => { e.stopPropagation(); router.go(settingsRoute(cap.feature)); }}>
                <Icon name="gear" size={12} />
                Fix
              </button>
            {/if}
            <Icon name={open ? 'arrowUp' : 'arrowDown'} size={14} />
          </div>

          <!-- Issues (reasons + fixes) -->
          {#if cap.reasons.length > 0}
            <div class="cap-issues">
              {#each cap.reasons as reason, i (reason)}
                <div class="issue-row">
                  <Icon name="zap" size={13} />
                  <span class="issue-reason">{reason}</span>
                  {#if cap.fixes[i]}
                    <span class="issue-fix dim">{cap.fixes[i]}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}

          <!-- Expanded dep breakdown -->
          {#if open && cap.deps.length > 0}
            <div class="dep-list">
              {#each cap.deps as dep (dep.name + dep.kind)}
                <div class="dep-row">
                  <span class="dep-ok" title={dep.ok ? 'OK' : 'Not OK'}>
                    {#if dep.ok}
                      <Icon name="check" size={12} />
                    {:else}
                      <Icon name="x" size={12} />
                    {/if}
                  </span>
                  <span class="dep-kind dim">{dep.kind}</span>
                  <span class="dep-name">{dep.name}</span>
                  {#if dep.detail}
                    <span class="dep-detail dim">{dep.detail}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .page { padding: 24px; max-width: 900px; }

  .page-header { margin-bottom: 20px; }
  .head-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; }

  h1 { margin: 0 0 4px; font-size: 20px; font-weight: 600; }
  .sub { color: var(--text-muted); font-size: 13px; max-width: 560px; }

  /* summary chips */
  .summary-row { display: flex; gap: 8px; margin-bottom: 16px; flex-wrap: wrap; }
  .chip { padding: 3px 10px; border-radius: 99px; font-size: 12px; font-weight: 500; }
  .chip-green  { background: var(--color-success-bg, #d1fae5); color: var(--color-success, #065f46); }
  .chip-yellow { background: var(--color-warn-bg, #fef3c7);    color: var(--color-warn,    #92400e); }
  .chip-gray   { background: var(--bg-muted, #f1f5f9);         color: var(--text-muted);             }

  /* capability list */
  .cap-list { display: flex; flex-direction: column; gap: 8px; }

  .cap-card { border-radius: 8px; overflow: hidden; }
  .cap-card.has-issues { border-inline-start: 3px solid var(--color-warn, #f59e0b); }

  .cap-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    cursor: pointer;
    user-select: none;
  }
  .cap-head:hover { background: var(--bg-hover, rgba(0,0,0,.04)); }

  .feature-label { font-weight: 500; font-size: 14px; flex: 1; }
  .dep-count { font-size: 12px; }

  /* status dot (reuse global .status-dot colours if present, else inline) */
  .status-dot {
    width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0;
  }
  .status-dot.green  { background: var(--color-success, #10b981); }
  .status-dot.yellow { background: var(--color-warn, #f59e0b); }
  .status-dot.gray   { background: var(--text-muted); }

  /* status badge */
  .status-badge { font-size: 11px; padding: 2px 8px; border-radius: 99px; }
  .badge-green  { background: var(--color-success-bg, #d1fae5); color: var(--color-success, #065f46); }
  .badge-yellow { background: var(--color-warn-bg, #fef3c7);    color: var(--color-warn,    #92400e); }
  .badge-gray   { background: var(--bg-muted, #f1f5f9);         color: var(--text-muted); }

  /* quick-fix button */
  .btn-sm {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 2px 8px; border-radius: 4px; font-size: 12px;
    background: transparent; border: 1px solid var(--border, #e2e8f0);
    cursor: pointer; color: var(--text-muted);
  }
  .btn-sm:hover { background: var(--bg-hover, rgba(0,0,0,.04)); }

  /* issues (reasons + fixes) */
  .cap-issues { padding: 0 14px 10px; display: flex; flex-direction: column; gap: 6px; }
  .issue-row  { display: flex; align-items: flex-start; gap: 8px; font-size: 13px; flex-wrap: wrap; }
  .issue-reason { font-weight: 500; }
  .issue-fix  { font-size: 12px; }

  /* dep breakdown */
  .dep-list { border-top: 1px solid var(--border, #e2e8f0); padding: 8px 14px; }
  .dep-row  {
    display: flex; align-items: center; gap: 8px;
    padding: 3px 0; font-size: 12px;
  }
  .dep-ok  { display: flex; align-items: center; flex-shrink: 0; }
  .dep-kind  { text-transform: uppercase; font-size: 10px; letter-spacing: .04em; width: 56px; flex-shrink: 0; }
  .dep-name  { font-weight: 500; }
  .dep-detail { font-size: 11px; }

  .dim { color: var(--text-muted); }
</style>
