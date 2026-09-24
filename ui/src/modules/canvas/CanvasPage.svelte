<script lang="ts">
  // Canvas Studio entry. Left: scene list. Right: an infinite Mermaid board that
  // renders the scene's agent-edited `.mermaid` source (full rich diagrams), or a
  // hero to start a new canvas. You never write Mermaid — you describe what you
  // want in the Assistant and the agent edits the file; the board re-renders live.
  import { untrack } from 'svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import SceneList from './SceneList.svelte';
  import ExcalidrawCanvas from './ExcalidrawCanvas.svelte';
  import MermaidCanvas from './MermaidCanvas.svelte';
  import D2Canvas from './D2Canvas.svelte';
  import ConversationPanel from './ConversationPanel.svelte';
  import type { CanvasFormat } from './types';

  // Three modes the user picks at creation: Excalidraw (a fully editable board,
  // the agent writes canvas.json), Mermaid (rich auto-rendered diagrams of any
  // kind — flowchart / sequence / class — the agent writes canvas.mermaid), or
  // D2 (modern declarative diagrams — architecture / sequence / SQL tables — the
  // agent writes canvas.d2).
  const isExcalidraw = $derived(canvas.format === 'excalidraw');
  const isD2 = $derived(canvas.format === 'd2');

  // Phone collapses the scene list once a board is open (more room).
  const readonly = $derived(viewport.isPhone);

  // The board exposes generate()/isGenerating() for agent drawing.
  let editor = $state<
    { generate: (p: string) => Promise<void>; isGenerating: () => boolean } | undefined
  >(undefined);
  // The Assistant panel (the agent shell + Ask-AI input) — opens on demand.
  let showConvo = $state(false);

  // Canvas is global — list the user's scenes across all workspaces. A failure
  // lands in `canvas.listError` and renders inline with Retry.
  $effect(() => {
    void canvas.loadScenes().catch(() => {});
  });

  // List/detail: with scenes, open on one (the remembered scene, else the
  // first) instead of the "start a new canvas" hero. Not on phone — opening a
  // board collapses the scene list there, which is the first screen. Also
  // re-picks after the open scene is deleted. A failed open stops the loop
  // (it shows inline with Retry).
  let picking = false;
  $effect(() => {
    if (viewport.isPhone || picking) return;
    if (canvas.currentId || canvas.pendingOpenId || canvas.loadError) return;
    if (canvas.listLoading || canvas.scenes.length === 0) return;
    const id = initialSelection('canvas', canvas.scenes, (s) => s.id);
    if (!id) return;
    picking = true;
    untrack(() => {
      void canvas
        .open(id)
        .catch(() => {})
        .finally(() => (picking = false));
    });
  });
  $effect(() => {
    if (canvas.currentId) rememberSelection('canvas', canvas.currentId);
  });

  /** The last failed scene open — Retry re-opens it. */
  function retryOpen(): void {
    const id = canvas.loadErrorId;
    if (id) void canvas.open(id).catch(() => {});
  }

  // Honor a deep-link request (e.g. Discovery-Chat "Open in Canvas").
  $effect(() => {
    const id = canvas.pendingOpenId;
    if (id) {
      canvas.pendingOpenId = null;
      void canvas.open(id).catch(() => {});
    }
  });

  function blankDoc(format: CanvasFormat): unknown {
    if (format === 'excalidraw') {
      return {
        type: 'otto-canvas',
        version: 1,
        format: 'excalidraw',
        source: JSON.stringify({ type: 'excalidraw', version: 2, source: 'otto', elements: [] }),
      };
    }
    if (format === 'd2') {
      return { type: 'otto-canvas', version: 1, format: 'd2', source: '' };
    }
    return { type: 'otto-canvas', version: 1, format: 'mermaid', source: '' };
  }

  /** Header "New scene ▾": pick the format from the shared (viewport-clamped) menu. */
  function newSceneMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Excalidraw board', icon: 'shapes', action: () => void createBlank('excalidraw') },
      { label: 'Mermaid diagram', icon: 'branch', action: () => void createBlank('mermaid') },
      { label: 'D2 diagram', icon: 'layers', action: () => void createBlank('d2') },
    ]);
  }

  /** No scenes anywhere → no empty list pane; the hero's mode cards are the CTA. */
  const noScenes = $derived(!canvas.listLoading && !canvas.listError && canvas.scenes.length === 0);
  /** The list failed with nothing to show — the main pane owns the error + Retry. */
  const listFailed = $derived(!!canvas.listError && canvas.scenes.length === 0);

  async function createBlank(format: CanvasFormat = 'excalidraw'): Promise<void> {
    try {
      const created = await canvas.create('Untitled canvas', blankDoc(format));
      await canvas.open(created.id);
    } catch (e) {
      toasts.error('Could not create canvas', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="canvas-shell">
<!-- Canvas is Design Hall's Whiteboard studio: the crumb leads back to the Hall. -->
<PageHeader
  title="Canvas"
  subtitle="Describe a diagram — the agent draws it and keeps refining it as you chat."
  crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]}
>
  {#snippet actions()}
    <!-- One primary per page: with no scenes yet the hero's mode cards own "new". -->
    {#if ws.currentId && !noScenes}
      <button class="btn primary" onclick={newSceneMenu} aria-haspopup="menu" data-testid="canvas-new-scene">
        <Icon name="plus" size={13} /> New scene <Icon name="chevronDown" size={11} />
      </button>
    {/if}
  {/snippet}
</PageHeader>
{#if !ws.currentId}
  <div class="canvas-page empty-ws">
    <EmptyState
      variant="page"
      icon="shapes"
      title="Select a workspace"
      body="Canvas scenes live in a workspace. Pick or create one to start drawing."
    />
  </div>
{:else}
  <div class="canvas-page" class:phone={readonly}>
    <aside class="scenes" class:hidden={(readonly && canvas.currentId) || noScenes || listFailed}>
      <SceneList />
    </aside>

    <section class="main">
      {#if canvas.loadError && canvas.scene && canvas.currentId}
        <!-- Opening another scene failed: keep the open board, say so above it. -->
        <LoadState what="that scene" variant="compact" error={canvas.loadError} empty onretry={retryOpen} />
      {/if}
      {#if listFailed}
        <LoadState
          what="scenes"
          variant="page"
          loading={canvas.listLoading}
          error={canvas.listError}
          empty
          onretry={() => void canvas.loadScenes().catch(() => {})}
        />
      {:else if canvas.loadError && !(canvas.scene && canvas.currentId)}
        <LoadState what="this scene" variant="page" error={canvas.loadError} empty onretry={retryOpen} />
      {:else if canvas.scene && canvas.currentId}
        <!-- Remount the board when switching scenes so each loads its own source. -->
        {#key canvas.currentId}
          <div class="editor-split" class:with-convo={showConvo}>
            <div class="editor-host">
              {#if isExcalidraw}
                <ExcalidrawCanvas bind:this={editor} {readonly} />
              {:else if isD2}
                <D2Canvas bind:this={editor} {readonly} />
              {:else}
                <MermaidCanvas bind:this={editor} {readonly} />
              {/if}
              {#if !showConvo}
                <!-- Open the Assistant (Ask-AI lives in the conversation panel). -->
                <div class="ai-bar">
                  <button class="ai-fab" onclick={() => (showConvo = true)}>
                    <Icon name="zap" size={15} /> Ask AI
                  </button>
                </div>
              {/if}
            </div>
            {#if showConvo}
              <aside class="convo-panel" class:overlay={readonly}>
                <ConversationPanel {editor} onclose={() => (showConvo = false)} />
              </aside>
            {/if}
          </div>
        {/key}
      {:else if !noScenes}
        {#if canvas.listLoading || canvas.scenes.length === 0 || !viewport.isPhone}
          <!-- Listing / auto-opening a scene. -->
          <LoadState what="scenes" variant="page" loading empty />
        {:else}
          <EmptyState
            variant="page"
            icon="shapes"
            title="Pick a scene"
            body="Open one from the list, or start a new one with New scene."
          />
        {/if}
      {:else}
        <div class="hero">
          <h2>Start a new canvas</h2>
          <p class="sub">
            Describe a diagram in plain English — the agent draws it and keeps refining it as you
            chat. Pick how it's drawn:
          </p>
          <div class="modes">
            <button class="mode" onclick={() => createBlank('excalidraw')}>
              <Icon name="shapes" size={20} />
              <span class="m-title">Excalidraw board</span>
              <span class="m-sub">Fully editable shapes — draw &amp; arrange by hand too</span>
            </button>
            <button class="mode" onclick={() => createBlank('mermaid')}>
              <Icon name="branch" size={20} />
              <span class="m-title">Mermaid diagram</span>
              <span class="m-sub">Auto-rendered flowchart / sequence / class — rich &amp; clean</span>
            </button>
            <button class="mode" onclick={() => createBlank('d2')}>
              <Icon name="layers" size={20} />
              <span class="m-title">D2 diagram</span>
              <span class="m-sub">Modern declarative diagrams — architecture, sequence &amp; SQL tables</span>
            </button>
          </div>
        </div>
      {/if}
    </section>
  </div>
{/if}
</div>

<style>
  .canvas-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .canvas-page {
    display: flex;
    flex: 1;
    min-height: 0;
    background: var(--bg);
    color: var(--text);
  }
  .canvas-page.empty-ws {
    flex-direction: column;
  }
  .scenes {
    width: 240px;
    flex: 0 0 240px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
    background: var(--surface);
  }
  .scenes.hidden {
    display: none;
  }
  .main {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  /* Editor + optional conversation side panel. */
  .editor-split {
    flex: 1 1 auto;
    display: flex;
    min-height: 0;
    min-width: 0;
  }
  .editor-host {
    flex: 1 1 auto;
    position: relative;
    min-width: 0;
    min-height: 0;
  }
  .convo-panel {
    width: 380px;
    flex: none;
    border-inline-start: 1px solid var(--border);
    display: flex;
    min-height: 0;
    min-width: 0;
  }
  /* Ask-AI launcher — bottom-center so it clears Excalidraw's top toolbar. */
  .ai-bar {
    position: absolute;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ai-fab {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 9px 16px;
    border: none;
    border-radius: 999px;
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    box-shadow: var(--shadow, 0 4px 16px rgba(0, 0, 0, 0.25));
  }
  .ai-fab:hover {
    filter: brightness(1.08);
  }
  /* The page's empty state: same fixed top offset as EmptyState variant="page". */
  .hero {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    gap: 12px;
    padding: 15vh 24px 48px;
    overflow-y: auto;
    text-align: center;
  }
  .hero h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .sub {
    margin: 0;
    color: var(--text-dim, #888);
    font-size: 13px;
    max-width: 420px;
  }
  .modes {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    justify-content: center;
    margin-top: 6px;
  }
  .mode {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: 220px;
    padding: 20px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    text-align: center;
    transition:
      border-color 0.12s,
      transform 0.12s;
  }
  .mode:hover {
    border-color: var(--accent);
    transform: translateY(-2px);
  }
  .mode .m-title {
    font-size: 14px;
    font-weight: 700;
    margin-top: 2px;
  }
  .mode .m-sub {
    font-size: 12px;
    color: var(--text-dim, #888);
    line-height: 1.4;
  }
</style>
