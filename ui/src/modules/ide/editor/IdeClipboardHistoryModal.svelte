<script lang="ts">
  // JetBrains `⌘⇧V` ("Choose Content to Paste") Clipboard History Modal + "Compare with Current File".
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { clearIdeClipboardHistory, readIdeClipboardHistory, type IdeClipboardEntry } from '../../../lib/components/cm-ide-history';

  interface Props {
    filePath: string;
    readOnly: boolean;
    onpaste: (text: string) => void;
    onclose: () => void;
  }

  let { filePath, readOnly, onpaste, onclose }: Props = $props();
  let query = $state('');
  let entries = $state<IdeClipboardEntry[]>(readIdeClipboardHistory());
  let selectedIdx = $state(0);

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) => e.text.toLowerCase().includes(q) || (e.filePath ?? '').toLowerCase().includes(q));
  });

  const activeEntry = $derived(filtered[Math.min(selectedIdx, Math.max(0, filtered.length - 1))] ?? null);

  function pasteSelected(item: IdeClipboardEntry | null): void {
    if (!item || readOnly) return;
    onpaste(item.text);
    onclose();
  }

  function compareSelected(item: IdeClipboardEntry | null): void {
    if (!item) return;
    window.dispatchEvent(
      new CustomEvent('otto:ide-open-compare', {
        detail: { mode: 'clipboard', path: filePath, clipboardText: item.text, leftContent: item.text, leftLabel: 'Clipboard' },
      }),
    );
    onclose();
  }

  function handleClear(): void {
    clearIdeClipboardHistory();
    entries = [];
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') { e.preventDefault(); selectedIdx = (selectedIdx + 1) % Math.max(1, filtered.length); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); selectedIdx = (selectedIdx - 1 + filtered.length) % Math.max(1, filtered.length); }
    else if (e.key === 'Enter' && activeEntry && !readOnly) { e.preventDefault(); pasteSelected(activeEntry); }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<Modal title="Paste from Clipboard History" width={640} {onclose}>
  {#snippet children()}
    <div class="ide-clip-modal" data-testid="ide-clipboard-history-popover">
      <div class="ide-clip-top">
        <input
          class="ide-clip-search"
          type="search"
          dir="ltr"
          aria-label="Filter clipboard history"
          placeholder="Search copied snippets…"
          bind:value={query}
        />
        {#if entries.length > 0}
          <button type="button" class="btn small ghost" onclick={handleClear}>Clear history</button>
        {/if}
      </div>

      {#if filtered.length === 0}
        <EmptyState icon="copy" title="No clipboard entries" body="Copy or cut code in the IDE (⌘C / ⌘X) to build your 30-item clipboard ring." />
      {:else}
        <div class="ide-clip-body">
          <div class="ide-clip-list" role="listbox" aria-label="Clipboard ring">
            {#each filtered as item, idx (item.id)}
              <button
                type="button"
                role="option"
                aria-selected={idx === selectedIdx}
                class="ide-clip-row"
                class:active={idx === selectedIdx}
                onclick={() => (selectedIdx = idx)}
                ondblclick={() => pasteSelected(item)}
                data-testid="ide-clipboard-item"
              >
                <span class="ide-clip-idx">{idx + 1}</span>
                <span class="ide-clip-prev" dir="ltr">{item.preview}</span>
                <span class="ide-clip-lines">{item.lineCount}L</span>
              </button>
            {/each}
          </div>

          {#if activeEntry}
            <pre class="ide-clip-code" dir="ltr">{activeEntry.text}</pre>
          {/if}
        </div>
      {/if}
    </div>
  {/snippet}

  {#snippet footer()}
    <button type="button" class="btn ghost" onclick={onclose}>Cancel</button>
    <button type="button" class="btn secondary" disabled={!activeEntry || !filePath} onclick={() => compareSelected(activeEntry)} data-testid="ide-clipboard-compare-btn">
      <Icon name="split" size={13} />Compare with File
    </button>
    <button type="button" class="btn primary" disabled={!activeEntry || readOnly} onclick={() => pasteSelected(activeEntry)} data-testid="ide-clipboard-paste-btn">
      Paste
    </button>
  {/snippet}
</Modal>

<style>
  .ide-clip-modal { display: flex; flex-direction: column; gap: var(--sp-3); min-block-size: 280px; }
  .ide-clip-top { display: flex; align-items: center; gap: var(--sp-2); }
  .ide-clip-search {
    flex: 1; block-size: 28px; padding-inline: var(--sp-3);
    background: var(--bg); color: var(--text); border: 1px solid var(--control-border);
    border-radius: var(--radius-s); font-size: var(--fs-xs);
  }
  .ide-clip-body { display: flex; gap: var(--sp-3); block-size: 260px; min-block-size: 0; }
  .ide-clip-list { inline-size: 250px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; border: 1px solid var(--border); border-radius: var(--radius-s); padding: var(--sp-1); }
  .ide-clip-row {
    display: flex; align-items: center; gap: var(--sp-2); padding: 4px var(--sp-2);
    background: transparent; border: none; border-radius: var(--radius-s);
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .ide-clip-row.active, .ide-clip-row:hover { background: var(--accent-soft); color: var(--accent-text); }
  .ide-clip-idx { font-family: var(--font-mono); color: var(--text-dim); inline-size: 18px; flex-shrink: 0; }
  .ide-clip-prev { flex: 1; min-inline-size: 0; font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ide-clip-lines { font-family: var(--font-mono); color: var(--text-dim); flex-shrink: 0; }
  .ide-clip-code {
    flex: 1; margin: 0; padding: var(--sp-3); overflow: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius-s);
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text);
  }
</style>
