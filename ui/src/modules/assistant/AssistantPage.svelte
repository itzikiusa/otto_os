<script lang="ts">
  // Otto Assistant (`#/assistant[/<threadId> | /tasks | /memory | /permissions]`).
  // List/detail: threads on the left (Spaces 01–04 + Recent), the selected
  // thread's conversation on the right — opening on the remembered or latest
  // thread, never on an empty pane. Tasks · Memory · Permissions are the other
  // tabs. On wide windows a rail shows what needs you, what's running and how
  // much of each subscription this week has used. Phone: push navigation.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { recallSelection, rememberSelection } from '../../lib/lastSelection';
  import { assistant, describeError } from '../../lib/stores/assistant.svelte';
  import { latestThreadId, providerLabel, spaceLabel } from './model';
  import ThreadList from './ThreadList.svelte';
  import ChatView from './ChatView.svelte';
  import TasksTab from './TasksTab.svelte';
  import MemoryTab from './MemoryTab.svelte';
  import PermissionsTab from './PermissionsTab.svelte';
  import NeedsYouRail from './NeedsYouRail.svelte';

  type Tab = 'chat' | 'tasks' | 'memory' | 'permissions';
  const TABS: { id: Tab; label: string }[] = [
    { id: 'chat', label: 'Chat' },
    { id: 'tasks', label: 'Tasks' },
    { id: 'memory', label: 'Memory' },
    { id: 'permissions', label: 'Permissions' },
  ];
  const TAB_IDS = new Set<string>(['tasks', 'memory', 'permissions']);

  const seg = $derived(router.parts[1] ?? '');
  const tab = $derived<Tab>(TAB_IDS.has(seg) ? (seg as Tab) : 'chat');
  const routeThread = $derived(tab === 'chat' && seg ? seg : null);

  $effect(() => {
    void assistant.loadThreads();
  });

  const threads = $derived(assistant.threads.data);
  const listState = $derived(assistant.threads.state);
  // The open thread: the route's, else the remembered / latest one (desktop).
  // On phone the list IS the page until a thread is opened.
  const selectedId = $derived.by(() => {
    if (routeThread) return routeThread;
    if (viewport.isPhone && tab === 'chat') return null;
    return latestThreadId(threads, recallSelection('assistant'));
  });
  const thread = $derived(assistant.thread(selectedId));
  $effect(() => {
    if (thread) rememberSelection('assistant', thread.id);
  });
  // A thread id in the URL that isn't in the (loaded) list: say so, don't spin.
  const missing = $derived(!!routeThread && listState === 'ready' && !thread);

  function open(id: string): void {
    router.go(`assistant/${id}`);
  }
  function go(t: Tab): void {
    router.go(t === 'chat' ? (selectedId ? `assistant/${selectedId}` : 'assistant') : `assistant/${t}`);
  }

  let creating = $state(false);
  async function newThread(slot: 1 | 2 | 3 | 4 | null = null, title?: string): Promise<void> {
    creating = true;
    try {
      const t = await assistant.createThread({ space_slot: slot, title });
      open(t.id);
    } catch (e) {
      toasts.error('Couldn’t start a thread', describeError(e));
    } finally {
      creating = false;
    }
  }
  async function newSpace(slot: 1 | 2 | 3 | 4): Promise<void> {
    const name = await confirmer.promptText(`Name space ${spaceLabel(slot)} — a thread you come back to, like Personal or Work.`, {
      title: `New space ${spaceLabel(slot)}`,
      confirmLabel: 'Create space',
      initial: '',
    });
    if (name) await newThread(slot, name);
  }

  function showWork(): void {
    if (thread?.session_id) router.go(`agents/${thread.session_id}`);
  }

  // Tab list keyboard: ←/→, Home/End move between tabs.
  function onTabKey(e: KeyboardEvent): void {
    const at = TABS.findIndex((t) => t.id === tab);
    let next = -1;
    if (e.key === 'ArrowRight') next = (at + 1) % TABS.length;
    else if (e.key === 'ArrowLeft') next = (at - 1 + TABS.length) % TABS.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = TABS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    go(TABS[next].id);
    queueMicrotask(() => (e.currentTarget as HTMLElement | null)?.querySelector<HTMLButtonElement>('[aria-selected="true"]')?.focus());
  }

  // ⌘K verbs.
  $effect(() => {
    return registry.register('assistant', [
      { id: 'assistant.new', title: 'New assistant thread', group: 'Assistant', keywords: 'otto chat ask conversation', run: () => void newThread() },
      { id: 'assistant.tasks', title: 'Open assistant tasks', group: 'Assistant', keywords: 'reminders running needs you approvals', run: () => go('tasks') },
      { id: 'assistant.memory', title: 'Open assistant memory', group: 'Assistant', keywords: 'profile remember forget', run: () => go('memory') },
      { id: 'assistant.routing', title: 'Assistant routing settings', group: 'Assistant', keywords: 'claude codex model limit failover subscription', run: () => router.go('settings/assistant') },
      ...(thread?.session_id
        ? [{ id: 'assistant.work', title: 'Show work (terminal)', group: 'Assistant', keywords: 'terminal session pty', run: showWork }]
        : []),
    ]);
  });

  const title = $derived(tab === 'chat' && thread ? thread.title : 'Assistant');
  const subtitle = $derived(
    tab === 'chat' && thread
      ? [
          thread.space_slot ? `Space ${spaceLabel(thread.space_slot)}` : 'Thread',
          thread.route_pinned ? `pinned to ${providerLabel(thread.provider, thread.model)}` : 'routed by your rules',
          thread.incognito ? 'incognito — nothing is remembered' : '',
        ]
          .filter(Boolean)
          .join(' · ')
      : 'Chats, remembers, reminds — and asks before anything leaves your Mac',
  );
  const needsCount = $derived(assistant.needsYouCount);
  const showList = $derived(!viewport.isPhone || (tab === 'chat' && !selectedId));
  const showMain = $derived(!viewport.isPhone || tab !== 'chat' || !!selectedId);
