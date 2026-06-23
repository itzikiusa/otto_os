<script lang="ts">
  // Editor shell — wraps the actual <SvelteFlow> integration (CanvasFlow) in a
  // <SvelteFlowProvider> so that CanvasFlow's `useSvelteFlow()` call resolves the
  // flow store from context (the provider must be an ANCESTOR of the component
  // that calls the hook — it isn't available to the component that *renders*
  // <SvelteFlow> itself). This file is just plumbing: it forwards props down and
  // re-exposes CanvasFlow's `fit()` (zoom-to-fit) up to the Toolbar.
  import { SvelteFlowProvider } from '@xyflow/svelte';
  import CanvasFlow from './CanvasFlow.svelte';
  import type { Tool } from './tools';

  interface Props {
    activeTool: Tool;
    onToolDone: () => void;
    onselect: (ids: string[]) => void;
    ontool?: (t: Tool) => void;
    readonly?: boolean;
  }
  let { activeTool, onToolDone, onselect, ontool, readonly = false }: Props = $props();

  // Bind the child's exported zoom-to-fit so the page (Toolbar) can call it.
  let flow = $state<{ fit: () => void } | undefined>(undefined);
  export function fit(): void {
    flow?.fit();
  }
</script>

<SvelteFlowProvider>
  <CanvasFlow bind:this={flow} {activeTool} {onToolDone} {onselect} {ontool} {readonly} />
</SvelteFlowProvider>
