<script lang="ts">
  // The 3D Studio toolbar: ONE row that always fits. Generate is the primary;
  // what fits the width stays inline (Scene / Details toggles when the side
  // panes are collapsed, Shading ▾, Comment, then Play and Export ▾ on wide
  // panes) and everything else lives under ⋯ (the shared, viewport-clamped
  // ctxMenu). Shading and the wireframe overlay share one menu, and the 1.8 m
  // scale reference is named for what it is, so neither reads as a duplicate
  // of the shading "Wire" mode or the gizmo's "Scale".
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import type { SelectionKind3d, ShadingMode } from '../../../lib/design/scene3d';

  interface Props {
    isModelArtifact: boolean;
    canEdit: boolean;
    canComment: boolean;
    sceneOpen: boolean;
    detailsOpen: boolean;
    onscene: () => void;
    ondetails: () => void;
    ongenerate: (e: MouseEvent) => void;
    exportItems: () => MenuItem[];
    /** Make game-ready / Rig / Animate (empty when no provider offers a stage). */
    pipelineItems: MenuItem[];
    /** "Use this view as the scene camera" (omitted when read-only). */
    oncamera?: () => void;
    pickMode: boolean;
    onpickmode: (on: boolean) => void;
    shading: ShadingMode;
    wireframeOverlay: boolean;
    scaleRef: boolean;
    pickKind: SelectionKind3d;
    turntable: boolean;
    play: boolean;
    showSource: boolean;
  }
  let {
    isModelArtifact,
    canEdit,
    canComment,
    sceneOpen,
    detailsOpen,
    onscene,
    ondetails,
    ongenerate,
    exportItems,
    pipelineItems,
    oncamera,
    pickMode,
    onpickmode,
    shading = $bindable(),
    wireframeOverlay = $bindable(),
    scaleRef = $bindable(),
    pickKind = $bindable(),
    turntable = $bindable(),
    play = $bindable(),
    showSource = $bindable(),
  }: Props = $props();

  const SHADING: readonly { id: ShadingMode; label: string }[] = [
    { id: 'shaded', label: 'Shaded' },
    { id: 'wireframe', label: 'Wireframe' },
    { id: 'matcap', label: 'Matcap' },
    { id: 'normals', label: 'Normals' },
  ];
  const PICK: readonly { id: SelectionKind3d; label: string; hint: string }[] = [
    { id: 'polygon', label: 'Polygon', hint: '⇧ adds' },
    { id: 'region', label: 'Region', hint: 'Tap to grow' },
    { id: 'mesh', label: 'Whole mesh', hint: '' },
    { id: 'point', label: 'Exact point', hint: '' },
  ];

  // Width tiers (the bar's own width, so a split pane counts, not the window).
  let width = $state(0);
  const roomy = $derived(width >= 400);
  const showShading = $derived(width >= 480);
  const labelComment = $derived(width >= 600);
  const wide = $derived(width >= 760);
  const shadingLabel = $derived(SHADING.find((s) => s.id === shading)?.label ?? 'Shaded');

  function shadingItems(): MenuItem[] {
    return [
      ...SHADING.map<MenuItem>((s) => ({ label: s.label, checked: shading === s.id, action: () => (shading = s.id) })),
      { separator: true },
      { label: 'Wireframe overlay', checked: wireframeOverlay, disabled: shading === 'wireframe', title: 'Draw the mesh edges over the shaded model', action: () => (wireframeOverlay = !wireframeOverlay) },
    ];
  }
  function more(e: MouseEvent): void {
    const items: MenuItem[] = [...pipelineItems];
    if (items.length) items.push({ separator: true });
    if (!showShading) items.push({ label: `Shading: ${shadingLabel}`, icon: 'eye', children: shadingItems() });
    if (canComment) {
      items.push({
        label: 'Comment on',
        icon: 'pin',
        children: PICK.map<MenuItem>((k) => ({ label: k.label, hint: k.hint, checked: pickMode && pickKind === k.id, action: () => ((pickKind = k.id), onpickmode(true)) })),
      });
    }
    items.push(
      { separator: true },
      { label: 'Turntable', checked: turntable, action: () => (turntable = !turntable) },
      ...(!wide && !isModelArtifact ? [{ label: 'Play scene', checked: play, action: () => (play = !play) }] : []),
      { label: 'Scale reference (1.8 m figure)', checked: scaleRef, action: () => (scaleRef = !scaleRef) },
      ...(oncamera ? [{ label: 'View → camera', icon: 'eye' as const, title: 'Use this view as the scene camera (Play, the agent and Blender)', action: oncamera }] : []),
      ...(!isModelArtifact ? [{ label: 'Scene JSON source', checked: showSource, action: () => (showSource = !showSource) }] : []),
      ...(!wide ? [{ separator: true }, { label: 'Export', icon: 'download' as const, children: exportItems() }] : []),
    );
    ctxMenu.show(e, items);
  }
