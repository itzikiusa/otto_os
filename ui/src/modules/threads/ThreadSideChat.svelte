<script lang="ts">
  // The right panel's "Side chat" tab: ask about the focused thread without
  // interrupting it. Empty: one question box. Running: the side conversation
  // (its own composer for follow-ups) with two quiet actions — open it as a
  // normal thread, or dismiss it. Loaded lazily by ThreadInspector.
  import { enterSends } from '../../lib/enterSends';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import ConversationView from '../agents/conversation/ConversationView.svelte'; // ui-guards: allow — the side chat renders the shared conversation
  import { dismissSideChat, promoteSideChat, sideChatFor, startSideChat } from './threadSideChat';
  import { isPlainShellSession } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';
  import { asThreadSession, runtimeThreads } from './runtime/runtimeThreads.svelte';

  interface Props {
    mainId: string | null;
  }
  let { mainId }: Props = $props();

  const main = $derived.by(() => {
    const rt = runtimeThreads.get(mainId);
    return (mainId ? ws.getSession(mainId) : null) ?? (rt ? asThreadSession(rt) : null);
  });
  const side = $derived(sideChatFor(mainId));
  let question = $state('');
  let busy = $state(false);

  async function ask(q: string): Promise<void> {
    if (!main || busy || !q.trim()) return;
    busy = true;
    try {
      const created = await startSideChat(main, q);
      if (created) question = '';
    } finally {
      busy = false;
    }
  }

  // A `/btw <question>` from the main composer: ask it here (or, when a side
  // chat already runs, send it as its next message).
  $effect(() => {
    const req = threads.sideAsk;
    if (!req || req.mainId !== mainId || !main) return;
    threads.sideAsk = null;
    if (side) void ws.sendInput(side.id, req.question);
    else void ask(req.question);
  });

  function onKey(e: KeyboardEvent): void {
    if (enterSends(e)) {
      e.preventDefault();
      void ask(question);
    }
  }
</script>

<div class="sc-wrap" data-testid="threads-side-chat">
  {#if !main || isPlainShellSession(main)}
    <EmptyState
      variant="panel"
      icon="comment"
      title="No thread to ask about"
      body="Open an agent thread, then ask a side question here or with /btw in its composer."
    />
  {:else if side}
    <div class="sc-bar">
      <span class="sc-about ellipsis" dir="auto" title="Side chat about {main.title}">About {main.title}</span>
      <button
        type="button"
        class="icon-btn"
        aria-label="Open the side chat as a thread"
        title="Open as a thread (beside this one)"
        data-testid="side-chat-promote"
        onclick={() => void promoteSideChat(side)}
      >
        <Icon name="split" size={13} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Dismiss the side chat"
        title="Dismiss (archives the side chat)"
        data-testid="side-chat-dismiss"
        onclick={() => void dismissSideChat(side)}
      >
        <Icon name="x" size={13} />
      </button>
    </div>
    <div class="sc-conv">
      {#key side.id}
        <ConversationView sessionId={side.id} workspaceId={side.workspace_id} surface="threads" />
      {/key}
    </div>
  {:else}
    <div class="sc-ask">
      <p class="sc-lead">
        Ask about <strong dir="auto">{main.title}</strong> without interrupting it. The side chat gets the thread’s
        recent turns, folder, branch and changed files, and runs read-only.
      </p>
      <textarea
        bind:value={question}
        dir="auto"
        rows={3}
        placeholder="Ask a side question…"
        aria-label="Side question about this thread"
        data-testid="side-chat-input"
        disabled={busy}
        onkeydown={onKey}
      ></textarea>
      <div class="sc-row">
        <button
          type="button"
          class="btn small primary"
          disabled={busy || !question.trim()}
          data-testid="side-chat-send"
          onclick={() => void ask(question)}
        >
          {busy ? 'Starting…' : 'Ask'}
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .sc-wrap {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .sc-bar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: 4px;
    padding-inline: 10px 6px;
    flex-shrink: 0;
  }
  .sc-about {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .sc-conv {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .sc-ask {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
  }
  .sc-lead {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .sc-lead strong {
    font-weight: 500;
    color: var(--text);
  }
  .sc-ask textarea {
    inline-size: 100%;
    resize: vertical;
    min-block-size: 64px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
  }
  .sc-ask textarea:focus-visible {
    outline: none;
    border-color: var(--accent-line-strong);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .sc-row {
    display: flex;
    justify-content: flex-end;
  }
</style>
