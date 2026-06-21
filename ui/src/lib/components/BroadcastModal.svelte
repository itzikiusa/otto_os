<script lang="ts">
  // Dedicated broadcast composer. Relays a literal message to the selected live
  // agent sessions via POST /workspaces/{id}/broadcast — NO AI, no parsing, no
  // fallback. Separate from the ⌘K orchestrator on purpose.
  import { api } from '../api/client';
  import type { BroadcastResp, Id, Session } from '../api/types';
  import { ui } from '../stores/ui.svelte';
  import { ws } from '../stores/workspace.svelte';
  import { toasts } from '../toast.svelte';
  import Modal from './Modal.svelte';
  import StatusDot from './StatusDot.svelte';

  let text = $state('');
  let busy = $state(false);
  let textareaEl: HTMLTextAreaElement | null = $state(null);
  // Sessions the user explicitly unticked. Tracking the *negative* set means
  // freshly-spawned sessions are included by default without extra bookkeeping.
  let deselected = $state<Set<Id>>(new Set());

  // Live agent sessions are the only valid broadcast targets (connections and
  // dead/suspended sessions can't receive a prompt).
  const eligible = $derived<Session[]>(
    ws.agentSessions.filter((s) => {
      const st = ws.statusMap[s.id] ?? s.status;
      return st === 'running' || st === 'working' || st === 'idle';
    }),
  );

  const selected = $derived(eligible.filter((s) => !deselected.has(s.id)));
  const allSelected = $derived(eligible.length > 0 && selected.length === eligible.length);

  function toggle(id: Id): void {
    const next = new Set(deselected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    deselected = next;
  }

  function toggleAll(): void {
    deselected = allSelected ? new Set(eligible.map((s) => s.id)) : new Set();
  }

  function close(): void {
    ui.broadcastOpen = false;
    text = '';
    deselected = new Set();
  }

  async function send(): Promise<void> {
    if (busy) return;
    const msg = text.trim();
    if (msg === '') {
      toasts.warn('Nothing to broadcast', 'Type a message first.');
      return;
    }
    if (!ws.currentId) {
      toasts.error('No workspace selected', 'Pick a workspace first.');
      return;
    }
    const ids = selected.map((s) => s.id);
    if (ids.length === 0) {
      toasts.warn('No sessions selected', 'Pick at least one session to broadcast to.');
      return;
    }
    busy = true;
    try {
      const resp = await api.post<BroadcastResp>(`/workspaces/${ws.currentId}/broadcast`, {
        text: msg,
        session_ids: ids,
      });
      const n = resp.session_ids.length;
      if (n === 0) toasts.warn('Broadcast sent to nobody', 'No selected session was live.');
      else toasts.success('Broadcast sent', `Delivered to ${n} session${n === 1 ? '' : 's'}`);
      close();
    } catch (e) {
      toasts.error('Broadcast failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent): void {
    // Enter sends; Shift+Enter inserts a newline; ⌘/Ctrl+Enter also sends.
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  $effect(() => {
    if (ui.broadcastOpen) queueMicrotask(() => textareaEl?.focus());
  });
</script>

<Modal title="Broadcast message" width={520} onclose={close}>
  <div class="bc">
    <textarea
      bind:this={textareaEl}
      bind:value={text}
      placeholder="Message to send to the selected sessions…  (Enter to send, ⇧Enter for newline)"
      rows="3"
      spellcheck="false"
      onkeydown={onKey}
    ></textarea>

    <div class="bc-list-head">
      <span class="bc-label">Sessions</span>
      <span class="grow"></span>
      {#if eligible.length > 0}
        <button class="bc-link" onclick={toggleAll}>
          {allSelected ? 'Select none' : 'Select all'}
        </button>
      {/if}
    </div>

    <div class="bc-list">
      {#each eligible as s (s.id)}
        <label class="bc-row" class:on={!deselected.has(s.id)}>
          <input
            type="checkbox"
            checked={!deselected.has(s.id)}
            onchange={() => toggle(s.id)}
          />
          <StatusDot status={ws.statusMap[s.id] ?? s.status} size={7} />
          <span class="bc-title">{s.title}</span>
          <span class="bc-provider">{s.provider}</span>
        </label>
      {:else}
        <div class="bc-empty">No live agent sessions to broadcast to.</div>
      {/each}
    </div>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={close}>Cancel</button>
    <button
      class="btn primary"
      disabled={busy || selected.length === 0 || text.trim() === ''}
      onclick={send}
    >
      {busy ? 'Sending…' : `Broadcast to ${selected.length}`}
    </button>
  {/snippet}
</Modal>

<style>
  .bc {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    padding: 9px 11px;
    font-size: 13px;
    line-height: 1.5;
    resize: vertical;
    color: var(--text);
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent);
  }
  .bc-list-head {
    display: flex;
    align-items: center;
  }
  .bc-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .bc-link {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 12px;
    cursor: pointer;
    padding: 2px 4px;
  }
  .bc-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 320px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 4px;
  }
  .bc-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    cursor: pointer;
  }
  .bc-row:hover {
    background: var(--surface-2);
  }
  .bc-row.on {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .bc-row input {
    accent-color: var(--accent);
    cursor: pointer;
  }
  .bc-title {
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bc-provider {
    margin-inline-start: auto;
    font-size: 10.5px;
    color: var(--text-dim);
    text-transform: lowercase;
  }
  .bc-empty {
    padding: 16px;
    text-align: center;
    font-size: 12px;
    color: var(--text-dim);
  }
</style>
