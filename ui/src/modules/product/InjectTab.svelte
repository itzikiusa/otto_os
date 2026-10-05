<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  import { toastError } from '../../lib/toastError';
  // Inject tab — build/preview the inject bundle for the selected story, copy
  // markdown to clipboard, and open an agent session seeded with the bundle.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { toasts } from '../../lib/toast.svelte';
  import type { InjectBundle } from './types';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { copyTextOrThrow } from '../../lib/clipboard';

  // The rewrite/tests/inject run spawns an agent CLI session via the live
  // registry, so the provider must be a real registered agent (built-in or
  // custom like grok) — openai/gemini were never valid here.
  const PROVIDERS = $derived(agentProviders());

  let bundle = $state<InjectBundle | null>(null);
  let loading = $state(false);
  let copying = $state(false);
  let launching = $state(false);
  let provider = $state<string>(defaultAgentProvider());
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
      toastError('Couldn’t build inject bundle', e);
    } finally {
      loading = false;
    }
  }

  async function copyMarkdown(): Promise<void> {
    if (!bundle || copying) return;
    copying = true;
    try {
      await copyTextOrThrow(bundle.markdown);
      toasts.success('Copied to clipboard', 'Inject bundle markdown copied.');
    } catch (e) {
      toastError('Couldn’t copy the bundle', e);
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
      toasts.success('Agent session started', session.title ? `“${session.title}” is open in Agents.` : 'It is open in Agents.');
      // "Open in agent" means open it: land on the new session, not a toast with its id.
      ws.navigateToSession(session.id);
    } catch (e) {
      toastError('Couldn’t create agent session', e);
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
        class="btn primary"
        onclick={buildPreview}
        disabled={loading}
      >
        <Icon name="zap" size={13} />
        {loading ? 'Building…' : bundle ? 'Rebuild' : 'Build / Preview'}
      </button>

      {#if bundle}
        <button
          class="btn"
          onclick={copyMarkdown}
          disabled={copying}
          title="Copy inject bundle markdown to clipboard"
        >
          <Icon name="copy" size={13} />
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
        <PathField bind:value={cwd} disabled={launching}><input dir="ltr"
          id="inject-cwd"
          class="cwd-input"
          type="text"
          placeholder="optional working dir"
          bind:value={cwd}
          disabled={launching}
        /></PathField>

        <button
          class="btn"
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
                <span class="coll-arrow" aria-hidden="true"><Icon name={collapsed[idx] ? 'chevronRight' : 'chevronDown'} size={11} /></span>
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
          <span class="section-label">Full markdown</span>
          <button
            class="btn small"
            onclick={copyMarkdown}
            disabled={copying}
            title="Copy to clipboard"
          >
            <Icon name="copy" size={12} />
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
      <EmptyState
        icon="zap"
        title="No bundle built yet"
        body="Build / Preview assembles all of this story’s context into one bundle, so an agent can start coding immediately."
      />
    {/if}
  </div>
{/if}

<style>
  .muted {
    padding: 24px 0;
    font-size: var(--fs-m);
    color: var(--text-dim);
    font-style: italic;
  }
  .inject-tab {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: min(860px, 92vw);
    width: 100%;
  }

  /* Card */
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    background: var(--surface);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .mini-select {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 2px 6px;
  }
  .cwd-input {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 4px 8px;
    min-width: 160px;
    max-width: 280px;
    font-family: var(--font-mono);
  }
  .cwd-input::placeholder {
    color: var(--text-dim);
  }
  .cwd-input:focus {
    outline: none;
    border-color: var(--accent-text); box-shadow: 0 0 0 3px var(--accent-soft-strong)
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
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .section-block {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--surface);
  }
  .sec-trigger {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 8px 12px;
    background: none;
    border: none;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    transition: background var(--dur-fast);
  }
  .sec-trigger:hover {
    background: var(--hover);
  }
  .coll-arrow {
    display: inline-flex;
    align-items: center;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .sec-heading {
    font-size: var(--fs-m);
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
</style>
