<script lang="ts">
  // A background command's output (Agents & tasks' drill-in): Claude writes
  // it to a file and sends nothing while it runs, so the daemon reads the
  // tail (GET /runtime-sessions/{id}/tasks/{task}/output), polled while it
  // runs. Follows the bottom unless you scrolled up (T3 Code's
  // CommandOutputPanel, MIT, Copyright (c) 2026 T3 Tools Inc.).
  import { onDestroy, tick } from 'svelte';
  import { api } from '../../../lib/api/client';
  import type { RuntimeTaskOutput } from '../../../lib/api/types';

  interface Props {
    sessionId: string;
    taskId: string;
    live: boolean;
  }
  let { sessionId, taskId, live }: Props = $props();

  const POLL_MS = 2000;
  let out = $state<RuntimeTaskOutput | null>(null);
  let missing = $state(false);
  let el = $state<HTMLPreElement | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let gone = false;

  async function load(): Promise<void> {
    const follow = !el || el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
    try {
      out = await api.get<RuntimeTaskOutput>(`/runtime-sessions/${encodeURIComponent(sessionId)}/tasks/${encodeURIComponent(taskId)}/output`);
      missing = false;
      if (follow) void tick().then(() => el && (el.scrollTop = el.scrollHeight));
    } catch {
      missing = true;
    }
    if (!gone && live) timer = setTimeout(() => void load(), POLL_MS);
  }
  $effect(() => {
    void sessionId;
    void taskId;
    void load();
    return () => {
      if (timer) clearTimeout(timer);
    };
  });
  onDestroy(() => (gone = true));
</script>

<div class="to" data-testid="runtime-task-output">
  <p class="to-label">Output{#if out?.truncated} (latest){/if}</p>
  {#if out && out.text}
    <pre class="to-out mono" dir="ltr" bind:this={el}>{out.text}</pre>
  {:else if out}
    <p class="to-meta">{live ? 'No output yet…' : 'It printed nothing.'}</p>
  {:else if missing}
    <p class="to-meta">The output isn’t available (the agent no longer keeps it).</p>
  {:else}
    <p class="to-meta">Reading its output…</p>
  {/if}
</div>

<style>
  .to {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .to-label {
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .to-out {
    margin: 0;
    max-block-size: 50vh;
    overflow: auto;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .to-meta {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
</style>
