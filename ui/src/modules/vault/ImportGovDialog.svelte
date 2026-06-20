<script lang="ts">
  // Import-governed-file dialog: paste AGENTS.md / CLAUDE.md / .cursorrules into
  // a textarea and import the text as a batch of `suggested` memories.
  import { vault } from './vault.svelte';
  import type { GovImportKind } from './vault.svelte';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  let kind = $state<GovImportKind>('agents-md');
  let label = $state('');
  let content = $state('');
  let busy = $state(false);
  let error = $state('');

  async function submit() {
    if (!content.trim()) {
      error = 'Paste some content first.';
      return;
    }
    busy = true;
    error = '';
    try {
      await vault.importGoverned(kind, content.trim(), label.trim() || undefined);
      onclose();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Import failed';
    } finally {
      busy = false;
    }
  }

  const KIND_LABELS: Record<GovImportKind, string> = {
    'agents-md': 'AGENTS.md',
    'claude-md': 'CLAUDE.md',
    cursorrules: '.cursorrules',
    custom: 'Custom',
  };
</script>

<div class="overlay" role="dialog" aria-modal="true" aria-label="Import governed file">
  <div class="dialog">
    <header class="dialog-head">
      <h2>Import governed file</h2>
      <button class="icon-btn" onclick={onclose} aria-label="Close">✕</button>
    </header>

    <div class="dialog-body">
      <label class="field">
        <span class="label">File type</span>
        <select bind:value={kind}>
          {#each Object.entries(KIND_LABELS) as [k, l] (k)}
            <option value={k}>{l}</option>
          {/each}
        </select>
      </label>

      <label class="field">
        <span class="label">Label (optional — e.g. path)</span>
        <input type="text" bind:value={label} placeholder="e.g. /workspace/AGENTS.md" />
      </label>

      <label class="field">
        <span class="label">Content</span>
        <textarea
          bind:value={content}
          rows={14}
          placeholder="Paste AGENTS.md / CLAUDE.md / .cursorrules content here…"
        ></textarea>
      </label>

      {#if error}
        <p class="err">{error}</p>
      {/if}
    </div>

    <footer class="dialog-foot">
      <button onclick={onclose} disabled={busy}>Cancel</button>
      <button class="primary" onclick={submit} disabled={busy || !content.trim()}>
        {busy ? 'Importing…' : 'Import'}
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
    width: min(640px, 94vw);
    display: flex;
    flex-direction: column;
    gap: 0;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  .dialog-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px 12px;
    border-bottom: 1px solid var(--border, #333);
  }
  .dialog-head h2 {
    margin: 0;
    font-size: 15px;
  }
  .dialog-body {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow-y: auto;
    max-height: 62vh;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .label {
    font-size: 11px;
    opacity: 0.7;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  select,
  input,
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 7px 10px;
    border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: var(--surface-2, #1e2330);
    color: var(--text, #ddd);
    font-size: 13px;
    resize: vertical;
  }
  .err {
    color: var(--status-exited, #fa5252);
    font-size: 12px;
    margin: 0;
  }
  .dialog-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid var(--border, #333);
  }
  button:not(.icon-btn):not(.primary) {
    padding: 6px 14px;
    font-size: 13px;
    border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: transparent;
    color: var(--text, #ddd);
    cursor: pointer;
  }
  button.primary {
    padding: 6px 16px;
    font-size: 13px;
    border-radius: 5px;
    border: none;
    background: var(--accent, #4c6ef5);
    color: #fff;
    cursor: pointer;
  }
  button.primary:disabled,
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .icon-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim, #888);
    font-size: 14px;
  }
</style>
