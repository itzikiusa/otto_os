<script lang="ts">
  // MockupsTab — lists attachments that qualify as mockups (kind==='mockup', or
  // an HTML/SVG/image mime, or a `.mmd` filename) and shows the selected one in
  // a MockupViewer. The viewer isolates untrusted content in sandboxed iframes
  // (HTML and Mermaid) or renders images as <img> — never inlining markup into
  // the parent DOM (see §8 of the design spec).
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import MockupViewer from './MockupViewer.svelte';
  import type { ProductAttachment } from './types';

  // ── State ─────────────────────────────────────────────────────────────────
  let atts = $state<ProductAttachment[]>([]);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let selectedId = $state<string | null>(null);

  /** True when an attachment should appear in the Mockups list. */
  function isMockup(a: ProductAttachment): boolean {
    if (a.kind === 'mockup') return true;
    const mime = (a.mime || '').toLowerCase();
    if (mime === 'text/html' || mime === 'image/svg+xml' || mime.startsWith('image/')) {
      return true;
    }
    if (mime === 'text/vnd.mermaid') return true;
    return (a.filename || '').toLowerCase().endsWith('.mmd');
  }

  const mockups = $derived(atts.filter(isMockup));
  const selected = $derived(mockups.find((m) => m.id === selectedId) ?? null);

  // ── Load on mount / story change ────────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    void loadMockups();
  });

  async function loadMockups(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      atts = await product.listAttachments();
      // Keep a valid selection: default to the first mockup if none chosen or
      // the previously-selected one is gone.
      const list = atts.filter(isMockup);
      if (!selectedId || !list.some((m) => m.id === selectedId)) {
        selectedId = list[0]?.id ?? null;
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
      toasts.error('Could not load mockups', loadError);
    } finally {
      loading = false;
    }
  }

  /** A short type label for the list row. */
  function typeLabel(a: ProductAttachment): string {
    const mime = (a.mime || '').toLowerCase();
    if (mime === 'text/html') return 'HTML';
    if (mime === 'image/svg+xml') return 'SVG';
    if (mime === 'text/vnd.mermaid' || (a.filename || '').toLowerCase().endsWith('.mmd')) {
      return 'Mermaid';
    }
    if (mime.startsWith('image/')) return 'Image';
    return mime || 'File';
  }
</script>

<div class="mockups-tab">
  <!-- List of mockup attachments -->
  <aside class="mockup-list">
    <div class="list-head">
      <span class="list-title">Mockups</span>
      <button class="refresh-btn" onclick={loadMockups} title="Refresh">
        <Icon name="refresh" size={12} />
      </button>
    </div>
    {#if loading}
      <div class="list-empty">Loading…</div>
    {:else if loadError}
      <div class="list-empty err">{loadError}</div>
    {:else if mockups.length === 0}
      <div class="list-empty">
        No mockups yet. Attach an HTML, image, SVG, or <code>.mmd</code> file on the Overview
        tab (and "Mark as mockup"), or let a discovery agent generate one.
      </div>
    {:else}
      {#each mockups as m (m.id)}
        <button
          class="mockup-row"
          class:active={selectedId === m.id}
          onclick={() => (selectedId = m.id)}
          title={m.filename}
        >
          <span class="mockup-type">{typeLabel(m)}</span>
          <span class="mockup-name">{m.filename}</span>
          {#if m.source === 'agent'}<span class="agent-badge">agent</span>{/if}
        </button>
      {/each}
    {/if}
  </aside>

  <!-- Viewer -->
  <div class="mockup-stage">
    {#if selected}
      {#key selected.id}
        <MockupViewer attachment={selected} />
      {/key}
    {:else if !loading}
      <div class="stage-empty">
        <Icon name="box" size={28} />
        <p>Select a mockup from the list to preview and annotate it.</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .mockups-tab {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 12px;
  }
  .mockup-list {
    width: 240px;
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }
  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .list-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .refresh-btn {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .refresh-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .list-empty {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 10px;
    line-height: 1.5;
  }
  .list-empty.err {
    color: #ef4444;
  }
  .list-empty code {
    font-family: var(--font-mono, monospace);
    font-size: 10.5px;
  }
  .mockup-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 8px 10px;
    border: none;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .mockup-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .mockup-row.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .mockup-type {
    flex-shrink: 0;
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .mockup-row.active .mockup-type {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .mockup-name {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .agent-badge {
    flex-shrink: 0;
    font-size: 8.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .mockup-stage {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .stage-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
  }
  .stage-empty p {
    margin: 0;
    font-size: 13px;
    max-width: 320px;
    line-height: 1.5;
  }

  @media (max-width: 640px) {
    .mockups-tab {
      flex-direction: column;
    }
    .mockup-list {
      width: 100%;
      max-height: 220px;
    }
  }
</style>
