<script lang="ts">
  // Canvas Studio entry. Left: scene list. Right: either the editor (Toolbar +
  // ToolRail + CanvasEditor + Inspector) when a scene is open, or the empty-scene
  // hero (AI-first + template gallery). AI pill + Present mode are overlays.
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import SceneList from './SceneList.svelte';
  import CanvasEditor from './CanvasEditor.svelte';
  import Toolbar from './Toolbar.svelte';
  import ToolRail from './ToolRail.svelte';
  import Inspector from './Inspector.svelte';
  import AiPromptPill from './AiPromptPill.svelte';
  import PresentMode from './PresentMode.svelte';
  import { TEMPLATES } from './templates';
  import type { Tool } from './tools';

  let activeTool = $state<Tool>('select');
  let selectedIds = $state<string[]>([]);
  let showAi = $state(false);
  let presenting = $state(false);
  let editor = $state<{ fit: () => void } | undefined>(undefined);

  const readonly = $derived(viewport.isPhone);

  // Load the workspace's scenes when the workspace is known / changes.
  $effect(() => {
    // Canvas is global — list the user's scenes across all workspaces.
    void canvas.loadScenes().catch(() => {});
  });

  // Honor a deep-link request (e.g. Discovery-Chat "Open in Canvas").
  $effect(() => {
    const id = canvas.pendingOpenId;
    if (id) {
      canvas.pendingOpenId = null;
      void canvas.open(id).then(() => (selectedIds = [])).catch(() => {});
    }
  });

  async function createBlank(): Promise<void> {
    try {
      const created = await canvas.create('Untitled scene');
      await canvas.open(created.id);
      selectedIds = [];
    } catch (e) {
      toasts.error('Could not create scene', e instanceof Error ? e.message : String(e));
    }
  }

  async function useTemplate(id: string): Promise<void> {
    const t = TEMPLATES.find((x) => x.id === id);
    if (!t) return;
    try {
      const doc = t.build();
      const created = await canvas.create(doc.title, doc);
      await canvas.open(created.id);
      selectedIds = [];
    } catch (e) {
      toasts.error('Could not create scene', e instanceof Error ? e.message : String(e));
    }
  }

  async function aiHero(): Promise<void> {
    await createBlank();
    showAi = true;
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
        <Toolbar
          {readonly}
          onaskai={() => (showAi = true)}
          onpresent={() => (presenting = true)}
          onfit={() => editor?.fit()}
        />
        {#if readonly}
          <div class="ro-banner">Canvas editing needs a larger screen — you can still Present and Ask AI.</div>
        {/if}
        <div class="editor-row">
          {#if !readonly}
            <ToolRail {activeTool} onpick={(t) => (activeTool = t)} />
          {/if}
          <div class="editor-host">
            <CanvasEditor
              bind:this={editor}
              {activeTool}
              {readonly}
              onToolDone={() => (activeTool = 'select')}
              onselect={(ids) => (selectedIds = ids)}
              ontool={(t) => (activeTool = t)}
            />
            {#if showAi}
              <AiPromptPill onclose={() => (showAi = false)} />
            {/if}
          </div>
          {#if !readonly}
            <Inspector {selectedIds} />
          {/if}
        </div>
      {:else}
        <!-- Empty-scene hero: AI is the hero, then templates -->
        <div class="hero">
          <h2>Start with a sketch or just describe it</h2>
          <button class="hero-ai" onclick={aiHero}>
            <Icon name="zap" /> Describe it and I'll draw it
          </button>
          <div class="or">or start from a template</div>
          <div class="templates">
            {#each TEMPLATES as t (t.id)}
              <button class="tpl" onclick={() => useTemplate(t.id)}>
                <span class="tpl-icon"><Icon name={t.icon} /></span>
                <span class="tpl-name">{t.name}</span>
                <span class="tpl-hint">{t.hint}</span>
              </button>
            {/each}
          </div>
          <button class="blank" onclick={createBlank}>or open a blank canvas</button>
        </div>
      {/if}
    </section>
  </div>
{/if}

{#if presenting}
  <PresentMode onexit={() => (presenting = false)} />
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
  .editor-row {
    flex: 1 1 auto;
    display: flex;
    min-height: 0;
  }
  .editor-host {
    flex: 1 1 auto;
    position: relative;
    min-width: 0;
  }
  .ro-banner {
    padding: 8px 12px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    color: var(--text-dim, #888);
  }
  /* Empty-scene hero */
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
  .or {
    color: var(--text-dim, #888);
    font-size: 13px;
  }
  .templates {
    display: grid;
    grid-template-columns: repeat(3, minmax(150px, 1fr));
    gap: 10px;
    max-width: 560px;
  }
  .tpl {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: 12px;
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius-m);
    cursor: pointer;
    text-align: left;
  }
  .tpl:hover {
    border-color: var(--accent);
    background: var(--surface-2);
  }
  .tpl-icon {
    color: var(--accent);
  }
  .tpl-name {
    font-weight: 600;
    font-size: 13px;
  }
  .tpl-hint {
    font-size: 11px;
    color: var(--text-dim, #888);
  }
  .blank {
    border: none;
    background: none;
    color: var(--text-dim, #888);
    font-size: 12px;
    cursor: pointer;
    text-decoration: underline;
  }
  @media (max-width: 1024px) {
    .templates {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>
