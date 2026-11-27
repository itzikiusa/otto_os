<script lang="ts">
  // The open step (ux.md §4.4): crumb, name and word, the model that ran, the
  // phase bar (where its time went, a wait's reason in words), then the tabs
  // its kind has — the read-only conversation, output, log, files; a review's
  // summary and summarizer; an approval's preview; the run's Result, Input
  // and (a goal run) Criteria. Its body loads on demand, at most once a second.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import JsonTree from '../../../lib/components/JsonTree.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Tabs from '../../../lib/components/Tabs.svelte';
  import { authedBlobUrl } from '../../../lib/api/client';
  import { copyWithToast } from '../../../lib/clipboard';
  import { downloadBlobUrl } from '../../../lib/downloadBlob';
  import { toastError } from '../../../lib/toastError';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { readRunContext, workflowNodeDetail } from '../../../lib/api/workflows';
  import { loadErrorText } from '../../../lib/loadError';
  import type { AttachmentRef, Criterion, NodeRunState, RunState, StepThread, WorkflowRun } from '../../../lib/api/types';
  import RunFinalOutput from '../RunFinalOutput.svelte';
  import type { RunSync } from '../runSync.svelte';
  import CriteriaTable from '../goal/CriteriaTable.svelte';
  import { criterionRows, latestCheck } from '../goal/goalCheck';
  import GoalCard from './GoalCard.svelte';
  import { cardKind } from './StateCards.svelte';
  import StepConversation from './StepConversation.svelte';
  import StepFiles from './StepFiles.svelte';
  import { openReview, retryStep, skipStep } from './runActions';
  import { reviewIds } from '../reviewAgents';
  import { runReviews } from '../runReviews.svelte';
  import { childWord, errorParts, stepDuration, stepWord } from './runWords';
  import { childCrumb, childTrail, resolveThreadSelection, type StepChild, type StepItem } from './stepOrder';
  import { paneTabs, phaseSegments, TAB_LABEL, type PaneTab } from './stepTabs';

  interface Props {
    run: WorkflowRun;
    sync: RunSync;
    workspaceId: string;
    runState: RunState;
    /** The open step; null = the run's Result. */
    step: StepItem | null;
    childId: string | null;
    /** The open step's number (its place in the run's order on an older run). */
    stepNo?: number | null;
    stepCount: number;
    now: number;
    /** A tab a card asked for ("conversation", "preview"). */
    tab?: string | null;
    criteria?: readonly Criterion[];
    /** The step whose failure the run's card beside the pane already shows. */
    cardNode?: string | null;
    /** The pane's own Maximize (the run column folds behind "Run"). */
    maximized?: boolean;
    onmaximize?: () => void;
    /** Select a sub-thread row (or null to return to the parent step). */
    onselectchild?: (childId: string | null) => void;
  }
  let { run, sync, workspaceId, runState, step, childId, stepNo = null, stepCount, now, tab: asked = null, criteria = [], cardNode = null, maximized = false, onmaximize, onselectchild }: Props = $props();

  const EMPTY_SESSIONS: readonly string[] = [];
  const node = $derived(step?.node ?? null);
  const stepChildren = $derived((node?.children ?? []) as StepChild[]);
  const child = $derived<StepChild | null>(stepChildren.find((c) => c.id === childId || (!!c.thread_id && c.thread_id === childId)) ?? null);
  const trail = $derived(child ? childTrail(stepChildren, child.id) : []);
  const childKind = $derived(child ? childCrumb(step?.kind, child) : null);
  function selectThread(threadId: string): void {
    if (!onselectchild) return;
    onselectchild(resolveThreadSelection(stepChildren, node?.thread?.thread_id, threadId));
  }
  const tabs = $derived(paneTabs({ result: !step, goal: !!run.overview?.goal, child: !!child, kind: step?.kind, node: node ?? undefined }));
  let picked = $state<PaneTab | null>(null);
  function pickTab(id: PaneTab): void {
    picked = id;
  }
  // Another step, child or asked-for tab starts over; a live update of the
  // same step (a new `step` object, a new `tabs` array) keeps the tab the
  // person picked. An asked tab the step does not offer yet falls back to the
  // first (`tab` below).
  const stepId = $derived(step?.id);
  const kid = $derived(childId);
  const want = $derived((asked ?? null) as PaneTab | null);
  $effect(() => {
    void stepId;
    void kid;
    picked = want;
  });
  const tab = $derived(picked && tabs.includes(picked) ? picked : tabs[0]);

  // The full node (output, logs, digest): on demand, at most once a second.
  // A live conversation reads directly from Threads (`runtimeViews`), so skip
  // node-detail fetches unless the step errored or fell back to its digest.
  let detail = $state<NodeRunState | null>(null);
  let detailError = $state<string | null>(null);
  let retry = $state(0);
  let lastFetch = 0;
  const nodeId = $derived(node?.node_id ?? null);
  const nodeAttempts = $derived(node?.attempts ?? 0);
  const nodeStartedAt = $derived(node?.started_at ?? null);
  const detailVersion = $derived(node?.detail_version);

  // Tracks when the embedded thread reports itself gone/forbidden so the pane
  // releases its `.conv` viewport lock (allowing digest scroll) and fetches the
  // node detail body on demand for the kept digest.
  let convGone = $state(false);
  const activeThreadId = $derived(child ? (child.thread_id ?? null) : (node?.thread?.thread_id ?? null));
  $effect(() => {
    void activeThreadId;
    void nodeId;
    void nodeAttempts;
    void tab;
    convGone = false;
  });
  function onConversationUnavailable(): void {
    convGone = true;
  }

  const liveConversation = $derived(tab === 'conversation' && node?.thread?.runner === 'threads' && node.thread.available !== false && !convGone);
  const needsBody = $derived(
    !!node &&
      !child &&
      (['output', 'log', 'summary', 'preview'].includes(tab) ||
        (tab === 'conversation' && (!liveConversation || node.status === 'error')) ||
        (tab === 'criteria' && step?.kind === 'check')),
  );
  $effect(() => {
    const id = nodeId;
    const att = nodeAttempts;
    const started = nodeStartedAt;
    void detailVersion;
    void retry;
    if (!id || !needsBody) return;
    // Another step's (or an earlier retry attempt's) error never shows while this one's body loads.
    if (untrack(() => detail?.node_id !== id || detail?.attempts !== att || (detail?.started_at ?? null) !== started)) {
      detailError = null;
    }
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
  // Bind cached detail to the current execution identity (node, attempt, start
  // timestamp, and associated thread) so a manual Retry (which resets attempts
  // to 1) never flashes a previous attempt's body or thread.
  const body = $derived(
    detail &&
      node &&
      detail.node_id === node.node_id &&
      detail.attempts === node.attempts &&
      (detail.started_at ?? null) === (node.started_at ?? null) &&
      (!detail.thread?.thread_id || !node.thread?.thread_id || detail.thread.thread_id === node.thread.thread_id)
      ? detail
      : null,
  );

  const word = $derived(child ? childWord(child, now) : node ? stepWord(node, { now, run: runState, kind: step?.kind }) : null);
  const thread = $derived<StepThread | null>(
    child
      ? child.thread_id ? { thread_id: child.thread_id, runner: 'threads', provider: child.provider, model: child.model, effort: child.effort, turns: 0, usage: {} as StepThread['usage'] } : null
      : (node?.thread
          ? (body?.thread?.thread_id === node.thread.thread_id ? body.thread : node.thread)
          : null),
  );
  const modelChip = $derived(thread ? [thread.provider, thread.model || 'Default', thread.effort || 'Default'].join(' · ') : null);
  const segs = $derived(!child && node?.phases?.length ? phaseSegments(node.phases, node.status === 'running' ? now : Date.parse(node.started_at ?? '') + (node.duration_ms ?? 0)) : []);
  const segTotal = $derived(Math.max(1, segs.reduce((s, x) => s + x.ms, 0)));
  const summarizer = $derived(node?.children?.find((c) => /^summar/i.test(c.label) || c.id === 'summarizer') ?? null);
  const summarizerThread = $derived<StepThread | null>(
    summarizer?.thread_id
      ? { thread_id: summarizer.thread_id, runner: 'threads', provider: summarizer.provider, turns: 0, usage: {} as StepThread['usage'] }
      : null,
  );
  /** The conversation tab mounts Threads' own header (provider · model · tokens), so don't repeat the model chip above it. */
  const showingThread = $derived(
    !convGone &&
      ((tab === 'conversation' && thread?.runner === 'threads' && thread.available !== false) ||
        (tab === 'summarizer' && !!summarizer?.thread_id)),
  );
  const output = $derived(body?.output);
  const reviewOut = $derived((output ?? {}) as { score_text?: string; outcome?: string; blocking?: unknown[]; advisory?: unknown[]; findings?: unknown[] });
  // A review step's records: the reviewer's own findings, "Open review".
  const reviews = $derived(step?.kind === 'review_run' && node ? reviewIds(body ?? node) : []);
  const reviewKey = $derived(child || tab === 'summary' ? reviews.join(',') : '');
  $effect(() => runReviews.watch(reviewKey ? reviewKey.split(',') : []));
  const agent = $derived(child ? reviews.flatMap((id) => runReviews.reviews[id]?.agents ?? []).find((a) => a.name === child.id || a.name === child.label || (!!child.thread_id && a.session_id === child.thread_id)) ?? null : null);
  const findings = $derived(agent?.findings ?? []);
  const tabItems = $derived(
    tabs.map((id) => ({ id, label: TAB_LABEL[id], testid: `wf-step-tab-${id}`, count: id === 'findings' && findings.length ? findings.length : undefined })),
  );
  /** The summary came from the deterministic fallback, not a verified summarizer. */
  const fallback = $derived(reviews.some((id) => runReviews.reviews[id]?.agents.some((a) => a.fallback)));
  const repoOf = (out: unknown): string | null => {
    const r = (out as { repo_id?: unknown } | null)?.repo_id ?? (run.input as { repo_id?: unknown } | null)?.repo_id;
    return typeof r === 'string' && r ? r : null;
  };
  const attachments = $derived<AttachmentRef[]>(run.attachments ?? []);
  /** The failing step's own words, on every tab (the run's card shows one failure only, and none on a pushed phone pane). */
  const failed = $derived(!child && node?.status === 'error' && node.node_id !== cardNode && (body?.error ?? node.error) ? errorParts((body?.error ?? node.error)!) : null);
  // Result → Input: a text attachment previews in place; any downloads.
  let shown = $state<{ path: string; text: string | null; error: string | null } | null>(null);
  const previewable = (a: AttachmentRef) => /^text\/|json|xml|yaml|markdown/.test(a.mime);
  async function preview(path: string): Promise<void> {
    if (shown?.path === path) return void (shown = null);
    shown = { path, text: null, error: null };
    try {
      const r = await readRunContext(run.id, path);
      if (shown?.path === path) shown.text = r.content;
    } catch (e) {
      if (shown?.path === path) shown.error = loadErrorText(e);
    }
  }
  async function download(a: AttachmentRef): Promise<void> {
    try {
      downloadBlobUrl(await authedBlobUrl(`/workflow-runs/${encodeURIComponent(run.id)}/context/read?path=${encodeURIComponent(a.path ?? '')}&download=1`), a.name);
    } catch (e) {
      toastError('Couldn’t download the file', e);
    }
  }

  function stepMenu(e: MouseEvent): void {
    if (!node) return;
    const id = node.node_id;
    const items: MenuItem[] = [];
    // Only a failed step re-runs alone (the server refuses it otherwise); any other step that ran re-runs with what follows.
    if (!(run.status === 'pending' || run.status === 'running')) {
      if (node.status === 'error') items.push({ label: 'Retry only this step', icon: 'refresh', action: () => void retryStep(run.id, id, false) });
      else if (node.status !== 'pending' && node.status !== 'running') items.push({ label: 'Retry from here', icon: 'refresh', action: () => void retryStep(run.id, id) });
    }
    items.push({ label: 'Copy step output', icon: 'copy', action: async () => {
      const d = body ?? (await workflowNodeDetail(run.id, id)).body;
      void copyWithToast(typeof d.output === 'string' ? d.output : JSON.stringify(d.output ?? null, null, 2), 'Step output');
    } });
    if (node.status === 'running' && step?.kind !== 'human_approval') items.push({ separator: true }, { label: 'Skip this step', icon: 'stepOver', action: () => void skipStep(run.id, id) });
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }
</script>

<section class="pane" data-testid="wf-step-pane" data-node-id={step?.id ?? '_result'} data-child-id={child?.id ?? undefined} aria-label={child?.label ?? step?.name ?? 'Run result'}>
  <header class="ph">
    <div class="crumb" data-testid="wf-step-crumb">
      {#if child}
        {#if onselectchild}
          <button type="button" class="crumb-link" data-testid="wf-crumb-parent" onclick={() => onselectchild(null)}>{step?.name}</button>
        {:else}
          <span>{step?.name}</span>
        {/if}
        {#each trail.slice(0, -1) as anc (anc.id)}
          <Icon name="chevronRight" size={12} />
          {#if onselectchild}
            <button type="button" class="crumb-link" data-testid="wf-crumb-ancestor" onclick={() => onselectchild(anc.id)}>{anc.label}</button>
          {:else}
            <span>{anc.label}</span>
          {/if}
        {/each}
        {#if childKind}<Icon name="chevronRight" size={12} />{childKind}{/if}
      {:else if step}Step {stepNo ?? node?.step_no ?? '–'} of {stepCount}{:else}Run result{/if}
    </div>
    <div class="prow">
      <h3 class="name ellipsis" dir="auto">{child?.label ?? step?.name ?? 'final-output.md'}</h3>
      {#if word?.word}<span class="word {word.tone}">{word.word}</span>{/if}
      <span class="grow"></span>
      {#if modelChip && !showingThread}<span class="chip" title="What ran">{modelChip}</span>{/if}
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
    {#if failed}
      <div class="failed" data-testid="wf-step-error">
        <p class="ftitle" dir="auto"><Icon name="x" size={13} />{failed.title}</p>
        {#if failed.body}<p class="fbody" dir="auto">{failed.body}</p>{/if}
      </div>
    {/if}
  </header>

  {#if tabs.length > 1}
    <Tabs label="Step details" tabs={tabItems} value={tab} onchange={pickTab} idBase="wf-step" />
  {/if}

  <div class="body" class:conv={showingThread} data-conv-thread={showingThread ? '1' : undefined} role="tabpanel" id="wf-step-panel-{tab}" aria-labelledby="wf-step-tab-{tab}">
    {#if tab === 'conversation'}
      <!-- A reviewer keeps no digest file of its own: no nodeId, so it never reads the step's. -->
      <StepConversation {thread} sessions={child ? EMPTY_SESSIONS : (node?.sessions ?? EMPTY_SESSIONS)} digest={child ? null : (body?.thread?.digest ?? null)} {workspaceId} runId={run.id} nodeId={child ? '' : (node?.node_id ?? '')} attempt={node?.attempts ?? 1} onselectthread={onselectchild ? selectThread : undefined} onunavailable={onConversationUnavailable} />
    {:else if tab === 'summarizer'}
      {#if summarizerThread}
        <StepConversation thread={summarizerThread} {workspaceId} runId={run.id} nodeId="" attempt={node?.attempts ?? 1} onselectthread={onselectchild ? selectThread : undefined} onunavailable={onConversationUnavailable} />
      {:else}
        <p class="none">The summarizer starts after the reviewers.</p>
      {/if}
    {:else if tab === 'findings'}
      {#if findings.length}
        <ul class="kids findings">
          {#each findings as f, i (i)}
            <li><span class="word {f.severity === 'bug' ? 'danger' : f.severity === 'warn' ? 'warning' : ''}">{f.severity === 'bug' ? 'Bug' : f.severity === 'warn' ? 'Warning' : 'Info'}</span>
              <span class="ftext"><span class="fpath" dir="ltr">{f.path ?? 'General'}{f.line != null ? `:${f.line}` : ''}</span><span dir="auto">{f.body}</span></span></li>
          {/each}
        </ul>
      {:else}
        <p class="none">{child?.status === 'success' ? 'This reviewer reported no findings.' : agent ? 'Findings appear when this reviewer finishes.' : 'No findings recorded for this reviewer.'}</p>
      {/if}
    {:else if tab === 'files'}
      <StepFiles runId={run.id} contextDir={run.context_dir} stepFile={node?.step_file ?? null} />
    {:else if tab === 'result'}
      <!-- A goal run whose card is taken by a hold, a person or an error: its criteria head the Result. -->
      {#if run.overview?.goal && cardKind(run, runState) !== 'goal'}<GoalCard goal={run.overview.goal} {run} {criteria} />{/if}
      {#if run.status === 'success'}
        <RunFinalOutput mon={sync} {run} />
        {#if !sync.hasFinalOutput(run) && !sync.finalOutputError}<p class="none">This run wrote no final output.</p>{/if}
      {:else}
        <p class="none">The run has no result{run.status === 'running' || run.status === 'pending' ? ' yet' : ''}.</p>
      {/if}
    {:else if tab === 'criteria'}
      {#if step?.kind === 'check'}
        <CriteriaTable rows={criterionRows(criteria, latestCheck(body ? [body] : []), node?.attention?.criterion_id ? [node.attention.criterion_id] : [])} />
      {:else if run.overview?.goal}<GoalCard goal={run.overview.goal} {run} {criteria} />{/if}
    {:else if tab === 'input'}
      {#if attachments.length}
        <h4>Files sent with the run</h4>
        <ul class="att" data-testid="wf-run-attachments">
          {#each attachments as a, i (a.path ?? `${a.source}:${a.name}:${i}`)}
            {@const path = !a.error ? a.path : undefined}
            <li data-testid="wf-run-attachment" data-source={a.source} data-error={a.error ? 'true' : undefined}>
              <Icon name="file" size={13} /><span class="ellipsis" dir="auto">{a.name}</span>
              <span class="dim">{a.mime}{a.size_bytes != null ? ` · ${Math.ceil(a.size_bytes / 1024)} KB` : ''}{a.source === 'steer' ? ' · sent during the run' : ''}{a.error ? ` · not downloaded: ${a.error}` : ''}</span>
              {#if path}
                {#if previewable(a)}<button type="button" class="btn small" aria-pressed={shown?.path === path} onclick={() => void preview(path)}>Preview</button>{/if}
                <button type="button" class="btn small" onclick={() => void download(a)}><Icon name="download" size={12} />Download</button>
              {/if}
            </li>
            {#if shown && shown.path === path}
              <li class="attprev" data-testid="wf-run-attachment-preview">
                {#if shown.error}<LoadState what="the file" variant="compact" error={shown.error} empty onretry={() => { const p = shown!.path; shown = null; void preview(p); }} />
                {:else if shown.text === null}<LoadState what="the file" variant="compact" loading empty />
                {:else}<pre class="log" dir="auto">{shown.text}</pre>{/if}
              </li>
            {/if}
          {/each}
        </ul>
      {/if}
      <div class="json"><JsonTree value={run.input} /></div>
    {:else}
      <LoadState what="the step" loading={!body && !detailError} error={detailError} empty={!body} onretry={() => retry++}>
        {#if tab === 'summary'}
          <p class="score">{reviewOut.score_text ?? (node?.status === 'running' ? 'Reviewing…' : '—')}{reviewOut.outcome ? ` · ${reviewOut.outcome}` : ''}</p>
          {#if fallback}<p class="dim" data-testid="summarizer-fallback">Deterministic fallback — the reviewers’ findings, not a verified summary.</p>{/if}
          <p class="dim">{reviewOut.blocking?.length ?? 0} must-fix · {reviewOut.advisory?.length ?? 0} advisory · {reviewOut.findings?.length ?? 0} findings</p>
          {#if node?.children?.length}
            <h4>Reviewers</h4>
            <ul class="kids">{#each node.children as c (c.id)}{@const w = childWord(c, now)}<li><span class="ellipsis" dir="auto">{c.label}</span><span class="word {w.tone}">{w.word}</span></li>{/each}</ul>
          {/if}
          {#if reviews.length}
            <button type="button" class="btn small" onclick={() => void openReview(reviews[0], repoOf(output))}><Icon name="eye" size={12} /> Open review</button>
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
    flex: 1;
    gap: var(--sp-4);
    min-inline-size: 0;
    min-block-size: 0;
  }
  .crumb {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .crumb-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .crumb-link:hover {
    text-decoration: underline;
  }
  .crumb-link:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
    border-radius: var(--radius-s);
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
  .body.conv {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-block-size: 0;
    overflow: hidden;
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
  .att li .ellipsis {
    min-inline-size: 12ch;
  }
  .att li {
    flex-wrap: wrap;
  }
  .att .dim {
    font-size: var(--fs-s);
  }
  .att li.attprev {
    display: block;
    border-block-start: 0;
  }
  .failed {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin-block-start: var(--sp-4);
    padding: var(--sp-4) var(--sp-5);
    border: 1px solid var(--danger);
    border-radius: var(--radius-m);
    background: var(--danger-soft);
  }
  .ftitle {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .ftitle :global(svg) {
    flex: none;
    margin-block-start: var(--sp-1);
  }
  .fbody {
    margin: 0;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .findings li {
    align-items: flex-start;
  }
  .findings li .word {
    flex: none;
    min-inline-size: 56px;
  }
  .ftext {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-inline-size: 0;
    overflow-wrap: anywhere;
  }
  .fpath {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
