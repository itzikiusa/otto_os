<script lang="ts">
  // First-run panel: the daemon can't find the `aws` CLI. Offers "Install now"
  // (Admin on `aws`), shows the installer's progress + a collapsible log tail
  // while `/aws/status` is polled every 1.5 s (the store re-arms the poll on
  // each response), and auto-continues when the binary appears.
  import { aws } from '../../lib/stores/aws.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  const job = $derived(aws.status?.install ?? null);
  const running = $derived(job?.state === 'running');
  const failed = $derived(job?.state === 'failed');
  const canInstall = $derived(auth.can('aws', 'admin'));
  let logOpen = $state(false);

  async function install(): Promise<void> {
    try {
      await aws.startInstall();
    } catch (e) {
      toasts.error('Install failed to start', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="install" data-testid="aws-install-panel">
  <div class="card">
    <div class="ico"><Icon name="cloud" size={28} /></div>
    <h1>Otto needs the AWS CLI</h1>
    <p class="body">
      The AWS console shells out to <code>aws</code> (CLI v2) for every call — nothing else is
      installed and your <code>~/.aws</code> files are never written. Otto installs it with
      Homebrew when available, otherwise into its own <code>bin/</code> directory. No
      <code>sudo</code>.
    </p>

    {#if running}
      <div class="progress" role="progressbar" aria-label="Installing the AWS CLI" aria-busy="true">
        <div class="bar"></div>
      </div>
      <p class="status">Installing… this can take a minute or two.</p>
    {:else if failed}
      <p class="status err">
        Install failed{job?.error ? `: ${job.error}` : ''}.
      </p>
    {/if}

    <div class="actions">
      {#if canInstall}
        <button class="primary" onclick={() => void install()} disabled={running || aws.installBusy}>
          {running ? 'Installing…' : failed ? 'Retry install' : 'Install now'}
        </button>
      {:else}
        <span class="dim">Ask an administrator to install the CLI (Admin on <code>aws</code>).</span>
      {/if}
      <button class="ghost" onclick={() => void aws.loadStatus()}>Check again</button>
    </div>

    {#if job?.log_tail}
      <button class="log-toggle" onclick={() => (logOpen = !logOpen)} aria-expanded={logOpen}>
        <Icon name={logOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        Installer log
      </button>
      {#if logOpen}
        <pre class="log">{job.log_tail}</pre>
      {/if}
    {/if}
  </div>
</div>

<style>
  .install {
    display: grid;
    place-items: start center;
    padding: 48px 16px;
    overflow: auto;
    height: 100%;
    box-sizing: border-box;
  }
  .card {
    width: min(560px, 100%);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .ico {
    width: 52px;
    height: 52px;
    display: grid;
    place-items: center;
    border-radius: 14px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  h1 {
    margin: 0;
    font-size: 18px;
  }
  .body {
    margin: 0;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.5;
  }
  code {
    font-family: var(--font-mono);
    font-size: 12px;
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
    background: var(--accent);
    border-radius: 999px;
    animation: slide 1.4s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(260%);
    }
  }
  .status {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .status.err {
    color: var(--status-exited);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .primary {
    padding: 7px 14px;
    border-radius: var(--radius-m);
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .ghost {
    padding: 7px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .log-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    padding: 0;
  }
  .log {
    margin: 0;
    max-height: 220px;
    overflow: auto;
    padding: 10px;
    border-radius: var(--radius-m);
    background: var(--term-bg);
    color: #ddd;
    font-family: var(--font-mono);
    font-size: 11.5px;
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
