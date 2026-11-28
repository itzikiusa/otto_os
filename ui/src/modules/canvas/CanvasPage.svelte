<script lang="ts">
  import { toastError } from '../../lib/toastError';
  // Canvas Studio entry. Left: scene list. Right: an infinite Mermaid board that
  // renders the scene's agent-edited `.mermaid` source (full rich diagrams), or a
  // hero to start a new canvas. You never write Mermaid — you describe what you
  // want in the Assistant and the agent edits the file; the board re-renders live.
  import { untrack } from 'svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import SceneList from './SceneList.svelte';
  import ExcalidrawCanvas from './ExcalidrawCanvas.svelte';
  import MermaidCanvas from './MermaidCanvas.svelte';
  import D2Canvas from './D2Canvas.svelte';
  import ConversationPanel from './ConversationPanel.svelte';
  import type { CanvasFormat } from '../../lib/canvas/types';

  // Three modes the user picks at creation: Excalidraw (a fully editable board,
  // the agent writes canvas.json), Mermaid (rich auto-rendered diagrams of any
  // kind — flowchart / sequence / class — the agent writes canvas.mermaid), or
  // D2 (modern declarative diagrams — architecture / sequence / SQL tables — the
  // agent writes canvas.d2).
  const isExcalidraw = $derived(canvas.format === 'excalidraw');
  const isD2 = $derived(canvas.format === 'd2');

  const isPhone = $derived(viewport.isPhone);
  // Phone push (ListDetail): the board is the page once a scene was chosen;
  // Back returns to the list with the scene still open and marked.
  let phoneOpen = $state(false);
  const openTitle = $derived(
    canvas.scenes.find((scene) => scene.id === canvas.currentId)?.title,
  );

  // The board exposes generate()/isGenerating() for agent drawing.
  let editor = $state<
    { generate: (p: string) => Promise<boolean>; isGenerating: () => boolean } | undefined
  >(undefined);
  // The Assistant panel (the agent shell + Ask-AI input) — opens on demand.
  let showConvo = $state(false);
  let workspaceEmpty: HTMLDivElement | undefined = $state();

  // Canvas is global — list the user's scenes across all workspaces. A failure
  // lands in `canvas.listError` and renders inline with Retry.
  $effect(() => {
    void canvas.loadScenes().catch(() => {});
  });

  // List/detail: with scenes, open on one (the remembered scene, else the
  // first) instead of the "start a new canvas" hero. Not on a phone, where the
  // list is the first screen (a board loaded behind it would size to 0). Also
  // re-picks after the open scene is deleted. A failed open stops the loop
  // (it shows inline with Retry).
  //
  // `picking` (plain, untracked) is true while ANY open this page started is in
  // flight — the auto-pick's own, or a deep link's — so the auto-pick never
  // races a deep link to the remembered scene. It is checked LAST: returning
  // before the reactive reads would drop this effect's dependencies, and it
  // would then never re-run (e.g. never re-pick after the open scene is deleted).
  let picking = false;
  $effect(() => {
    if (viewport.navDrawer) return;
    if (canvas.currentId || canvas.pendingOpenId || canvas.loadError) return;
    if (canvas.listLoading || canvas.scenes.length === 0) return;
    if (picking) return;
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
      picking = true;
      phoneOpen = true;
      void canvas
        .open(id)
        .catch(() => {})
        .finally(() => (picking = false));
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

  /** The list pane's "New scene": pick the format from the shared (viewport-clamped) menu. */
  function newSceneMenu(anchor: Element | null): void {
    ctxMenu.showAt(anchor, [
      { label: 'Excalidraw board', icon: 'shapes', action: () => void createBlank('excalidraw') },
      { label: 'Mermaid diagram', icon: 'branch', action: () => void createBlank('mermaid') },
      { label: 'D2 diagram', icon: 'layers', action: () => void createBlank('d2') },
    ]);
  }

  /** No scenes anywhere → no empty list pane; the hero's mode cards are the CTA. */
  const noScenes = $derived(!canvas.listLoading && !canvas.listError && canvas.scenes.length === 0);
  /** The list failed with nothing to show — the main pane owns the error + Retry. */
  const listFailed = $derived(!!canvas.listError && canvas.scenes.length === 0);


  // ⌘K: the page's verbs (new scene per format; the assistant while a scene
  // is open). Group = the module name.
  $effect(() => {
    if (!ws.currentId) return registry.register('canvas', []);
    const open = !!canvas.currentId;
    return registry.register('canvas', [
      { id: 'canvas.new-excalidraw', title: 'New Excalidraw board', group: 'Canvas', keywords: 'create scene whiteboard draw', run: () => void createBlank('excalidraw') },
      { id: 'canvas.new-mermaid', title: 'New Mermaid diagram', group: 'Canvas', keywords: 'create scene flowchart sequence', run: () => void createBlank('mermaid') },
      { id: 'canvas.new-d2', title: 'New D2 diagram', group: 'Canvas', keywords: 'create scene architecture', run: () => void createBlank('d2') },
      ...(open
        ? [{ id: 'canvas.assistant', title: 'Open the canvas assistant', group: 'Canvas', keywords: 'agent ai redraw chat', run: () => (showConvo = true) }]
        : []),
    ]);
  });

  async function createBlank(format: CanvasFormat = 'excalidraw'): Promise<void> {
    try {
      const created = await canvas.create('Untitled canvas', blankDoc(format));
      phoneOpen = true;
      await canvas.open(created.id);
    } catch (e) {
      toastError('Couldn’t create canvas', e);
    }
  }
</script>

<div class="canvas-shell">
<!-- Canvas is Design Hall's Whiteboard studio: the crumb leads back to the Hall. -->
<PageHeader
  title={openTitle || 'Canvas'}
  subtitle="Diagrams an agent draws and refines as you chat"
  crumbs={isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }]}
