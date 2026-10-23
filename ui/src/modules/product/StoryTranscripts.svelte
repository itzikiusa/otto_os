<script lang="ts">
  // The draft story's transcripts (Overview, right column): the paged list
  // with lazily loaded bodies, ⌘F over every transcript (collapsed ones too),
  // download, remove and the add form. Split out of OverviewTab
  // (architecture-5-04); OverviewTab is keyed by story, so a fresh instance —
  // and fresh form/expansion state — per story.
  import { announceShown } from '../../lib/announce.svelte';
  import { tick, untrack } from 'svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { toastError } from '../../lib/toastError';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { registerFindProvider } from '../../lib/findProviders';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ProductTranscriptSummary } from './types';

  // ── Transcript state ──────────────────────────────────────────────────────
  let newTranscriptTitle = $state('');
  let newTranscriptBody = $state('');
  let addingTranscript = $state(false);
  let expandedTranscripts = $state<Record<string, boolean>>({});

  // First page once per mounted story.
  $effect(() => {
    untrack(() => void product.loadTranscripts());
  });

  async function doAddTranscript(): Promise<void> {
    if (!newTranscriptBody.trim()) return;
    addingTranscript = true;
    try {
      await product.addTranscript({
        title: newTranscriptTitle.trim() || null,
        body: newTranscriptBody.trim(),
      });
      newTranscriptTitle = '';
      newTranscriptBody = '';
      toasts.success('Transcript added');
    } catch (e) {
      toastError('Couldn’t add transcript', e);
    } finally {
      addingTranscript = false;
    }
  }

  async function doDeleteTranscript(t: ProductTranscriptSummary): Promise<void> {
    const ok = await confirmer.ask(
      `Remove transcript “${t.title || 'untitled'}”?`,
      { title: 'Remove transcript', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      await product.deleteTranscript(t.id);
      toasts.info('Transcript removed');
    } catch (e) {
      toastError('Couldn’t remove the transcript', e);
    }
  }

  async function toggleTranscript(id: string): Promise<void> {
    expandedTranscripts = { ...expandedTranscripts, [id]: !expandedTranscripts[id] };
    if (expandedTranscripts[id]) await product.loadTranscriptBody(id);
  }

  async function downloadTranscript(t: ProductTranscriptSummary): Promise<void> {
    try {
      const full = await product.getTranscript(t.id);
      const url = URL.createObjectURL(new Blob([full.body], { type: 'text/plain;charset=utf-8' }));
      const anchor = document.createElement('a');
      anchor.href = url; anchor.download = `${t.title || 'transcript'}.txt`; anchor.click();
      setTimeout(() => URL.revokeObjectURL(url), 0);
    } catch (e) { toasts.error('Couldn’t download transcript', product.errMsg(e)); }
  }

  // ⌘F reaches COLLAPSED transcripts too: their bodies aren't mounted, so the
  // page-wide find overlay's DOM walk never saw them. This provider
  // (lib/findProviders.ts) searches title + body of every transcript; going to
  // a match in a collapsed one expands it. The date is `data-find-skip` in the
  // row, so the row's text order is title → body, matching `text(i)`.
  let transcriptListEl = $state<HTMLElement | undefined>();
  $effect(() => {
    if (!transcriptListEl) return;
    const list = transcriptListEl;
    let revealGeneration = 0;
    return registerFindProvider({
      root: () => list,
      count: () => product.transcriptSearchRows.length,
      search: (query, limit, signal) => { revealGeneration++; return product.searchTranscripts(query, limit, signal); },
      text: (i) => {
        const t = product.transcriptSearchRows[i];
        return t ? `${t.title || 'Untitled transcript'}\n${product.transcriptBodies[t.id] ?? ''}` : '';
      },
      reveal: async (i, signal) => {
        const t = product.transcriptSearchRows[i], owner = product.selectedId;
        const generation = revealGeneration;
        if (!t || signal?.aborted) return;
        await product.revealTranscript(t);
        if (signal?.aborted || generation !== revealGeneration || product.selectedId !== owner) return;
        expandedTranscripts = { ...expandedTranscripts, [t.id]: true };
        await tick();
      },
      // Only an expanded row has its body mounted; a collapsed one → reveal.
      rowElement: (i) => {
        const t = product.transcriptSearchRows[i];
        return t && expandedTranscripts[t.id] && product.transcriptBodies[t.id] !== undefined
          ? list.querySelector(`[data-transcript-id="${CSS.escape(t.id)}"]`) : null;
      },
      release: () => { revealGeneration++; product.releaseTranscriptSearch(); },
    });
  });
</script>

<div class="transcripts-section">
  <div class="transcripts-header">
    <span class="section-title">Transcripts</span>
  </div>
  <div class="muted" data-find-skip>⌘F searches all imported transcripts, including older pages. Download keeps the complete text available.</div>

  {#if product.transcriptsError}
    <div use:announceShown={{ text: `Couldn’t load transcripts. ${product.transcriptsError}`, assertive: true }}>Couldn’t load transcripts. {product.transcriptsError}</div>
    <button class="btn small" onclick={() => void product.loadTranscripts()}>Retry</button>
  {/if}
  {#if product.loadingTranscripts}
    <LoadState what="transcripts" variant="compact" loading empty />
  {:else if product.transcripts.length === 0 && !product.transcriptsError}
    <div class="muted">No transcripts yet. Paste a conversation below.</div>
  {:else}
    <div class="transcript-list" bind:this={transcriptListEl}>
      {#each product.transcripts as t (t.id)}
        <div class="transcript-item" data-transcript-id={t.id}>
          <div class="transcript-header">
            <button
              class="transcript-toggle"
              onclick={() => toggleTranscript(t.id)}
              aria-expanded={expandedTranscripts[t.id] ?? false}
            >
              <span class="coll-arrow" aria-hidden="true"><Icon name={expandedTranscripts[t.id] ? 'chevronDown' : 'chevronRight'} size={12} /></span>
              <span class="transcript-title">{t.title || 'Untitled transcript'}</span>
              <span class="transcript-date" data-find-skip title={new Date(t.created_at).toLocaleString()}>{rel(t.created_at)}</span>
            </button>
            <button
              class="del-transcript-btn"
              onclick={() => doDeleteTranscript(t)}
              title="Remove transcript"
              aria-label="Remove transcript"
            ><Icon name="x" size={12} /></button>
            <button class="btn small" data-find-skip onclick={() => void downloadTranscript(t)}>Download</button>
          </div>
          {#if expandedTranscripts[t.id]}
            {#if product.transcriptBodies[t.id] !== undefined}
              <div class="transcript-body">{product.transcriptBodies[t.id]}</div>
            {:else if product.transcriptBodyLoading[t.id]}
              <div data-find-skip><LoadState what="this transcript" variant="compact" loading empty /></div>
            {:else}
              {#if product.transcriptBodyErrors[t.id]}<div data-find-skip use:announceShown={{ text: product.transcriptBodyErrors[t.id], assertive: true }}>{product.transcriptBodyErrors[t.id]}</div>{/if}
              <button class="btn small" data-find-skip onclick={() => void product.loadTranscriptBody(t.id)}>{product.transcriptBodyErrors[t.id] ? 'Retry' : 'Load transcript'}</button>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  {/if}
  {#if product.transcriptPreviousCursors.length || product.transcriptNextCursor}
    <div data-find-skip>
      <button class="btn small" disabled={product.loadingTranscripts || !product.transcriptPreviousCursors.length} onclick={() => void product.previousTranscriptPage()}>Previous transcripts</button>
      <button class="btn small" disabled={product.loadingTranscripts || !product.transcriptNextCursor} onclick={() => void product.loadTranscripts(product.transcriptNextCursor)}>Next transcripts</button>
    </div>
  {/if}

  <!-- Add transcript form -->
  <div class="add-transcript-form">
    <input dir="auto"
      class="input wide-input"
      bind:value={newTranscriptTitle}
      placeholder="e.g. Kickoff call, 12 Oct" aria-label="Transcript title (optional)"
      spellcheck="false"
    />
    <textarea dir="auto" aria-label="Transcript"
      class="textarea"
      bind:value={newTranscriptBody}
      rows={5}
      placeholder="e.g. PO: the limit resets every Monday at midnight…"
      spellcheck="false"
    ></textarea>
    <button
      class="btn small"
      onclick={doAddTranscript}
      disabled={addingTranscript || !newTranscriptBody.trim()}
    >
      {addingTranscript ? 'Adding…' : 'Add transcript'}
    </button>
  </div>
</div>

<style>
  .muted {
    padding: 24px 0;
    font-size: var(--fs-m);
    color: var(--text-dim);
    font-style: italic;
  }  .coll-arrow {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }  .wide-input {
    width: 100%;
    box-sizing: border-box;
  }  .textarea {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    font-family: var(--font-mono);
    padding: 8px 10px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    line-height: 1.55;
  }  .textarea:focus {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }  /* ── Transcripts (right column in draft mode) ──────────────── */
  .transcripts-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 4px;
  }
  .transcripts-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .section-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .transcript-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .transcript-item {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .transcript-header {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .transcript-toggle {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    padding: 8px 10px;
    cursor: pointer;
    text-align: start;
    color: var(--text);
    font-size: var(--fs-s);
    transition: background var(--dur-fast);
  }
  .transcript-toggle:hover {
    background: var(--hover);
  }
  .transcript-title {
    flex: 1;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .transcript-date {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .del-transcript-btn {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    transition: color var(--dur-fast), background var(--dur-fast);
    margin-inline-end: 4px;
    border-radius: var(--radius-s);
  }
  .del-transcript-btn:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .transcript-body {
    padding: 8px 12px 10px;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    color: var(--text-dim);
    line-height: 1.55;
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--text-dim) 3%, transparent);
    font-family: var(--font-mono);
    max-height: 320px;
    overflow-y: auto;
  }
  .add-transcript-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
  }
</style>
