<script lang="ts">
  // MockupLivePreview — renders an in-memory mockup SOURCE (not a stored
  // attachment) for the live "Create with AI" preview. Same isolation posture as
  // MockupViewer: HTML and Mermaid both render inside a SANDBOXED iframe with the
  // most-restrictive sandbox="" (scripts OFF, no allow-same-origin) — the source
  // is agent-authored and untrusted. Mermaid is rendered to an SVG string first
  // (never inlined into the parent DOM).
  import mermaid from 'mermaid';
  import type { MockupFormat } from '../../lib/stores/mockup-assist.svelte';

  interface Props {
    format: MockupFormat;
    content: string;
  }
  const { format, content }: Props = $props();

  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' });

  let mermaidSvg = $state<string | null>(null);
  let mermaidErr = $state<string | null>(null);
  let seq = 0;

  /** Wrap an SVG / HTML body string in a minimal sandbox-safe doc (mirrors
   *  MockupViewer.svgDoc). */
  function svgDoc(svg: string): string {
    return (
      `<!doctype html><html><head><meta charset="utf-8">` +
      `<style>html,body{margin:0;padding:8px;background:#fff;}` +
      `svg{max-width:100%;height:auto;display:block;margin:0 auto;}</style></head>` +
      `<body>${svg}</body></html>`
    );
  }

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
</script>

<div class="live-box">
  {#if !content || !content.trim()}
    <div class="live-msg">Waiting for the agent…</div>
  {:else if format === 'html'}
    <iframe class="live-frame" title="Mockup preview" sandbox="" srcdoc={content}></iframe>
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
</style>
