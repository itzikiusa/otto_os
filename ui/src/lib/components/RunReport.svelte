<!--
  A run's result, rendered as markdown (sanitized) in the shared Modal — the
  one viewer for every run the side panel and Settings › Activity list. A run
  with no report (stopped, failed, still running) shows its status and reason;
  its thread is one click away inside.
-->
<script lang="ts">
  import Modal from './Modal.svelte';
  import Skeleton from './Skeleton.svelte';
  import LoadState from './LoadState.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import { authedText } from '../api/client';
  import { personalAgentsApi } from '../api/personalAgents';
  import { loadErrorText } from '../loadError';
  import { runStatus } from '../status';
  import { openAgentSession } from '../openAgentSession';
  import { firstLine } from '../plainText';
  import type { PersonalAgentRun } from '../api/types';

  /** `routine`: the routine's name — the title of any run of a routine, so it
   *  reads the same everywhere ("Midnight brief", not the report's own
   *  "Midnight Brief — Sat 7 Nov"). `name`: the run's trigger label, the title
   *  of a run with neither a routine nor a result. */
  let {
    run,
    routine = '',
    name = '',
    onclose,
  }: { run: PersonalAgentRun; routine?: string; name?: string; onclose: () => void } = $props();

  const title = $derived(routine || (run.status === 'ok' && firstLine(run.summary)) || name || 'Run');
  const reason = $derived(run.status === 'ok' ? '' : firstLine(run.error ?? ''));
  let html = $state('');
  let error = $state('');
  let loading = $state(true);
  let attempt = $state(0);

  $effect(() => {
    const id = run.id;
    void attempt;
    if (!run.report_rel) {
      loading = false;
      return;
    }
    let live = true;
    loading = true;
    error = '';
    html = '';
    Promise.all([authedText(personalAgentsApi.reportPath(id)), import('../md')])
      .then(([text, { renderMarkdownGfm }]) => {
        // The modal title stands for the report's own heading; don't say it twice.
        const lines = text.split('\n');
        const i = lines.findIndex((l) => l.trim() !== '');
        const dup = i >= 0 && (firstLine(lines[i]) === title || (!!routine && lines[i].trimStart().startsWith('#')));
        const body = dup ? lines.slice(i + 1).join('\n') : text;
        if (live) html = renderMarkdownGfm(body, { breaks: true });
      })
      .catch((e: unknown) => {
        if (live) error = loadErrorText(e);
      })
      .finally(() => {
        if (live) loading = false;
      });
    return () => {
      live = false;
    };
  });
</script>

{#snippet footer()}
  {#if run.session_id}
    <button type="button" class="btn" onclick={() => { const sid = run.session_id; onclose(); if (sid) void openAgentSession(sid); }}>Open its thread</button>
  {/if}
  <button type="button" class="btn primary" onclick={onclose}>Done</button>
{/snippet}

<Modal {title} width={760} {onclose} {footer}>
  <StatusBadge status={runStatus(run.status)} variant="text" />
  {#if reason}<p class="rr-why" dir="auto">{reason}</p>{/if}
  {#if !run.report_rel}
    {#if !reason}<p class="rr-why">{run.status === 'running' ? 'Still working — the result lands here when it finishes.' : 'This run wrote no report.'}</p>{/if}
  {:else if loading}
    <Skeleton rows={6} height={18} label="the report" />
  {:else if error}
    <LoadState what="the report" {error} empty onretry={() => (attempt += 1)} />
  {:else}
    <!-- renderMarkdownGfm output goes through the allowlist sanitizer. -->
    <div class="md-body rr">{@html html}</div>
  {/if}
</Modal>

<style>
  .rr-why { margin: 8px 0 0; color: var(--text-dim); }
  .rr { color: var(--text); overflow-wrap: anywhere; margin-block-start: 8px; }
  .rr :global(h1), .rr :global(h2), .rr :global(h3) { font-size: var(--fs-m); font-weight: 600; margin: 14px 0 6px; }
  .rr :global(h1:first-child), .rr :global(h2:first-child), .rr :global(h3:first-child) { margin-block-start: 0; }
  .rr :global(ul), .rr :global(ol) { padding-inline: 22px 0; }
  .rr :global(table) { border-collapse: collapse; display: block; overflow-x: auto; font-size: var(--fs-s); margin: 8px 0; }
  .rr :global(th), .rr :global(td) { border: 1px solid var(--border); padding: 4px 8px; text-align: start; }
  .rr :global(a) { color: var(--accent-text); }
  .rr :global(hr) { border: none; border-block-start: 1px solid var(--border); margin: 12px 0; }
</style>
