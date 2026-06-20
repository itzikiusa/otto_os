<script lang="ts">
  // Rewrite tab — generate a suggested rewrite of the story, show a two-column
  // before/after diff vs the current source version, and allow publishing
  // back to Jira/Confluence.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdown } from '../../lib/md';
  import DiffView from '../../lib/components/DiffView.svelte';
  import type { ProductStoryVersion } from './types';

  const PROVIDERS = ['claude', 'openai'] as const;

  // ── Local UI state ──────────────────────────────────────────────────────────
  let provider = $state<string>('claude');
  let generating = $state(false);
  let publishing = $state(false);
  let confirmPublish = $state(false);

  // Loaded version bodies
  let sourceVersion = $state<ProductStoryVersion | null>(null);
  let suggestedVersion = $state<ProductStoryVersion | null>(null);
  let loadingBodies = $state(false);

  // Diff view toggle
  let diffView = $state<'split' | 'source' | 'suggested'>('split');

  // ── Polling ──────────────────────────────────────────────────────────────────
  let pollTimer = $state<ReturnType<typeof setInterval> | null>(null);
  const POLL_INTERVAL_MS = 3000;
  const POLL_MAX_MS = 120_000;
  let pollStartedAt = 0;
  // Count of versions when we triggered a rewrite — to detect a new one appearing.
  let versionsCountAtStart = 0;

  function clearPoll(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  async function pollVersions(): Promise<void> {
    // Stop if timeout exceeded.
    if (Date.now() - pollStartedAt > POLL_MAX_MS) {
      clearPoll();
      toasts.warn('Rewrite timed out', 'No suggested version appeared within 2 minutes.');
      return;
    }
    try {
      await product.loadVersions();
      const suggested = latestSuggested();
      if (suggested) {
        clearPoll();
        await loadVersionBodies(suggested);
      }
    } catch (e) {
      console.error('[RewriteTab] poll error', e);
    }
  }

  function startPolling(): void {
    clearPoll();
    versionsCountAtStart = product.versions.length;
    pollStartedAt = Date.now();
    // Immediate first poll, then interval.
    void pollVersions();
    pollTimer = setInterval(() => { void pollVersions(); }, POLL_INTERVAL_MS);
  }

  // Clear on unmount or story change.
  $effect(() => {
    product.selectedId;
    // Reset local state when story changes.
    sourceVersion = null;
    suggestedVersion = null;
    confirmPublish = false;
    clearPoll();
    // Kick off initial load if we have a story.
    if (product.selectedId) {
      void initialLoad();
    }
    return () => { clearPoll(); };
  });

  // Subscribe to `product_changed { section: 'rewrite' }` WS events.
  $effect(() => {
    const off = product.onSectionChange('rewrite', (_status: string) => {
      void pollVersions(); // final refresh — clears poll if version appeared
    });
    return off;
  });

  // ── Derived ─────────────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);
  const source = $derived(product.detail?.source ?? null);

  function latestSuggested(): ProductStoryVersion | null {
    const all = product.versions;
    // Find the most recent 'suggested' kind (highest version_no).
    let best: ProductStoryVersion | null = null;
    for (const v of all) {
      if (v.kind === 'suggested') {
        if (!best || v.version_no > best.version_no) best = v;
      }
    }
    return best;
  }

  async function initialLoad(): Promise<void> {
    try {
      await product.loadVersions();
      const suggested = latestSuggested();
      if (suggested) {
        await loadVersionBodies(suggested);
      }
    } catch (e) {
      console.error('[RewriteTab] initialLoad error', e);
    }
  }

  async function loadVersionBodies(suggested: ProductStoryVersion): Promise<void> {
    loadingBodies = true;
    try {
      // Load suggested body (full).
      const fullSuggested = await product.getVersion(suggested.id);
      suggestedVersion = fullSuggested;

      // Load source body — use the current source from detail, or fetch by id.
      if (source) {
        const fullSource = await product.getVersion(source.id);
        sourceVersion = fullSource;
      }
    } catch (e) {
      toasts.error('Could not load version bodies', product.errMsg(e));
    } finally {
      loadingBodies = false;
    }
  }

  // ── Actions ──────────────────────────────────────────────────────────────────

  async function generate(): Promise<void> {
    if (generating) return;
    generating = true;
    try {
      await product.rewrite({ provider: provider || null });
      toasts.info('Rewrite triggered', 'Waiting for suggested version to appear…');
      startPolling();
    } catch (e) {
      toasts.error('Rewrite failed', product.errMsg(e));
    } finally {
      generating = false;
    }
  }

  async function publish(): Promise<void> {
    if (!suggestedVersion || publishing) return;
    publishing = true;
    confirmPublish = false;
    try {
      await product.publishVersion(suggestedVersion.id);
      toasts.success('Published', 'Suggested version published back to source.');
    } catch (e) {
      toasts.error('Publish failed', product.errMsg(e));
    } finally {
      publishing = false;
    }
  }

  // Rendered markdown for the two panes.
  const renderedSource = $derived(
    sourceVersion?.body_md ? renderMarkdown(sourceVersion.body_md) : ''
  );
  const renderedSuggested = $derived(
    suggestedVersion?.body_md ? renderMarkdown(suggestedVersion.body_md) : ''
  );
</script>

{#if !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="rewrite-tab">

    <!-- ── Generate panel ───────────────────────────────────────────────────── -->
    <section class="card gen-panel">
      <div class="gen-row">
        <div class="provider-wrap">
          <label class="field-label" for="rw-provider-sel">Provider</label>
          <select
            id="rw-provider-sel"
            class="sel"
            bind:value={provider}
            disabled={generating}
          >
            {#each PROVIDERS as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </div>

        <button
          class="action-btn primary"
          onclick={generate}
          disabled={generating || pollTimer !== null}
        >
          {#if generating}
            Triggering…
          {:else if pollTimer !== null}
            Polling for result…
          {:else}
            Generate suggested rewrite
          {/if}
        </button>

        {#if pollTimer !== null}
          <span class="polling-indicator">checking every 3s…</span>
        {/if}
      </div>
    </section>

    <!-- ── Suggested version display ────────────────────────────────────────── -->
    {#if loadingBodies}
      <div class="muted">Loading version content…</div>
    {:else if suggestedVersion}
      <!-- Version metadata -->
      <section class="card version-meta">
        <div class="vm-row">
          <div class="vm-info">
            <span class="vm-label">Suggested v{suggestedVersion.version_no}</span>
            <span class="vm-date">{new Date(suggestedVersion.created_at).toLocaleString()}</span>
          </div>
          {#if suggestedVersion.change_notes}
            <div class="change-notes">
              <span class="cn-label">Change notes:</span>
              <span class="cn-body">{suggestedVersion.change_notes}</span>
            </div>
          {/if}

          <!-- Publish button -->
          <div class="publish-wrap">
            {#if confirmPublish}
              <span class="confirm-text">This will overwrite the live ticket/page. Are you sure?</span>
              <button class="action-btn danger" onclick={publish} disabled={publishing}>
                {publishing ? 'Publishing…' : 'Yes, publish'}
              </button>
              <button class="action-btn" onclick={() => (confirmPublish = false)} disabled={publishing}>
                Cancel
              </button>
            {:else}
              <button
                class="action-btn primary"
                onclick={() => (confirmPublish = true)}
                disabled={publishing}
              >
                Publish to Jira/Confluence
              </button>
            {/if}
          </div>
        </div>
      </section>

      <!-- Diff view toggle -->
      <div class="view-toggle-row">
        <span class="field-label">View</span>
        <div class="segmented">
          <button class:active={diffView === 'split'} onclick={() => (diffView = 'split')}>Word diff</button>
          <button class:active={diffView === 'source'} onclick={() => (diffView = 'source')}>Source only</button>
          <button class:active={diffView === 'suggested'} onclick={() => (diffView = 'suggested')}>Suggested only</button>
        </div>
      </div>

      <!-- Word-level diff using the shared DiffView component (T3) -->
      {#if diffView === 'split'}
        <div class="diff-wrap card">
          <div class="pane-header diff-pane-header">
            <span class="pane-label source-label">Source{sourceVersion ? ` v${sourceVersion.version_no}` : ''}</span>
            <span class="pane-label suggested-label">Suggested v{suggestedVersion.version_no}</span>
          </div>
          <DiffView
            before={sourceVersion?.body_md ?? ''}
            after={suggestedVersion?.body_md ?? ''}
            mode="split"
            contextLines={4}
          />
        </div>
      {:else if diffView === 'source'}
        <div class="single-pane card">
          <div class="pane-header">
            <span class="pane-label source-label">Source</span>
            {#if sourceVersion}
              <span class="pane-meta">v{sourceVersion.version_no}</span>
            {/if}
          </div>
          <div class="md-body">
            {#if renderedSource}
              {@html renderedSource}
            {:else}
              <span class="muted">No source content.</span>
            {/if}
          </div>
        </div>
      {:else}
        <div class="single-pane card">
          <div class="pane-header">
            <span class="pane-label suggested-label">Suggested</span>
            <span class="pane-meta">v{suggestedVersion.version_no}</span>
          </div>
          <div class="md-body">
            {#if renderedSuggested}
              {@html renderedSuggested}
            {:else}
              <span class="muted">No suggested content.</span>
            {/if}
          </div>
        </div>
      {/if}

    {:else}
      <div class="muted">No suggested version yet. Click "Generate suggested rewrite" above.</div>
    {/if}
  </div>
{/if}

<style>
  .muted {
    padding: 24px 0;
    font-size: 13px;
    color: var(--text-dim);
    font-style: italic;
  }
  .rewrite-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: min(1100px, 92vw);
    width: 100%;
  }

  /* ── Card ────────────────────────────────────────────────────── */
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    background: var(--surface-raised, var(--surface));
  }

  /* ── Generate panel ──────────────────────────────────────────── */
  .gen-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .provider-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .field-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .sel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 4px 8px;
  }
  .polling-indicator {
    font-size: 11.5px;
    color: var(--text-dim);
    font-style: italic;
  }

  /* ── Buttons ─────────────────────────────────────────────────── */
  .action-btn {
    height: 30px;
    padding: 0 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms, color 110ms, opacity 110ms;
  }
  .action-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .action-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .action-btn.primary {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    font-weight: 600;
  }
  .action-btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .action-btn.danger {
    border-color: #ef4444;
    color: #b91c1c;
    background: color-mix(in srgb, #ef4444 10%, transparent);
    font-weight: 600;
  }
  .action-btn.danger:hover:not(:disabled) {
    background: color-mix(in srgb, #ef4444 20%, transparent);
  }

  /* ── Version metadata card ───────────────────────────────────── */
  .vm-row {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    flex-wrap: wrap;
  }
  .vm-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 120px;
  }
  .vm-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .vm-date {
    font-size: 11px;
    color: var(--text-dim);
  }
  .change-notes {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 200px;
  }
  .cn-label {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .cn-body {
    font-size: 13px;
    color: var(--text);
    line-height: 1.5;
    font-style: italic;
  }
  .publish-wrap {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-left: auto;
  }
  .confirm-text {
    font-size: 12px;
    color: #b45309;
  }

  /* ── View toggle row ─────────────────────────────────────────── */
  .view-toggle-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .segmented {
    display: flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .segmented button {
    height: 26px;
    padding: 0 12px;
    border: none;
    border-right: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
    white-space: nowrap;
    transition: background 90ms, color 90ms;
  }
  .segmented button:last-child {
    border-right: none;
  }
  .segmented button:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .segmented button.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    font-weight: 600;
  }

  /* ── DiffView wrapper (split word-diff view, T3) ────────────── */
  .diff-wrap {
    padding: 0;
    overflow: hidden;
    min-height: 300px;
  }
  .diff-pane-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 14px 7px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-raised, var(--surface));
  }
  .pane-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px 7px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-raised, var(--surface));
    flex-shrink: 0;
  }
  .pane-label {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 8px;
    border-radius: 999px;
  }
  .source-label {
    background: color-mix(in srgb, var(--text-dim) 15%, transparent);
    color: var(--text-dim);
  }
  .suggested-label {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .pane-meta {
    font-size: 11px;
    color: var(--text-dim);
  }

  /* ── Single pane ─────────────────────────────────────────────── */
  .single-pane {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0;
    overflow: hidden;
  }
  .single-pane .pane-header {
    border-bottom: 1px solid var(--border);
    padding: 8px 14px 7px;
  }
  .single-pane .md-body {
    padding: 14px 16px;
  }

  /* ── Markdown body ───────────────────────────────────────────── */
  .md-body {
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--text);
  }
  .md-body :global(h1),
  .md-body :global(h2),
  .md-body :global(h3),
  .md-body :global(h4) {
    margin: 1.2em 0 0.4em;
    font-weight: 700;
    line-height: 1.25;
    color: var(--text);
  }
  .md-body :global(h1) { font-size: 1.35em; }
  .md-body :global(h2) { font-size: 1.2em; }
  .md-body :global(h3) { font-size: 1.05em; }
  .md-body :global(p) {
    margin: 0 0 0.75em;
  }
  .md-body :global(ul),
  .md-body :global(ol) {
    padding-left: 1.5em;
    margin: 0 0 0.75em;
  }
  .md-body :global(li) {
    margin-bottom: 0.25em;
  }
  .md-body :global(code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.88em;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    padding: 1px 5px;
    border-radius: 3px;
  }
  .md-body :global(pre) {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    overflow-x: auto;
    margin: 0 0 0.75em;
  }
  .md-body :global(pre code) {
    background: none;
    padding: 0;
    font-size: 0.86em;
  }
  .md-body :global(blockquote) {
    border-left: 3px solid var(--border);
    padding-left: 12px;
    color: var(--text-dim);
    margin: 0 0 0.75em;
    font-style: italic;
  }
  .md-body :global(a) {
    color: var(--accent);
    text-decoration: none;
  }
  .md-body :global(a:hover) {
    text-decoration: underline;
  }
</style>
