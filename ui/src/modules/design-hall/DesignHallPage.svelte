<script lang="ts">
  // Design Hall — module entry (`#/design…`). Resolves the sub-route to a view
  // and owns the creation affordances every view shares: the "New ▾" menu
  // (global, viewport-clamped ctxMenu), the New design sheet, file import and
  // New project. Routes:
  //   #/design                lobby (grid)        #/design/spatial     lobby (spatial, beta)
  //   #/design/a/<id>         one artifact        #/design/p/<id>      a project
  //   #/design/studio/<id>    a studio            #/design/story/<id>  designs for a story
  //   #/design/brand          Brand Kit           #/design/learned[/rules|/memory]
  import { router } from '../../lib/router.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { createProject } from '../../lib/api/design';
  import type { DesignStudio } from '../../lib/api/types';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import Lobby from './Lobby.svelte';
  import ArtifactView from './ArtifactView.svelte';
  import CollectionView from './CollectionView.svelte';
  import BrandKitPage from './BrandKitPage.svelte';
  import LearnedPage from './LearnedPage.svelte';
  import NewDesignModal from './NewDesignModal.svelte';
  import { parseDesignRoute, studioInfo } from './model';
  import { IMPORT_ACCEPT, importDesignFile } from './create';
  import { library } from './library.svelte';

  const route = $derived(parseDesignRoute(router.parts));
  const canView = $derived(auth.can('design', 'view'));
  const canEdit = $derived(auth.can('design', 'edit'));

  // ── New design sheet ──────────────────────────────────────────────────────
  interface NewInit {
    studio?: DesignStudio;
    format?: string;
    templateId?: string;
    projectId?: string | null;
    storyId?: string | null;
  }
  let newInit = $state<NewInit | null>(null);
  function openNew(init: NewInit = {}): void {
    if (!canEdit) {
      toasts.warn('You can view designs but not create them', 'Ask a workspace admin for Design Hall edit access.');
      return;
    }
    newInit = init;
  }
  function created(id: string): void {
    newInit = null;
    void library.load();
    router.go(`design/a/${encodeURIComponent(id)}`);
  }

  /** Header "New ▾": one row per studio (planned ones visible but disabled). */
  function newMenu(e: MouseEvent): void {
    const s = (id: DesignStudio, label: string, format?: string): MenuItem => ({
      label,
      icon: studioInfo(id).icon,
      disabled: !canEdit,
      action: () => openNew({ studio: id, format }),
    });
    ctxMenu.show(e, [
      s('frames', 'Frame screen (HTML)'),
      s('graphics', 'Graphic'),
      s('3d', '3D scene'),
      s('whiteboard', 'Whiteboard diagram (Mermaid)', 'mermaid'),
      s('whiteboard', 'Whiteboard diagram (D2)', 'd2'),
      s('whiteboard', 'Whiteboard board (Excalidraw)', 'excalidraw'),
      s('brand', 'Brand kit'),
      { label: 'Site (planned for Phase 1)', icon: 'layout', disabled: true },
      { label: 'Exhibition (planned for v3)', icon: 'gallery', disabled: true },
      { separator: true },
      { label: 'Import file…', icon: 'download', disabled: !canEdit, action: pickFile },
      { label: 'New project…', icon: 'folder', disabled: !canEdit, action: () => void newProject() },
    ]);
  }

  // ── Import ────────────────────────────────────────────────────────────────
  let fileInput = $state<HTMLInputElement | null>(null);
  function pickFile(): void {
    fileInput?.click();
  }
  async function onFile(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'Imported designs are filed under a workspace.');
      return;
    }
    const projectId = route.view === 'project' ? route.id : null;
    try {
      const id = await importDesignFile(file, { workspaceId: wsId, projectId });
      toasts.success('Imported', file.name);
      created(id);
    } catch (err) {
      toasts.error('Couldn’t import the file', err instanceof Error ? err.message : String(err));
    }
  }

  async function newProject(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'Projects are filed under a workspace.');
      return;
    }
    const name = await confirmer.promptText('A project groups the designs for one launch or epic.', {
      title: 'New project',
      confirmLabel: 'Create project',
      placeholder: 'Loyalty relaunch',
    });
    if (!name) return;
    try {
      const p = await createProject({ workspace_id: wsId, name });
      void library.load();
      router.go(`design/p/${encodeURIComponent(p.id)}`);
    } catch (e) {
      toasts.error('Couldn’t create the project', e instanceof Error ? e.message : String(e));
    }
  }

  // ── ⌘K verbs for the module ──────────────────────────────────────────────
  $effect(() => {
    if (!canView) return registry.register('design-hall', []);
    return registry.register('design-hall', [
      { id: 'design.new', title: 'New design…', group: 'Design Hall', keywords: 'create frame graphic 3d whiteboard mockup', run: () => openNew({}) },
      { id: 'design.new-project', title: 'New design project…', group: 'Design Hall', keywords: 'create project epic', run: () => void newProject() },
      { id: 'design.import', title: 'Import design file…', group: 'Design Hall', keywords: 'upload png svg glb html excalidraw', run: pickFile },
      { id: 'design.brand', title: 'Open Brand Kit', group: 'Design Hall', keywords: 'tokens colours typography', run: () => router.go('design/brand') },
      { id: 'design.learned', title: 'Open what Otto learned', group: 'Design Hall', keywords: 'signals rules memory learning', run: () => router.go('design/learned') },
    ]);
  });
</script>

<div class="design-hall">
  {#if !canView}
    <PageHeader title="Design Hall" />
    <EmptyState variant="page" icon="designHall" title="You don’t have access to Design Hall"
      body="Ask a workspace admin to grant the Design Hall feature (Settings → Users)." />
  {:else if route.view === 'lobby' || route.view === 'spatial'}
    <Lobby view={route.view === 'spatial' ? 'spatial' : 'grid'} onnew={newMenu} onimport={pickFile} onnewproject={() => void newProject()} />
  {:else if route.view === 'artifact'}
    {#key route.id}
      <ArtifactView id={route.id} />
    {/key}
  {:else if route.view === 'project'}
    <CollectionView scope={{ kind: 'project', id: route.id }} onnew={openNew} />
  {:else if route.view === 'studio'}
    <CollectionView scope={{ kind: 'studio', id: route.id }} onnew={openNew} />
  {:else if route.view === 'story'}
    <CollectionView scope={{ kind: 'story', id: route.id }} onnew={openNew} />
  {:else if route.view === 'brand'}
    <BrandKitPage />
  {:else if route.view === 'learned'}
    <LearnedPage tab={route.tab} />
  {/if}
</div>

<input
  bind:this={fileInput}
  class="file"
  type="file"
  accept={IMPORT_ACCEPT}
  onchange={onFile}
  tabindex="-1"
  aria-hidden="true"
  data-testid="design-import-input"
/>

{#if newInit}
  <NewDesignModal
    studio={newInit.studio}
    format={newInit.format}
    templateId={newInit.templateId}
    projectId={newInit.projectId}
    storyId={newInit.storyId}
    onclose={() => (newInit = null)}
    oncreated={created}
  />
{/if}

<style>
  .design-hall {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .file {
    display: none;
  }
</style>
