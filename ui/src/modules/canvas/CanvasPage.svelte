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
            <ExcalidrawCanvas {readonly} />
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
