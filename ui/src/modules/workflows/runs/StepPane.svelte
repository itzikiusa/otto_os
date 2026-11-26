<script lang="ts">
  // The open step (ux.md §4.4): crumb, name and word, the model that ran, the
  // phase bar (where its time went, a wait's reason in words), then the tabs
  // its kind has — the read-only conversation, output, log, files; a review's
  // summary and summarizer; an approval's preview; the run's Result, Input
  // and (a goal run) Criteria. Its body loads on demand, at most once a second.
  import Icon from '../../../lib/components/Icon.svelte';
  import JsonTree from '../../../lib/components/JsonTree.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Tabs from '../../../lib/components/Tabs.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { workflowNodeDetail } from '../../../lib/api/workflows';
  import { loadErrorText } from '../../../lib/loadError';
  import type { ChildThread, Criterion, NodeRunState, RunState, StepThread, WorkflowRun } from '../../../lib/api/types';
  import RunFinalOutput from '../RunFinalOutput.svelte';
  import type { RunSync } from '../runSync.svelte';
  import GoalCard from './GoalCard.svelte';
  import StepConversation from './StepConversation.svelte';
  import StepFiles from './StepFiles.svelte';
  import { retryStep, skipStep } from './runActions';
  import { childWord, stepDuration, stepWord } from './runWords';
  import type { StepItem } from './stepOrder';
  import { paneTabs, phaseSegments, TAB_LABEL, type PaneTab } from './stepTabs';

  interface Props {
    run: WorkflowRun;
    sync: RunSync;
    workspaceId: string;
    runState: RunState;
    /** The open step; null = the run's Result. */
    step: StepItem | null;
    childId: string | null;
    stepCount: number;
    now: number;
    /** A tab a card asked for ("conversation", "preview"). */
    tab?: string | null;
    criteria?: readonly Criterion[];
    /** The pane's own Maximize (the run column folds behind "Run"). */
    maximized?: boolean;
    onmaximize?: () => void;
  }
  let { run, sync, workspaceId, runState, step, childId, stepCount, now, tab: asked = null, criteria = [], maximized = false, onmaximize }: Props = $props();

  const node = $derived(step?.node ?? null);
  const child = $derived<ChildThread | null>(node?.children?.find((c) => c.id === childId) ?? null);
  const tabs = $derived(paneTabs({ result: !step, goal: !!run.overview?.goal, child: !!child, kind: step?.kind, node: node ?? undefined }));
  let picked = $state<PaneTab | null>(null);
  $effect(() => {
    void step?.id;
    void childId;
    picked = (asked && tabs.includes(asked as PaneTab) ? asked : null) as PaneTab | null;
  });
  const tab = $derived(picked && tabs.includes(picked) ? picked : tabs[0]);

  // The full node (output, logs, digest): on demand, at most once a second.
  let detail = $state<NodeRunState | null>(null);
  let detailError = $state<string | null>(null);
  let lastFetch = 0;
  const needsBody = $derived(!!node && !child && ['output', 'log', 'summary', 'preview', 'conversation'].includes(tab));
  $effect(() => {
    const id = node?.node_id;
    void node?.detail_version;
    if (!id || !needsBody) return;
    const ac = new AbortController();
    const t = setTimeout(() => {
      lastFetch = Date.now();
      workflowNodeDetail(run.id, id, ac.signal).then(
        (r) => {
          detail = r.body;
          detailError = null;
        },
        (e) => {
          if (!ac.signal.aborted) detailError = loadErrorText(e);
        },
      );
    }, Math.max(0, lastFetch + 1000 - Date.now()));
    return () => {
      clearTimeout(t);
      ac.abort();
    };
  });
  const body = $derived(detail?.node_id === node?.node_id ? detail : null);

  const word = $derived(child ? childWord(child, now) : node ? stepWord(node, { now, run: runState, kind: step?.kind }) : null);
  const thread = $derived<StepThread | null>(
    child
      ? child.thread_id ? { thread_id: child.thread_id, runner: 'threads', provider: child.provider, model: child.model, effort: child.effort, turns: 0, usage: {} as StepThread['usage'] } : null
      : (body?.thread ?? node?.thread ?? null),
  );
  const modelChip = $derived(thread ? [thread.provider, thread.model || 'Default', thread.effort || 'Default'].join(' · ') : null);
  const segs = $derived(!child && node?.phases?.length ? phaseSegments(node.phases, node.status === 'running' ? now : Date.parse(node.started_at ?? '') + (node.duration_ms ?? 0)) : []);
  const segTotal = $derived(Math.max(1, segs.reduce((s, x) => s + x.ms, 0)));
  const summarizer = $derived(node?.children?.find((c) => /^summar/i.test(c.label) || c.id === 'summarizer') ?? null);
  const output = $derived(body?.output);
  const reviewOut = $derived((output ?? {}) as { score_text?: string; outcome?: string; blocking?: unknown[]; advisory?: unknown[]; findings?: unknown[] });
  const attachments = $derived((run as WorkflowRun & { attachments?: { name: string; mime: string; size_bytes?: number; source: string; error?: string }[] }).attachments ?? []);

  function stepMenu(e: MouseEvent): void {
    if (!node) return;
    const id = node.node_id;
    const items: MenuItem[] = [];
    if (!(run.status === 'pending' || run.status === 'running')) items.push({ label: 'Retry only this step', icon: 'refresh', action: () => void retryStep(run.id, id, false) });
    items.push({ label: 'Copy step output', icon: 'copy', action: async () => {
      const d = body ?? (await workflowNodeDetail(run.id, id)).body;
      void copyWithToast(typeof d.output === 'string' ? d.output : JSON.stringify(d.output ?? null, null, 2), 'Step output');
    } });
    if (node.status === 'running' && step?.kind !== 'human_approval') items.push({ separator: true }, { label: 'Skip this step', icon: 'stepOver', action: () => void skipStep(run.id, id) });
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }
</script>

