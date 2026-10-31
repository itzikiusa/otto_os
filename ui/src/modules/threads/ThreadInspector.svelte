<script lang="ts">
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import ThreadChangesTab from './ThreadChangesTab.svelte';
  import ThreadConnectionsTab from './ThreadConnectionsTab.svelte';
  import ThreadFileTabs from './ThreadFileTabs.svelte';
  import ThreadPeersTab from './ThreadPeersTab.svelte';
  import { threads, type ThreadsInspectorTab } from './threadsStore.svelte';

  interface Props {
    onstartroom: (sessionId: string) => void;
  }
  let { onstartroom }: Props = $props();

  const focusedSession = $derived(
    threads.focusedSessionId ? (ws.getSession(threads.focusedSessionId) ?? null) : null,
  );
  const workspaceId = $derived(focusedSession?.workspace_id ?? ws.currentId ?? null);
  const wsKey = $derived(workspaceId ?? 'scratch');
  const rootPath = $derived(
    focusedSession?.cwd ?? ws.current?.root_path ?? ws.scratch?.root_path ?? '',
  );

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
      <ThreadChangesTab {workspaceId} sessionCwd={focusedSession?.cwd ?? null} {wsKey} />
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
  .ti-panel {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
</style>
