<script lang="ts">
  // MockupViewer — renders a single mockup attachment by type, isolating all
  // untrusted/agent-controlled content from the parent DOM (design §8):
  //   • image (incl. svg) → <img src={authed blob url}>  (svg rendered AS AN
  //     IMAGE, never inlined — avoids SVG-borne script execution).
  //   • html (text/html) → sandboxed <iframe srcdoc={content}>; sandbox does NOT
  //     include allow-scripts by default. A per-mockup "Enable interactivity"
  //     toggle adds allow-scripts and shows a visible warning.
  //   • mermaid (text/vnd.mermaid or *.mmd) → mermaid.render() → SVG STRING, then
  //     embedded inside a SANDBOXED iframe srcdoc (NOT inlined into the parent
  //     DOM, since mermaid input is agent/user-controlled).
  // The render box is exposed to a sibling MockupAnnotations overlay so pins sit
  // exactly over the mockup.
  import { onMount } from 'svelte';
  import mermaid from 'mermaid';
  import { authedBlobUrl, baseUrl, getToken } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import MockupAnnotations from './MockupAnnotations.svelte';
  import type { ProductAttachment } from './types';

  interface Props {
    attachment: ProductAttachment;
  }
  const { attachment }: Props = $props();

  // Mermaid is initialized once, with auto-rendering off — we drive render()
  // ourselves and never let it touch the parent document.
  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' });

  // ── Derived type classification ─────────────────────────────────────────────
  type Kind = 'image' | 'html' | 'mermaid' | 'unknown';
  function classify(a: ProductAttachment): Kind {
    const mime = (a.mime || '').toLowerCase();
    const name = (a.filename || '').toLowerCase();
    if (mime === 'text/vnd.mermaid' || name.endsWith('.mmd')) return 'mermaid';
    if (mime === 'text/html') return 'html';
    // svg + raster images both render as <img>.
    if (mime === 'image/svg+xml' || mime.startsWith('image/')) return 'image';
    return 'unknown';
  }
  const kind = $derived(classify(attachment));

  // ── State ─────────────────────────────────────────────────────────────────
  // image: authed blob URL.
  let imgUrl = $state<string | null>(null);
  // html: raw content text rendered in the iframe srcdoc.
  let htmlContent = $state<string | null>(null);
  // mermaid: the rendered SVG string (embedded in a sandboxed iframe srcdoc).
  let mermaidSvg = $state<string | null>(null);

  let rendering = $state(false);
  let renderError = $state<string | null>(null);

  // Per-mockup "Enable interactivity" — adds allow-scripts to the HTML sandbox.
  let allowScripts = $state(false);

  // The render-box element passed to the annotation overlay so pins anchor to it.
  let renderBox = $state<HTMLDivElement | null>(null);

  // Object URLs we created (revoke on unmount / re-render).
  let createdUrls: string[] = [];

  // Unique counter so mermaid render ids never collide across re-renders.
  let mermaidSeq = 0;

  // ── Helpers ─────────────────────────────────────────────────────────────────

  /** Fetch attachment bytes as text with the bearer token. */
  async function fetchText(id: string): Promise<string> {
    const token = getToken();
    const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
    const resp = await fetch(`${baseUrl()}/api/v1/product/attachments/${id}`, { headers });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    return resp.text();
  }

  /** Wrap an SVG string in a minimal sandboxed HTML doc for iframe srcdoc. */
  function svgDoc(svg: string): string {
    return `<!doctype html><html><head><meta charset="utf-8">` +
      `<style>html,body{margin:0;padding:8px;background:#fff;}` +
      `svg{max-width:100%;height:auto;display:block;margin:0 auto;}</style></head>` +
      `<body>${svg}</body></html>`;
  }

  function revokeCreated(): void {
    const stale = createdUrls.splice(0);
    for (const u of stale) URL.revokeObjectURL(u);
  }

  // ── Render pipeline — re-runs whenever the attachment changes ────────────────
  $effect(() => {
    // Read the attachment id so this effect re-runs on change.
    const a = attachment;
    void renderFor(a);
  });

  $effect(() => {
    // Revoke object URLs on unmount.
    return () => revokeCreated();
  });

  async function renderFor(a: ProductAttachment): Promise<void> {
    rendering = true;
    renderError = null;
    imgUrl = null;
    htmlContent = null;
    mermaidSvg = null;
    revokeCreated();
    try {
      const k = classify(a);
      if (k === 'image') {
        const url = await authedBlobUrl(`/product/attachments/${a.id}`);
        createdUrls.push(url);
        imgUrl = url;
      } else if (k === 'html') {
        htmlContent = await fetchText(a.id);
      } else if (k === 'mermaid') {
        const src = await fetchText(a.id);
        const id = `otto-mermaid-${Date.now()}-${mermaidSeq++}`;
        const { svg } = await mermaid.render(id, src);
        mermaidSvg = svg;
      } else {
        renderError = `Unsupported mockup type: ${a.mime || 'unknown'}`;
      }
    } catch (e) {
      renderError = e instanceof Error ? e.message : String(e);
      toasts.error('Could not render mockup', renderError);
    } finally {
      rendering = false;
    }
  }

  // Sandbox string for the HTML iframe. Without interactivity it is the bare
  // sandbox="" (most restrictive: scripts, forms, popups, same-origin all off).
  // With interactivity, only allow-scripts is added (still no allow-same-origin,
  // so the frame stays in an opaque origin and cannot reach the daemon/cookies).
  const htmlSandbox = $derived(allowScripts ? 'allow-scripts' : '');

  onMount(() => {
    // No-op; kept for symmetry / future hooks.
  });
