<script lang="ts">
  // Canvas Studio entry. Left: scene list. Right: the embedded Excalidraw editor
  // when a scene is open, or a hero to start a new canvas. (Agent generation +
  // live streaming land on top of the embed in the next milestones.)
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import SceneList from './SceneList.svelte';
  import ExcalidrawCanvas from './ExcalidrawCanvas.svelte';

  // Phone: Excalidraw opens in view mode (its editing chrome needs room).
  const readonly = $derived(viewport.isPhone);

  // The embedded editor exposes generate() for agent drawing.
  let editor = $state<{ generate: (p: string) => Promise<void> } | undefined>(undefined);
  let aiOpen = $state(false);
  let aiPrompt = $state('');
  let aiBusy = $state(false);

  async function runAi(): Promise<void> {
    const p = aiPrompt.trim();
    if (!p || aiBusy || !editor) return;
    aiBusy = true;
    try {
      await editor.generate(p);
      aiPrompt = '';
    } finally {
      aiBusy = false;
    }
  }
  function onAiKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') void runAi();
    else if (e.key === 'Escape') aiOpen = false;
  }

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

  async function createBlank(): Promise<void> {
    try {
      const created = await canvas.create('Untitled canvas');
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
        <!-- Remount Excalidraw when switching scenes so each loads its own doc. -->
        {#key canvas.currentId}
          <div class="editor-host">
            <ExcalidrawCanvas bind:this={editor} {readonly} />
            {#if !readonly}
              <!-- Agent "draw it for me" overlay (top-center, above Excalidraw). -->
              <div class="ai-bar">
                {#if !aiOpen}
                  <button class="ai-fab" onclick={() => (aiOpen = true)}>
                    <Icon name="zap" size={15} /> Ask AI to draw
                  </button>
                {:else}
                  <div class="ai-input" class:busy={aiBusy}>
                    <Icon name="zap" size={15} />
                    <!-- svelte-ignore a11y_autofocus -->
                    <input
                      bind:value={aiPrompt}
                      placeholder="Describe a diagram… e.g. 'order flow across microservices'"
                      onkeydown={onAiKey}
                      disabled={aiBusy}
                      autofocus
                    />
                    <button class="ai-draw" onclick={runAi} disabled={aiBusy || !aiPrompt.trim()}>
                      {aiBusy ? 'Drawing…' : 'Draw'}
                    </button>
                    <button class="ai-close" onclick={() => (aiOpen = false)} aria-label="Close">
                      <Icon name="x" size={14} />
                    </button>
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        {/key}
      {:else}
        <div class="hero">
          <h2>Start a new canvas</h2>
          <p class="sub">A full Excalidraw board — shapes, sketches, diagrams, images and more.</p>
          <button class="hero-ai" onclick={createBlank}>
            <Icon name="plus" /> New canvas
          </button>
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
  .editor-host {
    flex: 1 1 auto;
    position: relative;
    min-width: 0;
    min-height: 0;
  }
  /* Agent draw overlay — bottom-center so it clears Excalidraw's top toolbar. */
  .ai-bar {
    position: absolute;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 6;
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
  .ai-input {
    display: flex;
    align-items: center;
    gap: 8px;
    width: min(560px, 80vw);
    padding: 7px 9px 7px 14px;
    background: var(--surface);
    border: 1px solid var(--accent);
    border-radius: 999px;
    box-shadow: var(--shadow, 0 4px 20px rgba(0, 0, 0, 0.3));
    color: var(--accent);
  }
  .ai-input.busy {
    opacity: 0.9;
  }
  .ai-input input {
    flex: 1;
    min-width: 0;
    border: none;
    background: none;
    color: var(--text);
    font-size: 13px;
    outline: none;
  }
  .ai-draw {
    padding: 6px 14px;
    border: none;
    border-radius: 999px;
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }
  .ai-draw:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .ai-close {
    display: inline-flex;
    border: none;
    background: none;
    color: var(--text-dim, #888);
    cursor: pointer;
    padding: 4px;
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
  .hero-ai {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 14px 22px;
    font-size: 16px;
    font-weight: 600;
    color: #fff;
    background: var(--accent);
    border: none;
    border-radius: var(--radius-m);
    cursor: pointer;
    box-shadow: var(--shadow);
  }
  .hero-ai:hover {
    filter: brightness(1.08);
  }
</style>
