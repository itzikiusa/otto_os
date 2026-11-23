<script lang="ts">
  // The viewport's top-left tools: gizmo mode (only with an object selected),
  // Frame, and "View → camera". Touch gets 40 px targets.
  import { radioKey } from '../../radioKey';
  import type { GizmoMode } from './types';

  interface Props {
    gizmoModes: boolean;
    mode: GizmoMode;
    onmode: (m: GizmoMode) => void;
    onframe: () => void;
    /** Omitted → no "View → camera" button. */
    oncamera?: () => void;
  }
  let { gizmoModes, mode, onmode, onframe, oncamera }: Props = $props();
  const MODES: readonly { id: GizmoMode; label: string; key: string }[] = [
    { id: 'translate', label: 'Move', key: 'W' },
    { id: 'rotate', label: 'Rotate', key: 'E' },
    { id: 'scale', label: 'Scale', key: 'R' },
  ];
</script>

<div class="s3d-hud" role="group" aria-label="Viewport tools" data-testid="s3d-hud" onpointerdown={(e) => e.stopPropagation()} onpointerup={(e) => e.stopPropagation()}>
  {#if gizmoModes}
    <div class="s3d-seg" role="radiogroup" aria-label="Gizmo mode">
      {#each MODES as m (m.id)}
        <button role="radio" aria-checked={mode === m.id} tabindex={mode === m.id ? 0 : -1} onkeydown={radioKey} class:on={mode === m.id} title="{m.label} ({m.key})" onclick={() => onmode(m.id)}>{m.label}</button>
      {/each}
    </div>
  {/if}
  <button class="s3d-hbtn" title="Frame selection (F)" onclick={onframe}>Frame</button>
  {#if oncamera}
    <button class="s3d-hbtn" title="Use this view as the scene camera (Play + agent + Blender)" onclick={oncamera}>View → camera</button>
  {/if}
</div>

<style>
  .s3d-hud {
    position: absolute;
    inset-block-start: 8px;
    inset-inline-start: 8px;
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
    /* Wrap before the view cube's corner (≈ 60 px + its inset) instead of touching it. */
    max-inline-size: calc(100% - 84px);
  }
  .s3d-seg {
    display: inline-flex;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .s3d-seg button,
  .s3d-hbtn {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--text);
    font: var(--fs-xs) / 1 var(--font-ui);
    padding: 6px 8px;
    cursor: pointer;
  }
  .s3d-seg button + button {
    border-inline-start: 1px solid var(--border);
  }
  .s3d-seg button.on {
    background: var(--accent-solid);
    color: var(--accent-contrast);
  }
  .s3d-hbtn {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .s3d-hbtn:hover,
  .s3d-seg button:hover:not(.on) {
    background: color-mix(in srgb, var(--surface-2) 92%, transparent);
  }
  .s3d-seg button:focus-visible,
  .s3d-hbtn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  /* Fingers: the viewport tools are the primary manipulation controls. */
  @media (pointer: coarse) {
    .s3d-seg button,
    .s3d-hbtn {
      min-block-size: 40px;
      padding-inline: 12px;
      font-size: var(--fs-s);
    }
  }
</style>
