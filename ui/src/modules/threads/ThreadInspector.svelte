<script lang="ts">
  // Thread details: the Threads Detail pane (design.md §3, DetailPane) as T3's
  // "Open a surface" (person-facing: "panel"): with nothing open it lists them
  // (B F C P S while the panel has focus); opened ones are the pane's views
  // (its header), with + (another) as its one action and Close <surface> /
  // Widen under ⋯. Inline while the conversation keeps its floor, else a right
  // sheet (a phone, a tablet): there the card folds to one line and the key
  // badges and Widen go. Every surface is its own lazy chunk, prefetched on
  // hover, so the panel itself stays light.
  import { tick } from 'svelte';
  import { runtimeDrafts } from '../../lib/stores/runtimeDrafts.svelte';
  import { addComposerContext, type ComposerContext } from './runtime/composerContext';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import type { IconName } from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { isRuntimeThread } from './runtime/runtimeThreads.svelte';
  import { loadBrowserView } from '../../lib/browserPort';
  import { ws } from '../../lib/stores/workspace.svelte';
  import ThreadDetailsCard from './ThreadDetailsCard.svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
  import { DETAIL_PANE } from '../../lib/components/side/paneKeys';
  import { MIN_CONVERSATION_BESIDE_SURFACE_W, MIN_CONVERSATION_W, SURFACE_W } from './threadsLayout';
  import ThreadAgentsTasks from './runtime/ThreadAgentsTasks.svelte';
  import RuntimeSubagentPane from './runtime/RuntimeSubagentPane.svelte';
  import { PrSurfaceView, SideChatView } from './threadLazy.svelte';
  import { branchPrs as threadPrs } from '../../lib/stores/branchPrs.svelte';
  import { threadRepos } from './threadRepos.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import { isPlainShellSession } from './threadsHelpers';
  import { threads, type ThreadsInspectorTab } from './threadsStore.svelte';
  import { asThreadSession, runtimeThreads } from './runtime/runtimeThreads.svelte';

  const FilesView = lazyComponent(() => import('./ThreadFileTabs.svelte'));
  const ChangesView = lazyComponent(() => import('./ThreadChangesTab.svelte'));
  const ConnectionsView = lazyComponent(() => import('./ThreadConnectionsTab.svelte'));
  // The Browser opens HERE, as an inspector tab — never the shell's Side
  // panel, which Threads hides (it would be a second right column).
  const BrowserTab = lazyComponent(loadBrowserView);

  // On the new-thread stage the panel follows the draft's folder.
  const focusedSession = $derived.by(() => {
    const id = threads.newDraftMode ? null : threads.focusedSessionId;
    const rt = runtimeThreads.get(id);
    return id ? (ws.getSession(id) ?? (rt ? asThreadSession(rt) : null)) : null;
  });
  const contextCwd = $derived(
    focusedSession?.cwd ?? (threads.newDraftMode ? threads.stageCwd : null) ?? ws.current?.root_path ?? null,
  );
  const workspaceId = $derived(focusedSession?.workspace_id ?? ws.currentId ?? null);
  const wsKey = $derived(workspaceId ?? 'scratch');
  const rootPath = $derived(contextCwd ?? ws.scratch?.root_path ?? '');
  const repo = $derived(threadRepos.repoFor(contextCwd, workspaceId));
  const branch = $derived(repo ? (git.statusById[repo.id]?.branch ?? null) : null);
  const pr = $derived(threadPrs.prFor(repo, branch));
  const canSide = $derived(!!focusedSession && !isPlainShellSession(focusedSession));

  const contextThreadId = $derived(focusedSession && isRuntimeThread(focusedSession) ? focusedSession.id : null);

  function addContext(context: ComposerContext, ownerId: string): void {
    if (contextThreadId !== ownerId || threads.focusedSessionId !== ownerId) return;
    const draft = runtimeDrafts.for(ownerId);
    draft.context = addComposerContext(draft.context, context);
    if (threads.inspectorCompact) threads.setInspectorOpen(false);
    void tick().then(() => {
      if (threads.focusedSessionId !== ownerId) return;
      document.querySelector<HTMLTextAreaElement>(`[data-testid="runtime-thread-${CSS.escape(ownerId)}"] [data-testid="runtime-composer"]`)?.focus();
    });
  }

  interface Surface {
    id: ThreadsInspectorTab;
    label: string;
    key: string;
    icon: IconName;
    disabled?: string | null;
    prefetch?: () => void;
  }
  const SURFACE_LABEL: Record<ThreadsInspectorTab, string> = {
    files: 'Files',
    changes: 'Changes',
    pr: 'Pull request',
    side: 'Side chat',
    connections: 'Connections',
    browser: 'Browser',
    subagent: 'Subagent',
  };
  const launcher = $derived<Surface[]>([
    { id: 'browser', label: 'Browser', key: 'B', icon: 'globe', prefetch: () => BrowserTab.prefetch() },
    { id: 'files', label: 'Files', key: 'F', icon: 'file', prefetch: () => FilesView.prefetch() },
    { id: 'changes', label: 'Changes', key: 'C', icon: 'branch', disabled: repo ? null : 'Not a git repository', prefetch: () => ChangesView.prefetch() },
    {
      id: 'pr',
      label: 'Pull request',
      key: 'P',
      icon: 'pr',
      disabled: pr ? null : 'No open pull request for this branch',
      prefetch: () => PrSurfaceView.prefetch(),
    },
    { id: 'side', label: 'Side chat', key: 'S', icon: 'comment', disabled: canSide ? null : 'Open an agent thread first', prefetch: () => SideChatView.prefetch() },
  ]);

  const open = $derived(threadsChrome.surfaces);
  const active = $derived(open.includes(threads.inspectorTab) ? threads.inspectorTab : null);
  const tabs = $derived<TabItem<ThreadsInspectorTab>[]>(
    open.map((id) => ({ id, label: id === 'subagent' ? (runtimeThreads.get(threadsChrome.subagentId)?.title ?? 'Subagent') : SURFACE_LABEL[id], testid: `threads-tab-${id}` })),
  );

  // No Terminal surface: the drawer is the toolbar ⋯ › Terminal drawer, the
  // pane's own button and /terminal. Connections shows once the thread has
  // touched a database (or its tab is already open); related threads live in
  // ⋯ › Send to another thread… (subagents sit in the details card).
  const dbActive = $derived(
    open.includes('connections') || threads.mcpActivities.some((a) => a.session_id === focusedSession?.id),
  );

  // A surface opens wider, at its own remembered width; Widen gives it all but the conversation's floor.
  const surfaceOpen = $derived(open.includes(threads.inspectorTab));
  const SURFACE_RANGE = { default: SURFACE_W, min: 300, max: 1200 };
  // svelte-ignore state_referenced_locally
  let sheet = $state(threads.inspectorCompact);
  $effect(() => threads.setInspectorCompact(sheet));

  function paneMenu(): MenuItem[] {
    const items: MenuItem[] = [];
    if (active) items.push({ label: `Close ${SURFACE_LABEL[active]}`, icon: 'x', action: closeActive });
    if (!sheet) items.push({ label: threadsChrome.maximized ? 'Restore panel width' : 'Widen panel', icon: threadsChrome.maximized ? 'minimize' : 'maximize', title: 'The conversation stays beside it', action: () => (threadsChrome.maximized = !threadsChrome.maximized) });
    return items;
  }

  function launch(s: Surface): void {
    if (!s.disabled) threads.setInspectorTab(s.id);
  }

  let addBtnEl = $state<HTMLButtonElement | null>(null);

  function closeActive(): void {
    if (!active) return;
    const next = threadsChrome.closeSurface(active);
    if (active === 'subagent') threadsChrome.subagentId = null;
    if (next) threads.setInspectorTab(next);
    void tick().then(() => {
      const target = next
        ? document.querySelector<HTMLElement>(`[data-testid="threads-inspector"] [data-testid="threads-tab-${next}"]`)
        : addBtnEl;
      target?.focus({ preventScroll: true });
    });
  }

  function openAddMenu(e: MouseEvent): void {
    const extra: Surface[] = dbActive
      ? [{ id: 'connections', label: 'Connections', key: '', icon: 'plug', prefetch: () => ConnectionsView.prefetch() }]
      : [];
    const items: MenuItem[] = [...launcher, ...extra].map((s) => ({
      label: s.label,
      icon: s.icon,
      hint: s.key || undefined,
      disabled: !!s.disabled,
      title: s.disabled ?? undefined,
      action: () => launch(s),
    }));
    ctxMenu.show(e, items);
  }

  // Single-key surface shortcuts only while the empty launcher is shown
  // (never hijack bare letters while viewing Files, Changes, PR, Browser, etc.).
  function surfaceKeys(node: HTMLElement): { destroy(): void } {
    node.addEventListener('keydown', onKey);
    return { destroy: () => node.removeEventListener('keydown', onKey) };
  }

  function onKey(e: KeyboardEvent): void {
    if (active !== null || e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest('input, textarea, select, [contenteditable="true"], .xterm, .cm-editor')) return;
    const s = launcher.find((x) => x.key.toLowerCase() === e.key.toLowerCase());
    if (s && !s.disabled) {
      e.preventDefault();
      launch(s);
    }
  }
