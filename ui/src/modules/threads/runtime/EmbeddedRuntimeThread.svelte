<!--
  One structured-runtime thread hosted outside Threads (IDE AI Agent dock,
  Personal Agents' chat, Pull Requests, Connections), through
  lib/runtimeThreadPort.ts or direct IDE embed. Reads only its own row and its
  linked group, keeps them live while mounted, and switches cleanly between the
  lead thread and any delegated child threads.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { EMBED_DEFAULTS, openInThreads, type RuntimeThreadViewProps } from '../../../lib/runtimeThreadPort';
  import { embedStatus } from './embedStatus';
  import RuntimeThread from './RuntimeThread.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { isRunning } from './timeline';

  let {
    sessionId,
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
  const chrome = $derived(linked || (!hideEmbedBarWhenUnlinked && (compact || !!onmaximize || !!ondetach)));
  const detach = (): void => (ondetach ?? openInThreads)(shown);

  onMount(() => runtimeThreads.mount());

  const status = $derived.by(() => {
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
    return untrack(() => runtimeThreads.watch(id));
  });
  $effect(() => {
    const id = sessionId;
    void attempt;
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
    {#if rowError && !runtimeThreads.get(sessionId)}
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
          {transformOutgoingText}
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
