<script lang="ts">
  // The stage of the artifact view (and each side of Compare): renders one
  // version of a design by format with the EXISTING viewers/editors —
  // DeviceFrame + a sandboxed iframe for HTML, the Canvas Mermaid/D2 renderers,
  // the Design Arena's Excalidraw board and scene3d viewport — and, when
  // editable, the shared CodeEditor for text sources. It never saves: every
  // edit goes up through `onchange` and the view owns versions + conflicts.
  //
  // Untrusted content stays isolated: HTML and rendered diagrams only ever run
  // inside `sandbox=""` iframes (no scripts, opaque origin); SVG renders as an
  // <img>; scene3d JSON is parsed + validated before it reaches the viewport.
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { DesignArtifact } from '../../lib/api/types';
  import DeviceFrame, { type DeviceKind } from '../product/design/DeviceFrame.svelte';
  import DesignBoard from '../product/design/DesignBoard.svelte';
  import {
    Scene3DViewport,
    emptyScene,
    parseScene,
    serializeScene,
    type Scene3dDoc,
    type Scene3dObject,
  } from '../product/design/scene3d';
  import { renderMermaid } from '../canvas/mermaid';
  import { renderD2 } from '../canvas/d2';
  import { brandColors, contrastRatio, renderKind } from './model';
  import SiteStudio from './site/SiteStudio.svelte';
  import { resolveModelRef } from './studio3d/sources';

  interface Props {
    artifact: DesignArtifact;
    /** Text/JSON source of the shown version (null for binaries). */
    source: string | null;
    /** Object URL of the shown version's bytes (binaries). */
    blobUrl?: string | null;
    readonly?: boolean;
    /** Show the source editor beside the preview (text formats). */
    showSource?: boolean;
    device?: DeviceKind;
    /** scene3d selection (the view's Hierarchy/Inspector share it). */
    selectedId?: string | null;
    /** A compact render for Compare panes (no source pane, no chrome). */
    compact?: boolean;
    onchange?: (source: string) => void;
  }
  let {
    artifact,
    source,
    blobUrl = null,
    readonly = false,
    showSource = false,
    device = 'none',
    selectedId = $bindable<string | null>(null),
    compact = false,
    onchange,
  }: Props = $props();

  const kind = $derived(renderKind(artifact.format));

  // "Fit" HTML: a fixed desktop viewport scaled down (never up) to the pane.
  const FIT_W = 1280;
  let fitW = $state(0);
  let fitH = $state(0);
  const fitScale = $derived(fitW > 0 ? Math.min(1, fitW / FIT_W) : 1);
  const editable = $derived(!readonly && !!onchange);

  // ── otto-canvas (an imported Canvas scene): {type, format, source} ─────────
  interface CanvasDoc {
    type?: string;
    format?: string;
    source?: string;
    [k: string]: unknown;
  }
  const canvasDoc = $derived.by<CanvasDoc | null>(() => {
    if (kind !== 'canvas' || !source) return null;
    try {
      const d = JSON.parse(source);
      return d && typeof d === 'object' ? (d as CanvasDoc) : null;
    } catch {
      return null;
    }
  });
  /** What actually renders: the diagram kind + its inner source. */
  const inner = $derived.by<{ kind: 'mermaid' | 'd2' | 'excalidraw' | 'html' | 'svg' | null; src: string }>(() => {
    if (kind === 'canvas') {
      const f = canvasDoc?.format;
      const k = f === 'mermaid' || f === 'd2' || f === 'excalidraw' ? f : null;
      return { kind: k, src: typeof canvasDoc?.source === 'string' ? canvasDoc.source : '' };
    }
    if (kind === 'mermaid' || kind === 'd2' || kind === 'excalidraw' || kind === 'html' || kind === 'svg') {
      return { kind, src: source ?? '' };
    }
    return { kind: null, src: '' };
  });

  /** Emit an edit of the inner source, re-wrapped for canvas docs. */
  function emitInner(next: string): void {
    if (!editable) return;
    if (kind === 'canvas' && canvasDoc) onchange?.(JSON.stringify({ ...canvasDoc, source: next }));
    else onchange?.(next);
  }

  // ── Diagram rendering (mermaid / d2) into a sandboxed frame ───────────────
  let diagramSvg = $state<string | null>(null);
  let diagramError = $state<string | null>(null);
  let renderSeq = 0;
  $effect(() => {
    const k = inner.kind;
    const src = inner.src;
    if (k !== 'mermaid' && k !== 'd2') return;
    const my = ++renderSeq;
    const t = setTimeout(async () => {
      const r = k === 'mermaid' ? await renderMermaid(`dh-stage-${my}`, src) : await renderD2(`dh-${my}`, src);
      if (my !== renderSeq) return;
      diagramSvg = r.svg ?? null;
      diagramError = r.svg ? null : (r.error ?? 'Diagram error');
    }, 150);
    return () => clearTimeout(t);
  });

  function svgDoc(svg: string): string {
    return (
      '<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;background:#fff}' +
      'body{padding:24px;display:flex;justify-content:center}svg{max-width:100%;height:auto}</style></head>' +
      `<body>${svg}</body></html>`
    );
  }

  const svgDataUrl = $derived(
    inner.kind === 'svg' && inner.src.trim() ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(inner.src)}` : null,
  );

  // ── scene3d / models ──────────────────────────────────────────────────────
  const sceneParse = $derived(kind === 'scene3d' && source ? parseScene(source) : null);
  const sceneDoc = $derived<Scene3dDoc | null>(sceneParse?.ok ? sceneParse.doc : null);
  const modelDoc = $derived.by<Scene3dDoc | null>(() => {
    if (kind !== 'model') return null;
    const base = emptyScene();
    const hero: Scene3dObject = {
      id: 'model',
      name: artifact.title,
      type: 'gltf',
      attachment_id: artifact.id,
      position: [0, 0, 0],
      rotation: [0, 0, 0],
      scale: [1, 1, 1],
    };
    return { ...base, objects: [...base.objects, hero] };
  });
  /** gltf references: the model's own bytes, else a v2 `otto://design` src or a
   *  legacy id (a Design Hall artifact, else a Product attachment). */
  function resolveAttachment(aid: string): Promise<string> {
    if (kind === 'model' && aid === artifact.id && blobUrl) return Promise.resolve(blobUrl);
    return resolveModelRef(aid);
  }

  // ── Brand kit ─────────────────────────────────────────────────────────────
  const brandDoc = $derived.by<Record<string, unknown> | null>(() => {
    if (kind !== 'brand' || !source) return null;
    try {
      return JSON.parse(source) as Record<string, unknown>;
    } catch {
      return null;
    }
  });
  const swatches = $derived(brandColors(brandDoc));

  // Source pane: which file the editor highlights as.
  const editorPath = $derived(
    ({ html: 'design.html', svg: 'design.svg', mermaid: 'design.mmd', d2: 'design.d2' } as Record<string, string>)[
      inner.kind ?? ''
    ] ?? 'design.json',
  );
  const hasSourcePane = $derived(
    !compact && showSource && (inner.kind === 'html' || inner.kind === 'svg' || inner.kind === 'mermaid' || inner.kind === 'd2' || kind === 'brand' || kind === 'json' || kind === 'scene3d'),
  );
  /** The source pane edits the INNER source for canvas docs, the document otherwise. */
  const paneSource = $derived(kind === 'canvas' ? inner.src : (source ?? ''));
  function onPane(v: string): void {
    if (kind === 'canvas') emitInner(v);
    else if (editable) onchange?.(v);
  }