</script>

<div class="viewer">
  <!-- Toolbar -->
  <div class="viewer-toolbar">
    <span class="vt-name" title={attachment.filename}>{attachment.filename}</span>
    <span class="vt-mime">{attachment.mime}</span>
    {#if kind === 'html'}
      <label class="vt-toggle" title="Run scripts inside the sandboxed iframe">
        <input type="checkbox" bind:checked={allowScripts} />
        Enable interactivity
      </label>
    {/if}
  </div>

  {#if kind === 'html' && allowScripts}
    <div class="warn">
      <Icon name="info" size={13} />
      Interactivity is ON — this mockup's scripts run inside a sandboxed iframe
      (no same-origin access). Only enable for content you trust.
    </div>
  {/if}

  <!-- Render box: the annotation overlay is positioned over this exact element.
       The annotations component renders the overlay (pins + inline editor) as a
       child of this box, plus a side note-list below it. -->
  <div class="render-wrap">
    <div class="render-box" bind:this={renderBox}>
      {#if rendering}
        <div class="render-msg">Rendering…</div>
      {:else if renderError}
        <div class="render-msg err">{renderError}</div>
      {:else if kind === 'image' && imgUrl}
        <!-- SVG and raster images both render as an image (never inlined). -->
        <img class="mockup-img" src={imgUrl} alt={attachment.filename} />
      {:else if kind === 'html' && htmlContent !== null}
        <iframe
          class="mockup-frame"
          title={attachment.filename}
          sandbox={htmlSandbox}
          srcdoc={htmlContent}
        ></iframe>
      {:else if kind === 'mermaid' && mermaidSvg !== null}
        <!-- Mermaid SVG embedded inside a sandboxed iframe (never inlined). -->
        <iframe
          class="mockup-frame"
          title={attachment.filename}
          sandbox=""
          srcdoc={svgDoc(mermaidSvg)}
        ></iframe>
      {/if}
    </div>

    <!-- Pinned-annotation overlay + side note list. Mounted once the render box
         exists so the overlay can anchor to it. -->
    {#if !rendering && !renderError && renderBox}
      <MockupAnnotations attachmentId={attachment.id} box={renderBox} />
    {/if}
  </div>
</div>

<style>
  .viewer {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .viewer-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .vt-name {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 50%;
  }
  .vt-mime {
    font-size: 10.5px;
    color: var(--text-dim);
    font-family: var(--font-mono, monospace);
  }
  .vt-toggle {
    margin-inline-start: auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .vt-toggle input {
    cursor: pointer;
  }
  .warn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: 11.5px;
    color: #b45309;
    background: color-mix(in srgb, #f59e0b 14%, transparent);
    border-bottom: 1px solid color-mix(in srgb, #f59e0b 30%, transparent);
    flex-shrink: 0;
    line-height: 1.4;
  }
  .render-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
    padding: 12px;
  }
  /* The render box is position:relative so the absolute overlay anchors to it. */
  .render-box {
    position: relative;
    min-height: 200px;
    background: #fff;
    border-radius: var(--radius-s);
    box-shadow: 0 0 0 1px var(--border);
    overflow: hidden;
  }
  .render-msg {
    padding: 24px;
    font-size: 12.5px;
    color: var(--text-dim);
    text-align: center;
  }
  .render-msg.err {
    color: #ef4444;
  }
  .mockup-img {
    display: block;
    max-width: 100%;
    height: auto;
    margin: 0 auto;
  }
  .mockup-frame {
    display: block;
    width: 100%;
    min-height: 480px;
    border: none;
    background: #fff;
  }
</style>
