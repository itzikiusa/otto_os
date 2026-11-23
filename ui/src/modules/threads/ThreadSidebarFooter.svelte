<script lang="ts">
  // The Threads sidebar's quiet footer: Settings · Pull requests · Usage ·
  // Refresh (threads, runtime rows, repos and their git status); "Back"
  // while a stage view (Pull requests) is open.
  import Icon from '../../lib/components/Icon.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { PullRequestsView, SettingsView, UsageView } from './threadLazy.svelte';
  import { threadRepos } from './threadRepos.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import { runtimeThreads } from './runtime/runtimeThreads.svelte';
  import { limits } from '../../lib/stores/usageLimits.svelte';
  import { providerMeter } from '../usage/limitsText'; // ui-guards: allow — the Usage page's own wording for the same numbers
  import { threads } from './threadsStore.svelte';

  // The Usage button is a meter for the focused thread's agent: its tightest
  // plan window ("5h 62%" used), warning at ≥ 80 %, danger at ≥ 95 %, every
  // window and its reset in the tooltip. No fresh numbers: the plain icon.
  $effect(() => limits.mount());
  const provider = $derived.by(() => {
    const id = threads.focusedSessionId;
    if (!id || threads.newDraftMode) return null;
    return runtimeThreads.get(id)?.provider ?? ws.getSession(id)?.provider ?? null;
  });
  const meter = $derived(provider && limits.data ? providerMeter(limits.data.accounts, provider, limits.data.stale_after_secs, limits.now) : null);

  let { onselect }: { onselect?: () => void } = $props();
  let syncing = $state(false);

  async function sync(): Promise<void> {
    if (syncing) return;
    syncing = true;
    try {
      threadRepos.reset();
      await ws.refreshSessions();
      await ws.refreshOtherSessions();
      if (ws.currentId) await runtimeThreads.load(ws.currentId);
      await runtimeThreads.loadAll();
      if (ws.currentId) await git.loadRepos(ws.currentId, true);
      for (const id of Object.keys(git.statusById)) void git.refreshStatus(id);
    } catch (e) {
      toastError('Couldn’t refresh threads', e);
    } finally {
      syncing = false;
    }
  }
</script>

<footer class="th-foot">
  {#if threadsChrome.stageView}
    <button type="button" class="btn small ghost" data-testid="threads-stage-back" onclick={() => (threadsChrome.stageView = null)}>
      <Icon name="chevronLeft" size={12} /> Back
    </button>
  {:else}
    <button
      type="button"
      class="icon-btn"
      aria-label="Settings"
      title="Settings"
      data-testid="threads-open-settings"
      onpointerenter={() => SettingsView.prefetch()}
      onclick={() => (threadsChrome.sheet = 'settings')}
    >
      <Icon name="gear" size={14} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Pull requests"
      title="Pull requests"
      data-testid="threads-open-prs"
      onpointerenter={() => PullRequestsView.prefetch()}
      onclick={() => {
        threadsChrome.stageView = 'prs';
        onselect?.();
      }}
    >
      <Icon name="branch" size={14} />
    </button>
    {#if meter}
      <button
        type="button"
        class="th-meter {meter.tone}"
        aria-label="Usage: {meter.short} window {meter.used}% used. Open Usage"
        title={[...meter.lines, 'Open Usage'].join('\n')}
        data-testid="threads-open-usage"
        onpointerenter={() => UsageView.prefetch()}
        onclick={() => (threadsChrome.sheet = 'usage')}
      >
        <span>{meter.short}</span>
        <span class="th-meter-bar" aria-hidden="true"><span class="th-meter-fill" style:inline-size="{Math.min(100, meter.used)}%"></span></span>
        <span class="th-meter-pct" data-testid="threads-usage-meter">{meter.used}%</span>
      </button>
    {:else}
      <button
        type="button"
        class="icon-btn"
        aria-label="Usage statistics"
        title="Usage statistics"
        data-testid="threads-open-usage"
        onpointerenter={() => UsageView.prefetch()}
        onclick={() => (threadsChrome.sheet = 'usage')}
      >
        <Icon name="chart" size={14} />
      </button>
    {/if}
    <span class="th-spacer"></span>
    <button
      type="button"
      class="icon-btn"
      aria-label="Refresh threads and git status"
      title="Refresh threads and git status"
      aria-busy={syncing}
      disabled={syncing}
      onclick={() => void sync()}
    >
      <Icon name="refresh" size={14} />
    </button>
  {/if}
</footer>

<style>
  .th-foot {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px;
    flex-shrink: 0;
  }
  .th-meter {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    min-block-size: 28px;
    padding-inline: var(--sp-3);
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .th-meter:hover {
    background: var(--hover);
    color: var(--text);
  }
  .th-meter-bar {
    inline-size: 36px;
    block-size: 4px;
    border-radius: var(--radius-s);
    background: var(--surface-3);
    overflow: hidden;
  }
  .th-meter-fill {
    display: block;
    block-size: 100%;
    background: var(--text-dim);
  }
  .th-meter.warning {
    color: var(--warning);
  }
  .th-meter.warning .th-meter-fill {
    background: var(--warning);
  }
  .th-meter.danger {
    color: var(--danger);
  }
  .th-meter.danger .th-meter-fill {
    background: var(--danger);
  }
  @media (hover: none), (pointer: coarse) {
    .th-meter {
      min-block-size: var(--hit-min);
    }
  }
  .th-spacer {
    flex: 1;
  }
</style>