/>
{#each Object.entries(canvas.docSaveErrors) as [id, error] (id)}
  <div class="save-error" role="alert">
    <span>Couldn’t save {canvas.scenes.find(scene => scene.id === id)?.title ?? 'scene'}. Your draft is kept in this app. {error}</span>
    <button class="btn small" onclick={() => void canvas.open(id).catch(() => {})}>Open draft</button>
    <button class="btn small" onclick={() => void canvas.retryDoc(id).catch(() => {})}>Retry save</button><!-- ui-guards: allow — retries a failed draft save, not a list load -->
  </div>
{/each}
{#if canvas.conflict}
  {@const conflictTitle = canvas.scenes.find(scene => scene.id === canvas.conflict?.id)?.title ?? 'This scene'}
  <div class="save-error" data-testid="canvas-conflict" use:announceShown={{ text: `${conflictTitle} changed elsewhere; choose which board to keep`, assertive: true }}>
    <span>{conflictTitle} was changed elsewhere (an agent, another tab or person) after you opened it. Your unsaved board is kept until you choose.</span>
    <button class="btn small" onclick={() => void canvas.reloadTheirs().catch((e) => toastError('Couldn’t reload the scene', e))}>Reload theirs</button>
    <button class="btn small" onclick={() => void canvas.keepMineAsCopy().catch((e) => toastError('Couldn’t save your copy', e))}>Keep mine as a copy</button>
  </div>
{/if}
<PageBody fill padded={false}>
{#if !ws.currentId}
  <div class="canvas-page empty-ws" bind:this={workspaceEmpty}>
    <EmptyState
      variant="page"
      icon="shapes"
      title="Select a workspace"
      body="Canvas scenes live in a workspace. Pick or create one to start drawing."
      actionLabel="Choose workspace"
      onaction={() => {
        const trigger = workspaceEmpty?.querySelector('button');
        trigger?.focus();
        if (!ws.workspaces.length) { ui.newWorkspaceOpen = true; return; }
        ctxMenu.showAt(trigger!, [
          ...ws.workspaces.map(w => ({ label: w.name, icon: 'folder', action: () => void ws.select(w.id) })),
          { separator: true },
          { label: 'Add workspace…', icon: 'plus', action: () => (ui.newWorkspaceOpen = true) },
        ], { filter: true, filterPlaceholder: 'Find a workspace…' });
      }}
    />
  </div>
{:else}
  <ListDetail
    module="canvas"
    name="Scenes"
    open={phoneOpen && (!!canvas.currentId || !!canvas.loadError)}
    onback={() => (phoneOpen = false)}
    empty={noScenes || listFailed}
    legacy={{ w: 'canvas.listW' }}
    testid="canvas-page"
  >
    {#snippet list()}
      <SceneList onnew={newSceneMenu} onpick={() => (phoneOpen = true)} />
    {/snippet}
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
          <div class="editor-host">
            {#if isExcalidraw}
              <ExcalidrawCanvas bind:this={editor} />
            {:else if isD2}
              <D2Canvas bind:this={editor} />
            {:else}
              <MermaidCanvas bind:this={editor} />
            {/if}
            {#if !showConvo}
              <!-- Open the Assistant (the Detail pane; Ask-AI lives in it). -->
              <div class="ai-bar">
                <button class="ai-fab" onclick={() => (showConvo = true)} title="Open the assistant: describe a change and the agent redraws the scene (⌥⌘B)">
                  <Icon name="sparkle" size={14} /> Ask Otto
                </button>
              </div>
            {/if}
          </div>
        {/key}
      {:else if !noScenes}
        <!-- Listing / auto-opening a scene. -->
        <LoadState what="scenes" variant="page" loading empty />
      {:else}
        <div class="hero">
          <h2>Start a new canvas</h2>
          <p class="sub">
            Describe a diagram in plain English — the agent draws it and keeps refining it as you
            chat. Pick how it’s drawn:
          </p>
          <div class="modes">
            <button class="mode" onclick={() => createBlank('excalidraw')}>
              <Icon name="shapes" size={24} />
              <span class="m-title">Excalidraw board</span>
              <span class="m-sub">Fully editable shapes — draw &amp; arrange by hand too</span>
            </button>
            <button class="mode" onclick={() => createBlank('mermaid')}>
              <Icon name="branch" size={24} />
              <span class="m-title">Mermaid diagram</span>
              <span class="m-sub">Auto-rendered flowchart / sequence / class — rich &amp; clean</span>
            </button>
            <button class="mode" onclick={() => createBlank('d2')}>
              <Icon name="layers" size={24} />
              <span class="m-title">D2 diagram</span>
              <span class="m-sub">Modern declarative diagrams — architecture, sequence &amp; SQL tables</span>
            </button>
          </div>
        </div>
      {/if}
    </section>
    {#snippet detail()}
      {#if canvas.scene && canvas.currentId}
        <!-- The assistant is the page's Detail pane: inline beside the board
             while it keeps 480 px, else a right sheet (full width on a phone). -->
        <DetailPane
          module="canvas"
          pane="assistant"
          title="Assistant"
          open={showConvo}
          onclose={() => (showConvo = false)}
          ontoggle={() => (showConvo = !showConvo)}
          testid="canvas-assistant"
        >
          {#key canvas.currentId}<ConversationPanel {editor} />{/key}
        </DetailPane>
      {/if}
    {/snippet}
  </ListDetail>
{/if}
</PageBody>
</div>

<style>
  .save-error { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; padding: 8px 12px; border-block-end: 1px solid var(--danger); background: var(--danger-soft); font-size: var(--fs-s); }
  .save-error span { flex: 1; min-width: 160px; overflow-wrap: anywhere; }

  .canvas-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .canvas-page.empty-ws {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }
  .main {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .editor-host {
    container: canvas-editor / inline-size;
    flex: 1 1 auto;
    position: relative;
    min-width: 0;
    min-height: 0;
  }
  /* Ask-AI launcher — bottom-center so it clears Excalidraw's top toolbar. */
  .ai-bar {
    position: absolute;
    bottom: 18px;
    left: 50%; /* ui-guards: allow — centred with translateX(-50%), direction-neutral */
    transform: translateX(-50%);
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    gap: 8px;
  }
  @container canvas-editor (max-width: 700px) {
    .ai-bar { bottom: 66px; }
  }
  /* A floating launcher, not a second primary: a raised neutral pill with an
     accent glyph (the list pane's New scene is the page's one primary). */
  .ai-fab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-m);
    font-weight: 600;
    cursor: pointer;
    box-shadow: var(--glass-shadow);
  }
  .ai-fab :global(svg) {
    color: var(--accent-text);
  }
  .ai-fab:hover {
    border-color: var(--accent);
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
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .sub {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-m);
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
      border-color var(--dur-fast),
      transform var(--dur-fast);
  }
  .mode:hover {
    border-color: var(--accent);
    transform: translateY(-2px);
  }
  @media (prefers-reduced-motion: reduce) {
    .mode,
    .mode:hover {
      transition: none;
      transform: none;
    }
  }
  .mode :global(svg) {
    color: var(--accent-text);
  }
  .mode .m-title {
    font-size: var(--fs-m);
    font-weight: 600;
    margin-top: 2px;
  }
  .mode .m-sub {
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.4;
  }
</style>
