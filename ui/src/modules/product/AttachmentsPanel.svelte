<script lang="ts">
  // AttachmentsPanel — local story attachments with paste/drag-drop/file-picker,
  // thumbnails (image), iframe (PDF), file chip+download (other), "Mark as
  // mockup", and delete. Screenshot paste from clipboardData is the headline
  // feature: calling `uploadBlob(blob)` is the shared entry-point used by both
  // the panel's own paste handler and the body-textarea paste handler in
  // OverviewTab (which also splices a markdown ref into draftBody).

  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { authedBlobUrl } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ProductAttachment } from './types';

  // ── Props ─────────────────────────────────────────────────────────────────

  interface Props {
    // Called by the parent when a blob is pasted into the body textarea so the
    // panel can show the optimistic thumbnail. Returns the created attachment.
    // `insertIntoBody` is handled by the parent (OverviewTab) itself.
    onAttachmentCreated?: (att: ProductAttachment) => void;
  }

  const { onAttachmentCreated }: Props = $props();

  // ── Local state (names prefixed to avoid collisions with OverviewTab state) ──

  let localAtts = $state<ProductAttachment[]>([]);
  let localAttLoading = $state(false);

  // Object URL cache for previews: id → blob URL string (authed).
  let localAttUrls = $state<Record<string, string>>({});
  // Per-id loading flags for blob URL fetch.
  let localAttUrlLoading = $state<Record<string, boolean>>({});
  // All created object URLs — revoked on unmount.
  let createdLocalUrls: string[] = [];

  // Upload-in-progress count (for optimistic thumbnails).
  let uploading = $state(false);

  // Drag-over styling toggle.
  let dragOver = $state(false);

  // Screenshot index for unique filenames within a session.
  let screenshotIdx = $state(0);

  // ── Lifecycle ─────────────────────────────────────────────────────────────

  $effect(() => {
    // Re-load when selectedId changes (product.selectedId as a reactive read).
    product.selectedId;
    // Revoke stale URLs, reset state.
    for (const url of createdLocalUrls) URL.revokeObjectURL(url);
    createdLocalUrls = [];
    localAttUrls = {};
    localAttUrlLoading = {};
    localAtts = [];
    void loadAttachments();
  });

  $effect(() => {
    // Cleanup on unmount.
    return () => {
      for (const url of createdLocalUrls) URL.revokeObjectURL(url);
    };
  });

  // ── Helpers ───────────────────────────────────────────────────────────────

  /** Read a Blob with FileReader → base64 string (no data-URL prefix). */
  function fileToB64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onerror = () => reject(reader.error);
      reader.onload = () => {
        const result = reader.result as string;
        // Strip the "data:<mime>;base64," prefix.
        const idx = result.indexOf(',');
        resolve(idx >= 0 ? result.slice(idx + 1) : result);
      };
      reader.readAsDataURL(blob);
    });
  }

  /** Format bytes as human-readable. */
  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  /** Load the attachment list from the server. */
  async function loadAttachments(): Promise<void> {
    localAttLoading = true;
    try {
      localAtts = await product.listAttachments();
      // Eagerly fetch preview URLs for images.
      for (const att of localAtts) {
        if (att.mime.startsWith('image/')) void loadAttUrl(att.id);
      }
    } catch (e) {
      toasts.error('Could not load attachments', e instanceof Error ? e.message : String(e));
    } finally {
      localAttLoading = false;
    }
  }

  /** Fetch an authed blob URL for one attachment (cached, idempotent). */
  async function loadAttUrl(id: string): Promise<void> {
    if (localAttUrls[id] || localAttUrlLoading[id]) return;
    localAttUrlLoading = { ...localAttUrlLoading, [id]: true };
    try {
      const url = await authedBlobUrl(`/product/attachments/${id}`);
      createdLocalUrls.push(url);
      localAttUrls = { ...localAttUrls, [id]: url };
    } catch (e) {
      console.warn('[AttachmentsPanel] preview load failed', id, e);
    } finally {
      localAttUrlLoading = { ...localAttUrlLoading, [id]: false };
    }
  }

  /**
   * Upload a single Blob as an attachment. Used by:
   *   - Panel paste handler (image pasted directly here)
   *   - OverviewTab body-textarea paste handler (calls `panelRef.uploadBlob`)
   *   - Drag-drop handler
   *   - File-picker handler
   *
   * Returns the created attachment so callers can splice a markdown ref into
   * the body text.
   */
  export async function uploadBlob(
    blob: Blob,
    opts: { filename?: string; kind?: string } = {},
  ): Promise<ProductAttachment> {
    const mime = blob.type || 'application/octet-stream';
    const filename = opts.filename ?? (blob instanceof File ? blob.name : `file-${Date.now()}`);
    const kind = opts.kind ?? (mime.startsWith('image/') ? 'image' : undefined);
    const data_b64 = await fileToB64(blob);
    uploading = true;
    try {
      const att = await product.uploadAttachment({ filename, mime, kind, data_b64 });
      localAtts = [att, ...localAtts];
      // Eagerly load preview for images.
      if (mime.startsWith('image/')) void loadAttUrl(att.id);
      onAttachmentCreated?.(att);
      return att;
    } finally {
      uploading = false;
    }
  }

  /** Upload files from a FileList (drag-drop or file-picker). */
  async function uploadFiles(files: FileList | File[]): Promise<void> {
    for (const file of Array.from(files)) {
      try {
        await uploadBlob(file, { filename: file.name });
      } catch (e) {
        toasts.error(`Upload failed: ${file.name}`, e instanceof Error ? e.message : String(e));
      }
    }
  }

  /** Handle paste event directly on the panel (not in the body textarea). */
  function handlePanelPaste(e: ClipboardEvent): void {
    if (!e.clipboardData) return;
    for (const item of Array.from(e.clipboardData.items)) {
      if (item.kind === 'file' && item.type.startsWith('image/')) {
        e.preventDefault();
        const blob = item.getAsFile();
        if (!blob) continue;
        const idx = ++screenshotIdx;
        void (async () => {
          try {
            await uploadBlob(blob, { filename: `screenshot-${idx}.png`, kind: 'image' });
          } catch (ex) {
            toasts.error('Screenshot upload failed', ex instanceof Error ? ex.message : String(ex));
          }
        })();
        break; // handle first image item only
      }
    }
  }

  /** Drag-over: allow drop. */
  function handleDragOver(e: DragEvent): void {
    e.preventDefault();
    dragOver = true;
  }
  function handleDragLeave(): void {
    dragOver = false;
  }

  /** Drop handler: upload all dropped files. */
  function handleDrop(e: DragEvent): void {
    e.preventDefault();
    dragOver = false;
    if (e.dataTransfer?.files?.length) {
      void uploadFiles(e.dataTransfer.files);
    }
  }

  /** File-picker change handler. */
  function handleFileInput(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    if (input.files?.length) {
      void uploadFiles(input.files);
      input.value = ''; // reset so the same file can be re-selected
    }
  }

  /** Mark as mockup. */
  async function markAsMockup(att: ProductAttachment): Promise<void> {
    try {
      const updated = await product.patchAttachment(att.id, { kind: 'mockup' });
      localAtts = localAtts.map((a) => (a.id === att.id ? updated : a));
      toasts.info('Marked as mockup');
    } catch (e) {
      toasts.error('Could not update attachment', e instanceof Error ? e.message : String(e));
    }
  }

  /** Delete an attachment with confirm. */
  async function deleteAtt(att: ProductAttachment): Promise<void> {
    const ok = await confirmer.ask(`Delete "${att.filename}"?`, {
      title: 'Delete attachment',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await product.deleteAttachment(att.id);
      localAtts = localAtts.filter((a) => a.id !== att.id);
      // Revoke cached blob URL if any.
      const url = localAttUrls[att.id];
      if (url) URL.revokeObjectURL(url);
      const { [att.id]: _removed, ...rest } = localAttUrls;
      localAttUrls = rest;
      toasts.info('Attachment deleted');
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="att-panel"
  class:drag-over={dragOver}
  role="region"
  aria-label="Story attachments"
  onpaste={handlePanelPaste}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  <!-- Header row -->
  <div class="att-header-row">
    <span class="att-panel-title">Attachments</span>
    <label class="att-pick-btn" title="Attach files">
      {uploading ? 'Uploading…' : '+ File'}
      <input
        type="file"
        multiple
        style="display:none"
        onchange={handleFileInput}
        disabled={uploading}
      />
    </label>
  </div>

  <!-- Drop-zone hint when empty -->
  {#if localAttLoading}
    <div class="att-empty">Loading…</div>
  {:else if localAtts.length === 0 && !uploading}
    <div class="att-drop-hint">
      Drop files here, use + File, or paste a screenshot (⌘⌃⇧4)
    </div>
  {/if}

  <!-- Uploading indicator -->
  {#if uploading}
    <div class="att-uploading">Uploading…</div>
  {/if}

  <!-- Attachment list -->
  {#if localAtts.length > 0}
    <div class="att-list">
      {#each localAtts as att (att.id)}
        <div class="att-item">
          <!-- Name + meta -->
          <div class="att-meta-row">
            <span class="att-fname" title={att.filename}>{att.filename}</span>
            <span class="att-size">{fmtBytes(att.size_bytes)}</span>
            {#if att.kind && att.kind !== 'image'}
              <span class="att-kind-badge">{att.kind}</span>
            {/if}
          </div>

          <!-- Preview area -->
          {#if att.mime.startsWith('image/')}
            <div class="att-preview">
              {#if localAttUrls[att.id]}
                <img
                  class="att-img"
                  src={localAttUrls[att.id]}
                  alt={att.filename}
                />
              {:else if localAttUrlLoading[att.id]}
                <span class="att-loading-hint">Loading preview…</span>
              {:else}
                <button
                  class="att-load-btn"
                  onclick={() => loadAttUrl(att.id)}
                >
                  Load preview
                </button>
              {/if}
            </div>
          {:else if att.mime === 'application/pdf'}
            <div class="att-preview att-pdf-wrap">
              {#if localAttUrls[att.id]}
                <iframe
                  class="att-pdf"
                  src={localAttUrls[att.id]}
                  title={att.filename}
                  sandbox="allow-scripts allow-same-origin"
                ></iframe>
              {:else}
                <button
                  class="att-load-btn"
                  onclick={() => loadAttUrl(att.id)}
                  disabled={localAttUrlLoading[att.id]}
                >
                  {localAttUrlLoading[att.id] ? 'Loading…' : 'Preview PDF'}
                </button>
              {/if}
            </div>
          {:else}
            <!-- Non-image, non-PDF: file chip + download -->
            <div class="att-file-chip">
              <span class="att-chip-icon">📄</span>
              <span class="att-chip-name">{att.filename}</span>
              {#if localAttUrls[att.id]}
                <a
                  class="att-dl-link"
                  href={localAttUrls[att.id]}
                  download={att.filename}
                >Download</a>
              {:else}
                <button
                  class="att-load-btn"
                  onclick={() => loadAttUrl(att.id)}
                  disabled={localAttUrlLoading[att.id]}
                >
                  {localAttUrlLoading[att.id] ? 'Preparing…' : 'Download'}
                </button>
              {/if}
            </div>
          {/if}

          <!-- Action row -->
          <div class="att-action-row">
            {#if att.kind !== 'mockup'}
              <button
                class="att-action-btn"
                onclick={() => markAsMockup(att)}
                title="Mark as mockup"
              >Mark as mockup</button>
            {:else}
              <span class="att-kind-badge mockup-badge">mockup</span>
            {/if}
            <button
              class="att-action-btn att-delete-btn"
              onclick={() => deleteAtt(att)}
              title="Delete attachment"
              aria-label="Delete {att.filename}"
            >Delete</button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .att-panel {
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    padding: 10px 12px;
    margin-top: 12px;
    background: var(--surface, transparent);
    transition: border-color 120ms, background 120ms;
  }
  .att-panel.drag-over {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }

  .att-header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .att-panel-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .att-pick-btn {
    font-size: 11px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    cursor: pointer;
    color: var(--text-dim);
    background: transparent;
    transition: border-color 100ms, color 100ms;
  }
  .att-pick-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .att-empty,
  .att-uploading {
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
    padding: 4px 0;
  }
  .att-drop-hint {
    font-size: 11.5px;
    color: var(--text-dim);
    text-align: center;
    padding: 10px 0;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s, 4px);
  }

  .att-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .att-item {
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    padding: 8px 10px;
    background: var(--bg, transparent);
  }
  .att-meta-row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 4px;
  }
  .att-fname {
    font-size: 12px;
    font-weight: 500;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .att-size {
    font-size: 10.5px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .att-kind-badge {
    font-size: 9.5px;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 28%, transparent);
    flex-shrink: 0;
  }
  .mockup-badge {
    background: color-mix(in srgb, #f59e0b 14%, transparent);
    color: #d97706;
    border-color: color-mix(in srgb, #f59e0b 28%, transparent);
  }

  /* Preview containers */
  .att-preview {
    margin: 6px 0 4px;
    min-height: 40px;
  }
  .att-img {
    max-width: 100%;
    max-height: 200px;
    border-radius: var(--radius-s, 4px);
    object-fit: contain;
    display: block;
  }
  .att-pdf-wrap {
    height: 200px;
  }
  .att-pdf {
    width: 100%;
    height: 100%;
    border: none;
    border-radius: var(--radius-s, 4px);
  }
  .att-loading-hint {
    font-size: 11px;
    color: var(--text-dim);
    font-style: italic;
  }
  .att-load-btn {
    font-size: 11px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .att-load-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .att-load-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* File chip for non-image, non-PDF */
  .att-file-chip {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 0;
  }
  .att-chip-icon {
    font-size: 14px;
  }
  .att-chip-name {
    font-size: 12px;
    color: var(--text-dim);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .att-dl-link {
    font-size: 11px;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
    flex-shrink: 0;
  }

  /* Action row */
  .att-action-row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
  }
  .att-action-btn {
    font-size: 10.5px;
    padding: 2px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    transition: border-color 100ms, color 100ms;
  }
  .att-action-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .att-delete-btn:hover {
    border-color: #ef4444;
    color: #ef4444;
  }
</style>