<section class="pane" data-testid="wf-step-pane" data-node-id={step?.id ?? '_result'} aria-label={child?.label ?? step?.name ?? 'Run result'}>
  <header class="ph">
    <div class="crumb">
      {#if child}{step?.name}<Icon name="chevronRight" size={12} />Reviewer{:else if step}Step {node?.step_no ?? '–'} of {stepCount}{:else}Run result{/if}
    </div>
    <div class="prow">
      <h3 class="name ellipsis" dir="auto">{child?.label ?? step?.name ?? 'final-output.md'}</h3>
      {#if word?.word}<span class="word {word.tone}">{word.word}</span>{/if}
      <span class="grow"></span>
      {#if modelChip}<span class="chip" title="What ran">{modelChip}</span>{/if}
      {#if onmaximize}
        <button type="button" class="icon-btn" aria-label={maximized ? 'Show the run' : 'Maximize conversation'} title={maximized ? 'Show the run' : 'Maximize conversation'} onclick={onmaximize}><Icon name={maximized ? 'minimize' : 'maximize'} size={13} /></button>
      {/if}
      {#if node && !child}
        <button type="button" class="icon-btn" aria-label="Step actions" title="Step actions" onclick={stepMenu}><Icon name="more" size={14} /></button>
      {/if}
    </div>
    {#if segs.length}
      <div class="phases" aria-label="Where the step’s time went">
        <div class="pbar">{#each segs as sg, i (i)}<span class="seg {sg.phase}" class:failed={node?.status === 'error' && i === segs.length - 1} style:flex-grow={sg.ms / segTotal} title={sg.note ?? sg.label}></span>{/each}</div>
        <div class="plabels">{#each segs as sg, i (i)}<span>{sg.label} {stepDuration(sg.ms)}{sg.note && sg.phase === 'waiting' && sg.ms > 10_000 ? ` — ${sg.note}` : ''}</span>{/each}</div>
      </div>
    {:else if node?.wait && node.phase === 'waiting'}
      <p class="wait">{node.wait.text}</p>
    {/if}
  </header>

  {#if tabs.length > 1}
    <Tabs label="Step details" tabs={tabs.map((id) => ({ id, label: TAB_LABEL[id], testid: `wf-step-tab-${id}`, count: id === 'files' ? undefined : undefined }))} value={tab} onchange={(id) => (picked = id)} idBase="wf-step" />
  {/if}

  <div class="body" role="tabpanel" id="wf-step-panel-{tab}" aria-labelledby="wf-step-tab-{tab}">
    {#if tab === 'conversation'}
      <StepConversation {thread} sessions={child ? [] : (node?.sessions ?? [])} digest={body?.thread?.digest ?? null} {workspaceId} runId={run.id} nodeId={node?.node_id ?? ''} attempt={node?.attempts ?? 1} />
    {:else if tab === 'summarizer'}
      {#if summarizer?.thread_id}
        <StepConversation thread={{ thread_id: summarizer.thread_id, runner: 'threads', provider: summarizer.provider, turns: 0, usage: {} as StepThread['usage'] }} {workspaceId} runId={run.id} nodeId={node?.node_id ?? ''} attempt={node?.attempts ?? 1} />
      {:else}
        <p class="none">The summarizer starts after the reviewers.</p>
      {/if}
    {:else if tab === 'files'}
      <StepFiles runId={run.id} contextDir={run.context_dir} stepFile={step ? (node?.step_file ?? null) : null} />
    {:else if tab === 'result'}
      {#if run.status === 'success'}
        <RunFinalOutput mon={sync} {run} />
        {#if !sync.hasFinalOutput(run) && !sync.finalOutputError}<p class="none">This run wrote no final output.</p>{/if}
      {:else}
        <p class="none">The run has no result{run.status === 'running' || run.status === 'pending' ? ' yet' : ''}.</p>
      {/if}
    {:else if tab === 'criteria'}
      {#if run.overview?.goal}<GoalCard goal={run.overview.goal} {run} {criteria} />{/if}
    {:else if tab === 'input'}
      {#if attachments.length}
        <h4>Files sent with the run</h4>
        <ul class="att" data-testid="wf-run-attachments">
          {#each attachments as a (a.name)}
            <li data-testid="wf-run-attachment" data-source={a.source} data-error={a.error ? 'true' : undefined}>
              <Icon name="file" size={13} /><span class="ellipsis" dir="auto">{a.name}</span>
              <span class="dim">{a.mime}{a.size_bytes != null ? ` · ${Math.ceil(a.size_bytes / 1024)} KB` : ''}{a.source === 'steer' ? ' · sent during the run' : ''}{a.error ? ` · not downloaded: ${a.error}` : ''}</span>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="json"><JsonTree value={run.input} /></div>
    {:else}
      <LoadState what="the step" loading={!body && !detailError} error={detailError} empty={!body}>
        {#if tab === 'summary'}
          <p class="score">{reviewOut.score_text ?? (node?.status === 'running' ? 'Reviewing…' : '—')}{reviewOut.outcome ? ` · ${reviewOut.outcome}` : ''}</p>
          <p class="dim">{reviewOut.blocking?.length ?? 0} must-fix · {reviewOut.advisory?.length ?? 0} advisory · {reviewOut.findings?.length ?? 0} findings</p>
          {#if node?.children?.length}
            <h4>Reviewers</h4>
            <ul class="kids">{#each node.children as c (c.id)}{@const w = childWord(c, now)}<li><span class="ellipsis" dir="auto">{c.label}</span><span class="word {w.tone}">{w.word}</span></li>{/each}</ul>
          {/if}
          {#if body?.logs.length}<h4>Log</h4><pre class="log">{body.logs.slice(-4).join('\n')}</pre>{/if}
        {:else if tab === 'log'}
          {#if body?.logs.length}<pre class="log">{body.logs.slice(-500).join('\n')}</pre>{:else}<p class="none">No log lines.</p>{/if}
        {:else if tab === 'preview'}
          {#if node?.attention?.text}<p class="lead" dir="auto">{node.attention.text}</p>{/if}
          {#if sync.approvalBody}<pre class="log" dir="auto">{sync.approvalBody}</pre>{/if}
          {#if output != null}<div class="json"><JsonTree value={output} /></div>{/if}
        {:else if tab === 'output'}
          {#if typeof output === 'string'}<pre class="log" dir="auto">{output}</pre>
          {:else if output != null}<div class="json"><JsonTree value={output} /></div>
          {:else}<p class="none">This step has no output{node?.status === 'running' ? ' yet' : ''}.</p>{/if}
        {/if}
      </LoadState>
    {/if}
  </div>
</section>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-inline-size: 0;
    min-block-size: 0;
  }
  .crumb {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .prow {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-4);
    min-inline-size: 0;
    margin-block-start: var(--sp-2);
  }
  .name {
    max-inline-size: 100%;
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .grow {
    flex: 1;
  }
  .word {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .word.danger {
    color: var(--danger);
  }
  .word.warning {
    color: var(--warning);
  }
  .word.success {
    color: var(--success);
  }
  .phases {
    margin-block-start: var(--sp-4);
  }
  .pbar {
    display: flex;
    gap: var(--sp-1);
    block-size: 4px;
  }
  .seg {
    flex-basis: 0;
    min-inline-size: 4px;
    border-radius: var(--radius-s);
    background: var(--text-dim);
  }
  .seg.working {
    background: var(--accent);
  }
  .seg.waiting,
  .seg.queued {
    background: var(--border-strong);
  }
  .seg.needs_you {
    background: var(--warning);
  }
  .seg.finishing {
    background: var(--success);
  }
  .seg.failed {
    background: var(--danger);
  }
  .plabels {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1) var(--sp-5);
    margin-block-start: var(--sp-3);
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .wait {
    margin: 0;
    margin-block-start: var(--sp-3);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .body {
    min-block-size: 0;
    min-inline-size: 0;
  }
  .none,
  .dim {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text-dim);
  }
  .score {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .lead {
    margin: 0;
    margin-block-end: var(--sp-4);
    font-size: var(--fs-m);
  }
  h4 {
    margin: 0;
    margin-block: var(--sp-5) var(--sp-3);
    font-size: var(--fs-m);
  }
  .log {
    margin: 0;
    padding: var(--sp-5);
    max-block-size: 520px;
    overflow: auto;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .json {
    overflow: auto;
    font-size: var(--fs-s);
  }
  .kids,
  .att {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .kids li,
  .att li {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding-block: var(--sp-3);
    border-block-start: 1px solid var(--border);
    font-size: var(--fs-m);
  }
  .kids li span:first-child,
  .att li .ellipsis {
    flex: 1;
    min-inline-size: 0;
  }
  .att .dim {
    font-size: var(--fs-s);
  }
</style>
