<script lang="ts">
  // "Refine with AI" — a bottom drawer under the note editor/reading view.
  // One refine session per note (server-side): the first Send spawns it (the
  // POST is LONG — it resolves when the agent's turn completes), and ~800ms
  // after POSTing we poll GET refine-session until the session id lands so the
  // live terminal attaches while the agent is still typing. Once a session
  // exists the provider is locked (follow-up prompts reuse the same session).
  // The drawer is remounted per note ({#key vault.notePath} in NoteView), so
  // on mount we reattach to any session an earlier open of this note started.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { refineNote, refineSession, resetRefineSession } from '../../lib/api/vault';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { toasts } from '../../lib/toast.svelte';
  import { vault } from './vault.svelte';

  let { path }: { path: string } = $props();

  let provider = $state(defaultAgentProvider());
  let prompt = $state('');
  let sending = $state(false);
  let sessionId = $state<string | null>(null);
  /** Bumped on reset — a long-running send from a previous epoch must not
   *  re-apply its (stale) session/state when it finally resolves. */
  let epoch = 0;

  const providers = $derived(agentProviders());

  // -- session polling (starts ~800ms after the POST goes out) -----------------
  let pollDelay: ReturnType<typeof setTimeout> | null = null;
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  function stopPolling(): void {
    if (pollDelay) clearTimeout(pollDelay);
    if (pollTimer) clearInterval(pollTimer);
    pollDelay = null;
    pollTimer = null;
  }

  function startSessionPoll(): void {
    stopPolling();
    pollDelay = setTimeout(() => {
      pollTimer = setInterval(() => void checkSession(), 1000);
    }, 800);
  }

  async function checkSession(): Promise<void> {
    if (!vault.current) return;
    const myEpoch = epoch;
    try {
      const s = await refineSession(vault.wsId, vault.current.id, path);
      if (myEpoch !== epoch) return; // reset raced this poll
      if (s.session_id) {
        sessionId = s.session_id;
        stopPolling();
      }
    } catch {
      /* transient — next tick retries */
    }
  }

  async function send(): Promise<void> {
    const p = prompt.trim();
    if (!p || sending || !vault.current) return;
    sending = true;
    const myEpoch = epoch;
    startSessionPoll();
    try {
      const r = await refineNote(vault.wsId, vault.current.id, { path, prompt: p, provider });
      if (myEpoch !== epoch) return; // reset happened mid-turn — stale result
      sessionId = r.session_id;
      stopPolling();
      prompt = '';
      toasts.success('Refined', (r.reply.split('\n')[0] || 'done').slice(0, 200));
      // Reload the agent's changes — but never clobber in-flight local edits.
      if (!vault.dirty && !vault.editing && vault.notePath === path) {
        void vault.open(path);
      }
    } catch (e) {
      if (myEpoch !== epoch) return;
      stopPolling();
      toasts.error('Refine failed', e instanceof Error ? e.message : String(e));
    } finally {
      if (myEpoch === epoch) sending = false;
    }
  }

  /** Detach the note's session: unblocks a stuck/exited agent — the next Send
   *  starts a FRESH session with whatever provider is selected. */
  async function reset(): Promise<void> {
    if (!vault.current) return;
    epoch += 1;
    stopPolling();
    try {
      await resetRefineSession(vault.wsId, vault.current.id, path);
    } catch (e) {
      toasts.error('Reset failed', e instanceof Error ? e.message : String(e));
      return;
    }
    sessionId = null;
    sending = false;
  }

  onMount(() => {
    // Reattach an existing refine session for this note (survives drawer close).
    void checkSession();
    // "Review + fix" from the tree: consume the queued prompt and fire it.
    const pending = vault.pendingRefine;
    if (pending && pending.path === path) {
      vault.pendingRefine = null;
      prompt = pending.prompt;
      void send();
    }
    return () => stopPolling();
  });
</script>

<div class="refine-drawer">
  <div class="bar">
    <span class="spark"><Icon name="zap" size={13} /></span>
    <select
      bind:value={provider}
      disabled={sending}
      title={sessionId
        ? 'Picking a different provider starts a fresh agent session on the next Send'
        : 'Agent provider'}
    >
      {#each providers as p (p)}
        <option value={p}>{p}</option>
      {/each}
    </select>
    <input
      bind:value={prompt}
      placeholder="Refine this note… (e.g. tighten the intro, add a troubleshooting section)"
      disabled={sending}
      onkeydown={(e) => {
        if (e.key === 'Enter') void send();
      }}
    />
    <button class="send" disabled={sending || !prompt.trim()} onclick={() => void send()}>
      {#if sending}<span class="spinner-xs"></span> Working…{:else}Send{/if}
    </button>
    {#if sessionId || sending}
      <button
        class="reset"
        title="Detach this note’s agent session — unblocks a stuck or exited agent; the next Send starts a fresh one"
        onclick={() => void reset()}
      >
        <Icon name="refresh" size={12} /> New agent
      </button>
    {/if}
  </div>

  {#if sessionId}
    <div class="term">
      {#key sessionId}
        <Terminal sessionId={sessionId} preferDom />
      {/key}
    </div>
  {:else}
    <div class="placeholder">
      {sending
        ? 'Starting the agent — its terminal will attach here…'
        : 'The agent works in a live session on this note; its terminal appears here after the first prompt.'}
    </div>
  {/if}
</div>

<style>
  .refine-drawer {
    flex: 0 0 40%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .spark {
    display: inline-flex;
    color: var(--accent, #9ab4ff);
    flex-shrink: 0;
  }
  .bar select {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 12px;
    padding: 6px 8px;
    flex: 0 0 auto;
  }
  .bar select:disabled {
    opacity: 0.6;
  }
  .bar input {
    flex: 1;
    min-width: 0;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 13px;
    padding: 6px 10px;
  }
  .bar input:disabled {
    opacity: 0.6;
  }
  .send {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--accent, #4c6fff);
    border: none;
    color: var(--accent-contrast, #fff);
    border-radius: 7px;
    padding: 6px 14px;
    font-size: 12.5px;
    cursor: pointer;
    white-space: nowrap;
  }
  .send:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .reset {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-dim);
    border-radius: 7px;
    padding: 6px 10px;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
  }
  .reset:hover {
    color: var(--text);
    border-color: var(--text-dim);
  }
  .term {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    overscroll-behavior: contain;
  }
  .placeholder {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px 24px;
    text-align: center;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.5;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
