<script lang="ts">
  // Canvas Studio entry. Left: scene list. Right: an infinite Mermaid board that
  // renders the scene's agent-edited `.mermaid` source (full rich diagrams), or a
  // hero to start a new canvas. You never write Mermaid — you describe what you
  // want in the Assistant and the agent edits the file; the board re-renders live.
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import SceneList from './SceneList.svelte';
  import ExcalidrawCanvas from './ExcalidrawCanvas.svelte';
  import MermaidCanvas from './MermaidCanvas.svelte';
  import ConversationPanel from './ConversationPanel.svelte';
  import type { CanvasFormat } from './types';

  // Two modes the user picks at creation: Excalidraw (a fully editable board, the
  // agent writes canvas.json) or Mermaid (rich auto-rendered diagrams of any kind
  // — flowchart / sequence / class — the agent writes canvas.mermaid).
  const isExcalidraw = $derived(canvas.format === 'excalidraw');

  // Phone collapses the scene list once a board is open (more room).
  const readonly = $derived(viewport.isPhone);

  // The board exposes generate()/isGenerating() for agent drawing.
  let editor = $state<
    { generate: (p: string) => Promise<void>; isGenerating: () => boolean } | undefined
  >(undefined);
  // The Assistant panel (the agent shell + Ask-AI input) — opens on demand.
  let showConvo = $state(false);

  // Canvas is global — list the user's scenes across all workspaces.
  $effect(() => {
    void canvas.loadScenes().catch(() => {});
  });

  // Honor a deep-link request (e.g. Discovery-Chat "Open in Canvas").
  $effect(() => {
    const id = canvas.pendingOpenId;
    if (id) {
      canvas.pendingOpenId = null;
      void canvas.open(id).catch(() => {});
    }
  });

  function blankDoc(format: CanvasFormat): unknown {
    return format === 'excalidraw'
      ? {
          type: 'otto-canvas',
          version: 1,
          format: 'excalidraw',
          source: JSON.stringify({ type: 'excalidraw', version: 2, source: 'otto', elements: [] }),
        }
      : { type: 'otto-canvas', version: 1, format: 'mermaid', source: '' };
  }

  async function createBlank(format: CanvasFormat = 'excalidraw'): Promise<void> {
    try {
      const created = await canvas.create('Untitled canvas', blankDoc(format));
      await canvas.open(created.id);
    } catch (e) {
      toasts.error('Could not create canvas', e instanceof Error ? e.message : String(e));
    }
  }
</script>

{#if !ws.currentId}
  <div class="canvas-page empty-ws">
    <EmptyState
      icon="shapes"
      title="Select a workspace"
      body="Canvas scenes live in a workspace. Pick or create one to start drawing."
    />
  </div>
{:else}
  <div class="canvas-page" class:phone={readonly}>
    <aside class="scenes" class:hidden={readonly && canvas.currentId}>
      <SceneList oncreate={createBlank} />
    </aside>

    <section class="main">
      {#if canvas.scene && canvas.currentId}
        <!-- Remount the board when switching scenes so each loads its own source. -->
        {#key canvas.currentId}
          <div class="editor-split" class:with-convo={showConvo}>
            <div class="editor-host">
              {#if isExcalidraw}
                <ExcalidrawCanvas bind:this={editor} {readonly} />
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
          </div>
        </div>
      {/if}
    </section>
  </div>
{/if}

<style>
  .canvas-page {
    display: flex;
    height: 100%;
    min-height: 0;
    background: var(--bg);
    color: var(--text);
  }
  .canvas-page.empty-ws {
    align-items: center;
    justify-content: center;
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
  .hero {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    padding: 24px;
    text-align: center;
  }
  .hero h2 {
    margin: 0;
    font-size: 20px;
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
