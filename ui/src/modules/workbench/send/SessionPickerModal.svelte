<script lang="ts">
  // Pick the live session a workbench file is PASTED into (Send to → Terminal /
  // agent session…). Picking still asks a confirm naming the session; the text
  // goes in as a bracketed paste with no Enter, so nothing executes.
  import Modal from '../../../lib/components/Modal.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ws as wsStore } from '../../../lib/stores/workspace.svelte';
  import type { Session } from '../../../lib/api/types';
  import { pasteTargets } from './sendTo';

  interface Props {
    ws: string;
    onpick: (session: Session) => void;
    onclose: () => void;
  }
  let { ws, onpick, onclose }: Props = $props();

  let query = $state('');
  const all = $derived(pasteTargets(wsStore.sessions, ws));
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return all;
    return all.filter((s) => `${s.title} ${s.provider} ${s.kind} ${s.cwd}`.toLowerCase().includes(q));
  });
</script>

<Modal title="Paste into which session?" width={520} {onclose}>
  <div class="sp" data-testid="wb-session-picker">
    <p class="sp-hint">The file is pasted, never run — you press Enter in the session yourself.</p>
    {#if all.length > 0}
      <input
        class="input sp-filter"
        type="search"
        placeholder="Filter sessions…"
        aria-label="Filter sessions"
        bind:value={query}
      />
    {/if}
    {#if all.length === 0}
      <EmptyState
        variant="panel"
        icon="terminal"
        title="No live sessions"
        body="Start an agent or terminal session in this workspace, then send the file again."
      />
    {:else if shown.length === 0}
      <p class="sp-hint">No session matches “{query}”.</p>
    {:else}
      <ul class="sp-list">
        {#each shown as s (s.id)}
          <li>
            <button class="sp-row" type="button" onclick={() => onpick(s)} title="Paste into {s.title || s.id}">
              <Icon name={s.kind === 'agent' ? 'assistant' : 'terminal'} size={14} />
              <span class="sp-title">{s.title || s.id}</span>
              <span class="sp-meta">{s.provider} · {s.status}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</Modal>

<style>
  .sp {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sp-hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .sp-filter {
    inline-size: 100%;
  }
  .sp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-block-size: min(50vh, 360px);
    overflow-y: auto;
  }
  .sp-row {
    display: flex;
    align-items: center;
    gap: 8px;
    inline-size: 100%;
    padding-block: 6px;
    padding-inline: 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .sp-row:hover,
  .sp-row:focus-visible {
    background: var(--accent-soft);
  }
  .sp-title {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sp-meta {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
</style>
