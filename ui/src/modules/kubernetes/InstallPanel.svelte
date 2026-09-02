<script lang="ts">
  // First-run installer for kubectl (full-page) and k9s (compact, inside a
  // sheet). Admin-gated; progress comes from polling `/k8s/status` every
  // 1.5 s while the job is `running` (the `k8s_install_updated` WS event also
  // refetches). The kubectl panel auto-continues: once `status.kubectl.installed`
  // flips, the page's `needsInstall` derived turns false and the module renders.
  import { untrack } from 'svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { K8sTool } from '../../lib/api/types';

  interface Props {
    tool: K8sTool;
    /** Rendered inside a sheet: no page chrome, no "continue" link. */
    compact?: boolean;
    oncontinue?: () => void;
  }
  let { tool, compact = false, oncontinue }: Props = $props();

  const POLL_MS = 1500;
  const job = $derived(k8s.status?.install[tool] ?? null);
  const installed = $derived(k8s.status?.[tool].installed ?? false);
  const canInstall = $derived(auth.can('kubernetes', 'admin'));
  let starting = $state(false);
  let logOpen = $state(false);

  const label = $derived(tool === 'kubectl' ? 'kubectl' : 'k9s');
  const blurb = $derived(
    tool === 'kubectl'
      ? 'Otto drives clusters through the kubectl CLI. It is not installed on this Mac yet — Otto can install it for you (Homebrew when available, otherwise a direct download into Otto’s bin directory; never sudo).'
      : 'k9s is a terminal UI for Kubernetes. Otto can install it on demand (Homebrew when available, otherwise the GitHub release tarball into Otto’s bin directory).',
  );

  // Poll while running. The dependency is the job state only; the fetch is
  // untracked so the status refresh never re-arms the interval mid-tick.
  $effect(() => {
    if (job?.state !== 'running') return;
    const t = setInterval(() => untrack(() => void k8s.loadStatus()), POLL_MS);
    return () => clearInterval(t);
  });

  // Announce the terminal states once.
  let announced = $state<string | null>(null);
  $effect(() => {
    const st = job?.state;
    if (!st || st === 'running' || st === 'idle') return;
    const key = `${st}:${job?.finished_at ?? ''}`;
    if (untrack(() => announced) === key) return;
    announced = key;
    if (st === 'done') toasts.success(`${label} installed`);
    else if (st === 'failed') toasts.error(`${label} install failed`, job?.error ?? undefined);
  });

  async function start(): Promise<void> {
    starting = true;
    try {
      await k8s.install(tool);
      logOpen = true;
    } catch (e) {
      toasts.error('Install failed to start', e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }
</script>

<div class="install" class:compact data-testid="k8s-install-panel">
  <div class="install-head">
    <div class="install-icon"><Icon name={tool === 'kubectl' ? 'helm' : 'terminal'} size={compact ? 18 : 26} /></div>
    <div>
      <h2>{installed ? `${label} is installed` : `Otto needs ${label}`}</h2>
      {#if !installed}<p class="sub">{blurb}</p>{/if}
    </div>
  </div>

  {#if installed}
    <div class="install-ok">
      <Icon name="check" size={14} />
      <span class="mono">{k8s.status?.[tool].version ?? ''}</span>
      {#if k8s.status?.[tool].path}<span class="dim mono">{k8s.status?.[tool].path}</span>{/if}
    </div>
  {:else if job?.state === 'running'}
    <div class="progress" role="progressbar" aria-label="Installing {label}" aria-busy="true"><div class="bar"></div></div>
    <div class="dim">Installing… this can take a minute (polling every 1.5 s).</div>
  {:else if job?.state === 'failed'}
    <div class="fail">
      <Icon name="info" size={14} />
      <span>Install failed{job.error ? `: ${job.error}` : ''}.</span>
    </div>
  {/if}

  {#if !installed}
    <div class="actions">
      {#if canInstall}
        <button class="btn primary" onclick={() => void start()} disabled={starting || job?.state === 'running'}>
          {job?.state === 'running' ? 'Installing…' : job?.state === 'failed' ? 'Retry install' : `Install ${label}`}
        </button>
      {:else}
        <span class="dim">Ask an Otto admin to install {label} (needs the Kubernetes admin grant).</span>
      {/if}
      <button class="btn ghost" onclick={() => void k8s.loadStatus()} title="Re-check">
        <Icon name="refresh" size={13} /> Re-check
      </button>
      {#if !compact && oncontinue}
        <button class="btn ghost dim" onclick={oncontinue}>Continue without installing</button>
      {/if}
    </div>
  {/if}

  {#if job?.log_tail}
    <details class="log" bind:open={logOpen}>
      <summary>Install log</summary>
      <pre class="mono">{job.log_tail}</pre>
    </details>
  {/if}
</div>

<style>
  .install {
    max-width: 640px;
    margin: 48px auto;
    padding: 24px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .install.compact {
    margin: 0;
    padding: 0;
    border: none;
    background: transparent;
    max-width: none;
  }
  .install-head {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .install-icon {
    flex-shrink: 0;
    width: 44px;
    height: 44px;
    border-radius: var(--radius-m);
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  .compact .install-icon {
    width: 32px;
    height: 32px;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 15px;
    font-weight: 600;
  }
  .sub,
  .dim {
    color: var(--text-dim);
    font-size: 12.5px;
    line-height: 1.5;
    margin: 0;
  }
  .install-ok {
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--status-working);
    font-size: 12.5px;
    flex-wrap: wrap;
  }
  .progress {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar {
    width: 40%;
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(260%);
    }
  }
  .fail {
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--status-exited);
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .log summary {
    cursor: pointer;
    font-size: 12px;
    color: var(--text-dim);
  }
  .log pre {
    margin: 8px 0 0;
    max-height: 240px;
    overflow: auto;
    padding: 10px;
    font-size: 11px;
    line-height: 1.45;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    white-space: pre-wrap;
    word-break: break-all;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
