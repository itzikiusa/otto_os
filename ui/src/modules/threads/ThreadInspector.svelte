<script lang="ts">
  // The right panel as T3's "Open a surface": with nothing open it lists the
  // surfaces (B F D P S while the panel has focus); opened ones sit as tabs
  // with + (another surface), close and maximize. Every surface is its own
  // lazy chunk, prefetched on hover, so the panel itself stays light.
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
  import ThreadProjectCard from './ThreadProjectCard.svelte';
  import ThreadAgentsTasks from './runtime/ThreadAgentsTasks.svelte';
  import { PrSurfaceView, SideChatView } from './threadLazy.svelte';
  import { threadPrs } from './threadPrs.svelte';
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

  interface Props {
    /** Phone / tablet drawer: the project card can't float there, so it heads the panel. */
    showCard?: boolean;
  }
  let { showCard = false }: Props = $props();

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
    changes: 'Diff',
    pr: 'Pull request',
    side: 'Side chat',
    connections: 'Connections',
    browser: 'Browser',
  };
  const launcher = $derived<Surface[]>([
    { id: 'browser', label: 'Browser', key: 'B', icon: 'globe', prefetch: () => BrowserTab.prefetch() },
    { id: 'files', label: 'Files', key: 'F', icon: 'file', prefetch: () => FilesView.prefetch() },
    { id: 'changes', label: 'Diff', key: 'D', icon: 'branch', disabled: repo ? null : 'Not a git repository', prefetch: () => ChangesView.prefetch() },
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
    open.map((id) => ({ id, label: SURFACE_LABEL[id], testid: `threads-tab-${id}` })),
  );

  // No Terminal surface: the drawer is the toolbar ⋯ › Terminal drawer, the
  // pane's own button and /terminal. Connections shows once the thread has
  // touched a database (or its tab is already open); related threads live in
  // the header's Related threads popover.
  const dbActive = $derived(
    open.includes('connections') || threads.mcpActivities.some((a) => a.session_id === focusedSession?.id),
  );

  function launch(s: Surface): void {
    if (!s.disabled) threads.setInspectorTab(s.id);
  }

  function closeActive(): void {
    if (!active) return;
    const next = threadsChrome.closeSurface(active);
    if (next) threads.setInspectorTab(next);
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

  // Single-key surface shortcuts while focus is inside the panel.
  function surfaceKeys(node: HTMLElement): { destroy(): void } {
    node.addEventListener('keydown', onKey);
    return { destroy: () => node.removeEventListener('keydown', onKey) };
  }

  function onKey(e: KeyboardEvent): void {
    if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest('input, textarea, select, [contenteditable="true"], .xterm, .cm-editor')) return;
    const s = launcher.find((x) => x.key.toLowerCase() === e.key.toLowerCase());
    if (s && !s.disabled) {
      e.preventDefault();
      launch(s);
    }
  }
</script>

<aside class="ti" aria-label="Thread panel" data-testid="threads-inspector" use:surfaceKeys>
  {#if showCard && contextCwd}
    <div class="ti-card"><ThreadProjectCard cwd={contextCwd} /></div>
  {/if}
  {#if contextThreadId}<ThreadAgentsTasks sessionId={contextThreadId} />{/if}
  <header class="ti-head">
    {#if tabs.length > 0}
      <div class="ti-tabs">
        <Tabs
          label="Open surfaces"
          size="s"
          idBase="threads-insp"
          testid="threads-inspector-tabs"
          {tabs}
          value={active ?? tabs[0].id}
          onchange={(id) => threads.setInspectorTab(id)}
        />
      </div>
    {:else}
      <span class="ti-title">Open a surface</span>
    {/if}
    {#if active}
      <button type="button" class="icon-btn" aria-label="Close {SURFACE_LABEL[active]}" title="Close surface" onclick={closeActive}>
        <Icon name="x" size={12} />
      </button>
    {/if}
    <button
      type="button"
      class="icon-btn"
      aria-haspopup="menu"
      aria-label="Open another surface"
      title="Open another surface"
      data-testid="threads-surface-add"
      onclick={openAddMenu}
    >
      <Icon name="plus" size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      class:ti-on={threadsChrome.maximized}
      aria-pressed={threadsChrome.maximized}
      aria-label={threadsChrome.maximized ? 'Restore panel' : 'Maximize panel'}
      title={threadsChrome.maximized ? 'Restore' : 'Maximize'}
      onclick={() => (threadsChrome.maximized = !threadsChrome.maximized)}
    >
      <Icon name={threadsChrome.maximized ? 'minimize' : 'maximize'} size={12} />
    </button>
  </header>

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
</aside>

<style>
  .ti {
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
  .ti-head {
    display: flex;
    align-items: center;
    gap: 2px;
    min-block-size: 38px;
    padding-inline: 10px 6px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ti-tabs {
    flex: 1;
    min-width: 0;
    overflow-x: auto;
  }
  .ti-title {
    flex: 1;
    font-size: var(--fs-m);
    font-weight: 500;
    color: var(--text);
  }
  .ti-on {
    color: var(--accent-text);
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
</style>
