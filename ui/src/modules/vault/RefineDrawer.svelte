<script lang="ts">
  import { toastError } from '../../lib/toastError';
  // "Refine with Otto" — a bottom drawer under the note editor/reading view.
  // One refine session per note (server-side): the first Send spawns it (the
  // POST is LONG — it resolves when the agent's turn completes), and ~800ms
  // after POSTing we poll GET refine-session until the session id lands so the
  // live terminal attaches while the agent is still typing. Once a session
  // exists the provider is locked (follow-up prompts reuse the same session).
  // The drawer is remounted per note ({#key vault.notePath} in NoteView), so
  // on mount we reattach to any session an earlier open of this note started.
  import { onMount } from 'svelte';
  import type { Poller } from '../../lib/poll';
  import { liveQuery } from '../../lib/live';
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import AgentByline from '../../lib/components/AgentByline.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { refineNote, refineSession, resetRefineSession, vaultNote, writeVaultNote } from '../../lib/api/vault';
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

  /** What the last refine turn did to the note — the agent's edit lands on disk
   *  directly, so we keep the pre-turn text to show a summary and offer Undo
   *  (patterns §1: agent output is attributed and reversible). */
  interface RefineResult {
    before: string;
    hash: string;
    added: number;
    removed: number;
    summary: string;
    provider: string;
    at: number;
  }
  let result = $state<RefineResult | null>(null);
  let undoing = $state(false);
  /** A queued "Review + fix" prompt waits for an explicit Send. */
  let queued = $state(false);

  /** Rough line delta (multiset diff) — enough for a "+N / −M lines" summary. */
  function lineDelta(before: string, after: string): { added: number; removed: number } {
    const count = new Map<string, number>();
    for (const l of before.split('\n')) count.set(l, (count.get(l) ?? 0) + 1);
    let added = 0;
    for (const l of after.split('\n')) {
      const n = count.get(l) ?? 0;
      if (n > 0) count.set(l, n - 1);
      else added += 1;
    }
    let removed = 0;
    for (const n of count.values()) removed += n;
    return { added, removed };
  }

  // -- session polling (starts ~800ms after the POST goes out) -----------------
  // Event-fed: the refine agent's session announces itself (`session_created`);
  // a disciplined 1 s poll (in-flight guard, hidden pause) only while the
  // event socket is down, a 10 s safety net otherwise.
  let pollDelay: ReturnType<typeof setTimeout> | null = null;
  let pollTimer: Poller | null = null;

  function stopPolling(): void {
    if (pollDelay) clearTimeout(pollDelay);
    pollTimer?.stop();
    pollDelay = null;
    pollTimer = null;
  }

  function startSessionPoll(): void {
    stopPolling();
    pollDelay = setTimeout(() => {
      pollTimer = liveQuery({ run: () => checkSession(), on: ['session_created'], fallbackMs: 1000, safetyMs: 10_000 });
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
    queued = false;
    result = null;
    const myEpoch = epoch;
    const wsId = vault.wsId;
    const vaultId = vault.current.id;
    // Snapshot the note as it is on disk BEFORE the agent touches it (Undo).
    let before: string | null = null;
    try {
      before = (await vaultNote(wsId, vaultId, path)).raw;
    } catch {
      /* no snapshot → the turn still runs, just without Undo */
    }
    startSessionPoll();
    try {
      const r = await refineNote(wsId, vaultId, { path, prompt: p, provider });
      if (myEpoch !== epoch) return; // reset happened mid-turn — stale result
      sessionId = r.session_id;
      stopPolling();
      prompt = '';
      toasts.success('Refined', (r.reply.split('\n')[0] || 'done').slice(0, 200));
      if (before !== null) {
        try {
          const after = await vaultNote(wsId, vaultId, path);
          if (after.raw !== before) {
            result = {
              before,
              hash: after.meta.hash,
              ...lineDelta(before, after.raw),
              summary: (r.reply.split('\n')[0] || '').slice(0, 200),
              provider,
              at: Date.now(),
            };
          }
        } catch {
          /* summary is best-effort */
        }
      }
      // Reload the agent's changes — but never clobber in-flight local edits.
      if (!vault.dirty && !vault.editing && vault.notePath === path) {
        void vault.open(path);
      }
    } catch (e) {
      if (myEpoch !== epoch) return;
      stopPolling();
      toastError('Couldn’t refine', e);
    } finally {
      if (myEpoch === epoch) sending = false;
    }
  }

  /** Put the pre-turn text back (the agent's edit is the thing being undone). */
  async function undoRefine(): Promise<void> {
    const r = result;
    if (!r || undoing || !vault.current) return;
    if (vault.dirty) {
      toasts.error('Couldn’t undo the refine', 'Save or discard your own edits to this note first.');
      return;
    }
    undoing = true;
    try {
      await writeVaultNote(vault.wsId, vault.current.id, { path, content: r.before, if_hash: r.hash });
      result = null;
      toasts.success('Refine undone', 'The note is back to what it was before the agent’s edit.');
      if (!vault.editing && vault.notePath === path) void vault.open(path);
    } catch (e) {
      toastError('Couldn’t undo the refine', e);
    } finally {
      undoing = false;
    }
  }

  /** Detach the note's session: unblocks a stuck/exited agent — the next Send
   *  starts a FRESH session with whatever provider is selected. */
  async function reset(): Promise<void> {
    if (!vault.current) return;
    // Mid-edit the agent may have half-written the note — say so before
    // detaching its session (an idle session can be dropped without asking).
    if (sending) {
      const ok = await confirmer.ask(
        'The agent is still editing this note. Stopping detaches its session and the edit may be left half-done; Send starts a fresh agent. Your note stays as it is on disk.',
        { title: 'Stop and start over?', confirmLabel: 'Stop and start over', danger: true },
      );
      if (!ok) return;
    }
    epoch += 1;
    stopPolling();
    try {
      await resetRefineSession(vault.wsId, vault.current.id, path);
    } catch (e) {
      toastError('Couldn’t reset', e);
      return;
    }
    sessionId = null;
    sending = false;
    result = null;
  }

  onMount(() => {
    // Reattach an existing refine session for this note (survives drawer close).
    void checkSession();
    // "Review + fix" from the tree: consume the queued prompt but wait for an
    // explicit Send — the agent edits the note on disk, so it never starts on mount.
    const pending = vault.pendingRefine;
    if (pending && pending.path === path) {
      vault.pendingRefine = null;
      prompt = pending.prompt;
      queued = true;
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
    <input dir="auto"
      bind:value={prompt}
      aria-label="Refinement request"
      placeholder="Refine this note… (e.g. tighten the intro, add a troubleshooting section)"
      disabled={sending}
      onkeydown={(e) => {
        if (e.key === 'Enter') void send();
      }}
    />
    <button class="send" disabled={sending || !prompt.trim()} onclick={() => void send()}>
      {#if sending}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span> Refining…{:else}Send{/if}
    </button>
    {#if sessionId || sending}
      <button
        class="btn small"
        title="Detach this note’s agent session — unblocks a stuck or exited agent; the next Send starts a fresh one"
        onclick={() => void reset()}
      >
        <Icon name="refresh" size={12} /> Stop and start over…
      </button>
    {/if}
  </div>

  {#if queued && !sending}
    <div class="notice" role="status">Review + fix is ready — press Send to start. The agent edits this note in place; you can undo it afterwards.</div>
  {/if}
  {#if result}
    <div class="notice result" role="status" data-testid="refine-result">
      <AgentByline provider={result.provider} at={result.at} label="Refined this note" />
      <span class="delta">+{result.added} / −{result.removed} lines{result.summary ? ` · ${result.summary}` : ''}</span>
      <button class="btn small" disabled={undoing} onclick={() => void undoRefine()}>
        {#if undoing}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span> Undoing…{:else}Undo refine{/if}
      </button>
    </div>
  {/if}

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
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .bar select {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
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
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-m);
    padding: 6px 10px;
  }
  .bar input:disabled {
    opacity: 0.6;
  }
  .send {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--accent-solid);
    border: none;
    color: var(--accent-contrast);
    border-radius: var(--radius-s);
    padding: 6px 14px;
    font-size: var(--fs-s);
    cursor: pointer;
    white-space: nowrap;
  }
  .send:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .notice {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 10px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .notice .delta {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    font-size: var(--fs-s);
    line-height: 1.5;
  }
</style>
