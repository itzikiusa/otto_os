<script lang="ts">
  // A structured thread as an Agents pane (spec A2: terminals and chats tile
  // together). ONE header line, the terminal pane's — status word · title ·
  // folder or branch · ⋯ (· ✕ in a split) — over the Threads conversation and
  // composer, imported through the runtime-thread port (EmbeddedRuntimeThread:
  // approvals and questions are answered in it), never copied. Its own chunk:
  // a canvas of terminals only never loads it. No pop-out (terminals only), no
  // Terminal ⇄ Chat switch (a thread is only a chat).
  import { onMount, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { loadRuntimeThreadView, type RuntimeEmbedStatus } from '../../lib/runtimeThreadPort';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { events } from '../../lib/events.svelte';
  import { router } from '../../lib/router.svelte';
  import { sessionState } from '../../lib/status';
  import { sessionWord } from '../../lib/statusWords';
  import { cwdLabel } from '../../lib/paneHeader';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { rightPanel } from '../../lib/stores/rightPanel.svelte';
  import { asThreadSession, runtimeThreads } from '../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — the thread's own row (one store with Threads)
  import { presetItems } from './SplitNode.svelte';

  interface Props {
    threadId: string;
    focused: boolean;
    /** In a split: ✕ closes the pane (the thread keeps running) and ⋯ has the layout presets; ⋯ › Close pane always. */
    showClose: boolean;
    onfocus: () => void;
    onclose: () => void;
    /** The title is the split / tiled drag handle, as on a terminal pane. */
    showGrip?: boolean;
    dragKey?: string;
    ondragpane?: (phase: 'start' | 'end') => void;
  }
  let { threadId, focused, showClose, onfocus, onclose, showGrip = false, dragKey, ondragpane }: Props = $props();

  const View = lazyComponent(() => loadRuntimeThreadView());
  const row = $derived(runtimeThreads.get(threadId));
  const s = $derived(row ? asThreadSession(row) : null);
  /** The embed's own reading (a pending approval or question is "needs you" at once). */
  let embed = $state<RuntimeEmbedStatus>('idle');
  const st = $derived(sessionState(s, s?.status ?? 'idle', embed === 'needs-you', { stale: events.state !== 'connected' }));
  const word = $derived(sessionWord(st));
  const where = $derived.by(() => {
    const m = s?.meta as Record<string, unknown> | undefined;
    const b = [m?.git_branch, m?.branch, m?.worktree_branch].find((v): v is string => typeof v === 'string' && v.trim() !== '');
    return b ?? (s?.cwd ? cwdLabel(s.cwd) : null);
  });
  const current = $derived(focused || ws.activeSessionId === threadId);
  // The side panel's Git / Files follow this pane's folder while it is focused.
  $effect(() => {
    const cwd = s?.cwd;
    if (!cwd) return;
    rightPanel.paneFolders.set(threadId, cwd);
    return () => void rightPanel.paneFolders.delete(threadId);
  });
  let paneW = $state(0);
  const gripOn = $derived(showGrip && dragKey != null && !viewport.isPhone);

  // A thread that is gone leaves the canvas: deleted while shown, or a stored
  // pane whose thread no longer exists (one read, only when no list has it).
  let seen = false;
  $effect(() => {
    if (row) seen = true;
    else if (seen) untrack(() => ws.closeTab(threadId));
  });
  onMount(() => {
    if (runtimeThreads.get(threadId)) return;
    runtimeThreads.loadOne(threadId).catch((e: unknown) => {
      if ((e as { status?: unknown } | null)?.status === 404) ws.closeTab(threadId);
    });
  });

  /** The thread's own menu (Threads' runtimeMenu) without what belongs to
   *  the Threads page (its pins, its "beside", its terminal drawer). */
  const THREADS_ONLY = new Set(['Open beside', 'Terminal drawer', 'Pin thread', 'Unpin thread']);
  function openMenu(e: MouseEvent): void {
    void import('../threads/runtime/runtimeMenu').then((m) => { // ui-guards: allow — a thread's own menu
      const own = m.runtimeThreadMenu(threadId).filter((i) => !THREADS_ONLY.has(i.label ?? ''));
      const sep = { separator: true } as MenuItem;
      ctxMenu.show(e, [
        { label: 'Open in Threads', icon: 'threads', action: () => router.go(`threads/${threadId}`) },
        ...own,
        ...(showClose ? [sep, ...presetItems()] : []),
        // Always: a lone chat tab has no strip ✕ (only ⌘W).
        sep,
        { label: 'Close pane', icon: 'x', title: 'The thread keeps running', action: onclose } as MenuItem,
      ]);
    });
  }
  function onDragStart(e: DragEvent): void {
    if (!dragKey) return;
    e.dataTransfer?.setData('text/plain', dragKey);
    e.dataTransfer?.setData('application/x-otto-pane', dragKey);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
    ondragpane?.('start');
  }
</script>

<section
  class="tpane"
  class:focused
  class:current
  aria-label="Thread {s?.title ?? ''}"
  data-testid="thread-pane-{threadId}"
  bind:clientWidth={paneW}
  onpointerdown={() => { if (!focused) onfocus(); }}
>
  <header class="tpane-head" class:touch={!viewport.isDesktop}>
    {#if viewport.isPhone}
      <button type="button" class="icon-btn tpane-back" aria-label="Back to sessions" title="Back to sessions" data-testid="agents-back-to-list" onclick={() => router.backTo('agents')}><Icon name="chevronLeft" size={16} /></button>
    {/if}
    <span class="tpane-word {word.tone}" data-testid="pane-status" data-state={st.key} title={st.hint ?? st.label}>{word.word}</span>
    <button
      type="button"
      class="tpane-title"
      class:draggable={gripOn}
      draggable={gripOn ? 'true' : undefined}
      dir="auto"
      title="{s?.title ?? 'Thread'} — right-click for options"
      ondragstart={gripOn ? onDragStart : undefined}
      ondragend={gripOn ? () => ondragpane?.('end') : undefined}
      oncontextmenu={(e) => { e.preventDefault(); openMenu(e); }}
    >{s?.title ?? 'Thread'}</button>
    {#if where}<span class="tpane-where mono" dir="ltr" title={s?.cwd ?? where}>{where}</span>{/if}
    <span class="grow"></span>
    <button type="button" class="icon-btn tpane-more" title="More…" aria-label="More… — {s?.title ?? 'thread'}" aria-haspopup="menu" onclick={openMenu}><Icon name="more" size={13} /></button>
    {#if showClose}<button type="button" class="icon-btn tpane-close" title="Close pane (the thread keeps running)" aria-label="Close pane" onclick={onclose}><Icon name="x" size={12} /></button>{/if}
  </header>
  <div class="tpane-body">
    <LazyMount lazy={View} what="the conversation" variant="panel" props={{ sessionId: threadId, workspaceId: row?.workspace_id ?? '', compact: paneW > 0 && paneW < 520, hideEmbedBarWhenUnlinked: true, onstatus: (x: RuntimeEmbedStatus) => (embed = x) }} />
  </div>
</section>

<style>
  /* The terminal pane's frame and header (SessionView), over the reading surface. */
  .tpane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--bg);
    transition: border-color var(--dur-fast) ease-out;
  }
  .tpane.focused {
    border-color: var(--accent-line-strong);
  }
  .tpane-head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding-inline: 10px 4px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    overflow: clip;
  }
  .tpane-head.touch {
    height: var(--mobile-topbar-h);
  }
  /* Phone / tablet: Back and ⋯ are finger-sized (44 px), as on a terminal pane. */
  .tpane-head.touch > :is(.tpane-back, .tpane-more) {
    inline-size: 44px;
    block-size: 44px;
  }
  .tpane-back {
    flex-shrink: 0;
    margin-inline-start: calc(-1 * var(--sp-3));
  }
  .tpane-word {
    flex-shrink: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .tpane-word:is(.warning, .danger, .success) {
    font-weight: 600;
  }
  .tpane-word.warning {
    color: var(--warning);
  }
  .tpane-word.danger {
    color: var(--danger);
  }
  .tpane-word.success {
    color: var(--success);
  }
  .tpane-title {
    flex: 0 1 auto;
    min-width: 0;
    border: 0;
    background: none;
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    text-align: start;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-radius: var(--radius-s);
    padding-inline: 2px;
    cursor: default;
  }
  .tpane-title.draggable {
    cursor: grab;
  }
  .tpane:not(.current) .tpane-title {
    color: color-mix(in srgb, var(--text) 82%, transparent);
    font-weight: 500;
  }
  .tpane-where {
    flex: 0 1000 auto;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .grow {
    flex: 1 1 0;
    min-width: 0;
  }
  .tpane-more,
  .tpane-close {
    flex-shrink: 0;
  }
  .tpane-body {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .tpane-body > :global(*) {
    flex: 1;
    min-height: 0;
  }
</style>