</script>

<div class="as-page">
  <PageHeader {title} {subtitle}>
    {#snippet leading()}
      {#if viewport.isPhone && tab === 'chat' && selectedId}
        <button class="icon-btn" onclick={() => router.go('assistant')} aria-label="Back to threads" title="Back to threads">
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
    {#snippet badge()}
      {#if tab === 'chat' && thread}
        <span class="prov" title={thread.route_pinned ? 'Pinned for this thread' : 'Current provider (routing rules)'}>
          <ProviderIcon provider={thread.provider} size={12} />{providerLabel(thread.provider, thread.model)}
        </span>
      {/if}
    {/snippet}
    {#snippet tabs()}
      <div class="segmented" role="tablist" aria-label="Assistant view" tabindex="-1" onkeydown={onTabKey}>
        {#each TABS as t (t.id)}
          <button
            role="tab"
            aria-selected={tab === t.id}
            tabindex={tab === t.id ? 0 : -1}
            class:active={tab === t.id}
            onclick={() => go(t.id)}
            data-testid={`assistant-tab-${t.id}`}
          >
            {t.label}{#if t.id === 'tasks' && needsCount > 0}<span class="tab-count" title={`${needsCount} waiting on you`}>{needsCount}</span>{/if}
          </button>
        {/each}
      </div>
    {/snippet}
    {#snippet actions()}
      {#if tab === 'chat' && thread}
        <button class="btn small" data-icon="terminal" onclick={showWork} disabled={!thread.session_id} title={thread.session_id ? 'Open the CLI session behind this thread' : 'No CLI session yet — it starts with your first message'}>
          <Icon name="terminal" size={12} /> Show work
        </button>
      {/if}
      <button class="icon-btn" data-overflow="-1" data-icon="gear" data-label="Routing settings" onclick={() => router.go('settings/assistant')} aria-label="Routing settings" title="Routing settings">
        <Icon name="gear" size={14} />
      </button>
    {/snippet}
  </PageHeader>

  <PageBody fill padded={false}>
    {#if listState === 'unsupported'}
      <EmptyState variant="page" icon="assistant" title="The assistant isn’t available yet" body="This Otto daemon doesn’t include the assistant. Update Otto, then come back here." />
    {:else if listState === 'error' && !threads.length}
      <LoadState variant="page" what="your threads" error={assistant.threads.error || 'Something went wrong.'} empty onretry={() => void assistant.loadThreads()} />
    {:else if listState === 'ready' && !threads.length && tab === 'chat'}
      <EmptyState
        variant="page"
        icon="assistant"
        title="Meet Otto, your assistant"
        body="One place to ask, plan and get things done. It runs on your Claude and Codex subscriptions, remembers what matters, and asks before anything leaves your Mac."
        actionLabel={creating ? 'Starting…' : 'Start a conversation'}
        actionIcon="plus"
        onaction={() => void newThread(1, 'Personal')}
      />
    {:else}
      <div class="split-wrap"><div class="split" class:phone={viewport.isPhone}>
        {#if showList}
          <div class="list-pane">
            {#if listState === 'loading' && !threads.length}
              <div class="pad" aria-busy="true" aria-label="Loading threads"><Skeleton rows={6} height={28} /></div>
            {:else}
              <ThreadList {threads} selected={tab === 'chat' ? selectedId : null} onopen={open} onnew={() => void newThread()} onnewspace={(s) => void newSpace(s)} {creating} />
            {/if}
          </div>
        {/if}
        {#if showMain}
          <section class="main" aria-label={tab === 'chat' ? 'Conversation' : TABS.find((t) => t.id === tab)?.label}>
            {#if tab === 'chat'}
              {#if missing}
                <EmptyState icon="assistant" title="This thread is gone" body="It may have been deleted, or it expired (incognito threads last 24 hours)." actionLabel="Open the latest thread" onaction={() => router.go('assistant')} />
              {:else if thread}
                {#key thread.id}
                  <ChatView {thread} onopentasks={() => go('tasks')} onopenmemory={() => go('memory')} />
                {/key}
              {:else}
                <div class="pad" aria-busy="true" aria-label="Loading the conversation"><Skeleton rows={4} height={44} /></div>
              {/if}
            {:else}
              <div class="tab-scroll">
                {#if tab === 'tasks'}
                  <TasksTab onopenthread={open} />
                {:else if tab === 'memory'}
                  <MemoryTab onopenthread={open} />
                {:else}
                  <PermissionsTab />
                {/if}
              </div>
            {/if}
          </section>
          {#if !viewport.isPhone}
            <div class="rail-pane">
              <NeedsYouRail onopenthread={open} onopentasks={() => go('tasks')} />
            </div>
          {/if}
        {/if}
      </div></div>
    {/if}
  </PageBody>
</div>

<style>
  .as-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .prov {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
    white-space: nowrap;
  }
  .tab-count {
    margin-inline-start: 6px;
    color: var(--warning);
    font-weight: 600;
  }
  .split-wrap {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    container-type: inline-size;
  }
  .split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr) 260px;
  }
  .list-pane {
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
  }
  .main {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .tab-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .rail-pane {
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .rail-pane > :global(*) {
    flex: 1;
  }
  .pad {
    padding: 12px;
  }
  /* The rail needs room: drop it when the split is narrower than list +
     a readable conversation + rail (≈ a 1100 px window with the sidebar). */
  @container (max-width: 1060px) {
    .split {
      grid-template-columns: 240px minmax(0, 1fr);
    }
    .rail-pane {
      display: none;
    }
  }
  @container (max-width: 700px) {
    .split {
      grid-template-columns: 200px minmax(0, 1fr);
    }
  }
  .split.phone {
    grid-template-columns: minmax(0, 1fr);
  }
  .split.phone .list-pane {
    border-inline-end: 0;
  }
</style>
