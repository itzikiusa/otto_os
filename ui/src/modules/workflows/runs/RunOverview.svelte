<script lang="ts">
  // The top of the run column (ux.md §4.1, §2.3): the ring, the engine's
  // one-sentence headline (an always-mounted polite live region: only its
  // text changes) and the UI's ticking sub line, the facts line in names
  // (who, where, the PR and ticket, when, which version), then the one runState card.
  import Icon from '../../../lib/components/Icon.svelte';
  import { elapsed } from '../../../lib/statusWords';
  import type { Criterion, RunState, WorkflowRun } from '../../../lib/api/types';
  import type { RunSync } from '../runSync.svelte';
  import { runOrigin } from '../runRoute';
  import RunOrbit from './RunOrbit.svelte';
  import StateCards from './StateCards.svelte';
  import { clockTime, headline, subLine } from './runWords';
  import type { StepEntry } from './stepOrder';

  interface Props {
    run: WorkflowRun;
    runState: RunState;
    word: string;
    entries: readonly StepEntry[];
    sync: RunSync;
    now: number;
    nodeName: (id: string) => string;
    criteria?: readonly Criterion[];
    onopenstep: (nodeId: string, tab?: string) => void;
  }
  let { run, runState, word, entries, sync, now, nodeName, criteria = [], onopenstep }: Props = $props();

  const ov = $derived(run.overview ?? null);
  const live = $derived(run.status === 'pending' || run.status === 'running');
  const started = $derived(Date.parse(run.started_at));
  const total = $derived((run.finished_at ? Date.parse(run.finished_at) : now) - started);
  const text = $derived(headline({ overview: ov, state: runState, currentStep: null, error: run.error ?? null, durationMs: run.finished_at ? total : null }) || nodeName(run.nodes.find((n) => n.status === 'running')?.node_id ?? '') || word);
  const sub = $derived(
    ov
      ? subLine(ov, { startedAt: run.started_at, finishedAt: run.finished_at, now, waitingSteps: run.nodes.filter((n) => n.phase === 'waiting').length })
      : live ? `Started at ${clockTime(run.started_at)}` : `After ${elapsed(total)}`,
  );
  const typical = $derived(ov?.typical_ms ? `of ~${elapsed(ov.typical_ms)}` : '');
  const chat = $derived(runOrigin(run.input));
  const origin = $derived(ov?.origin);
  const subject = $derived(ov?.subject);
  /** A run from before Workflows moved to threads: no phases, no threads. */
  const older = $derived(!live && !ov && run.nodes.every((n) => !n.phases?.length && !n.thread));
</script>

<div class="top">
  <RunOrbit {entries} {runState} {word} centre={elapsed(total)} sub={live ? typical : ''} />
  <div class="sentence">
    <h2 class="headline" aria-live="polite" data-testid="wf-run-headline" dir="auto">{text}</h2>
    <p class="sub" data-testid="wf-run-sub">{sub}</p>
  </div>
</div>

<StateCards {run} {runState} {sync} {nodeName} {onopenstep} {criteria} />

<p class="facts">
  {#if origin?.user_name || origin?.channel_name}
    <span class="fact"><Icon name="user" size={13} /><strong>{origin.user_name ?? ''}</strong>{#if origin.channel_name}{origin.user_name ? ' in ' : ''}{#if origin.thread_url}<a href={origin.thread_url} target="_blank" rel="noreferrer noopener">{origin.channel_name}</a>{:else}{origin.channel_name}{/if}{/if}{#if chat}<span class="dim"> ({chat.source})</span>{/if}</span>
  {:else if chat}
    <span class="fact"><Icon name="comment" size={13} />Started from {chat.source}</span>
  {/if}
  {#if subject?.pr}
    <span class="fact"><Icon name="pr" size={13} /><a href={subject.pr.url} target="_blank" rel="noreferrer noopener">{subject.pr.repo} #{subject.pr.number}</a></span>
  {:else if subject?.repo || subject?.branch}
    <span class="fact"><Icon name="branch" size={13} />{[subject.repo, subject.branch].filter(Boolean).join(' · ')}</span>
  {/if}
  {#if subject?.ticket}
    <span class="fact"><Icon name="ticket" size={13} /><a href={subject.ticket.url} target="_blank" rel="noreferrer noopener">{subject.ticket.key}</a></span>
  {/if}
  <span class="fact"><Icon name="clock" size={13} />{clockTime(run.started_at)}{run.workflow_version ? ` · version ${run.workflow_version}` : ''}</span>
  {#if run.steering_queued?.length}
    <span class="fact" data-testid="wf-run-notes"><Icon name="note" size={13} />{run.steering_queued.length === 1 ? '1 note waiting' : `${run.steering_queued.length} notes waiting`} for the next step</span>
  {/if}
  {#if (run.resume_attempts ?? 0) > 0}
    <span class="fact"><Icon name="refresh" size={13} />Resumed after Otto restarted</span>
  {/if}
  {#if older}
    <span class="chip" title="This run finished before Workflows moved to threads: phase timings and conversations are not available; logs and outputs are.">Older run</span>
  {/if}
</p>

<style>
  .top {
    display: flex;
    align-items: center;
    gap: var(--sp-6);
  }
  .sentence {
    min-inline-size: 0;
  }
  .headline {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    line-height: 1.3;
    overflow-wrap: anywhere;
  }
  .sub {
    margin: 0;
    margin-block-start: var(--sp-2);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2) var(--sp-6);
    margin: 0;
    font-size: var(--fs-s);
  }
  .fact {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-inline-size: 0;
  }
  .fact :global(svg) {
    color: var(--text-dim);
  }
  .fact a {
    color: var(--accent-text);
    text-decoration: none;
  }
  .fact strong {
    font-weight: 500;
  }
  .dim {
    color: var(--text-dim);
  }
</style>
