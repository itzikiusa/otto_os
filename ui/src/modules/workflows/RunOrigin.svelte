<script lang="ts">
  // The run header's provenance line for a chat-started run: "Started from
  // Slack — C123 · thread 1715.42", with the triggering message in a
  // disclosure. Nothing for a manual / scheduled / webhook run.
  import { runOrigin } from './runRoute';

  let { input }: { input: unknown } = $props();
  const origin = $derived(runOrigin(input));
</script>

{#if origin}
  <div class="run-origin" data-testid="run-origin">
    <span class="what">
      Started from {origin.source} — <code>{origin.chat}</code>{#if origin.thread} · thread <code>{origin.thread}</code>{/if}{#if origin.user} · by <code>{origin.user}</code>{/if}
    </span>
    {#if origin.raw}
      <details>
        <summary>Trigger message</summary>
        <pre>{origin.raw}</pre>
      </details>
    {/if}
  </div>
{/if}

<style>
  .run-origin {
    margin: 0 0 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    min-width: 0;
  }
  .what {
    overflow-wrap: anywhere;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  summary {
    cursor: pointer;
    font-size: var(--fs-xs);
    margin-top: 2px;
  }
  pre {
    margin: 4px 0 0;
    padding: 6px 8px;
    max-height: 200px;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--fs-xs);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
  }
</style>
