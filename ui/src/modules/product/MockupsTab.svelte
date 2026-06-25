<script lang="ts">
  // MockupsTab — lists attachments that qualify as mockups (kind==='mockup', or
  // an HTML/SVG/image mime, or a `.mmd` filename) and shows the selected one in
  // a MockupViewer. The viewer isolates untrusted content in sandboxed iframes
  // (HTML and Mermaid) or renders images as <img> — never inlining markup into
  // the parent DOM (see §8 of the design spec).
  import { onDestroy } from 'svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { mockupAssist } from '../../lib/stores/mockup-assist.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import MockupViewer from './MockupViewer.svelte';
  import MockupAssistPanel from './MockupAssistPanel.svelte';
  import type { ProductAttachment } from './types';

  // ── State ─────────────────────────────────────────────────────────────────
  let atts = $state<ProductAttachment[]>([]);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let selectedId = $state<string | null>(null);
  let importing = $state(false);
  let createMenu = $state(false);

  /** Read a Blob → base64 (no data-URL prefix). */
  function fileToB64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onerror = () => reject(reader.error);
      reader.onload = () => {
        const result = reader.result as string;
        const idx = result.indexOf(',');
        resolve(idx >= 0 ? result.slice(idx + 1) : result);
      };
      reader.readAsDataURL(blob);
    });
  }

  /** Map a picked file to an allow-listed mockup mime (best-effort by extension). */
  function mimeForFile(f: File): string {
    if (f.type) return f.type;
    const n = f.name.toLowerCase();
    if (n.endsWith('.mmd')) return 'text/vnd.mermaid';
    if (n.endsWith('.html') || n.endsWith('.htm')) return 'text/html';
    if (n.endsWith('.svg')) return 'image/svg+xml';
    return 'application/octet-stream';
  }

  /** Manual import: upload picked file(s) as mockups, then select the first. */
  async function importFiles(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const files = input.files ? Array.from(input.files) : [];
    input.value = '';
    if (!files.length) return;
    importing = true;
    let firstId: string | null = null;
    try {
      for (const f of files) {
        try {
          const att = await product.uploadAttachment({
            filename: f.name,
            mime: mimeForFile(f),
            kind: 'mockup',
            data_b64: await fileToB64(f),
          });
          firstId ??= att.id;
        } catch (err) {
          toasts.error(`Import failed: ${f.name}`, err instanceof Error ? err.message : String(err));
        }
      }
      await loadMockups();
      if (firstId) selectedId = firstId;
    } finally {
      importing = false;
    }
  }

  /** Create with AI: open the in-place mockup agent for a NEW mockup. */
  function createWithAi(format: 'html' | 'mermaid'): void {
    createMenu = false;
    const storyId = product.selectedId;
    if (!storyId) return;
    mockupAssist.openNew(storyId, format);
  }

  /** Refine an existing agent mockup with the in-place agent. */
  function refine(m: ProductAttachment): void {
    void mockupAssist.openRefine(m);
  }

  /** A turn committed — reload the list and select the committed mockup. */
  async function onAssistCommit(att: ProductAttachment): Promise<void> {
    await loadMockups();
    selectedId = att.id;
  }

  function closeAssist(): void {
    mockupAssist.close();
    void loadMockups();
  }

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
    const sid = product.selectedId;
    // The Assistant is a global singleton — if it's still open for a DIFFERENT
    // story (the user switched stories in the sidebar), close it so a turn can't
    // target the wrong story.
    if (mockupAssist.active && mockupAssist.storyId !== sid) mockupAssist.close();
    void loadMockups();
  });

  // Leaving the Mockups tab resets the singleton so it never lingers elsewhere.
  onDestroy(() => mockupAssist.close());

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
    <div class="list-actions">
      <label class="act-btn" title="Import a mockup file (HTML, image, SVG, .mmd)">
        <Icon name="plus" size={12} />
        {importing ? 'Importing…' : 'Import'}
        <input
          type="file"
          multiple
          accept="image/*,.svg,.html,.htm,.mmd,text/html,text/vnd.mermaid"
          style="display:none"
          onchange={importFiles}
          disabled={importing}
        />
      </label>
      <div class="create-wrap">
        <button class="act-btn primary" onclick={() => (createMenu = !createMenu)} title="Generate a mockup with AI">
          <Icon name="zap" size={12} /> Create with AI
        </button>
        {#if createMenu}
          <button class="menu-backdrop" aria-label="Close menu" onclick={() => (createMenu = false)}></button>
          <div class="create-menu">
            <button onclick={() => createWithAi('html')}>
              <strong>HTML screen</strong><small>A self-contained UI mockup</small>
            </button>
            <button onclick={() => createWithAi('mermaid')}>
              <strong>Diagram</strong><small>A Mermaid flow / sequence / model</small>
            </button>
          </div>
        {/if}
      </div>
    </div>
    {#if loading}
      <div class="list-empty">Loading…</div>
    {:else if loadError}
      <div class="list-empty err">{loadError}</div>
    {:else if mockups.length === 0}
      <div class="list-empty">
        No mockups yet. <strong>Import</strong> a file, or <strong>Create with AI</strong> to have a
        specialized agent build one — right here, no Agents detour.
      </div>
    {:else}
      {#each mockups as m (m.id)}
        <div class="mockup-row" class:active={selectedId === m.id}>
          <button class="mockup-open" onclick={() => (selectedId = m.id)} title={m.filename}>
            <span class="mockup-type">{typeLabel(m)}</span>
            <span class="mockup-name">{m.filename}</span>
            {#if m.source === 'agent'}<span class="agent-badge">agent</span>{/if}
          </button>
          {#if m.source === 'agent'}
            <button class="refine-btn" onclick={() => refine(m)} title="Refine with AI" aria-label="Refine with AI">
              <Icon name="zap" size={12} />
            </button>
          {/if}
        </div>
      {/each}
    {/if}
  </aside>

  <!-- Stage: the in-place mockup agent (when active) else the viewer. -->
  <div class="mockup-stage">
    {#if mockupAssist.active}
      <MockupAssistPanel oncommit={onAssistCommit} onclose={closeAssist} />
    {:else if selected}
      <!-- Key by id+updated_at so a refine (same id, new bytes) remounts + refetches. -->
      {#key `${selected.id}:${selected.updated_at}`}
        <MockupViewer attachment={selected} />
      {/key}
    {:else if !loading}
      <div class="stage-empty">
        <Icon name="box" size={28} />
        <p>Select a mockup to preview and annotate, or <strong>Create with AI</strong> to generate
          one in place.</p>
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
  .mockup-row {
    display: flex;
    align-items: center;
    gap: 2px;
    width: 100%;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .mockup-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border: none;
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
  .refine-btn {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    margin-inline-end: 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .refine-btn:hover {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .list-actions {
    display: flex;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .act-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 600;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .act-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .act-btn.primary {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 36%, transparent);
  }
  .create-wrap {
    position: relative;
    margin-inline-start: auto;
  }
  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 19;
    border: none;
    background: transparent;
    cursor: default;
  }
  .create-menu {
    position: absolute;
    top: 30px;
    right: 0;
    z-index: 20;
    min-width: 200px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    box-shadow: var(--shadow, 0 8px 28px rgba(0, 0, 0, 0.25));
    overflow: hidden;
  }
  .create-menu button {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    padding: 8px 11px;
    border: none;
    background: none;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .create-menu button:hover {
    background: var(--surface-2, color-mix(in srgb, var(--text-dim) 10%, transparent));
  }
  .create-menu small {
    color: var(--text-dim);
    font-size: 10.5px;
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
