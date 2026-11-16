<!--
  One structured-runtime thread hosted outside Threads (IDE AI Agent dock,
  Personal Agents' chat, Pull Requests, Connections), through
  lib/runtimeThreadPort.ts or direct IDE embed. Reads only its own row and its
  linked group, keeps them live while mounted, and switches cleanly between the
  lead thread and any delegated child threads. With no thread yet
  (`sessionId: null` + `draft`) it is the Threads start composer instead
  (EmbeddedRuntimeDraft); `onhandle` gives the host one send for its own asks.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { EMBED_DEFAULTS, openInThreads, type RuntimeEmbedHandle, type RuntimeThreadViewProps } from '../../../lib/runtimeThreadPort';
  import type { RuntimeAccessMode } from '../../../lib/api/types';
  import { runtimeDrafts } from '../../../lib/stores/runtimeDrafts.svelte';
  import EmbeddedRuntimeDraft from './EmbeddedRuntimeDraft.svelte';
  import { embedStatus } from './embedStatus';
  import RuntimeThread from './RuntimeThread.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { isRunning } from './timeline';

  let {
    sessionId,
    draft,
    onhandle,
    contextOnFresh,
    agentName,
    focusOnMount = EMBED_DEFAULTS.focusOnMount,
    compact = EMBED_DEFAULTS.compact,
    onmaximize,
    ondetach,
    onstatus,
    hostCommands,
    accessModeOverride = null,
    onaccessmodechange,
    hideEmbedBarWhenUnlinked = false,
    fallbackCwd = null,
    hasExternalContext = false,
    transformOutgoingText,
    transformRestoredQueuedText,
    onExternalContextConsumed,
    onopenpath,
    formatAssistantText,
    assistantFooter,
    composerTopSlot,
    composerFootLeadSlot,
    testIds,
  }: RuntimeThreadViewProps = $props();

  let picked = $state<string | null>(null);
  const shown = $derived(picked && runtimeThreads.group(sessionId).some((t) => t.id === picked) ? picked : sessionId);
  const current = $derived(runtimeThreads.get(shown));
  let rowError = $state<string | null>(null);
  let attempt = $state(0);
  const linked = $derived(shown !== sessionId);
  const chrome = $derived(!!sessionId && (linked || (!hideEmbedBarWhenUnlinked && (compact || !!onmaximize || !!ondetach))));
  const detach = (): void => {
    if (shown) (ondetach ?? openInThreads)(shown);
  };
  let draftEl = $state<{ send: (text: string, access?: RuntimeAccessMode) => Promise<boolean> } | null>(null);

  onMount(() => runtimeThreads.mount());

  // The host's own asks (a menu, a file, a line pick) go through the Threads
  // store like the composer's: before a thread, a start with the draft
  // composer's picks; after, a closed thread is reopened and the message
  // queues behind a running turn. The store then knows the access mode and
  // the status, so the chip and the working dot stay true.
  const handle: RuntimeEmbedHandle = {
    async send(text, opts) {
      const id = sessionId;
      if (!id) {
        if (!draftEl) throw new Error('The thread isn’t ready yet. Try again.');
        if (!(await draftEl.send(text, opts?.accessMode))) throw new Error('The thread didn’t start.');
        return;
      }
      const row = runtimeThreads.get(id) ?? (await runtimeThreads.loadOne(id));
      if (row.status === 'closed') await runtimeThreads.reopen(id);
      await runtimeThreads.prompt(id, { text, ...(opts?.accessMode ? { access_mode: opts.accessMode } : {}), delivery: 'queue' });
      // The mode now on the thread is the host's: a pick left in the chip
      // would show (and on the next send restore) another one.
      if (opts?.accessMode) runtimeDrafts.for(id).pickedMode = null;
    },
  };
  $effect(() => {
    if (!onhandle) return;
    untrack(() => onhandle(handle));
    return () => untrack(() => onhandle(null));
  });

  // `/clear` starts a fresh context: the first message after it carries the
  // host's context again (contextOnFresh), so the agent knows what it is
  // looking at.
  const outgoing = $derived.by(() => {
    const own = transformOutgoingText;
    const fresh = contextOnFresh;
    if (!fresh) return own;
    return (text: string, isFresh?: boolean): string => {
      const t = own ? own(text, isFresh) : text;
      const ctx = isFresh ? fresh() : null;
      return ctx ? `${ctx}\n\n${t}` : t;
    };
  });

  const status = $derived.by(() => {
    if (!sessionId) return 'idle' as const;
    const row = runtimeThreads.get(sessionId);
    const p = runtimeViews.projections[sessionId] ?? null;
    return embedStatus({
      status: row?.status,
      backgroundTasks: p?.session?.background_tasks?.length ?? row?.background_tasks?.length ?? 0,
      turnRunning: p ? isRunning(p) : false,
      pendingRequests: p ? p.requests.filter((r) => r.status === 'pending').length : 0,
    });
  });
  $effect(() => {
    const s = status;
    untrack(() => onstatus?.(s));
  });
  $effect(() => {
    const id = shown;
    if (!id) return;
    return untrack(() => runtimeThreads.watch(id));
  });
  $effect(() => {
    const id = sessionId;
    void attempt;
    if (!id) return;
    untrack(() => {
      rowError = null;
      runtimeThreads.loadOne(id).then(
        () => runtimeThreads.loadLinked(id).catch((e: unknown) => console.warn('linked threads unavailable', e)),
        (e: unknown) => { if (id === sessionId) rowError = loadErrorText(e); },
      );
    });
  });
  $effect(() => {
    void sessionId;
    picked = null;
  });
</script>

<div class="rt-embed" class:compact data-testid="embedded-runtime-thread">
  {#if chrome}
    <div class="rt-embed-bar">
      <span class="ellipsis" dir="auto" title={current?.title ?? ''}>{current?.title ?? 'Thread'}</span>
      {#if linked}
        <button type="button" class="btn small" onclick={() => (picked = null)}>{agentName ? 'Back to chat' : 'Back'}</button>
      {:else if hostCommands?.newThread}
        {@const id = sessionId}
        <button type="button" class="icon-btn" aria-label="New thread" title="New thread — pick the agent again (/new)" data-testid="embedded-thread-new"
          onclick={() => { if (id) hostCommands?.newThread?.(id, false); }}>
          <Icon name="plus" size={13} />
        </button>
      {/if}
      {#if onmaximize}
        <button type="button" class="icon-btn" aria-label="Maximise the thread" title="Maximise" data-testid="embedded-thread-maximize" onclick={onmaximize}>
          <Icon name="maximize" size={13} />
        </button>
      {/if}
      {#if ondetach}
        <button type="button" class="icon-btn" aria-label="Detach the thread" title="Detach" data-testid="embedded-thread-detach" onclick={detach}>
          <Icon name="external" size={13} />
        </button>
      {:else}
        <button type="button" class="btn small" aria-label="Open in Threads" title="Open this thread in Threads" data-testid="embedded-thread-open" onclick={detach}>
          <Icon name="external" size={12} />{#if !compact} Open in Threads{/if}
        </button>
      {/if}
    </div>
  {/if}
  <div class="rt-embed-body">
    {#if !sessionId || !shown}
      {#if draft}
        {#key draft.origin?.id ?? draft.cwd}
          <EmbeddedRuntimeDraft bind:this={draftEl} {draft} />
        {/key}
      {/if}
    {:else if rowError && !runtimeThreads.get(sessionId)}
      <LoadState what="the thread" error={rowError} empty onretry={() => (attempt += 1)} />
    {:else}
      {#key shown}
        <RuntimeThread
          sessionId={shown}
          focused={focusOnMount}
          {compact}
          {hostCommands}
          {accessModeOverride}
          {onaccessmodechange}
          {fallbackCwd}
          {hasExternalContext}
          transformOutgoingText={outgoing}
          {transformRestoredQueuedText}
          {onExternalContextConsumed}
          {onopenpath}
          {formatAssistantText}
          {assistantFooter}
          {composerTopSlot}
          {composerFootLeadSlot}
          {testIds}
          agentChat={shown === sessionId ? agentName : undefined}
          onswitch={(id) => (picked = id === sessionId ? null : id)}
        />
      {/key}
    {/if}
  </div>
</div>

<style>
  .rt-embed {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    block-size: 100%;
  }
  .rt-embed-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-s);
    flex: none;
  }
  .rt-embed.compact .rt-embed-bar {
    gap: 4px;
    padding: 4px 8px;
  }
  .rt-embed-bar > span {
    flex: 1;
    min-inline-size: 0;
  }
  .rt-embed-body {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
  }
</style>