</script>

<div class="toolbar" role="toolbar" aria-label="3D tools" bind:clientWidth={width} data-testid="s3d-toolbar">
  <button class="btn small scene-toggle" class:on={sceneOpen} aria-expanded={sceneOpen} aria-controls="s3d-scene-tools" onclick={onscene}>Scene</button>
  <button class="btn small details-toggle" class:on={detailsOpen} aria-expanded={detailsOpen} aria-controls="s3d-details-panel" onclick={ondetails}>Details</button>
  <button class="btn small primary" onclick={ongenerate} disabled={!canEdit} aria-haspopup="menu" data-testid="s3d-generate">
    <Icon name="sparkle" size={12} /> Generate {#if roomy}<Icon name="chevronDown" size={12} />{/if}
  </button>
  {#if showShading}
    <button class="btn small" onclick={(e) => ctxMenu.show(e, shadingItems())} aria-haspopup="menu" title="Shading" data-testid="s3d-shading">
      <Icon name="eye" size={12} /> {shadingLabel} <Icon name="chevronDown" size={12} />
    </button>
  {/if}
  {#if canComment}
    <button
      class="btn small"
      class:on={pickMode}
      aria-pressed={pickMode}
      aria-label="Comment on the model"
      title={pickMode ? 'Leave comment mode (C or Esc)' : 'Tap the model to comment on it (C)'}
      onclick={() => onpickmode(!pickMode)}
      data-testid="s3d-poly-comment-mode"
    >
      <Icon name="pin" size={12} />{#if labelComment}<span>{pickMode ? 'Commenting' : 'Comment'}</span>{/if}
    </button>
  {/if}
  {#if wide}
    {#if !isModelArtifact}
      <button class="btn small" class:on={play} aria-pressed={play} onclick={() => (play = !play)} data-testid="s3d-play">
        <Icon name="play" size={12} /> Play
      </button>
    {/if}
    <button class="btn small" onclick={(e) => ctxMenu.show(e, exportItems())} aria-haspopup="menu" data-testid="s3d-export">
      <Icon name="download" size={12} /> Export <Icon name="chevronDown" size={12} />
    </button>
  {/if}
  <span class="grow"></span>
  <button class="icon-btn more" onclick={more} aria-haspopup="menu" aria-label="More 3D tools" title="More 3D tools" data-testid="s3d-more">
    <Icon name="more" size={14} />
  </button>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    gap: 6px;
    min-block-size: 40px;
    padding-block: 6px;
    padding-inline: 12px;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
    overflow: hidden;
  }
  .toolbar > :global(*) {
    flex-shrink: 0;
  }
  .toolbar .on {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  .grow {
    flex: 1 1 0;
  }
  .scene-toggle,
  .details-toggle {
    display: none;
  }
  @media (pointer: coarse) {
    .toolbar {
      gap: 4px;
      padding-inline: 8px;
    }
    .toolbar :global(.btn),
    .more {
      min-block-size: 40px;
    }
    .more {
      min-inline-size: 40px;
    }
  }
</style>
