<script lang="ts">
  import type { GoalLoop } from '../../lib/api/types';
  import ExecutionDiagram from './ExecutionDiagram.svelte';
  import { iterationEdges, iterationNodes, iterationPhases } from './iterationDiagram';
  let { loop }: { loop?: Pick<GoalLoop, 'phase' | 'status'> } = $props();
  let selected = $state('');
  const current = $derived(loop?.phase === 'waiting' ? 'executing' : loop?.phase);
  const active = $derived(iterationPhases.find(phase => phase.id === selected) ?? iterationPhases.find(phase => phase.id === current) ?? iterationPhases[0]);
  const nodes = $derived(iterationNodes(loop));
</script>

<section class="iteration-visual" aria-label="Goal loop execution">
  <h3>Goal loop</h3>
  <p class="explanation">Each iteration works toward the whole goal. Repeat until the criteria pass or the budget ends.</p>
  <ExecutionDiagram {nodes} edges={iterationEdges} selected={active.id} onselect={(id) => selected = id} label="Goal loop phases" loop />
  <div class="phase-detail"><strong>{active.title}</strong><p>{active.explanation}</p></div>
</section>

<style>
  h3 { font-size: var(--fs-m); font-weight: 600; margin: 0; }
  .explanation, .phase-detail { font-size: var(--fs-s); line-height: 1.5; }
  .explanation { color: var(--text-dim); margin: 6px 0 12px; }
  .phase-detail { margin-block-start: 12px; }
  .phase-detail p { margin: 4px 0 0; }
</style>
