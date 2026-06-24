<script lang="ts">
  // Full-screen presentation overlay. Steps through the scene's slides; within a
  // slide, each RevealStep cumulatively fades in its nodeIds (fade+translate).
  // A slide bound to a mermaid sequence node steps that diagram's messages.
  // Controls auto-hide; keys: →/Space/← Home/End/Esc, N (notes), F (fullscreen).
  import { fly } from 'svelte/transition';
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { renderMermaid } from './mermaid';
  import { parseSeq, revealUpTo, revealAll, type SeqSteps } from './present_seq';
  import { makeNode, genId } from './scene';
  import type { CanvasNode, Scene, Slide } from './types';

  interface Props {
    onexit: () => void;
  }
  let { onexit }: Props = $props();

  const scene = $derived(canvas.scene);
  const slides = $derived<Slide[]>(scene?.slides ?? []);

  let slideIdx = $state(0);
  let stepIdx = $state(0);
  let showNotes = $state(false);
  let autoplay = $state(false);
  let controlsVisible = $state(true);
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  let autoTimer: ReturnType<typeof setInterval> | null = null;

  const slide = $derived<Slide | null>(slides[slideIdx] ?? null);
  const stepCount = $derived(Math.max(1, slide?.reveal.length ?? 1));

  // Focus the overlay on mount so its keydown handler (→/←/Esc/N/F) receives keys.
  let root = $state<HTMLDivElement | null>(null);
  $effect(() => {
    root?.focus();
  });

  // Node lookup + the scene bounding box (to scale the stage to fit).
  const nodeById = $derived.by(() => {
    const m = new Map<string, CanvasNode>();
    for (const n of scene?.nodes ?? []) m.set(n.id, n);
    return m;
  });

  // Cumulatively-revealed node ids up to the current step.
  const visibleIds = $derived.by((): Set<string> => {
    const ids = new Set<string>();
    if (!slide) return ids;
    for (let i = 0; i <= stepIdx && i < slide.reveal.length; i++) {
      for (const id of slide.reveal[i].nodeIds ?? []) ids.add(id);
    }
    // A slide with no reveal nodeIds (e.g. a pure mermaid slide) shows its frame
    // or its mermaid node by default.
    if (!ids.size && slide.mermaidNodeId) ids.add(slide.mermaidNodeId);
    return ids;
  });

  const visibleNodes = $derived.by((): CanvasNode[] =>
    [...visibleIds].map((id) => nodeById.get(id)).filter((n): n is CanvasNode => !!n),
  );

  // Stage transform: fit the visible (or all) nodes into the viewport.
  const bbox = $derived.by(() => {
    const ns = (slide && visibleNodes.length ? visibleNodes : scene?.nodes) ?? [];
    if (!ns.length) return { x: 0, y: 0, w: 800, h: 600 };
    const minX = Math.min(...ns.map((n) => n.x));
    const minY = Math.min(...ns.map((n) => n.y));
    const maxX = Math.max(...ns.map((n) => n.x + n.w));
    const maxY = Math.max(...ns.map((n) => n.y + n.h));
    return { x: minX - 40, y: minY - 40, w: maxX - minX + 80, h: maxY - minY + 80 };
  });

  function next(): void {
    if (!slide) return;
    if (stepIdx < stepCount - 1) stepIdx += 1;
    else if (slideIdx < slides.length - 1) {
      slideIdx += 1;
      stepIdx = 0;
    }
  }
  function prev(): void {
    if (stepIdx > 0) stepIdx -= 1;
    else if (slideIdx > 0) {
      slideIdx -= 1;
      stepIdx = (slides[slideIdx]?.reveal.length ?? 1) - 1;
    }
  }
  function gotoSlide(i: number): void {
    slideIdx = i;
    stepIdx = 0;
  }

  function onkeydown(e: KeyboardEvent): void {
    switch (e.key) {
      case 'ArrowRight':
      case ' ':
        e.preventDefault();
        next();
        break;
      case 'ArrowLeft':
        e.preventDefault();
        prev();
        break;
      case 'Home':
        slideIdx = 0;
        stepIdx = 0;
        break;
      case 'End':
        slideIdx = slides.length - 1;
        stepIdx = stepCount - 1;
        break;
      case 'Escape':
        onexit();
        break;
      case 'n':
      case 'N':
        showNotes = !showNotes;
        break;
      case 'f':
      case 'F':
        toggleFullscreen();
        break;
    }
  }

  function toggleFullscreen(): void {
    if (!document.fullscreenElement) document.documentElement.requestFullscreen?.().catch(() => {});
    else document.exitFullscreen?.().catch(() => {});
  }

  function nudgeControls(): void {
    controlsVisible = true;
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => (controlsVisible = false), 2200);
  }

  $effect(() => {
    if (autoplay) {
      autoTimer = setInterval(next, 2500);
      return () => autoTimer && clearInterval(autoTimer);
    }
  });

  // Teardown: clear the controls-hide timer and drop out of fullscreen if the
  // user entered it (F) and exits Present via the ✕ button rather than Esc.
  $effect(() => () => {
    if (hideTimer) clearTimeout(hideTimer);
    if (document.fullscreenElement) void document.exitFullscreen?.().catch(() => {});
  });

  // --- mermaid + sequence stepping ------------------------------------------
  let mermaidHost = $state<HTMLDivElement | null>(null);
  let seqSteps: SeqSteps | null = null;

  // Render the slide's mermaid node (if any) and wire sequence stepping.
  $effect(() => {
    const host = mermaidHost;
    const mid = slide?.mermaidNodeId;
    if (!host || !mid) {
      seqSteps = null;
      return;
    }
    const node = nodeById.get(mid);
    const src = node?.mermaid?.src;
    if (!src) return;
    void renderMermaid(`present-${genId('m')}`, src).then((r) => {
      if (r.svg && host) {
        host.innerHTML = r.svg;
        const svg = host.querySelector('svg');
        if (svg) svg.setAttribute('width', '100%');
        seqSteps = parseSeq(host.querySelector('svg'));
      }
    });
  });

  // Step the rendered sequence as the reveal step changes.
  $effect(() => {
    void stepIdx;
    const host = mermaidHost;
    if (!host || !seqSteps) return;
    const svg = host.querySelector('svg');
    const range = slide?.reveal[stepIdx]?.mermaidMessageRange;
    if (range) revealUpTo(svg, range[1], seqSteps);
    else if (slide?.mermaidNodeId && slide.reveal.length <= 1) revealAll(svg, seqSteps);
    else revealUpTo(svg, stepIdx, seqSteps);
  });

  const progress = $derived.by((): number => {
    const total = slides.reduce((a, s) => a + Math.max(1, s.reveal.length), 0) || 1;
    let done = 0;
    for (let i = 0; i < slideIdx; i++) done += Math.max(1, slides[i].reveal.length);
    done += stepIdx + 1;
    return Math.min(100, Math.round((done / total) * 100));
  });

  // --- empty: offer to auto-build slides ------------------------------------
  function autoBuild(): void {
    if (!scene) return;
    const built = buildSlides(scene);
    canvas.setScene({ ...scene, slides: built });
    slideIdx = 0;
    stepIdx = 0;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="present"
  role="dialog"
  aria-modal="true"
  tabindex="0"
  bind:this={root}
  onkeydown={onkeydown}
  onmousemove={nudgeControls}
>
  {#if !slides.length}
    <div class="empty">
      <p>This scene has no slides yet.</p>
      <button class="btn accent" onclick={autoBuild}>Auto-build slides</button>
      <button class="btn" onclick={onexit}>Exit</button>
    </div>
  {:else}
    <!-- top progress hairline -->
    <div class="progress"><div class="bar" style:width={`${progress}%`}></div></div>

    <div class="stage">
      {#if slide?.mermaidNodeId}
        <div class="mermaid-stage" bind:this={mermaidHost}></div>
      {:else}
        <div
          class="scene-stage"
          style:width={`${bbox.w}px`}
          style:height={`${bbox.h}px`}
        >
          {#each visibleNodes as n (n.id)}
            <div
              class="pnode"
              style:left={`${n.x - bbox.x}px`}
              style:top={`${n.y - bbox.y}px`}
              style:width={`${n.w}px`}
              style:height={`${n.h}px`}
              in:fly={{ y: 8, duration: 200 }}
            >
              {#if n.kind === 'sticky'}
                <div class="sticky" style:background={n.sticky?.color ?? '#ffe9a8'}>
                  {n.sticky?.value ?? n.label ?? ''}
                </div>
              {:else if n.kind === 'text'}
                <div class="text" style:text-align={n.text?.align ?? 'left'}>
                  {n.text?.value ?? n.label ?? ''}
                </div>
              {:else if n.kind === 'code'}
                <pre class="code">{n.code?.value ?? ''}</pre>
              {:else if n.kind === 'json'}
                <pre class="code">{n.json?.value ?? ''}</pre>
              {:else if n.kind === 'frame'}
                <div class="frame">{n.label ?? 'Frame'}</div>
              {:else}
                <div
                  class="shape"
                  class:ellipse={n.shape?.variant === 'ellipse'}
                  style:background={n.shape?.fill ?? 'var(--surface)'}
                  style:border-color={n.shape?.stroke ?? 'var(--border)'}
                >
                  {n.label ?? ''}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>

    {#if showNotes && slide?.notes}
      <div class="notes">{slide.notes}</div>
    {/if}

    <!-- controls -->
    <div class="controls" class:hidden={!controlsVisible}>
      <button class="ctl" onclick={prev} aria-label="Previous"><Icon name="chevronLeft" /></button>
      <span class="counter">{slideIdx + 1} / {slides.length} · step {stepIdx + 1}/{stepCount}</span>
      <button class="ctl" onclick={next} aria-label="Next"><Icon name="chevronRight" /></button>
      <span class="dots">
        {#each slides as s, i (s.id)}
          <button class="dot" class:on={i === slideIdx} onclick={() => gotoSlide(i)} aria-label={`Slide ${i + 1}`}></button>
        {/each}
      </span>
      <button class="ctl" class:on={autoplay} onclick={() => (autoplay = !autoplay)} title="Autoplay">
        <Icon name="play" />
      </button>
      <button class="ctl" class:on={showNotes} onclick={() => (showNotes = !showNotes)} title="Notes (N)">
        <Icon name="note" />
      </button>
      <button class="ctl" onclick={onexit} aria-label="Exit (Esc)"><Icon name="x" /></button>
    </div>
  {/if}
</div>

<script lang="ts" module>
  import type { Scene as SceneT, Slide as SlideT } from './types';
  // Auto-build one slide per frame (revealing the frame's children), else a
  // single slide revealing all nodes; for a lone sequence node, one step per msg.
  export function buildSlides(scene: SceneT): SlideT[] {
    const frames = scene.nodes.filter((n) => n.kind === 'frame');
    if (frames.length) {
      return frames.map((f, i) => {
        const children = scene.nodes.filter((n) => n.parent === f.id).map((n) => n.id);
        return {
          id: `slide-${i}-${f.id}`,
          title: f.label ?? `Slide ${i + 1}`,
          frameNodeId: f.id,
          reveal: [{ nodeIds: [f.id, ...children] }],
        };
      });
    }
    const seq = scene.nodes.find((n) => n.kind === 'mermaid' && n.mermaid?.kind === 'sequence');
    if (seq) {
      return [
        {
          id: `slide-seq-${seq.id}`,
          title: 'Sequence',
          mermaidNodeId: seq.id,
          reveal: Array.from({ length: 8 }, (_v, i) => ({ mermaidMessageRange: [0, i] as [number, number] })),
        },
      ];
    }
    return [{ id: 'slide-all', title: scene.title, reveal: [{ nodeIds: scene.nodes.map((n) => n.id) }] }];
  }
</script>

<style>
  .present {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: var(--bg);
    color: var(--text);
    display: flex;
    align-items: center;
    justify-content: center;
    outline: none;
    overflow: hidden;
  }
  .progress {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--surface-2);
  }
  .progress .bar {
    height: 100%;
    background: var(--accent);
    transition: width 200ms ease-out;
  }
  .stage {
    max-width: 92vw;
    max-height: 86vh;
    overflow: auto;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .mermaid-stage {
    width: min(86vw, 1100px);
  }
  .mermaid-stage :global(svg) {
    max-width: 100%;
    height: auto;
  }
  .scene-stage {
    position: relative;
  }
  .pnode {
    position: absolute;
  }
  .shape {
    width: 100%;
    height: 100%;
    border: 1.5px solid var(--border);
    border-radius: var(--radius-m);
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 6px;
    box-sizing: border-box;
  }
  .shape.ellipse {
    border-radius: 50%;
  }
  .sticky {
    width: 100%;
    height: 100%;
    border-radius: 4px;
    padding: 10px;
    box-sizing: border-box;
    color: #222;
    box-shadow: var(--shadow);
    overflow: auto;
    white-space: pre-wrap;
  }
  .text {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    font-size: 18px;
    overflow: hidden;
  }
  .code {
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: auto;
    font-size: 12px;
    box-sizing: border-box;
  }
  .frame {
    width: 100%;
    height: 100%;
    border: 1.5px dashed var(--border);
    border-radius: var(--radius-m);
    padding: 6px;
    box-sizing: border-box;
    color: var(--text-dim, #888);
    font-size: 12px;
  }
  .notes {
    position: absolute;
    bottom: 56px;
    left: 50%;
    transform: translateX(-50%);
    max-width: 80vw;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 12px;
    font-size: 13px;
  }
  .controls {
    position: absolute;
    bottom: 14px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 6px 12px;
    box-shadow: var(--shadow);
    transition: opacity 200ms;
  }
  .controls.hidden {
    opacity: 0;
    pointer-events: none;
  }
  .ctl {
    display: inline-flex;
    border: none;
    background: none;
    color: var(--text);
    cursor: pointer;
    padding: 4px;
    border-radius: 6px;
  }
  .ctl:hover,
  .ctl.on {
    background: var(--surface-2);
    color: var(--accent);
  }
  .counter {
    font-size: 12px;
    color: var(--text-dim, #888);
    white-space: nowrap;
  }
  .dots {
    display: inline-flex;
    gap: 4px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: none;
    background: var(--surface-2);
    cursor: pointer;
  }
  .dot.on {
    background: var(--accent);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }
  .btn {
    padding: 8px 16px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    border-radius: var(--radius-m);
    cursor: pointer;
  }
  .btn.accent {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
</style>
