<script lang="ts">
  // Inject tab — build/preview the inject bundle for the selected story, copy
  // markdown to clipboard, and open an agent session seeded with the bundle.
  import Icon from '../../lib/components/Icon.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { toasts } from '../../lib/toast.svelte';
  import type { InjectBundle } from './types';

  const PROVIDERS = ['claude', 'openai', 'gemini'] as const;

  let bundle = $state<InjectBundle | null>(null);
  let loading = $state(false);
  let copying = $state(false);
  let launching = $state(false);
  let provider = $state<string>('claude');
  let cwd = $state('');

  // Collapsible state per section index.
  let collapsed = $state<Record<number, boolean>>({});

  const story = $derived(product.detail?.story ?? null);

  // Reset when story changes.
  $effect(() => {
    product.selectedId;
    bundle = null;
    collapsed = {};
    cwd = story?.cwd ?? '';
  });

  const renderedMarkdown = $derived(bundle ? renderMarkdown(bundle.markdown) : '');

  async function buildPreview(): Promise<void> {
    if (loading) return;
    loading = true;
    try {
      bundle = await product.loadInject();
    } catch (e) {
      toasts.error('Could not build inject bundle', product.errMsg(e));
    } finally {
      loading = false;
    }
  }

  async function copyMarkdown(): Promise<void> {
    if (!bundle || copying) return;
    copying = true;
    try {
      await navigator.clipboard.writeText(bundle.markdown);
      toasts.success('Copied to clipboard', 'Inject bundle markdown copied.');
    } catch (e) {
      toasts.error('Copy failed', product.errMsg(e));
    } finally {
      copying = false;
    }
  }

  async function openInAgent(): Promise<void> {
    if (launching) return;
    launching = true;
    try {
      const session = await product.injectSession({
        provider: provider || undefined,
        cwd: cwd.trim() || undefined,
      });
      toasts.success(
        'Agent session created — open it in the Agents section',
        session.title ? `"${session.title}" (${session.id})` : session.id,
      );
    } catch (e) {
      toasts.error('Could not create agent session', product.errMsg(e));
    } finally {
      launching = false;
    }
  }

  function toggleSection(idx: number): void {
    collapsed = { ...collapsed, [idx]: !collapsed[idx] };
  }
</script>

