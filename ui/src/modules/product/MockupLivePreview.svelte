<script lang="ts">
  // MockupLivePreview — renders an in-memory design SOURCE (not a stored
  // attachment) for the live "Create with AI" preview, for every design format:
  //   • html / mermaid → SANDBOXED iframe with the most-restrictive sandbox=""
  //     (scripts OFF, no allow-same-origin) — the source is agent-authored and
  //     untrusted. Mermaid is rendered to an SVG string first (never inlined).
  //   • excalidraw → the read-only DesignBoard island.
  //   • scene3d → the read-only Scene3DViewport (validated JSON only; a doc that
  //     doesn't parse shows the raw source so you can see what the agent wrote).
  import mermaid from 'mermaid';
  import type { DesignFormat } from './types';
  import { svgDoc } from './design/format';
  import DesignBoard from './design/DesignBoard.svelte';
  import { Scene3DViewport, parseScene, type Scene3dDoc } from './design/scene3d';
  import { product } from '../../lib/stores/product.svelte';

  interface Props {
    format: DesignFormat;
    content: string;
  }
  const { format, content }: Props = $props();

  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' });

  let mermaidSvg = $state<string | null>(null);
  let mermaidErr = $state<string | null>(null);
  let seq = 0;

  // Re-render mermaid whenever the source changes. HTML needs no pre-processing —
  // it goes straight into the iframe srcdoc.
  $effect(() => {
    const src = content ?? '';
    if (format !== 'mermaid') {
      mermaidSvg = null;
      mermaidErr = null;
      return;
    }
    const text = src.trim();
    if (!text) {
      mermaidSvg = null;
      mermaidErr = null;
      return;
    }
    const token = ++seq;
    void mermaid
      .render(`otto-mockup-live-${Date.now()}-${token}`, text)
      .then(({ svg }) => {
        if (token === seq) {
          mermaidSvg = svg;
          mermaidErr = null;
        }
      })
      .catch((e: unknown) => {
        if (token === seq) mermaidErr = e instanceof Error ? e.message : String(e);
      });
  });

  /** The VALIDATED scene3d doc, or null while the agent's JSON is still
   *  incomplete / invalid (the raw source is shown instead). */
  const sceneDoc = $derived.by<Scene3dDoc | null>(() => {
    if (format !== 'scene3d' || !content?.trim()) return null;
    return parseScene(content).doc;
  });
  const resolveAttachment = (aid: string) => product.attachmentBlobUrl(aid);
</script>

<div class="live-box">
  {#if !content || !content.trim()}
    <div class="live-msg">Waiting for the agent…</div>
  {:else if format === 'html'}
    <iframe class="live-frame" title="Mockup preview" sandbox="" srcdoc={content}></iframe>
  {:else if format === 'excalidraw'}
    <DesignBoard source={content} readonly />
  {:else if format === 'scene3d'}
    {#if sceneDoc}
      <Scene3DViewport doc={sceneDoc} readonly resolveAttachment={resolveAttachment} onchange={() => {}} />
    {:else}
      <pre class="live-src">{content}</pre>
    {/if}
  {:else if mermaidErr}
    <div class="live-msg err">Diagram error: {mermaidErr}</div>
  {:else if mermaidSvg !== null}
    <iframe class="live-frame" title="Mockup preview" sandbox="" srcdoc={svgDoc(mermaidSvg)}></iframe>
  {:else}
    <div class="live-msg">Rendering…</div>
  {/if}
</div>

<style>
  .live-box {
    flex: 1;
    min-height: 0;
    background: #fff;
    border-radius: var(--radius-s);
    box-shadow: 0 0 0 1px var(--border);
    overflow: hidden;
    position: relative;
    display: flex;
    flex-direction: column;
  }
  .live-frame {
    display: block;
    width: 100%;
    height: 100%;
    min-height: 320px;
    border: none;
    background: #fff;
  }
  .live-msg {
    padding: 24px;
    font-size: 12.5px;
    color: var(--text-dim);
    text-align: center;
  }
  .live-msg.err {
    color: #ef4444;
  }
  .live-src {
    margin: 0;
    padding: 12px;
    flex: 1;
    overflow: auto;
    font: 11.5px/1.5 var(--font-mono, monospace);
    color: #334155;
    background: #f8fafc;
  }
</style>
