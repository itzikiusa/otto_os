<script lang="ts">
  // A successful run's deliverable (`final-output.md`) in a sandboxed,
  // token-themed iframe, or its failed read inline with Retry. Shown in the
  // inspector's run view on phone/tablet and in the Context sidebar on desktop.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import type { WorkflowRun } from '../../lib/api/types';
  import type { RunSync } from './runSync.svelte';

  let { mon, run }: { mon: RunSync; run: WorkflowRun } = $props();
</script>

{#if mon.hasFinalOutput(run)}
  <details open class="final-output">
    <summary>
      <Icon name="check" size={13} />
      <span>Final output</span>
    </summary>
    <iframe class="final-output-frame" title="Final output" sandbox="allow-same-origin" srcdoc={mon.finalOutputSrcdoc}></iframe>
  </details>
{:else if mon.finalOutputError?.runId === run.id}
  {@const runId = run.id}
  <LoadState what="the final output" variant="compact" error={mon.finalOutputError.text} empty onretry={() => void mon.loadFinalOutput(runId)} />
{/if}

<style>
  /* Final-output panel: the run's deliverable, shown above the context-file
     tree (both the mobile inline block and the desktop sidebar). Same shell
     as .ctx-files; content is a sandboxed iframe like FileTree's own markdown
     preview (see renderFinalOutputSrcdoc). */
  .final-output {
    margin: 8px 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    flex-shrink: 0;
  }
  .final-output > summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    user-select: none;
  }
  .final-output-frame {
    display: block;
    width: 100%;
    height: 320px;
    border: none;
    border-top: 1px solid var(--border);
    background: var(--surface); /* the srcdoc body is transparent + token-themed */
  }
</style>
