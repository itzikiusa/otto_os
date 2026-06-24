<script lang="ts">
  // Shared resize handles for canvas nodes. Wraps SvelteFlow's <NodeResizer>:
  // handles show only while the node is `selected`, and on resize-end we write
  // the final x/y/w/h back into the scene via `commitFromEditor` — which records
  // history + autosaves WITHOUT bumping `rev`, so the node is not remounted (no
  // flicker, the live size sticks).
  import { NodeResizer } from '@xyflow/svelte';
  import { canvas } from '../../../lib/stores/canvas.svelte';

  interface Props {
    id: string;
    /** Show handles only when the node is selected. */
    visible?: boolean;
    minWidth?: number;
    minHeight?: number;
    keepAspectRatio?: boolean;
  }
  let { id, visible = false, minWidth = 80, minHeight = 60, keepAspectRatio = false }: Props =
    $props();

  function onResizeEnd(
    _e: unknown,
    p: { x: number; y: number; width: number; height: number },
  ): void {
    if (!canvas.scene) return;
    canvas.commitFromEditor({
      ...canvas.scene,
      nodes: canvas.scene.nodes.map((n) =>
        n.id === id
          ? {
              ...n,
              x: Math.round(p.x),
              y: Math.round(p.y),
              w: Math.round(p.width),
              h: Math.round(p.height),
            }
          : n,
      ),
    });
  }
</script>

<NodeResizer
  isVisible={visible}
  {minWidth}
  {minHeight}
  {keepAspectRatio}
  {onResizeEnd}
  color="var(--accent)"
/>
