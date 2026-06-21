<script lang="ts">
  // Dialog to split one memory into N parts.
  // Starts with 2 parts; user can add more.
  import { vault } from './vault.svelte';
  import type { SplitPart } from './vault.svelte';
  import type { Memory } from '../../lib/api/types';

  interface Props {
    source: Memory;
    onclose: () => void;
  }
  let { source, onclose }: Props = $props();

  // Seed two parts from the source body (rough half-split).
  function seedParts(): SplitPart[] {
    const body = source.body.trim();
    const mid = Math.floor(body.length / 2);
    const cut = body.indexOf('\n', mid);
    const split = cut > 0 ? cut : mid;
    return [
      { title: source.title + ' (part 1)', body: body.slice(0, split).trim() },
      { title: source.title + ' (part 2)', body: body.slice(split).trim() },
    ];
  }

  let parts = $state<SplitPart[]>(seedParts());
  let busy = $state(false);
  let error = $state('');

  function addPart() {
    parts = [...parts, { title: `${source.title} (part ${parts.length + 1})`, body: '' }];
  }
  function removePart(i: number) {
    if (parts.length <= 2) return;
    parts = parts.filter((_, idx) => idx !== i);
  }

  async function submit() {
    if (parts.some((p) => !p.title.trim() || !p.body.trim())) {
      error = 'All parts need a title and body.';
      return;
    }
    busy = true;
    error = '';
    try {
      await vault.executeSplit(parts);
      onclose();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : 'Split failed';
    } finally {
      busy = false;
    }
  }
</script>

<div class="overlay" role="dialog" aria-modal="true" aria-label="Split memory">
  <div class="dialog">
    <header class="dialog-head">
      <h2>Split memory</h2>
      <button class="icon-btn" onclick={onclose} aria-label="Close">✕</button>
    </header>

    <div class="dialog-body">
      <p class="hint">Splitting: <em>{source.title}</em></p>

      {#each parts as part, i (i)}
        <div class="part-card">
          <div class="part-head">
            <span class="part-no">Part {i + 1}</span>
            {#if parts.length > 2}
              <button class="rm-btn" onclick={() => removePart(i)} aria-label="Remove part">✕</button>
            {/if}
          </div>
          <label class="field">
            <span class="label">Title</span>
            <input type="text" bind:value={part.title} />
          </label>
          <label class="field">
            <span class="label">Body</span>
            <textarea bind:value={part.body} rows={5}></textarea>
          </label>
        </div>
      {/each}

      <button class="add-btn" onclick={addPart}>+ Add part</button>

      {#if error}
        <p class="err">{error}</p>
      {/if}
    </div>

    <footer class="dialog-foot">
      <button onclick={onclose} disabled={busy}>Cancel</button>
      <button class="primary" onclick={submit} disabled={busy}>
        {busy ? 'Splitting…' : `Split into ${parts.length}`}
      </button>
    </footer>
  </div>
</div>

<style>
  .overlay {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex; align-items: center; justify-content: center;
    z-index: 200;
  }
  .dialog {
    background: var(--surface, #1a1a1a);
    border: 1px solid var(--border, #333);
    border-radius: 8px; width: min(640px, 94vw);
    display: flex; flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  .dialog-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 14px 16px 12px; border-bottom: 1px solid var(--border, #333);
  }
  .dialog-head h2 { margin: 0; font-size: 15px; }
  .dialog-body {
    padding: 16px; display: flex; flex-direction: column;
    gap: 14px; overflow-y: auto; max-height: 65vh;
  }
  .hint { font-size: 11px; opacity: 0.6; margin: 0; }
  .part-card {
    border: 1px solid var(--border, #333);
    border-radius: 6px; padding: 10px 12px;
    display: flex; flex-direction: column; gap: 8px;
  }
  .part-head {
    display: flex; align-items: center; justify-content: space-between;
  }
  .part-no { font-size: 11px; font-weight: 600; opacity: 0.7; text-transform: uppercase; }
  .rm-btn { background: none; border: none; cursor: pointer; color: var(--text-dim, #888); font-size: 12px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .label { font-size: 11px; opacity: 0.7; text-transform: uppercase; letter-spacing: 0.04em; }
  input, textarea {
    width: 100%; box-sizing: border-box;
    padding: 6px 9px; border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: var(--surface-2, #1e2330);
    color: var(--text, #ddd); font-size: 13px; resize: vertical;
  }
  .add-btn {
    align-self: flex-start; font-size: 12px; padding: 5px 12px;
    border-radius: 5px; border: 1px dashed var(--border, #555);
    background: transparent; color: var(--text-dim, #aaa); cursor: pointer;
  }
  .add-btn:hover { border-color: var(--accent, #4c6ef5); color: var(--accent, #4c6ef5); }
  .err { color: var(--status-exited, #fa5252); font-size: 12px; margin: 0; }
  .dialog-foot {
    display: flex; justify-content: flex-end;
    gap: 8px; padding: 12px 16px;
    border-top: 1px solid var(--border, #333);
  }
  button:not(.icon-btn):not(.primary):not(.add-btn):not(.rm-btn) {
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
