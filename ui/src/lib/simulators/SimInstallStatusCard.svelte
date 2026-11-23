<script lang="ts">
  // Shared live progress & log view for Otto's built-in Android Emulator and
  // iOS Simulator Runtime installer. Used by Browser and IDE.
  import { announceShown } from '../announce.svelte';
  import Badge from '../components/Badge.svelte';
  import Icon from '../components/Icon.svelte';
  import type { SimInstallStage, SimInstallStatus, SimInstallTarget } from '../api/types';

  interface Props {
    status: SimInstallStatus | null;
    busy?: boolean;
    oncancel: () => void;
    ondismiss?: () => void;
    onrefresh?: () => void;
    testid?: string;
  }

  let {
    status,
    busy = false,
    oncancel,
    ondismiss,
    onrefresh,
    testid = 'sim-install-card',
  }: Props = $props();

  let showLogs = $state(false);

  const TARGET_LABEL: Record<SimInstallTarget, string> = {
    android_emulator: 'Android Emulator & SDK',
    android_platform_tools: 'Android Platform Tools',
    ios_runtime: 'iOS Simulator Runtime',
  };

  const STAGE_LABEL: Record<SimInstallStage, string> = {
    idle: 'Idle',
    downloading: 'Downloading',
    extracting: 'Extracting',
    licensing: 'Accepting licenses',
    installing_packages: 'Installing packages',
    creating_avds: 'Creating AVDs',
    done: 'Installed',
    failed: 'Failed',
    cancelled: 'Canceled',
  };

  function stageTone(stage: SimInstallStage): 'ok' | 'info' | 'warn' | 'bad' | 'neutral' {
    if (stage === 'done') return 'ok';
    if (stage === 'failed') return 'bad';
    if (stage === 'cancelled') return 'warn';
    if (stage === 'idle') return 'neutral';
    return 'info';
  }

  const visible = $derived(Boolean(status && (status.running || status.stage !== 'idle')));
  const title = $derived(status?.target ? TARGET_LABEL[status.target] : 'Simulator installer');
  const stageText = $derived(status ? (STAGE_LABEL[status.stage] ?? status.stage) : 'Idle');
  const pct = $derived(
    status?.progress_pct !== null && status?.progress_pct !== undefined
      ? Math.max(0, Math.min(100, Math.round(status.progress_pct)))
      : null,
  );
  const isDone = $derived(!!status && !status.running && status.stage === 'done');
  const isFailed = $derived(!!status && !status.running && status.stage === 'failed');
</script>

{#if visible && status}
  <div
    class="sinst"
    class:done={isDone}
    class:failed={isFailed}
    data-testid={testid}
    use:announceShown={`${title}: ${stageText}`}
  >
    <div class="sinst-head">
      <span class="sinst-title">{title}</span>
      <Badge
        tone={stageTone(status.stage)}
        dot={status.running}
        live={status.running}
        label={stageText}
      />
      {#if pct !== null && status.running}
        <span class="sinst-pct">{pct}%</span>
      {/if}
      <div class="sinst-acts">
        {#if status.log_tail.length > 0}
          <button
            type="button"
            class="btn small ghost"
            onclick={() => (showLogs = !showLogs)}
            aria-expanded={showLogs}
          >
            <Icon name="terminal" size={12} />
            {showLogs ? 'Hide logs' : `Logs (${status.log_tail.length})`}
          </button>
        {/if}
        {#if status.running}
          <button
            type="button"
            class="btn small"
            onclick={oncancel}
            disabled={busy}
            data-testid="sim-install-cancel"
          >
            <Icon name="stop" size={12} /> {busy ? 'Canceling…' : 'Cancel'}
          </button>
        {:else}
          {#if onrefresh}
            <button type="button" class="btn small ghost" onclick={onrefresh}>
              <Icon name="refresh" size={12} /> Refresh
            </button>
          {/if}
          {#if ondismiss}
            <button
              type="button"
              class="icon-btn sinst-x"
              onclick={ondismiss}
              aria-label="Dismiss install status"
              title="Dismiss"
            >
              <Icon name="x" size={12} />
            </button>
          {/if}
        {/if}
      </div>
    </div>

    {#if status.running}
      <div
        class="sinst-bar"
        role="progressbar"
        aria-label={title}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct ?? undefined}
      >
        <div
          class="sinst-fill"
          class:indet={pct === null}
          style={pct !== null ? `width:${pct}%` : undefined}
        ></div>
      </div>
    {/if}

    {#if status.message}
      <p class="sinst-msg">{status.message}</p>
    {/if}

    {#if status.error}
      <p class="sinst-err">{status.error}</p>
    {/if}

    {#if showLogs && status.log_tail.length > 0}
      <pre class="sinst-logs scroll-thin" dir="ltr">{status.log_tail.join('\n')}</pre>
    {/if}
  </div>
{/if}

<style>
  .sinst {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    min-width: 0;
  }
  .sinst.done {
    border-color: color-mix(in srgb, var(--success) 40%, var(--border));
  }
  .sinst.failed {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }
  .sinst-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
  }
  .sinst-title {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .sinst-pct {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .sinst-acts {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .sinst-x {
    width: 24px;
    height: 24px;
  }
  .sinst-bar {
    position: relative;
    height: 4px;
    border-radius: 999px;
    background: var(--bg);
    overflow: hidden;
  }
  .sinst-fill {
    height: 100%;
    background: var(--accent-solid);
    border-radius: 999px;
  }
  .sinst-fill.indet {
    width: 100%;
    opacity: 0.65;
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  .sinst-msg {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .sinst-err {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .sinst-logs {
    margin: 0;
    padding: 8px;
    max-height: 150px;
    overflow: auto;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
  }
  @media (prefers-reduced-motion: reduce) {
    .sinst-fill.indet {
      animation: none;
      width: 50%;
    }
  }
</style>
