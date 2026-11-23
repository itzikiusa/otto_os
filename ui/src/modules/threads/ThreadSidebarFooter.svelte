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
  .th-spacer {
    flex: 1;
  }
</style>
