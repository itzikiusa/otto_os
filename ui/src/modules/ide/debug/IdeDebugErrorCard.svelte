<script lang="ts">
  // The debugger's one failure card (I2-10): what went wrong in full (wrapping,
  // never cut with an ellipsis) and the next step — install the missing
  // debugger with progress inline, open the SDK manager, or show the build
  // output — plus Retry / Dismiss.
  import Icon from '../../../lib/components/Icon.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { ide } from '../ideStore.svelte';
  import type { DebugErrorCard } from './ideDebugPanelModel';

  interface Props {
    card: DebugErrorCard;
    onshowoutput: () => void;
    ondismiss: () => void;
    onretry: () => void;
  }

  let { card, onshowoutput, ondismiss, onretry }: Props = $props();

  let jobId = $state<string | null>(null);
  let starting = $state(false);
  const job = $derived(jobId ? (ide.installJobs.find((j) => j.id === jobId) ?? null) : null);
  const installing = $derived(starting || job?.status === 'running');

  async function install(): Promise<void> {
    if (!card.ecosystem || installing) return;
    starting = true;
    try {
      const started = await ide.installToolchain(card.ecosystem, 'dap');
      jobId = started?.id ?? null;
    } finally {
      starting = false;
    }
  }

  function primary(): void {
    const id = card.primary?.id;
    if (id === 'install') void install();
    else if (id === 'sdks') ide.toolchainsOpen = true;
    else if (id === 'build-output') onshowoutput();
  }
</script>

<div class="dbg-err-card" data-testid="ide-debug-error-card" use:announceShown={{ text: `${card.title}. ${card.message}`, assertive: true }}>
  <Icon name="warning" size={14} />
  <div class="dbg-err-body">
    <p class="dbg-err-title">{card.title}</p>
    <p class="dbg-err-msg" dir="auto">{card.message}</p>
    <div class="dbg-err-progress" role="status" data-testid="ide-debug-error-install-status">
      {#if job}
        {#if job.status === 'running'}
          <progress max="100" value={job.progress_pct} aria-label="Install progress"></progress>
          <span>Installing… {Math.round(job.progress_pct)}%</span>
        {:else if job.status === 'done'}
          <span class="is-ok">Installed. Start debugging again to use it.</span>
        {:else}
          <span class="is-bad">Install failed: {job.error ?? 'see SDKs for the log'}</span>
        {/if}
      {/if}
    </div>
    <div class="dbg-err-actions">
      {#if job?.status === 'done'}
        <button type="button" class="btn small primary" onclick={onretry} data-testid="ide-debug-error-retry">Debug again</button>
      {:else if card.primary}
        <button type="button" class="btn small primary" disabled={installing} onclick={primary} data-testid="ide-debug-error-primary">
          {installing ? 'Installing…' : card.primary.label}
        </button>
      {/if}
      {#if card.canRetry && job?.status !== 'done'}
        <button type="button" class="btn small" onclick={onretry} data-testid="ide-debug-error-retry">Debug again</button>
      {/if}
      <button type="button" class="btn small ghost" onclick={ondismiss} data-testid="ide-debug-error-dismiss">Dismiss</button>
    </div>
  </div>
</div>

<style>
  .dbg-err-card {
    display: flex; align-items: flex-start; gap: 8px; margin: 8px; padding: 10px 12px;
    border: 1px solid var(--danger); border-radius: var(--radius-m);
    background: var(--danger-soft); color: var(--text); flex-shrink: 0;
  }
  .dbg-err-card > :global(svg) { color: var(--danger); flex-shrink: 0; margin-block-start: 2px; }
  .dbg-err-body { display: flex; flex-direction: column; gap: 6px; min-inline-size: 0; flex: 1; }
  .dbg-err-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--danger); }
  .dbg-err-msg { margin: 0; font-size: var(--fs-xs); white-space: pre-wrap; overflow-wrap: anywhere; }
  .dbg-err-progress:empty { display: none; }
  .dbg-err-progress { display: flex; align-items: center; gap: 8px; font-size: var(--fs-xs); }
  .dbg-err-progress progress { inline-size: 140px; }
  .dbg-err-progress .is-ok { color: var(--success); }
  .dbg-err-progress .is-bad { color: var(--danger); }
  .dbg-err-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
</style>