{#if !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="inject-tab">

    <!-- ── Action bar ────────────────────────────────────────────────────── -->
    <div class="action-bar card">
      <button
        class="action-btn primary"
        onclick={buildPreview}
        disabled={loading}
      >
        <Icon name="zap" size={13} />
        {loading ? 'Building…' : bundle ? 'Rebuild' : 'Build / Preview'}
      </button>

      {#if bundle}
        <button
          class="action-btn"
          onclick={copyMarkdown}
          disabled={copying}
          title="Copy inject bundle markdown to clipboard"
        >
          <Icon name="fetch" size={13} />
          {copying ? 'Copying…' : 'Copy'}
        </button>
      {/if}

      <span class="divider"></span>

      <!-- Open in agent -->
      <div class="agent-row">
        <label class="field-label" for="inject-provider">Provider</label>
        <select id="inject-provider" class="mini-select" bind:value={provider} disabled={launching}>
          {#each PROVIDERS as p (p)}
            <option value={p}>{p}</option>
          {/each}
        </select>

        <label class="field-label" for="inject-cwd">cwd</label>
        <input
          id="inject-cwd"
          class="cwd-input"
          type="text"
          placeholder="optional working dir"
          bind:value={cwd}
          disabled={launching}
        />

        <button
          class="action-btn accent"
          onclick={openInAgent}
          disabled={launching}
          title="Create an agent session seeded with this inject bundle"
        >
          <Icon name="play" size={13} />
          {launching ? 'Creating…' : 'Open in agent'}
        </button>
      </div>
    </div>

    <!-- ── Bundle preview ────────────────────────────────────────────────── -->
    {#if bundle}
      <!-- Sections as collapsible blocks -->
      {#if bundle.sections && bundle.sections.length > 0}
        <div class="sections-wrap">
          <div class="section-head-row">
            <span class="section-label">Sections ({bundle.sections.length})</span>
          </div>
          {#each bundle.sections as sec, idx (idx)}
            <div class="section-block">
              <button class="sec-trigger" onclick={() => toggleSection(idx)}>
                <span class="coll-arrow">{collapsed[idx] ? '▶' : '▼'}</span>
                <span class="sec-heading">{sec.heading}</span>
              </button>
              {#if !collapsed[idx]}
                <div class="sec-body md-body">{@html renderMarkdown(sec.body)}</div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}

      <!-- Full rendered markdown -->
      <div class="preview-card card">
        <div class="preview-header">
          <span class="section-label">Full Markdown</span>
          <button
            class="action-btn small"
            onclick={copyMarkdown}
            disabled={copying}
            title="Copy to clipboard"
          >
            <Icon name="fetch" size={12} />
            Copy
          </button>
        </div>
        {#if renderedMarkdown}
          <div class="md-body">{@html renderedMarkdown}</div>
        {:else}
          <div class="muted">Empty bundle.</div>
        {/if}
      </div>
    {:else if !loading}
      <div class="empty-hint">
        <Icon name="zap" size={28} />
        <p>Click <strong>Build / Preview</strong> to generate the inject bundle for this story.</p>
        <p class="dim">The bundle summarises all story context so an agent can start coding immediately.</p>
      </div>
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
  .inject-tab {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 860px;
    width: 100%;
  }

  /* Card */
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    background: var(--surface-raised, var(--surface));
  }

  /* Action bar */
  .action-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .divider {
    width: 1px;
    height: 20px;
    background: var(--border);
    flex-shrink: 0;
  }
  .agent-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    flex: 1;
  }
  .field-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .mini-select {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 3px 7px;
  }
  .cwd-input {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 4px 8px;
    min-width: 160px;
    max-width: 280px;
    font-family: var(--font-mono, monospace);
  }
  .cwd-input::placeholder {
    color: var(--text-dim);
  }
  .cwd-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  /* Buttons */
  .action-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 11px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms, color 110ms;
  }
  .action-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .action-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .action-btn.primary {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .action-btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .action-btn.accent {
    border-color: var(--status-working, #22c55e);
    color: var(--status-working, #22c55e);
    background: color-mix(in srgb, var(--status-working, #22c55e) 10%, transparent);
  }
  .action-btn.accent:hover:not(:disabled) {
    background: color-mix(in srgb, var(--status-working, #22c55e) 20%, transparent);
  }
  .action-btn.small {
    height: 24px;
    padding: 0 8px;
    font-size: 11.5px;
  }

  /* Sections */
  .sections-wrap {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .section-head-row {
    margin-bottom: 6px;
  }
  .section-label {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .section-block {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--surface-raised, var(--surface));
  }
  .sec-trigger {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 8px 12px;
    background: none;
    border: none;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    transition: background 100ms;
  }
  .sec-trigger:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .coll-arrow {
    font-size: 9px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .sec-heading {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .sec-body {
    padding: 10px 14px 12px;
    border-top: 1px solid var(--border);
  }

  /* Preview card */
  .preview-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .preview-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  /* Empty hint */
  .empty-hint {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 48px 24px;
    color: var(--text-dim);
    text-align: center;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
  }
  .empty-hint p {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    max-width: 360px;
  }
  .empty-hint .dim {
    font-size: 12px;
    color: var(--text-dim);
    opacity: 0.7;
  }

  /* Markdown body */
  .md-body {
    font-size: 13.5px;
    line-height: 1.65;
    color: var(--text);
  }
  .md-body :global(h1),
  .md-body :global(h2),
  .md-body :global(h3),
  .md-body :global(h4) {
    margin: 1.1em 0 0.35em;
    font-weight: 700;
    line-height: 1.25;
    color: var(--text);
  }
  .md-body :global(h1) { font-size: 1.35em; }
  .md-body :global(h2) { font-size: 1.2em; }
  .md-body :global(h3) { font-size: 1.05em; }
  .md-body :global(p) { margin: 0 0 0.7em; }
  .md-body :global(ul),
  .md-body :global(ol) { padding-left: 1.5em; margin: 0 0 0.7em; }
  .md-body :global(li) { margin-bottom: 0.2em; }
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
    margin: 0 0 0.7em;
  }
  .md-body :global(pre code) { background: none; padding: 0; font-size: 0.86em; }
  .md-body :global(blockquote) {
    border-left: 3px solid var(--border);
    padding-left: 12px;
    color: var(--text-dim);
    margin: 0 0 0.7em;
    font-style: italic;
  }
  .md-body :global(a) { color: var(--accent); text-decoration: none; }
  .md-body :global(a:hover) { text-decoration: underline; }
</style>