</script>

<DetailPane
  module="threads"
  pane={surfaceOpen ? 'surface' : 'inspector'}
  legacy={{ w: surfaceOpen ? 'threads.surfaceW' : 'threads.inspectorW' }}
  title="Thread details"
  open={threads.inspectorVisible}
  onclose={() => threads.setInspectorOpen(false)}
  ontoggle={() => threads.toggleInspector()}
  range={surfaceOpen ? SURFACE_RANGE : DETAIL_PANE}
  minContent={surfaceOpen ? MIN_CONVERSATION_BESIDE_SURFACE_W : MIN_CONVERSATION_W}
  maximized={threadsChrome.maximized}
  bind:sheet
  menu={active || !sheet ? paneMenu : undefined}
  testid="threads-inspector"
>
  {#snippet views()}
    {#if tabs.length > 0}
      <Tabs
        label="Open panels"
        size="s"
        idBase="threads-insp"
        testid="threads-inspector-tabs"
        {tabs}
        value={active ?? tabs[0].id}
        onchange={(id) => threads.setInspectorTab(id)}
      />
    {:else}
      <span class="ti-title">Open a panel</span>
    {/if}
  {/snippet}
  {#snippet action()}
    <button
      bind:this={addBtnEl}
      type="button"
      class="icon-btn"
      aria-haspopup="menu"
      aria-label="Open another panel"
      title="Open another panel"
      data-testid="threads-surface-add"
      onclick={openAddMenu}
    >
      <Icon name="plus" size={14} />
    </button>
  {/snippet}
<div class="ti" use:surfaceKeys>
  <!-- Thread details head the panel (they float beside the conversation
       only while this panel is closed). -->
  <!-- On a phone a subagent's tab is a full-height sheet: the card gives way. -->
  {#if contextCwd && !(threads.inspectorCompact && active === 'subagent')}
    <div class="ti-card"><ThreadDetailsCard cwd={contextCwd} threadId={focusedSession?.id ?? null} summary={active !== null} /></div>
  {/if}
  {#if contextThreadId}<ThreadAgentsTasks sessionId={contextThreadId} />{/if}

  <div
    class="ti-panel"
    role={active ? 'tabpanel' : undefined}
    id={active ? `threads-insp-panel-${active}` : undefined}
    aria-labelledby={active ? `threads-insp-tab-${active}` : undefined}
  >
    {#if active === 'files'}
      <LazyMount lazy={FilesView} what="files" variant="panel" props={{ rootPath, wsKey }} />
    {:else if active === 'changes'}
      <LazyMount lazy={ChangesView} what="changes" variant="panel" props={{ workspaceId, sessionCwd: contextCwd, wsKey, threadId: contextThreadId, oncontext: addContext }} />
    {:else if active === 'pr'}
      <LazyMount lazy={PrSurfaceView} what="the pull request" variant="panel" props={{ repo, branch, threadId: contextThreadId, oncontext: addContext }} />
    {:else if active === 'side'}
      <LazyMount lazy={SideChatView} what="the side chat" variant="panel" props={{ mainId: focusedSession?.id ?? null }} />
    {:else if active === 'connections'}
      <LazyMount lazy={ConnectionsView} what="connections" variant="panel" props={{ workspaceId, focusedSessionId: threads.focusedSessionId }} />
    {:else if active === 'subagent' && threadsChrome.subagentId}
      <RuntimeSubagentPane id={threadsChrome.subagentId} />
    {:else if active === 'browser'}
      <!-- The ask bar submits to the focused terminal thread; a structured
           thread has no PTY to type into, so it gets the unbound hint. -->
      <LazyMount lazy={BrowserTab} what="the browser" variant="panel" props={{ embedded: true, targetSessionId: focusedSession && !isRuntimeThread(focusedSession) && focusedSession.kind === 'agent' ? focusedSession.id : null }} />
    {:else}
      <ul class="ti-launch" data-testid="threads-surface-launcher">
        {#each launcher as s (s.id)}
          <li>
            <button
              type="button"
              class="ti-row"
              disabled={!!s.disabled}
              title={s.disabled ?? `Open ${s.label} (${s.key})`}
              aria-keyshortcuts={s.key}
              data-testid="threads-surface-{s.id}"
              onpointerenter={() => s.prefetch?.()}
              onclick={() => launch(s)}
            >
              <Icon name={s.icon} size={14} />
              <span class="ti-label">{s.label}</span>
              <kbd class="ti-key">{s.key}</kbd>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>
</DetailPane>

<style>
  .ti {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  /* The card never takes more than half the panel: the surface below keeps a usable height. */
  .ti-card {
    padding: 10px 10px 6px;
    flex-shrink: 0;
    max-block-size: 50%;
    overflow-y: auto;
  }
  .ti-title {
    flex: 1;
    padding-inline-start: 8px;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .ti-panel {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .ti-launch {
    list-style: none;
    margin: 0;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ti-row {
    inline-size: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    block-size: 34px;
    padding-inline: 10px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .ti-row :global(svg) {
    color: var(--text-dim);
  }
  .ti-row:hover:not(:disabled) {
    background: var(--hover);
  }
  .ti-row:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .ti-label {
    flex: 1;
  }
  .ti-key {
    min-inline-size: 20px;
    padding-inline: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-align: center;
  }
  /* Touch: no keyboard to press the key with. */
  @media (hover: none) {
    .ti-key {
      display: none;
    }
  }
</style>
