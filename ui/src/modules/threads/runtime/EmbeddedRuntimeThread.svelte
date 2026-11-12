<!--
  One structured-runtime thread hosted outside Threads (a personal agent's
  chat; a pull request's or a connection's column), through
  lib/runtimeThreadPort.ts — the contract (props, callbacks, host commands)
  is documented there. Reads only its own row and its linked group, keeps
  them live while mounted, and never takes focus unless asked. The
  linked-thread chips switch this view between the lead and the agents it
  delegated to; a bar then offers "Back" and "Open in Threads".
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { EMBED_DEFAULTS, openInThreads, type RuntimeThreadViewProps } from '../../../lib/runtimeThreadPort';
  import RuntimeThread from './RuntimeThread.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let {
    sessionId,
    agentName,
    focusOnMount = EMBED_DEFAULTS.focusOnMount,
    compact = EMBED_DEFAULTS.compact,
    onmaximize,
    ondetach,
    hostCommands,
  }: RuntimeThreadViewProps = $props();

  /** The thread shown: the host's, or a linked one picked from the chips. */
  let picked = $state<string | null>(null);
  const shown = $derived(picked && runtimeThreads.group(sessionId).some((t) => t.id === picked) ? picked : sessionId);
  const current = $derived(runtimeThreads.get(shown));
  let rowError = $state<string | null>(null);
  let attempt = $state(0);
  /** The bar: a linked thread is shown, or the host asked for embed chrome. */
  const linked = $derived(shown !== sessionId);
  const chrome = $derived(linked || compact || !!onmaximize || !!ondetach);
  const detach = (): void => (ondetach ?? openInThreads)(shown);

  onMount(() => runtimeThreads.mount());
  $effect(() => {
    const id = shown;
    return untrack(() => runtimeThreads.watch(id));
  });
  $effect(() => {
    const id = sessionId;
    void attempt;
    // Only this thread's row (a freshly created chat may not be held yet)
    // and its linked group — never the workspace's whole thread list.
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