</script>

<div class="stage" class:split={hasSourcePane} class:compact>
  <div class="view" data-testid="design-stage">
    {#if artifact.format === 'otto-site'}
      <SiteStudio {artifact} {source} readonly={!editable} {compact} onchange={(s) => editable && onchange?.(s)} />
    {:else if kind === 'html' || inner.kind === 'html'}
      {#if compact || device === 'none'}
        <!-- Fit: render at a 1280px desktop viewport and scale it into the pane,
             so a page reads as a page (not reflowed to the pane's width). -->
        <div class="fit" bind:clientWidth={fitW} bind:clientHeight={fitH}>
          <iframe
            class="doc scaled"
            title={artifact.title}
            sandbox=""
            srcdoc={inner.src}
            style:width={`${FIT_W}px`}
            style:height={`${Math.ceil(fitH / fitScale)}px`}
            style:transform={`scale(${fitScale})`}
          ></iframe>
          {#if !compact}<span class="fit-label">Desktop · {FIT_W} px · {Math.round(fitScale * 100)}%</span>{/if}
        </div>
      {:else}
        <DeviceFrame {device}>
          <iframe class="doc" title={artifact.title} sandbox="" srcdoc={inner.src}></iframe>
        </DeviceFrame>
      {/if}
    {:else if inner.kind === 'svg'}
      <div class="paper center">
        {#if svgDataUrl}<img class="svg" src={svgDataUrl} alt={artifact.title} />{:else}<p class="msg">Empty SVG.</p>{/if}
      </div>
    {:else if inner.kind === 'mermaid' || inner.kind === 'd2'}
      {#if diagramError}
        <div class="msg err" role="status"><Icon name="warning" size={14} /> {diagramError}</div>
      {/if}
      {#if diagramSvg}
        <iframe class="doc" title={artifact.title} sandbox="" srcdoc={svgDoc(diagramSvg)}></iframe>
      {:else if !diagramError}
        <p class="msg">Rendering…</p>
      {/if}
    {:else if inner.kind === 'excalidraw'}
      <div class="board">
        <DesignBoard source={inner.src} readonly={!editable} onchange={emitInner} />
      </div>
    {:else if kind === 'canvas'}
      <p class="msg">This Canvas board uses a format Design Hall can’t render yet. Open it in Canvas.</p>
    {:else if kind === 'scene3d'}
      {#if sceneDoc}
        <Scene3DViewport
          doc={sceneDoc}
          readonly={!editable}
          bind:selectedId
          onchange={(d) => editable && onchange?.(serializeScene(d))}
          {resolveAttachment}
        />
      {:else}
        <div class="msg err">
          <p>Not a valid 3D scene document{editable ? ' — fix it in the source view' : ''}.</p>
          {#if sceneParse && !sceneParse.ok}
            <ul>
              {#each sceneParse.issues.slice(0, 8) as i (i.path + i.message)}
                <li><span class="mono">{i.path || '(root)'}</span> — {i.message}</li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
    {:else if kind === 'model' && modelDoc && blobUrl}
      <Scene3DViewport doc={modelDoc} readonly onchange={() => {}} {resolveAttachment} />
    {:else if kind === 'image' && blobUrl}
      <div class="paper center"><img class="img" src={blobUrl} alt={artifact.title} /></div>
    {:else if kind === 'pdf' && blobUrl}
      <object class="doc" data={blobUrl} type="application/pdf" title={artifact.title}>
        <p class="msg">PDF preview isn’t available here. <a href={blobUrl} download={`${artifact.title}.pdf`}>Download the PDF</a>.</p>
      </object>
    {:else if kind === 'brand'}
      {#if brandDoc}
        <div class="brand">
          <h3>Colours</h3>
          {#if swatches.length}
            <div class="swatches">
              {#each swatches as c (c.name)}
                {@const onWhite = contrastRatio(c.value, '#ffffff')}
                <div class="swatch">
                  <span class="chip-color" style:background={c.value}></span>
                  <span class="sw-name">{c.name}</span>
                  <span class="mono">{c.value}</span>
                  {#if onWhite != null}
                    <span class="ratio" class:fail={onWhite < 4.5} title="Contrast as text on white">on white {onWhite}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {:else}
            <p class="msg">No colour tokens yet — add them under <span class="mono">color</span> in the source.</p>
          {/if}
          <p class="note"><Icon name="info" size={12} /> Edit colours, type, spacing, logos and voice — with live contrast and an impact preview — in the <a href="#/design/brand/{encodeURIComponent(artifact.id)}" data-testid="design-open-brand-kit">Brand Kit editor</a>.</p>
        </div>
      {:else}
        <p class="msg err">The brand kit document isn’t valid JSON.</p>
      {/if}
    {:else if kind === 'json'}
      <p class="msg">This studio’s editor isn’t built yet (planned). The document is stored and versioned; view it as source.</p>
    {:else if (kind === 'image' || kind === 'model' || kind === 'pdf') && !blobUrl}
      <p class="msg">Loading…</p>
    {:else}
      <p class="msg">Design Hall can’t preview {artifact.format} files yet.</p>
    {/if}
  </div>
  {#if hasSourcePane}
    <div class="source" aria-label="Source">
      <CodeEditor
        path={editorPath}
        root=""
        content={paneSource}
        readOnly={!editable}
        completionSource={() => null}
        onchange={onPane}
      />
    </div>
  {/if}
</div>

<style>
  .stage {
    height: 100%;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
  }
  .stage.split {
    grid-template-columns: minmax(0, 1fr) minmax(280px, 42%);
  }
  .view {
    position: relative;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: auto;
  }
  .source {
    min-width: 0;
    min-height: 0;
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
    display: flex;
    flex-direction: column;
  }
  .source > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .doc {
    flex: 1;
    width: 100%;
    min-height: 360px;
    border: 0;
    background: white;
  }
  .compact .doc {
    min-height: 0;
  }
  .fit {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .doc.scaled {
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
    min-height: 0;
    flex: none;
    transform-origin: 0 0;
  }
  :global([dir='rtl']) .doc.scaled {
    transform-origin: 100% 0;
  }
  .fit-label {
    position: absolute;
    inset-block-end: 10px;
    inset-inline-start: 10px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .paper {
    flex: 1;
    background: white;
  }
  .center {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
  }
  .svg,
  .img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .board {
    flex: 1;
    min-height: 360px;
    display: flex;
  }
  .board > :global(*) {
    flex: 1;
  }
  .msg {
    margin: 16px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .msg.err {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--danger);
  }
  .msg.err ul {
    margin: 0;
    padding-inline-start: 18px;
    color: var(--text);
  }
  .brand {
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .brand h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .swatches {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 10px;
  }
  .swatch {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-areas: 'c n' 'c v' 'c r';
    column-gap: 10px;
    padding: 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    align-items: center;
  }
  .chip-color {
    grid-area: c;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
  .sw-name {
    grid-area: n;
    font-weight: 600;
    font-size: var(--fs-s);
  }
  .swatch .mono {
    grid-area: v;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ratio {
    grid-area: r;
    font-size: var(--fs-xs);
    color: var(--success);
  }
  .ratio.fail {
    color: var(--danger);
  }
  .note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @container (max-width: 760px) {
    .stage.split {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 1fr) minmax(200px, 40%);
    }
    .source {
      border-inline-start: 0;
      border-block-start: 1px solid var(--border);
    }
  }
</style>
