<script lang="ts">
  // Dialog to compose a merged memory from the selected-for-merge set.
  // The user fills in the merged title/body (pre-filled with the first selected
  // memory's title, if available).
  import { untrack } from 'svelte';
  import { vault } from './vault.svelte';
  import type { Memory } from '../../lib/api/types';

  interface Props {
    sources: Memory[];
    onclose: () => void;
  }
  let { sources, onclose }: Props = $props();

  let title = $state(untrack(() => sources[0]?.title ?? ''));
  let body = $state(untrack(() => sources.map((m) => m.body).join('\n\n---\n\n')));
  let busy = $state(false);
  let error = $state('');

  async function submit() {
    if (!title.trim() || !body.trim()) {
      error = 'Title and body are required.';
      return;
    }
    busy = true;
    error = '';
    try {
      await vault.executeMerge(title.trim(), body.trim());
      onclose();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Merge failed';
    } finally {
      busy = false;
    }
  }
</script>

<div class="overlay" role="dialog" aria-modal="true" aria-label="Merge memories">
  <div class="dialog">
    <header class="dialog-head">
      <h2>Merge {sources.length} memories</h2>
      <button class="icon-btn" onclick={onclose} aria-label="Close">✕</button>
    </header>

    <div class="dialog-body">
      <p class="hint">
        Sources: {sources.map((m) => m.title).join(', ')}
      </p>

      <label class="field">
        <span class="label">Title of merged memory</span>
        <input type="text" bind:value={title} />
      </label>

      <label class="field">
        <span class="label">Body (edit to summarise)</span>
        <textarea bind:value={body} rows={10} style:resize="vertical"></textarea>
      </label>

      {#if error}
        <p class="err">{error}</p>
      {/if}
    </div>

    <footer class="dialog-foot">
      <button onclick={onclose} disabled={busy}>Cancel</button>
      <button class="primary" onclick={submit} disabled={busy}>
        {busy ? 'Merging…' : 'Merge'}
      </button>
    </footer>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
  }
  .dialog {
    background: var(--surface, #1a1a1a);
    border: 1px solid var(--border, #333);
    border-radius: 8px;
    width: min(600px, 94vw);
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  .dialog-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px 12px;
    border-bottom: 1px solid var(--border, #333);
  }
  .dialog-head h2 { margin: 0; font-size: 15px; }
  .dialog-body {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow-y: auto;
    max-height: 60vh;
  }
  .hint { font-size: 11px; opacity: 0.6; margin: 0; }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .label { font-size: 11px; opacity: 0.7; text-transform: uppercase; letter-spacing: 0.04em; }
  input, textarea {
    width: 100%; box-sizing: border-box;
    padding: 7px 10px; border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: var(--surface-2, #1e2330);
    color: var(--text, #ddd); font-size: 13px;
  }
  .err { color: var(--status-exited, #fa5252); font-size: 12px; margin: 0; }
  .dialog-foot {
    display: flex; justify-content: flex-end;
    gap: 8px; padding: 12px 16px;
    border-top: 1px solid var(--border, #333);
  }
  button:not(.icon-btn):not(.primary) {
    padding: 6px 14px; font-size: 13px; border-radius: 5px;
    border: 1px solid var(--border, #444); background: transparent;
    color: var(--text, #ddd); cursor: pointer;
  }
  button.primary {
    padding: 6px 16px; font-size: 13px; border-radius: 5px;
    border: none; background: var(--accent, #4c6ef5);
    color: #fff; cursor: pointer;
  }
  button:disabled { opacity: 0.45; cursor: default; }
  .icon-btn { background: none; border: none; cursor: pointer; color: var(--text-dim, #888); font-size: 14px; }
</style>
