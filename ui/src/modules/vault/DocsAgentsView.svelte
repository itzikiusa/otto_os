<script lang="ts">
  // Docs agents — fan 1-4 writer agents out over a prompt to author notes into
  // the vault (a summarizer consolidates drafts when >1 writer), plus the
  // vault's RUN HISTORY (docs runs + per-note refine turns, server-persisted
  // in `vault_docs_runs`). Center-stage view: a compact form, the selected
  // run's per-agent rows with live status + inline terminals, and the runs
  // list below. The selected run lives on the vault store; the LIST is
  // refetched from the server on every mount — that is what makes runs
  // reappear after a tab/module switch or a full app restart. This view owns
  // the 1.5s poll timer and stops it once nothing is active.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { api } from '../../lib/api/client';
  import { contextApi } from '../../lib/api/context';
  import type {
    VaultDocsFindingEvidence,
    VaultDocsReviewSkill,
    VaultDocsRun,
  } from '../../lib/api/types';
  import {
    cancelDocsRun,
    docsRun as getDocsRun,
    retryDocsAgent,
    retryDocsReviewer,
    retryDocsRevision,
    retryDocsSummarizer,
    runDocsAgents,
  } from '../../lib/api/vault';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { toasts } from '../../lib/toast.svelte';
  import { DOCS_TEMPLATES } from './docsTemplates';
  import { vault } from './vault.svelte';

  interface AgentRow {
    provider: string;
    model: string;
  }

  interface ReviewerRow extends AgentRow {
    skill: VaultDocsReviewSkill;
    focus: string;
  }

  const REVIEW_METHODS: { value: VaultDocsReviewSkill; label: string }[] = [
    { value: 'vault-docs-review', label: 'Generic — complete bundle' },
    { value: 'vault-api-review', label: 'API contracts and flows' },
    { value: 'vault-data-review', label: 'Datastores and impact' },
    { value: 'vault-runtime-review', label: 'Runtime, workers and messaging' },
    { value: 'vault-evidence-review', label: 'Evidence and coverage' },
  ];

  const newReviewer = (): ReviewerRow => ({
    provider: defaultAgentProvider(),
    model: '',
    skill: 'vault-docs-review',
    focus: '',
  });

  // -- form ---------------------------------------------------------------------
  let prompt = $state('');
  let targetDir = $state('');
  let agents = $state<AgentRow[]>([{ provider: defaultAgentProvider(), model: '' }]);
  let sumProvider = $state(defaultAgentProvider());
  let reviewEnabled = $state(false);
  let reviewers = $state<ReviewerRow[]>([newReviewer()]);
  let maxReviewIterations = $state(3);
  let starting = $state(false);

  // Prepared prompts — pick a template, fill the repo path, Insert. The
  // template's skills ride the run (RunReq.skills) and show as chips below.
  let tplId = $state('');
  let tplRepo = $state('');
  // Infra/library repo: no HTTP API of its own, flows are partial — the
  // template pivots its scope to packages + exported interfaces + consumers.
  let tplInfra = $state(false);
  let tplSkills = $state<string[]>([]);
  const tpl = $derived(DOCS_TEMPLATES.find((t) => t.id === tplId) ?? null);

  function applyTemplate(): void {
    if (!tpl) return;
    prompt = tpl.build(tplRepo.trim(), tplInfra);
    tplSkills = [...tpl.skills];
  }

  /** Everything the run will inject (template skills + okf on OKF vaults). */
  const runSkills = $derived([
    ...tplSkills,
    ...(vault.current?.okf && !tplSkills.includes('okf-authoring') ? ['okf-authoring'] : []),
  ]);

  // Skill viewer — chips are clickable so what gets injected is inspectable.
  let skillView = $state<{ name: string; body: string } | null>(null);
  async function viewSkill(name: string): Promise<void> {
    try {
      const s = await contextApi.getSkill(name);
      skillView = { name, body: s.body };
    } catch (e) {
      toasts.error('Skill', e instanceof Error ? e.message : String(e));
    }
  }

  const providers = $derived(agentProviders());
  const run = $derived(vault.docsRun);
  const isActive = (r: VaultDocsRun) =>
    r.state === 'running' ||
    r.state === 'summarizing' ||
    r.state === 'reviewing' ||
    r.state === 'revising';
  const active = $derived(run != null && isActive(run));
  /** Anything (selected or listed) still moving → the poll keeps ticking. */
  const anyActive = $derived((run != null && isActive(run)) || vault.docsRuns.some(isActive));

  // Prefill (and re-prefill on "Docs agent here" from a folder's context menu).
  $effect(() => {
    targetDir = vault.docsAgentsDir;
  });

  function addAgent(): void {
    if (agents.length < 4) agents = [...agents, { provider: defaultAgentProvider(), model: '' }];
  }

  function removeAgent(i: number): void {
    if (agents.length > 1) agents = agents.filter((_, x) => x !== i);
  }

  function addReviewer(): void {
    if (reviewers.length < 4) reviewers = [...reviewers, newReviewer()];
  }

  function removeReviewer(i: number): void {
    if (reviewers.length > 1) reviewers = reviewers.filter((_, x) => x !== i);
  }

  function evidenceLabel(evidence: VaultDocsFindingEvidence): string {
    const source = evidence.repo_path || evidence.doc_path || 'Evidence';
    const location = evidence.line ? `:${evidence.line}` : evidence.section ? ` · ${evidence.section}` : '';
    return `${source}${location}`;
  }

  function reviewStateLabel(r: VaultDocsRun): string {
    if (r.review.state === 'clean') return 'Review complete';
    if (r.review.state === 'exhausted') return 'Review limit reached';
    if (r.review.state === 'error') return 'Review failed';
    if (r.review.state === 'cancelled') return 'Review cancelled';
    if (r.review.state === 'interrupted') return 'Review interrupted';
    if (r.review.state === 'pending') return 'Review queued';
    return `Review round ${Math.max(r.review.current_iteration, 1)} of ${r.review.max_iterations}`;
  }

  const displayState = (state: string): string => state.replaceAll('_', ' ');

  function reviewIterationLimit(value: number): number {
    return Number.isFinite(value) ? Math.min(10, Math.max(1, Math.round(value))) : 3;
  }

  // -- run + polling ---------------------------------------------------------------
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  function stopPoll(): void {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = null;
  }

  function startPoll(): void {
    stopPoll();
    pollTimer = setInterval(() => void poll(), 1500);
  }

  async function poll(): Promise<void> {
    const r = vault.docsRun;
    try {
      if (r && isActive(r)) {
        const next = await getDocsRun(r.id);
        // Async guard: ignore if the user selected another run meanwhile.
        if (vault.docsRun?.id === next.id) vault.docsRun = next;
        if (!isActive(next)) {
          // The run wrote (or trashed drafts of) notes — reflect it everywhere.
          void vault.refreshTree();
          void vault.refreshStatus();
        }
      }
      // Keep the history list in step (it also carries refine turns that
      // complete server-side without this view's involvement).
      await vault.refreshDocsRuns();
    } catch {
      /* transient — next tick retries */
    }
    if (!anyActive) stopPoll();
  }

  async function start(): Promise<void> {
    if (!prompt.trim() || starting || !vault.current) return;
    starting = true;
    try {
      vault.docsRun = await runDocsAgents(vault.wsId, vault.current.id, {
        prompt: prompt.trim(),
        target_dir: targetDir.trim(),
        agents: agents.map((a) => ({ provider: a.provider, model: a.model.trim() || undefined })),
        summarizer: agents.length > 1 ? { provider: sumProvider } : undefined,
        skills: tplSkills.length ? tplSkills : undefined,
        review: reviewEnabled
          ? {
              max_iterations: reviewIterationLimit(maxReviewIterations),
              reviewers: reviewers.map((reviewer) => ({
                provider: reviewer.provider,
                model: reviewer.model.trim() || undefined,
                skill: reviewer.skill,
                focus: reviewer.focus.trim() || undefined,
              })),
            }
          : undefined,
      });
      openTerminals = new Set();
      void vault.refreshDocsRuns();
      startPoll();
    } catch (e) {
      toasts.error('Docs agent', e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }

  // Per-slot retry (writers by index, 'sum' = summarizer) — kills the stuck
  // session server-side; a fresh one re-spawns with the same prompt.
  let retrying = $state<Record<string, boolean>>({});
  async function retry(target: number | 'sum'): Promise<void> {
    const r = vault.docsRun;
    if (!r || retrying[String(target)]) return;
    retrying = { ...retrying, [String(target)]: true };
    try {
      if (target === 'sum') await retryDocsSummarizer(r.id);
      else await retryDocsAgent(r.id, target);
      startPoll();
    } catch (e) {
      toasts.error('Retry', e instanceof Error ? e.message : String(e));
    } finally {
      retrying = { ...retrying, [String(target)]: false };
    }
  }

  async function retryReviewer(iteration: number, index: number): Promise<void> {
    const r = vault.docsRun;
    const key = `reviewer-${iteration}-${index}`;
    if (!r || retrying[key]) return;
    retrying = { ...retrying, [key]: true };
    try {
      await retryDocsReviewer(r.id, iteration, index);
      startPoll();
    } catch (e) {
      toasts.error('Retry reviewer', e instanceof Error ? e.message : String(e));
    } finally {
      retrying = { ...retrying, [key]: false };
    }
  }

  async function retryRevision(iteration: number): Promise<void> {
    const r = vault.docsRun;
    const key = `revision-${iteration}`;
    if (!r || retrying[key]) return;
    retrying = { ...retrying, [key]: true };
    try {
      await retryDocsRevision(r.id, iteration);
      startPoll();
    } catch (e) {
      toasts.error('Retry revision', e instanceof Error ? e.message : String(e));
    } finally {
      retrying = { ...retrying, [key]: false };
    }
  }

  let cancelling = $state(false);
  async function cancel(): Promise<void> {
    const r = vault.docsRun;
    if (!r || cancelling) return;
    cancelling = true;
    try {
      await cancelDocsRun(r.id);
      await poll();
    } catch (e) {
      toasts.error('Cancel', e instanceof Error ? e.message : String(e));
    } finally {
      cancelling = false;
    }
  }

  /** Back to the form (keeps the prompt so a tweak-and-rerun is one edit). */
  function newRun(): void {
    vault.docsRun = null;
    openTerminals = new Set();
  }

  /** Show one run (live or history) — the poll follows the selection. */
  function selectRun(r: VaultDocsRun): void {
    vault.docsRun = r;
    openTerminals = new Set();
    if (isActive(r)) startPoll();
  }

  // Inline live terminals — multiple may be open at once, keyed by session id.
  // History runs may reference sessions retention has since pruned — verify
  // the session still exists before mounting a dead terminal.
  let openTerminals = $state<Set<string>>(new Set());
  async function toggleTerminal(sessionId: string | null): Promise<void> {
    if (!sessionId) return;
    const next = new Set(openTerminals);
    if (next.has(sessionId)) {
      next.delete(sessionId);
      openTerminals = next;
      return;
    }
    try {
      await api.get(`/sessions/${sessionId}`);
    } catch {
      toasts.error('Docs agent', 'This agent session no longer exists (cleaned up by retention).');
      return;
    }
    next.add(sessionId);
    openTerminals = next;
  }

  onMount(() => {
    // Refetch the persisted list, then re-surface the most recent ACTIVE run
    // when nothing is selected — a run launched before a tab switch (or a
    // daemon restart, as `interrupted` history) is visible again immediately.
    void vault.refreshDocsRuns().then(() => {
      if (!vault.docsRun) {
        const live = vault.docsRuns.find(isActive);
        if (live) vault.docsRun = live;
      }
      const r = vault.docsRun;
      if ((r && isActive(r)) || vault.docsRuns.some(isActive)) startPoll();
    });
    const r = vault.docsRun;
    if (r && isActive(r)) startPoll();
    return () => stopPoll();
  });
</script>

<div class="docs-agents">
  <div class="inner">
    <h2><Icon name="zap" size={15} /> Docs agent</h2>

    {#if !run}
      <!-- ── form ─────────────────────────────────────────────────────────── -->
      <div class="fld">
        <span>Prepared prompt (optional)</span>
        <div class="tpl-row">
          <select bind:value={tplId}>
            <option value="">— pick a template —</option>
            {#each DOCS_TEMPLATES as t (t.id)}
              <option value={t.id}>{t.label}</option>
            {/each}
          </select>
          {#if tpl?.needsRepo}
            <input class="tpl-repo" bind:value={tplRepo} placeholder="~/path/to/repo" />
          {/if}
          <button
            class="tpl-use"
            disabled={!tpl || (tpl.needsRepo && !tplRepo.trim())}
            onclick={applyTemplate}>Insert</button
          >
        </div>
        {#if tpl?.needsRepo}
          <label
            class="tpl-infra"
            title="Library/infrastructure repo: no HTTP API of its own, flows are partial — documents packages, exported interfaces, consumers and config instead"
          >
            <input type="checkbox" bind:checked={tplInfra} />
            Infra / library repo (no HTTP API — document exported packages instead)
          </label>
        {/if}
        {#if tpl}
          <div class="tpl-hint">{tpl.hint} You can edit the inserted prompt freely.</div>
        {/if}
      </div>

      <label class="fld">
        <span>What should be documented?</span>
        <textarea
          bind:value={prompt}
          rows="4"
          placeholder="e.g. Document the deploy pipeline: triggers, stages, rollback, and the runbook for a failed release."
        ></textarea>
      </label>
      <label class="fld">
        <span>Target folder (vault-relative, blank = root)</span>
        <input bind:value={targetDir} placeholder="runbooks/deploys" />
      </label>

      <div class="fld">
        <span>Writer agents ({agents.length}/4)</span>
        {#each agents as agent, i (i)}
          <div class="agent-row">
            <select bind:value={agent.provider}>
              {#each providers as p (p)}
                <option value={p}>{p}</option>
              {/each}
            </select>
            <input class="model" bind:value={agent.model} placeholder="model (optional)" />
            <button
              class="icon-btn"
              title="Remove agent"
              disabled={agents.length <= 1}
              onclick={() => removeAgent(i)}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
        {#if agents.length < 4}
          <button class="add-agent" onclick={addAgent}>+ add agent</button>
        {/if}
      </div>

      {#if agents.length > 1}
        <label class="fld">
          <span>Summarizer (consolidates the {agents.length} drafts into final notes)</span>
          <select class="sum-select" bind:value={sumProvider}>
            {#each providers as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </label>
      {/if}

      <section class="review-config" class:enabled={reviewEnabled}>
        <div class="review-config-head">
          <div>
            <strong>Review outcomes</strong>
            <span>Independent agents check the final bundle before the run finishes.</span>
          </div>
          <label class="switch">
            <input type="checkbox" bind:checked={reviewEnabled} aria-label="Review outcomes" />
            <span aria-hidden="true"></span>
          </label>
        </div>

        {#if reviewEnabled}
          <div class="review-settings">
            <div class="review-setting-title">
              <span>Reviewer agents ({reviewers.length}/4)</span>
              <label class="iteration-field">
                Maximum review iterations
                <input
                  type="number"
                  min="1"
                  max="10"
                  bind:value={maxReviewIterations}
                  onchange={() => (maxReviewIterations = reviewIterationLimit(maxReviewIterations))}
                  aria-label="Maximum review iterations"
                />
              </label>
            </div>
            {#each reviewers as reviewer, i (i)}
              <div class="reviewer-config-row">
                <span class="reviewer-number">{i + 1}</span>
                <div class="reviewer-fields">
                  <div class="reviewer-main-fields">
                    <select bind:value={reviewer.provider} aria-label="Reviewer provider">
                      {#each providers as p (p)}
                        <option value={p}>{p}</option>
                      {/each}
                    </select>
                    <input
                      bind:value={reviewer.model}
                      placeholder="model (optional)"
                      aria-label="Reviewer model"
                    />
                    <select class="review-method" bind:value={reviewer.skill} aria-label="Review method">
                      {#each REVIEW_METHODS as method (method.value)}
                        <option value={method.value}>{method.label}</option>
                      {/each}
                    </select>
                    <button
                      class="icon-btn"
                      title="Remove reviewer"
                      aria-label="Remove reviewer"
                      disabled={reviewers.length <= 1}
                      onclick={() => removeReviewer(i)}
                    >
                      <Icon name="x" size={12} />
                    </button>
                  </div>
                  <input
                    class="review-focus"
                    bind:value={reviewer.focus}
                    placeholder="Optional focus — e.g. request/response bodies"
                    aria-label="Review focus"
                  />
                </div>
              </div>
            {/each}
            {#if reviewers.length < 4}
              <button class="add-agent" onclick={addReviewer}>+ add reviewer</button>
            {/if}
            <p class="review-hint">
              Review stops early when every reviewer reports no findings in the same round.
            </p>
          </div>
        {/if}
      </section>

      {#if runSkills.length > 0}
        <div class="fld">
          <span>Skills injected into this run — click to view</span>
          <div class="skill-chips">
            {#each runSkills as s (s)}
              <button class="skill-chip" title="View {s}" onclick={() => void viewSkill(s)}>
                <Icon name="function" size={11} />
                {s}
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <div class="form-actions">
        <button class="primary" disabled={!prompt.trim() || starting} onclick={() => void start()}>
          {starting ? 'Starting…' : 'Run'}
        </button>
      </div>
    {:else}
      <!-- ── run view ──────────────────────────────────────────────────────── -->
      <div class="run-head">
        <span
          class="pill st-{run.state}"
          title={run.state === 'interrupted' ? 'The app/daemon restarted mid-run' : undefined}
        >
          {#if active}<span class="spinner-xs"></span>{/if}
          {displayState(run.state)}
        </span>
        <span class="kind-chip">{run.kind}</span>
        <span class="run-meta" title={run.prompt}>
          {#if run.kind === 'refine'}
            <button class="note-link" onclick={() => void vault.open(run.note_path)}>
              {run.note_path}
            </button>
            — {run.prompt}
          {:else}
            {run.prompt}{run.target_dir ? ` → ${run.target_dir}/` : ''}
          {/if}
        </span>
        <span class="grow"></span>
        {#if active && run.kind === 'docs'}
          <button class="ghost" disabled={cancelling} onclick={() => void cancel()}>
            {cancelling ? 'Cancelling…' : 'Cancel'}
          </button>
        {/if}
        <!-- Always available — a stuck-RUNNING run must never trap the user
             in the run view with no way to start a new run. -->
        <button class="ghost" onclick={newRun}>New run</button>
      </div>

      {#if run.error}
        <div class="err" role="alert">{run.error}</div>
      {/if}

      <div class="rows">
        {#each run.agents as agent (agent.index)}
          <div class="agent-card">
            <div class="agent-top">
              <span class="agent-name">{agent.name}</span>
              <span class="chip">{agent.provider}{agent.model ? ' · ' + agent.model : ''}</span>
              <span class="grow"></span>
              {#if agent.session_id}
                <button class="ghost small" onclick={() => void toggleTerminal(agent.session_id)}>
                  {openTerminals.has(agent.session_id) ? 'Hide' : 'Open'}
                </button>
              {/if}
              <!-- An errored writer stays retryable while the writers stage is
                   still open (run.state === 'running') — its slot keeps
                   listening for the retry flag as long as any peer is moving. -->
              {#if active && (agent.state === 'running' || agent.state === 'pending' || (agent.state === 'error' && run.state === 'running'))}
                <button
                  class="ghost small"
                  title={agent.state === 'error'
                    ? "Re-spawn this writer's turn in a fresh session"
                    : "Kill this writer's session and restart its turn fresh"}
                  disabled={retrying[String(agent.index)]}
                  onclick={() => void retry(agent.index)}
                >
                  {retrying[String(agent.index)] ? 'Retrying…' : 'Retry'}
                </button>
              {/if}
              <span class="pill st-{agent.state}">
                {#if agent.state === 'running'}<span class="spinner-xs"></span>{/if}
                {agent.state}
              </span>
            </div>
            {#if agent.error}
              <p class="agent-err">{agent.error}</p>
            {/if}
            {#if agent.drafts.length > 0 && active}
              <p class="drafts">
                {agent.drafts.length} draft{agent.drafts.length === 1 ? '' : 's'}:
                <span class="mono">{agent.drafts.join(' · ')}</span>
              </p>
            {/if}
            {#if agent.session_id && openTerminals.has(agent.session_id)}
              <div class="term">
                {#key agent.session_id}
                  <Terminal sessionId={agent.session_id} preferDom />
                {/key}
              </div>
            {/if}
          </div>
        {/each}

        <!-- Summarizer row — same treatment; "skipped" when there is 1 writer.
             Refine turns have no summarizer stage at all. -->
        {#if run.kind !== 'refine'}
          <div class="agent-card">
          <div class="agent-top">
            <span class="agent-name">summarizer</span>
            <span class="chip">
              {run.summarizer.provider}{run.summarizer.model ? ' · ' + run.summarizer.model : ''}
            </span>
            <span class="grow"></span>
            {#if run.summarizer.session_id}
              <button
                class="ghost small"
                onclick={() => void toggleTerminal(run.summarizer.session_id)}
              >
                {openTerminals.has(run.summarizer.session_id) ? 'Hide' : 'Open'}
              </button>
            {/if}
            {#if run.state === 'summarizing' && (run.summarizer.state === 'running' || run.summarizer.state === 'pending')}
              <button
                class="ghost small"
                title="Kill the summarizer's session and restart the consolidation fresh"
                disabled={retrying['sum']}
                onclick={() => void retry('sum')}
              >
                {retrying['sum'] ? 'Retrying…' : 'Retry'}
              </button>
            {/if}
            <span class="pill st-{run.summarizer.state}">
              {#if run.summarizer.state === 'running'}<span class="spinner-xs"></span>{/if}
              {run.summarizer.state}
            </span>
          </div>
          {#if run.summarizer.error}
            <p class="agent-err">{run.summarizer.error}</p>
          {/if}
            {#if run.summarizer.session_id && openTerminals.has(run.summarizer.session_id)}
              <div class="term">
                {#key run.summarizer.session_id}
                  <Terminal sessionId={run.summarizer.session_id} preferDom />
                {/key}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      {#if run.kind === 'docs' && run.review && run.review.state !== 'skipped'}
        <section class="review-ledger">
          <div class="review-progress">
            <div>
              <span class="review-eyebrow">Independent review</span>
              <strong>{reviewStateLabel(run)}</strong>
            </div>
            <span class="pill st-{run.review.state}">
              {#if run.review.state === 'reviewing' || run.review.state === 'revising'}
                <span class="spinner-xs"></span>
              {/if}
              {run.review.state}
            </span>
          </div>

          {#if run.review.state === 'clean'}
            <div class="review-outcome clean">
              Every reviewer returned a valid clean verdict in round {run.review.current_iteration}.
            </div>
          {:else if run.review.state === 'exhausted'}
            <div class="review-outcome exhausted">
              <strong>Review limit reached.</strong> Findings remain after {run.review.max_iterations}
              iteration{run.review.max_iterations === 1 ? '' : 's'}; the run kept the latest revisions
              and evidence below.
            </div>
          {:else if run.review.outcome}
            <div class="review-outcome">Outcome: {run.review.outcome.replaceAll('_', ' ')}</div>
          {/if}

          <div class="review-rounds">
            {#each run.review.rounds as round (round.iteration)}
              <article class="review-round" class:current={round.iteration === run.review.current_iteration}>
                <header class="review-round-head">
                  <span class="round-number">{round.iteration}</span>
                  <div>
                    <strong>Round {round.iteration}</strong>
                    <span>
                      {round.reviewers.length} reviewer{round.reviewers.length === 1 ? '' : 's'}
                    </span>
                  </div>
                  <span class="grow"></span>
                  <span class="pill st-{round.state}">{round.state}</span>
                </header>

                <div class="reviewer-list">
                  {#each round.reviewers as reviewer (reviewer.index)}
                    <div class="reviewer-card">
                      <div class="agent-top">
                        <span class="agent-name">{reviewer.skill}</span>
                        <span class="chip">
                          {reviewer.provider}{reviewer.model ? ' · ' + reviewer.model : ''}
                        </span>
                        <span class="grow"></span>
                        {#if reviewer.session_id}
                          <button
                            class="ghost small"
                            onclick={() => void toggleTerminal(reviewer.session_id)}
                          >
                            {openTerminals.has(reviewer.session_id) ? 'Hide' : 'Open'}
                          </button>
                        {/if}
                        {#if active &&
                          (reviewer.state === 'pending' ||
                            reviewer.state === 'running' ||
                            reviewer.state === 'error')}
                          <button
                            class="ghost small"
                            disabled={retrying[`reviewer-${round.iteration}-${reviewer.index}`]}
                            onclick={() => void retryReviewer(round.iteration, reviewer.index)}
                          >
                            {retrying[`reviewer-${round.iteration}-${reviewer.index}`]
                              ? 'Retrying…'
                              : 'Retry reviewer'}
                          </button>
                        {/if}
                        <span class="pill st-{reviewer.state}">
                          {#if reviewer.state === 'running'}<span class="spinner-xs"></span>{/if}
                          {reviewer.state}
                        </span>
                      </div>
                      {#if reviewer.focus}
                        <p class="review-focus-label">Focus: {reviewer.focus}</p>
                      {/if}
                      {#if reviewer.error}
                        <p class="agent-err">{reviewer.error}</p>
                      {/if}
                      {#if reviewer.findings.length === 0 && reviewer.state === 'done'}
                        <p class="clean-verdict"><Icon name="check" size={11} /> No findings</p>
                      {:else if reviewer.findings.length > 0}
                        <div class="finding-list">
                          {#each reviewer.findings as finding, findingIndex (`${reviewer.index}-${findingIndex}`)}
                            <div class="finding">
                              <div class="finding-head">
                                <span class="severity sev-{finding.severity}">{finding.severity}</span>
                                <span class="finding-category">{finding.category}</span>
                              </div>
                              <strong>{finding.summary}</strong>
                              <p><span>Missed:</span> {finding.missed_item}</p>
                              <p><span>Required fix:</span> {finding.required_fix}</p>
                              {#if finding.evidence.length > 0}
                                <div class="evidence-list">
                                  {#each finding.evidence as evidence}
                                    <span class="evidence">{evidenceLabel(evidence)}</span>
                                  {/each}
                                </div>
                              {/if}
                            </div>
                          {/each}
                        </div>
                      {/if}
                      {#if reviewer.session_id && openTerminals.has(reviewer.session_id)}
                        <div class="term">
                          {#key reviewer.session_id}
                            <Terminal sessionId={reviewer.session_id} preferDom />
                          {/key}
                        </div>
                      {/if}
                    </div>
                  {/each}
                </div>

                {#if round.revision.state !== 'skipped'}
                  <div class="revision-card">
                    <div class="agent-top">
                      <span class="revision-mark"><Icon name="edit" size={11} /></span>
                      <span class="agent-name">Final author revision</span>
                      <span class="grow"></span>
                      {#if round.revision.session_id}
                        <button
                          class="ghost small"
                          onclick={() => void toggleTerminal(round.revision.session_id)}
                        >
                          {openTerminals.has(round.revision.session_id) ? 'Hide' : 'Open'}
                        </button>
                      {/if}
                      {#if active &&
                        (round.revision.state === 'pending' ||
                          round.revision.state === 'running' ||
                          round.revision.state === 'error')}
                        <button
                          class="ghost small"
                          disabled={retrying[`revision-${round.iteration}`]}
                          onclick={() => void retryRevision(round.iteration)}
                        >
                          {retrying[`revision-${round.iteration}`] ? 'Retrying…' : 'Retry revision'}
                        </button>
                      {/if}
                      <span class="pill st-{round.revision.state}">
                        {#if round.revision.state === 'running'}<span class="spinner-xs"></span>{/if}
                        {round.revision.state}
                      </span>
                    </div>
                    {#if round.revision.error}
                      <p class="agent-err">{round.revision.error}</p>
                    {/if}
                    {#if round.revision.changed_paths.length > 0}
                      <div class="changed-paths">
                        {#each round.revision.changed_paths as path (path)}
                          <span>{path}</span>
                        {/each}
                      </div>
                    {/if}
                    {#if round.revision.session_id && openTerminals.has(round.revision.session_id)}
                      <div class="term">
                        {#key round.revision.session_id}
                          <Terminal sessionId={round.revision.session_id} preferDom />
                        {/key}
                      </div>
                    {/if}
                  </div>
                {/if}
              </article>
            {/each}
          </div>
        </section>
      {/if}

      {#if run.written.length > 0}
        <div class="written">
          <span class="written-title">
            {run.written.length} note{run.written.length === 1 ? '' : 's'} written
          </span>
          {#each run.written as p (p)}
            <button class="written-link" onclick={() => void vault.open(p)}>{p}</button>
          {/each}
        </div>
      {/if}
    {/if}

    <!-- ── runs (current + history, server-persisted) ─────────────────────── -->
    {#if vault.docsRuns.length > 0}
      <div class="runs-section">
        <span class="runs-title">Runs</span>
        {#each vault.docsRuns as r (r.id)}
          <button
            class="run-row"
            class:selected={run?.id === r.id}
            onclick={() => selectRun(r)}
            title={r.prompt}
          >
            <span
              class="pill st-{r.state}"
              title={r.state === 'interrupted' ? 'The app/daemon restarted mid-run' : undefined}
            >
              {#if isActive(r)}<span class="spinner-xs"></span>{/if}
              {displayState(r.state)}
            </span>
            <span class="kind-chip">{r.kind}</span>
            <span class="run-row-text">
              {r.kind === 'refine' ? `${r.note_path} — ${r.prompt}` : r.prompt}
            </span>
            <span class="run-row-when">{new Date(r.started_at).toLocaleString()}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

{#if skillView}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="skill-overlay" onclick={() => (skillView = null)}>
    <div
      class="skill-dialog"
      role="dialog"
      tabindex="-1"
      aria-label="Skill {skillView.name}"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="skill-dialog-head">
        <h3><Icon name="function" size={14} /> {skillView.name}</h3>
        <button class="icon-btn" title="Close" onclick={() => (skillView = null)}>
          <Icon name="x" size={13} />
        </button>
      </div>
      <pre class="skill-body">{skillView.body}</pre>
    </div>
  </div>
{/if}

<style>
  .docs-agents {
    overflow-y: auto;
    min-height: 0;
  }
  .inner {
    max-width: 760px;
    width: 100%;
    margin: 0 auto;
    padding: 18px 26px 60px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font-size: 15px;
    color: var(--text);
  }

  /* ── form ─────────────────────────────────────────────────────────────── */
  .tpl-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .tpl-row select {
    min-width: 0;
    flex: 0 1 auto;
  }
  .tpl-repo {
    flex: 1;
    min-width: 0;
  }
  .tpl-use {
    border: 1px solid var(--accent, #7a9cff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
    color: var(--accent, #9ab4ff);
    border-radius: 7px;
    padding: 6px 12px;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .tpl-use:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .tpl-hint {
    font-size: 11.5px;
    color: var(--text-dim);
    margin-top: 4px;
    line-height: 1.4;
  }
  .tpl-infra {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--text-dim);
    margin-top: 6px;
    cursor: pointer;
    user-select: none;
  }
  .tpl-infra input {
    accent-color: var(--accent, #7a9cff);
    margin: 0;
  }
  .skill-chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .skill-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    color: var(--text);
    background: var(--hover, rgba(127, 127, 127, 0.1));
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 3px 10px;
    cursor: pointer;
  }
  .skill-chip:hover {
    border-color: var(--accent, #7a9cff);
    color: var(--accent, #9ab4ff);
  }
  .skill-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    z-index: 95;
    display: flex;
    justify-content: center;
    align-items: center;
    padding: 5vh 16px;
  }
  .skill-dialog {
    width: min(760px, 94vw);
    max-height: 88vh;
    background: var(--panel, #1c1c1e);
    border: 1px solid var(--border);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .skill-dialog-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .skill-dialog-head h3 {
    margin: 0;
    font-size: 13.5px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .skill-body {
    margin: 0;
    padding: 14px 16px;
    overflow: auto;
    font-size: 12px;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .fld textarea,
  .fld input,
  .fld select,
  .agent-row select,
  .agent-row input {
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 13px;
    padding: 8px 10px;
    font-family: inherit;
  }
  .fld textarea {
    resize: vertical;
    min-height: 72px;
    line-height: 1.5;
  }
  .agent-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .agent-row select {
    flex: 0 0 140px;
  }
  .agent-row .model {
    flex: 1;
    min-width: 0;
  }
  .sum-select {
    max-width: 200px;
  }
  .review-config {
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--panel, #1c1c1e);
    overflow: hidden;
  }
  .review-config.enabled {
    border-color: color-mix(in srgb, var(--accent, #7a9cff) 45%, var(--border));
  }
  .review-config-head {
    min-height: 48px;
    padding: 9px 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .review-config-head > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .review-config-head strong {
    color: var(--text);
    font-size: 12.5px;
  }
  .review-config-head span {
    color: var(--text-dim);
    font-size: 11.5px;
    line-height: 1.35;
  }
  .switch {
    position: relative;
    flex: none;
    width: 34px;
    height: 20px;
  }
  .switch input {
    position: absolute;
    inset: 0;
    z-index: 2;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: pointer;
  }
  .switch span {
    position: absolute;
    inset: 0;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 24%, transparent);
    border: 1px solid var(--border);
    cursor: pointer;
    transition: background 0.15s ease;
  }
  .switch span::after {
    content: '';
    position: absolute;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    left: 2px;
    top: 2px;
    background: var(--text-dim);
    transition: transform 0.15s ease, background 0.15s ease;
  }
  .switch input:focus-visible + span {
    outline: 2px solid var(--accent, #7a9cff);
    outline-offset: 2px;
  }
  .switch input:checked + span {
    background: var(--accent, #4c6fff);
    border-color: transparent;
  }
  .switch input:checked + span::after {
    transform: translateX(14px);
    background: white;
  }
  .review-settings {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px 12px;
    border-top: 1px solid var(--border);
  }
  .review-setting-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .iteration-field {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .iteration-field input {
    width: 54px;
    padding: 5px 7px;
  }
  .reviewer-config-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .reviewer-number,
  .round-number {
    width: 22px;
    height: 22px;
    flex: none;
    display: inline-grid;
    place-items: center;
    border-radius: 50%;
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
    color: var(--accent, #9ab4ff);
    font: 700 10.5px var(--font-mono, monospace);
  }
  .reviewer-fields {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .reviewer-main-fields {
    display: grid;
    grid-template-columns: minmax(90px, 0.8fr) minmax(100px, 0.9fr) minmax(180px, 1.6fr) auto;
    gap: 5px;
  }
  .reviewer-main-fields select,
  .reviewer-main-fields input,
  .review-focus,
  .iteration-field input {
    min-width: 0;
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-family: inherit;
    font-size: 12px;
    padding: 7px 8px;
  }
  .review-focus {
    width: 100%;
    box-sizing: border-box;
  }
  .review-hint {
    margin: 0 0 0 30px;
    color: var(--text-dim);
    font-size: 11px;
    line-height: 1.4;
  }
  .icon-btn {
    display: inline-flex;
    background: none;
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text-dim);
    padding: 7px 8px;
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    color: #e88;
    border-color: #a33;
  }
  .icon-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .add-agent {
    align-self: flex-start;
    background: none;
    border: 1px dashed var(--border);
    border-radius: 7px;
    color: var(--text-dim);
    font-size: 12px;
    padding: 5px 12px;
    cursor: pointer;
  }
  .add-agent:hover {
    color: var(--accent, #9ab4ff);
    border-color: var(--accent, #7a9cff);
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
  }
  .primary {
    background: var(--accent, #4c6fff);
    border: none;
    color: #fff;
    border-radius: 8px;
    padding: 8px 18px;
    font-size: 13px;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* ── run view ─────────────────────────────────────────────────────────── */
  .run-head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .run-meta {
    font-size: 12px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .ghost {
    border: 1px solid var(--border);
    background: var(--panel-2, #222);
    color: var(--text);
    border-radius: 7px;
    padding: 5px 12px;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .ghost.small {
    padding: 3px 10px;
    font-size: 11.5px;
  }
  .ghost:hover:not(:disabled) {
    border-color: var(--accent, #7a9cff);
  }
  .ghost:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .agent-card {
    background: var(--panel, #1c1c1e);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 12px;
  }
  .agent-top {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .agent-name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .chip {
    font-size: 10.5px;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 8px;
  }
  .agent-err {
    margin: 6px 0 0;
    font-size: 11.5px;
    color: #e88;
    line-height: 1.4;
    word-break: break-word;
  }
  .drafts {
    margin: 5px 0 0;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.4;
    word-break: break-word;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
  .term {
    height: min(360px, 60vh);
    margin: 8px 0 2px;
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    overscroll-behavior: contain;
  }

  /* Review iterations form a compact audit ledger: reviewers first, then the
     author repair, so the quality loop is readable without opening terminals. */
  .review-ledger {
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    background: var(--panel, #1c1c1e);
  }
  .review-progress {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 11px 13px;
    border-bottom: 1px solid var(--border);
  }
  .review-progress > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .review-progress strong {
    font-size: 13px;
  }
  .review-eyebrow {
    color: var(--text-dim);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .review-outcome {
    margin: 10px 12px 0;
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 7px 9px;
    color: var(--text-dim);
    font-size: 11.5px;
    line-height: 1.45;
  }
  .review-outcome.clean {
    color: #7fc97f;
    border-color: rgba(127, 201, 127, 0.35);
    background: rgba(127, 201, 127, 0.06);
  }
  .review-outcome.exhausted {
    color: #d7a953;
    border-color: rgba(215, 169, 83, 0.35);
    background: rgba(215, 169, 83, 0.06);
  }
  .review-rounds {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px 12px;
  }
  .review-round {
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    background: var(--panel-2, #222);
  }
  .review-round.current {
    border-color: color-mix(in srgb, var(--accent, #7a9cff) 45%, var(--border));
  }
  .review-round-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
  }
  .review-round-head > div {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .review-round-head strong {
    font-size: 12px;
  }
  .review-round-head div span {
    color: var(--text-dim);
    font-size: 10.5px;
  }
  .reviewer-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 8px 8px;
  }
  .reviewer-card,
  .revision-card {
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--panel, #1c1c1e);
    padding: 8px 9px;
  }
  .revision-card {
    margin: 0 8px 8px;
    border-style: dashed;
  }
  .revision-mark {
    width: 20px;
    height: 20px;
    border-radius: 5px;
    display: inline-grid;
    place-items: center;
    color: var(--accent, #9ab4ff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
  }
  .review-focus-label,
  .clean-verdict {
    margin: 6px 0 0;
    color: var(--text-dim);
    font-size: 11px;
    line-height: 1.4;
  }
  .clean-verdict {
    color: #7fc97f;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .finding-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 7px;
  }
  .finding {
    border-left: 2px solid var(--border);
    padding: 3px 0 3px 8px;
  }
  .finding-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 3px;
  }
  .finding > strong {
    font-size: 11.5px;
    line-height: 1.4;
  }
  .finding p {
    margin: 3px 0 0;
    color: var(--text-dim);
    font-size: 10.75px;
    line-height: 1.4;
  }
  .finding p span {
    color: var(--text);
    font-weight: 600;
  }
  .severity {
    border-radius: 4px;
    padding: 1px 5px;
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
  }
  .sev-blocking,
  .sev-major {
    color: #e88;
    background: rgba(214, 86, 72, 0.12);
  }
  .sev-minor {
    color: #d7a953;
    background: rgba(215, 169, 83, 0.12);
  }
  .finding-category {
    color: var(--text-dim);
    font: 10px var(--font-mono, monospace);
  }
  .evidence-list,
  .changed-paths {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 6px;
  }
  .evidence,
  .changed-paths span {
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 2px 6px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
    font: 10px var(--font-mono, monospace);
    overflow-wrap: anywhere;
  }

  /* status pills */
  .pill {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 7px;
    border-radius: 5px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .st-pending,
  .st-skipped,
  .st-cancelled,
  .st-interrupted {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  .st-interrupted {
    border: 1px dashed color-mix(in srgb, var(--text-dim) 45%, transparent);
  }
  .st-running,
  .st-summarizing,
  .st-reviewing,
  .st-revising {
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
    color: var(--accent, #9ab4ff);
  }
  .st-done {
    background: rgba(127, 201, 127, 0.14);
    color: #7fc97f;
  }
  .st-done_with_findings,
  .st-exhausted {
    background: rgba(215, 169, 83, 0.12);
    color: #d7a953;
  }
  .st-clean,
  .st-revised {
    background: rgba(127, 201, 127, 0.14);
    color: #7fc97f;
  }
  .st-error {
    background: rgba(214, 86, 72, 0.12);
    color: #e88;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .err {
    color: #e88;
    font-size: 12px;
    border: 1px solid rgba(214, 86, 72, 0.4);
    background: rgba(214, 86, 72, 0.08);
    border-radius: 7px;
    padding: 6px 10px;
    word-break: break-word;
  }

  .written {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 12px;
  }
  .written-title {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .written-link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 1px 0;
    font-size: 12.5px;
    color: var(--accent, #7a9cff);
    cursor: pointer;
    text-align: start;
    word-break: break-all;
  }
  .written-link:hover {
    text-decoration: underline;
  }

  /* ── runs list (current + history) ────────────────────────────────────── */
  .runs-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-top: 1px solid var(--border);
    padding-top: 12px;
    margin-top: 4px;
  }
  .runs-title {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 2px;
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: 8px;
    background: none;
    border: 1px solid transparent;
    border-radius: 7px;
    padding: 5px 8px;
    cursor: pointer;
    text-align: start;
    min-width: 0;
    color: var(--text);
    font-size: 12px;
  }
  .run-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .run-row.selected {
    border-color: var(--accent, #7a9cff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.08));
  }
  .run-row-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .run-row-when {
    flex: none;
    font-size: 10.5px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .kind-chip {
    flex: none;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 7px;
  }
  .note-link {
    background: none;
    border: none;
    padding: 0;
    font-size: inherit;
    color: var(--accent, #7a9cff);
    cursor: pointer;
  }
  .note-link:hover {
    text-decoration: underline;
  }

  @media (max-width: 680px) {
    .inner {
      padding-inline: 14px;
    }
    .review-setting-title {
      align-items: flex-start;
      flex-direction: column;
    }
    .reviewer-main-fields {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto;
    }
    .review-method {
      grid-column: 1 / 3;
    }
    .run-row-when {
      display: none;
    }
  }
</style>
