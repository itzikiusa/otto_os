<script lang="ts">
  // The one-time "Enable live browsing" step. Outside the desktop app a live
  // tab runs in a daemon-owned Chromium, downloaded once — never silently:
  // this pane says what is downloaded, how big it is and where it runs, and
  // nothing happens until someone clicks (api.md "Browser — remote live
  // view": POST /browser/live/install is Browser Admin, sha256-pinned).
  //
  //   missing     → EmptyState + the one CTA ("Download Chrome (180 MB)"),
  //                 with a "lighter engine" option underneath
  //   installing  → progress (role=progressbar): downloading → verifying →
  //                 extracting
  //   failed      → inline error + Retry
  //   unsupported → honest note (this Mac can't run it) + Reader
  //   no admin    → says who can enable it
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { formatBytes } from '../../../lib/metric-format';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { browserLive } from '../../../lib/stores/browserLive.svelte';
  import type { BrowserChromeBuild } from '../../../lib/api/types';

  interface Props {
    /** "Switch to Reader" — the escape hatch for this tab. */
    onreader?: () => void;
  }
  let { onreader }: Props = $props();

  let lighter = $state(false);
  let startError = $state('');

  const st = $derived(browserLive.status);
  const engine = $derived(browserLive.engineState);
  const build: BrowserChromeBuild = $derived(lighter ? 'chrome-headless-shell' : 'chrome');
  const canInstall = $derived(auth.can('browser', 'admin'));
  const pinned = (b: BrowserChromeBuild) => browserLive.build(b)?.sha256_pinned !== false;
  const lighterAvailable = $derived(pinned('chrome-headless-shell'));

  async function enable(): Promise<void> {
    startError = '';
    try {
      await browserLive.install(build);
    } catch (e) {
      startError = e instanceof Error ? e.message : String(e);
    }
  }

  const job = $derived(st?.install ?? null);
  const received = $derived(job?.received_bytes ?? 0);
  const total = $derived(job?.total_bytes ?? (job ? browserLive.downloadBytes(job.build) : 0));
  const pct = $derived(total > 0 ? Math.min(100, Math.round((received / total) * 100)) : 0);
  const phase = $derived(
    job?.state === 'verifying' ? 'Checking the download…' : job?.state === 'extracting' ? 'Unpacking…' : null,
  );
  const jobName = $derived(job?.build === 'chrome-headless-shell' ? 'the lighter Chromium engine' : 'Chrome for Testing');

  const body = $derived(
    `Live tabs run in a Chromium browser on the Mac running Otto and stream here, so they work from any device, including your phone. ` +
      `Otto downloads ${lighter ? 'the lighter Chromium engine' : 'Chrome for Testing'} once (about ${formatBytes(browserLive.downloadBytes(build))}) and checks it against a pinned checksum. ` +
      `Every request the pages make goes through Otto's network guard.`,
  );
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
  {:else if engine === 'unknown'}
    <p class="loading" role="status">Checking the live browser engine…</p>
  {:else if engine === 'installing'}
    <div class="progress-card" role="status" aria-live="polite">
      <div class="icon-tile"><Icon name="download" size={24} /></div>
      <h3>{phase ?? `Downloading ${jobName}…`}</h3>
      <div
        class="bar"
        role="progressbar"
        aria-label="Download progress"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={phase ? 100 : pct}
      >
        <span style:width="{phase ? 100 : pct}%"></span>
      </div>
      <p class="meta">
        {#if phase}{formatBytes(total)} downloaded{:else}{formatBytes(received)} of {formatBytes(total)} · {pct}%{/if}
      </p>
      <p class="hint">It downloads once and stays on this Mac. You can keep using Otto meanwhile.</p>
    </div>
  {:else if engine === 'failed'}
    <div class="inline-error" role="alert">
      <Icon name="warning" size={16} />
      <div>
        <p class="title">The live browser engine didn't install</p>
        <p class="detail">{job?.error || 'The download stopped before it finished.'}</p>
      </div>
      {#if canInstall}
        <button class="btn" disabled={browserLive.starting} onclick={() => void browserLive.install(job?.build ?? 'chrome')}>Retry</button>
      {/if}
    </div>
  {:else if engine === 'unsupported'}
    <EmptyState
      variant="page"
      icon="globe"
      title="Live browsing isn't available on this Mac"
      body="The live browser engine runs on Apple silicon Macs only for now. Reader view still works for every page."
      actionLabel={onreader ? 'Switch to Reader' : undefined}
      onaction={onreader}
    />
  {:else if !canInstall}
    <EmptyState
      variant="page"
      icon="globe"
      title="Live browsing isn't enabled yet"
      body="Live tabs need a one-time Chromium download on the Mac running Otto. Ask an Otto admin to enable it in Settings → Browser. Reader view works meanwhile."
      actionLabel={onreader ? 'Switch to Reader' : undefined}
      onaction={onreader}
    />
  {:else}
    <EmptyState
      variant="page"
      icon="globe"
      title="Enable live browsing"
      {body}
      actionLabel={browserLive.starting
        ? 'Starting the download…'
        : `${lighter ? 'Download lighter engine' : 'Download Chrome'} (${formatBytes(browserLive.downloadBytes(build))})`}
      actionIcon="download"
      onaction={pinned(build) ? () => void enable() : undefined}
    >
      {#if lighterAvailable}
        <label class="checkbox-row lighter">
          <input type="checkbox" bind:checked={lighter} />
          <span>
            Use the lighter engine instead (about {formatBytes(browserLive.downloadBytes('chrome-headless-shell'))})
            <span class="hint">Faster to download. A few sites render differently than in Chrome.</span>
          </span>
        </label>
      {/if}
      {#if !pinned(build)}
        <p class="field-error" role="alert">This Otto build has no checksum for that engine, so it can't be downloaded safely.</p>
      {/if}
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
    margin: 12px auto 8px;
    text-align: start;
    font-size: var(--fs-s);
  }
  .lighter .hint {
    display: block;
  }
  .field-error {
    color: var(--danger);
    font-size: var(--fs-s);
    margin: 8px 0;
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
