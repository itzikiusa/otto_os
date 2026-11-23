<script lang="ts">
  // CSS 3D orientation cube synced to the orbit camera; faces snap the view.
  interface Props {
    cubeEl?: HTMLDivElement | null;
    onsnap: (dir: readonly [number, number, number]) => void;
  }
  let { cubeEl = $bindable<HTMLDivElement | null>(null), onsnap }: Props = $props();

  const CUBE_FACES = [
    { id: 'front', label: 'Front', dir: [0, 0, 1] },
    { id: 'back', label: 'Back', dir: [0, 0, -1] },
    { id: 'right', label: 'Right', dir: [1, 0, 0] },
    { id: 'left', label: 'Left', dir: [-1, 0, 0] },
    { id: 'top', label: 'Top', dir: [0, 1, 0.0001] },
    { id: 'bottom', label: 'Bot', dir: [0, -1, 0.0001] },
  ] as const;
</script>

<div
  class="s3d-cube-wrap"
  onpointerdown={(e) => e.stopPropagation()}
  onpointerup={(e) => e.stopPropagation()}
  role="group"
  aria-label="View cube"
>
  <div class="s3d-cube" bind:this={cubeEl}>
    {#each CUBE_FACES as f (f.id)}
      <button
        type="button"
        class="s3d-face {f.id}"
        title="{f.label} view"
        aria-label="{f.label} view"
        onclick={() => onsnap(f.dir)}
      >{f.label}</button>
    {/each}
  </div>
</div>

<style>
  .s3d-cube-wrap {
    position: absolute;
    inset-block-start: 12px;
    inset-inline-end: 12px;
    inline-size: 76px;
    block-size: 76px;
    perspective: 260px;
  }
  .s3d-cube {
    position: relative;
    inline-size: 56px;
    block-size: 56px;
    margin: 10px;
    transform-style: preserve-3d;
    transform: translateZ(-28px);
  }
  .s3d-face {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--border-strong);
    background: color-mix(in srgb, var(--surface) 92%, transparent);
    color: var(--text-dim);
    font: 600 var(--fs-xs) / 1 var(--font-ui);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
    backface-visibility: hidden;
  }
  .s3d-face:hover {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .s3d-face:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .s3d-face.front { transform: translateZ(28px); }
  .s3d-face.back { transform: rotateY(180deg) translateZ(28px); }
  .s3d-face.right { transform: rotateY(90deg) translateZ(28px); }
  .s3d-face.left { transform: rotateY(-90deg) translateZ(28px); }
  .s3d-face.top { transform: rotateX(90deg) translateZ(28px); }
  .s3d-face.bottom { transform: rotateX(-90deg) translateZ(28px); }
</style>
