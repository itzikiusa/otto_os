<script lang="ts">
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import ThreadChangesTab from './ThreadChangesTab.svelte';
  import ThreadConnectionsTab from './ThreadConnectionsTab.svelte';
  import ThreadFileTabs from './ThreadFileTabs.svelte';
  import ThreadPeersTab from './ThreadPeersTab.svelte';
  import ThreadProjectCard from './ThreadProjectCard.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  // The side chat loads with its first open, not with the page.
  const SideChat = lazyComponent(() => import('./ThreadSideChat.svelte'));
  import { threads, type ThreadsInspectorTab } from './threadsStore.svelte';

  interface Props {
    onstartroom: (sessionId: string) => void;
  }
  let { onstartroom }: Props = $props();

  // On the new-thread stage the panel follows the draft's folder, not the
  // last focused thread.
  const focusedSession = $derived(
    !threads.newDraftMode && threads.focusedSessionId ? (ws.getSession(threads.focusedSessionId) ?? null) : null,
  );
  const contextCwd = $derived(
    focusedSession?.cwd ?? (threads.newDraftMode ? threads.stageCwd : null) ?? ws.current?.root_path ?? null,
  );
  const workspaceId = $derived(focusedSession?.workspace_id ?? ws.currentId ?? null);
  const wsKey = $derived(workspaceId ?? 'scratch');
  const rootPath = $derived(contextCwd ?? ws.scratch?.root_path ?? '');

  const openFileCount = $derived(threads.fileState(wsKey).tabs.length);
  const mcpCount = $derived(threads.mcpActivities.length);
  const peerCount = $derived(threads.peerEvents.length);

  const inspectorTabs = $derived<TabItem<ThreadsInspectorTab>[]>([
    {
      id: 'files',
      label: 'Files',
      icon: 'file',
      count: openFileCount > 0 ? openFileCount : undefined,
      testid: 'threads-tab-files',
    },
    {
      id: 'changes',
      label: 'Changes',
      icon: 'branch',
      testid: 'threads-tab-changes',
    },
    {
      id: 'side',
      label: 'Side chat',
      icon: 'comment',
      testid: 'threads-tab-side',
    },
    {
      id: 'connections',
      label: 'Connections',
      icon: 'plug',
      count: mcpCount > 0 ? mcpCount : undefined,
      testid: 'threads-tab-connections',
    },
    {
      id: 'peers',
      label: 'Peers',
      icon: 'people',
      count: peerCount > 0 ? peerCount : undefined,
      testid: 'threads-tab-peers',
    },
  ]);
</script>

<aside class="ti-inspector" aria-label="Thread inspector" data-testid="threads-inspector">
  {#if contextCwd}
    <div class="ti-card">
      <ThreadProjectCard cwd={contextCwd} />
    </div>
  {/if}
  <Tabs
    label="Thread inspector"
    size="s"
    idBase="threads-insp"
    testid="threads-inspector-tabs"
    tabs={inspectorTabs}
    value={threads.inspectorTab}
    onchange={(id) => threads.setInspectorTab(id)}
  />

  <div
    class="ti-panel"
    role="tabpanel"
    id="threads-insp-panel-{threads.inspectorTab}"
    aria-labelledby="threads-insp-tab-{threads.inspectorTab}"
  >
    {#if threads.inspectorTab === 'files'}
      <ThreadFileTabs {rootPath} {wsKey} />
    {:else if threads.inspectorTab === 'changes'}
      <ThreadChangesTab {workspaceId} sessionCwd={contextCwd} {wsKey} />
    {:else if threads.inspectorTab === 'side'}
      {#if SideChat.component}
        <SideChat.component mainId={focusedSession?.id ?? null} />
      {:else}
        <LoadState
          what="the side chat"
          variant="panel"
          empty
          loading={!SideChat.error}
          error={SideChat.error}
          onretry={() => SideChat.retry()}
        />
      {/if}
    {:else if threads.inspectorTab === 'connections'}
      <ThreadConnectionsTab {workspaceId} focusedSessionId={threads.focusedSessionId} />
    {:else}
      <ThreadPeersTab {workspaceId} focusedSessionId={threads.focusedSessionId} {onstartroom} />
    {/if}
  </div>
</aside>

<style>
  .ti-inspector {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
  }
  .ti-card {
    padding: 10px 10px 6px;
    flex-shrink: 0;
  }
  .ti-panel {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
</style>
