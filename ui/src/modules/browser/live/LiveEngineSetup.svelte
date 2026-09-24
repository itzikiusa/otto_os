<script lang="ts">
  // The one-time "Enable live browsing" step. Otto's live tabs outside the
  // desktop app run in a daemon-managed Chromium, which is downloaded once —
  // never silently: this pane says what is downloaded, how big it is, and
  // where it runs, and nothing happens until the user clicks.
  //
  //   missing     → EmptyState + the one CTA ("Download Chrome · 150 MB"),
  //                 with a "lighter engine" option underneath
  //   installing  → progress (role=progressbar), received / total
  //   failed      → inline error + Retry
  //   unsupported → honest note (this Mac/daemon can't run it)
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { formatBytes } from '../../../lib/metric-format';
  import { browserLive } from '../../../lib/stores/browserLive.svelte';
  import { DEFAULT_DOWNLOAD_BYTES, type LiveEngineKind } from '../../../lib/api/browserLive';

  interface Props {
    /** "Switch to Reader" — the escape hatch for this tab. */
    onreader?: () => void;
  }
  let { onreader }: Props = $props();

  let lighter = $state(false);
  let startError = $state('');

  const st = $derived(browserLive.status);
  const kind: LiveEngineKind = $derived(lighter ? 'headless_shell' : 'chrome');
  const sizeOf = (k: LiveEngineKind) => st?.download_bytes?.[k] ?? DEFAULT_DOWNLOAD_BYTES[k];
  const approx = (n: number) => `about ${formatBytes(n)}`;
  const engineName = (k: LiveEngineKind | null | undefined) =>
    k === 'headless_shell' ? 'the lighter Chromium engine' : 'Chrome for Testing';

  async function enable(): Promise<void> {
    startError = '';
    try {
      await browserLive.install(kind);
    } catch (e) {
      startError = e instanceof Error ? e.message : String(e);
    }
  }

  const received = $derived(st?.progress?.received_bytes ?? 0);
  const total = $derived(st?.progress?.total_bytes ?? sizeOf(st?.engine ?? kind));
  const pct = $derived(total > 0 ? Math.min(100, Math.round((received / total) * 100)) : 0);
</script>

<div class="setup" data-testid="live-engine-setup">
  {#if browserLive.loadError && !st}
    <div class="inline-error" role="alert">
      <Icon name="warning" size={16} />
      <div>
        <p class="title">Couldn't check the live browser engine</p>
        <p class="detail">{browserLive.loadError}</p>
      </div>
      <button class="btn" onclick={() => void browserLive.load()}>Retry</button>
    </div>
  {:else if !st}
    <p class="loading" role="status">Checking the live browser engine…</p>
  {:else if st.state === 'installing'}
    <div class="progress-card" role="status" aria-live="polite">
      <div class="icon-tile"><Icon name="download" size={24} /></div>
      <h3>Downloading {engineName(st.engine)}…</h3>
      <div
        class="bar"
        role="progressbar"
        aria-label="Download progress"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={pct}
      >
        <span style:width="{pct}%"></span>
      </div>
      <p class="meta">
        {formatBytes(received)} of {formatBytes(total)} · {pct}%
      </p>
      <p class="hint">It downloads once and stays on this Mac. You can keep using Otto meanwhile.</p>
    </div>
  {:else if st.state === 'failed'}
    <div class="inline-error" role="alert">
      <Icon name="warning" size={16} />
      <div>
        <p class="title">The live browser engine didn't finish downloading</p>
        <p class="detail">{st.error || 'The download stopped before it completed.'}</p>
      </div>
      <button class="btn" disabled={browserLive.installing} onclick={() => void enable()}>Retry</button>
    </div>
  {:else if st.state === 'unsupported'}
    <EmptyState
      variant="page"
      icon="globe"
      title="Live browsing isn't available here"
      body={st.error || "This Otto daemon can't run the live browser engine. Reader view still works."}
      actionLabel={onreader ? 'Switch to Reader' : undefined}
      onaction={onreader}
    />
  {:else}
    <EmptyState
      variant="page"
      icon="globe"
      title="Enable live browsing"
      body={`Live tabs run in a Chromium browser on this Mac and stream here, so they work from any device — including your phone. Otto downloads ${engineName(kind)} once (${approx(sizeOf(kind))}). Pages load through Otto's network guard.`}
      actionLabel={browserLive.installing
        ? 'Starting download…'
        : `${lighter ? 'Download lighter engine' : 'Download Chrome'} (${formatBytes(sizeOf(kind))})`}
      actionIcon="download"
      onaction={() => void enable()}
    >
      <label class="checkbox-row lighter">
        <input type="checkbox" bind:checked={lighter} />
        <span>
          Use the lighter engine ({approx(sizeOf('headless_shell'))})
          <span class="hint">Faster to download; a few sites render differently than in Chrome.</span>
        </span>
      </label>
      {#if startError}
        <p class="field-error" role="alert">Couldn't start the download: {startError}</p>
      {/if}
      {#if onreader}
        <button class="btn ghost" onclick={onreader}>Use Reader view instead</button>
      {/if}
    </EmptyState>
  {/if}
</div>

<style>
  .setup {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    padding: 0 16px;
  }
  .lighter {
    max-width: 420px;
    margin: 12px auto 0;
    text-align: start;
    font-size: var(--fs-s);
  }
  .lighter .hint {
    display: block;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .field-error {
    color: var(--danger);
    font-size: var(--fs-s);
    margin: 8px 0 0;
  }
  .loading {
    margin: 15vh auto 0;
    text-align: center;
    color: var(--text-dim);
  }
  .progress-card {
    max-width: 420px;
    margin: 15vh auto 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    text-align: center;
  }
  .icon-tile {
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: var(--radius-l);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .progress-card h3 {
    margin: 4px 0 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .bar {
    width: 100%;
    height: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 200ms ease-out;
  }
  .meta {
    margin: 0;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .inline-error {
    max-width: 560px;
    margin: 15vh auto 0;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .inline-error > :global(svg) {
    color: var(--danger);
    flex: none;
    margin-top: 2px;
  }
  .inline-error > div {
    flex: 1;
    min-width: 0;
  }
  .inline-error .title {
    margin: 0;
    color: var(--text);
  }
  .inline-error .detail {
    margin: 2px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  @media (prefers-reduced-motion: reduce) {
    .bar span {
      transition: none;
    }
  }
</style>
