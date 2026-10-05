<script lang="ts">
  import { plural } from '../../lib/plural';
  import PathField from '../../lib/components/PathField.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  // Docs agents — fan 1-4 writer agents out over a prompt to author notes into
  // the vault (a summarizer consolidates drafts when >1 writer), plus the
  // vault's RUN HISTORY (docs runs + per-note refine turns, server-persisted
  // in `vault_docs_runs`). Center-stage view: a compact form, the selected
  // run's per-agent rows with live status + inline terminals, and the runs
  // list below. The selected run lives on the vault store; the LIST is
  // refetched from the server on every mount — that is what makes runs
  // reappear after a tab/module switch or a full app restart. This view owns
  // an event-fed refresh (1.5 s poll while the event socket is down) and stops
  // it once nothing is active.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
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
    deleteDocsRun,
    resolveDocsRun,
    retryDocsAgent,
    retryDocsReviewer,
    retryDocsRevision,
    retryDocsSummarizer,
    runDocsAgents,
  } from '../../lib/api/vault';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { kindLabel, runStateLabel, severityLabel } from '../../lib/labels';
  import Badge from '../../lib/components/Badge.svelte';
  import type { BadgeTone } from '../../lib/status';
  import { DOCS_TEMPLATES } from './docsTemplates';
  import { vault } from './vault.svelte';
  import type { Poller } from '../../lib/poll';
  import { liveQuery } from '../../lib/live';

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
      toastError('Couldn’t open the skill', e);
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

  // One-shot prompt/skills prefill ("Review + fix docs", "Send to agent…",
  // group actions, "Send to agent to fix" on a findings run). Consuming it
  // also deselects any run so the form is what the user lands on. `autorun`
  // (group-bar actions) launches immediately — the bar's button IS the send.
  $effect(() => {
    const p = vault.docsAgentsPrefill;
    if (!p) return;
    vault.docsAgentsPrefill = null;
    prompt = p.prompt;
    tplSkills = p.skills;
    targetDir = vault.docsAgentsDir;
    if (p.agents?.length) {
      agents = p.agents.map((a) => ({ provider: a.provider, model: a.model ?? '' }));
    }
    vault.docsRun = null;
    if (p.autorun) void start();
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
    if (r.review.state === 'cancelled') return 'Review canceled';
    if (r.review.state === 'interrupted') return 'Review interrupted';
    if (r.review.state === 'pending') return 'Review queued';
    return `Review round ${Math.max(r.review.current_iteration, 1)} of ${r.review.max_iterations}`;
  }

  /** `needs_review` → “Needs review”: raw enum values never reach the screen. */
  const displayState = (state: string): string => {
    const t = state.replaceAll('_', ' ');
    return t.charAt(0).toUpperCase() + t.slice(1);
  };

  function reviewIterationLimit(value: number): number {
    return Number.isFinite(value) ? Math.min(10, Math.max(1, Math.round(value))) : 3;
  }

  // -- run + polling ---------------------------------------------------------------
  // lib/poll: one request in flight at a time, paused while hidden.
  let poller: Poller | null = null;

  function stopPoll(): void {
    poller?.stop();
    poller = null;
  }

  function startPoll(): void {
    stopPoll();
    // A docs run advances as its agent sessions do: re-read on their status
    // events (coalesced), with a 10 s safety net for run-level transitions no
    // event names; the 1.5 s cadence only while the event socket is down.
    poller = liveQuery({
      run: () => poll(),
      on: ['session_status', 'session_created', 'session_removed'],
      fallbackMs: 1500,
      safetyMs: 10_000,
      debounceMs: 1000,
      maxWaitMs: 5000,
      minIntervalMs: 3000,
      immediate: false,
    });
  }

  // Status refresh failures were swallowed forever: the run then looked frozen.
  // One miss is transient (the next tick retries); three in a row are shown
  // inline until a refresh succeeds again.
  let pollFailures = 0;
  let pollError = $state('');

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
      pollFailures = 0;
      pollError = '';
    } catch (e) {
      // Transient misses retry on the next tick; a streak is surfaced.
      if (++pollFailures >= 3) pollError = loadErrorText(e);
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
      toastError('Couldn’t start the docs agent', e);
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
      toastError('Couldn’t retry the run', e);
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
      toastError('Couldn’t retry the reviewer', e);
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
      toastError('Couldn’t retry the revision', e);
    } finally {
      retrying = { ...retrying, [key]: false };
    }
  }

  let cancelling = $state(false);
  async function cancel(): Promise<void> {
    const r = vault.docsRun;
    if (!r || cancelling) return;
    // Stopping a multi-agent run throws away its in-progress work — ask first.
    const ok = await confirmer.ask('Stop this documentation run? Agents still working stop now and their unfinished work is discarded.', {
      title: 'Stop the run?', confirmLabel: 'Stop run', cancelLabel: 'Keep running', danger: true,
    });
    if (!ok || vault.docsRun?.id !== r.id) return;
    cancelling = true;
    try {
      await cancelDocsRun(r.id);
      await poll();
    } catch (e) {
      toastError('Couldn’t cancel the run', e);
    } finally {
      cancelling = false;
    }
  }

  /** Back to the form (keeps the prompt so a tweak-and-rerun is one edit). */
  function newRun(): void {
    vault.docsRun = null;
    openTerminals = new Set();
  }

  // -- done_with_findings dispositions ------------------------------------------
  // "Send to agent to fix" turns the run's outstanding findings into a
  // prefilled fix prompt; "Mark OK"/"Mark fixed" resolve the run durably.
  let resolving = $state(false);

  async function resolveRun(outcome: 'ok' | 'fixed'): Promise<void> {
    const r = vault.docsRun;
    if (!r || resolving) return;
    resolving = true;
    try {
      vault.docsRun = await resolveDocsRun(r.id, outcome);
      void vault.refreshDocsRuns();
      toasts.success(
        'Review resolved',
        outcome === 'ok' ? 'Findings accepted as-is' : 'Findings marked as fixed',
      );
    } catch (e) {
      toastError('Couldn’t resolve the review', e);
    } finally {
      resolving = false;
    }
  }

  // History cleanup — terminal runs only (the server 409s on active ones).
  let deleting = $state<string | null>(null);

  async function deleteRun(r: VaultDocsRun): Promise<void> {
    if (deleting) return;
    const ok = await confirmer.ask('Delete this run from the history? Its log and findings go with it; documents it already wrote are kept.', {
      title: 'Delete the run?', confirmLabel: 'Delete', danger: true,
    });
    if (!ok || deleting) return;
    deleting = r.id;
    try {
      await deleteDocsRun(r.id);
      if (vault.docsRun?.id === r.id) vault.docsRun = null;
      vault.docsRuns = vault.docsRuns.filter((x) => x.id !== r.id);
    } catch (e) {
      toastError('Couldn’t delete the run', e);
    } finally {
      deleting = null;
    }
  }

  function fixWithAgent(): void {
    const r = vault.docsRun;
    if (!r) return;
    const round = r.review.rounds.at(-1);
    const findings = (round?.reviewers ?? []).flatMap((rv) => rv.findings);
    if (findings.length === 0) {
      toasts.error('Couldn’t fix the findings', 'This run has no recorded findings');
      return;
    }
    const lines = findings.map((f, i) => {
      const ev = f.evidence
        .map((e) =>
          [
            e.repo_path ? `${e.repo_path}${e.line != null ? ':' + e.line : ''}` : null,
            e.doc_path ? `${e.doc_path}${e.section ? ' §' + e.section : ''}` : null,
          ]
            .filter(Boolean)
            .join(' → '),
        )
        .filter(Boolean)
        .join('; ');
      return (
        `${i + 1}. [${f.severity}/${f.category}] ${f.summary}\n` +
        `   missed: ${f.missed_item}\n` +
        `   fix required: ${f.required_fix}` +
        (ev ? `\n   evidence: ${ev}` : '')
      );
    });
    const scope = r.target_dir ? ` (bundle \`${r.target_dir}/\`)` : '';
    vault.docsAgentsPrefill = {
      prompt:
        `FIX the documentation in this vault: a review pass finished with UNRESOLVED ` +
        `findings${scope}. Address EVERY finding below by editing the affected notes in ` +
        `place — verify each fix against the CURRENT source code, use REAL examples ` +
        `(actual field names from the code), and do not rewrite unaffected content.\n\n` +
        `FINDINGS:\n${lines.join('\n')}\n\n` +
        `Finish with a one-line summary listing the notes you changed.`,
      skills: ['vault-repo-docs'],
    };
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
      toasts.error('Couldn’t open the agent session', 'This agent session no longer exists (cleaned up by retention).');
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

  /** Run / agent / review state → the shared Badge tone. */
  function stTone(state: string): BadgeTone {
    if (state === 'running' || state === 'summarizing' || state === 'reviewing' || state === 'revising') return 'accent';
    if (state === 'done' || state === 'clean' || state === 'revised') return 'ok';
    if (state === 'done_with_findings' || state === 'exhausted') return 'warn';
    if (state === 'error') return 'bad';
    return 'neutral';
  }
</script>

<div class="docs-agents">
  <div class="inner">
    <h2><Icon name="zap" size={14} /> Docs agent</h2>
    {#if pollError}
      <div class="poll-err" role="alert">
        <Icon name="warning" size={13} />
        <span>Couldn’t refresh the docs agent’s status — it may still be running. {pollError}</span>
        <button class="btn small" onclick={() => void poll()}>Retry</button> <!-- ui-guards: allow — re-polls a live agent; the view keeps its data -->
      </div>
    {/if}

    {#if !run}
      <!-- ── form ─────────────────────────────────────────────────────────── -->
      <div class="field">
        <label for="da-template">Prepared prompt (optional)</label>
        <div class="tpl-row">
          <select id="da-template" class="input" bind:value={tplId}>
            <option value="">— pick a template —</option>
            {#each DOCS_TEMPLATES as t (t.id)}
              <option value={t.id}>{t.label}</option>
            {/each}
          </select>
          {#if tpl?.needsRepo}
            <PathField bind:value={tplRepo}><input dir="ltr" class="input tpl-repo" bind:value={tplRepo} placeholder="e.g. ~/code/payments-service" aria-label="Repository folder" /></PathField>
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

      <div class="field">
        <label for="da-prompt">What should be documented?</label>
        <textarea dir="auto"
          id="da-prompt"
          class="input da-prompt"
          bind:value={prompt}
          rows="4"
          placeholder="e.g. Document the deploy pipeline: triggers, stages, rollback, and the runbook for a failed release."
        ></textarea>
      </div>
      <div class="field">
        <label for="da-target">Target folder (vault-relative, blank = root)</label>
        <input dir="ltr" id="da-target" class="input" bind:value={targetDir} placeholder="e.g. runbooks/deploys" />
      </div>

      <div class="field" role="group" aria-labelledby="da-writers">
        <span class="field-caption" id="da-writers">Writer agents ({agents.length}/4)</span>
        {#each agents as agent, i (i)}
          <div class="agent-row">
            <select class="input" bind:value={agent.provider} aria-label={`Writer agent ${i + 1} provider`}>
              {#each providers as p (p)}
                <option value={p}>{p}</option>
              {/each}
            </select>
            <input dir="ltr" class="input model" bind:value={agent.model} placeholder="Model (optional)" aria-label={`Writer agent ${i + 1} model`} />
            <button
              class="icon-btn"
              title="Remove agent" aria-label="Remove agent"
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
        <div class="field">
          <label for="da-summarizer">Summarizer (consolidates the {agents.length} drafts into final notes)</label>
          <select id="da-summarizer" class="input sum-select" bind:value={sumProvider}>
            {#each providers as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </div>
      {/if}

      <section class="review-config" class:enabled={reviewEnabled}>
        <div class="review-config-head">
          <div>
            <strong>Review outcomes</strong>
            <span>Independent agents check the final bundle before the run finishes.</span>
          </div>
          <Switch checked={reviewEnabled} onchange={(next) => (reviewEnabled = next)} label="Review outcomes" />
        </div>

        {#if reviewEnabled}
          <div class="review-settings">
            <div class="review-setting-title">
              <span>Reviewer agents ({reviewers.length}/4)</span>
              <label class="iteration-field">
                Maximum review iterations
                <input
                  class="input iteration-input"
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
                    <select class="input" bind:value={reviewer.provider} aria-label="Reviewer provider">
                      {#each providers as p (p)}
                        <option value={p}>{p}</option>
                      {/each}
                    </select>
                    <input dir="ltr"
                      class="input"
                      bind:value={reviewer.model}
                      placeholder="Model (optional)"
                      aria-label="Reviewer model"
                    />
                    <select class="input review-method" bind:value={reviewer.skill} aria-label="Review method">
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
                  <input dir="auto"
                    class="input review-focus"
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
        <div class="field" role="group" aria-labelledby="da-skills">
          <span class="field-caption" id="da-skills">Skills injected into this run — click to view</span>
          <div class="skill-chips">
            {#each runSkills as s (s)}
              <button class="skill-chip" title="Open {s}" onclick={() => void viewSkill(s)}>
                <Icon name="function" size={12} />
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
        <Badge tone={stTone(run.state)}
          title={run.state === 'interrupted' ? 'The app/daemon restarted mid-run' : undefined}>
          {#if active}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>{/if}
          {displayState(run.state)}
        </Badge>
        <span class="kind-chip">{kindLabel(run.kind)}</span>
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
            {cancelling ? 'Canceling…' : 'Cancel'}
          </button>
        {/if}
        {#if run.state === 'done_with_findings'}
          <button
            class="ghost"
            title="Prefill a fix run from this run’s outstanding findings"
            onclick={fixWithAgent}>Send to agent to fix</button
          >
          <button
            class="ghost"
            disabled={resolving}
            title="The findings were addressed — resolve this run as done"
            onclick={() => void resolveRun('fixed')}>Mark fixed</button
          >
          <button
            class="ghost"
            disabled={resolving}
            title="Accept the findings as-is — resolve this run as done"
            onclick={() => void resolveRun('ok')}>Mark OK</button
          >
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
              <Badge variant="outline">{agent.provider}{agent.model ? ' · ' + agent.model : ''}</Badge>
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
                    ? "Re-spawn this writer’s turn in a fresh session"
                    : "Kill this writer’s session and restart its turn fresh"}
                  disabled={retrying[String(agent.index)]}
                  onclick={() => void retry(agent.index)}
                >
                  {retrying[String(agent.index)] ? 'Retrying…' : 'Retry'}
                </button>
              {/if}
              <Badge tone={stTone(agent.state)}>
                {#if agent.state === 'running'}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>{/if}
                {runStateLabel(agent.state)}
              </Badge>
            </div>
            {#if agent.error}
              <p class="agent-err">{agent.error}</p>
            {/if}
            {#if agent.drafts.length > 0 && active}
              <p class="drafts">
                {plural(agent.drafts.length, 'draft')}:
                <span class="mono">{agent.drafts.join(' · ')}</span>
              </p>
            {/if}
            {#if agent.session_id && openTerminals.has(agent.session_id)}
              <div class="term">
                {#key agent.session_id}
                  <LazyTerminal sessionId={agent.session_id} preferDom resumeOnOpen={false} />
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
            <Badge variant="outline">
              {run.summarizer.provider}{run.summarizer.model ? ' · ' + run.summarizer.model : ''}
            </Badge>
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
                title="Kill the summarizer’s session and restart the consolidation fresh"
                disabled={retrying['sum']}
                onclick={() => void retry('sum')}
              >
                {retrying['sum'] ? 'Retrying…' : 'Retry'}
              </button>
            {/if}
            <Badge tone={stTone(run.summarizer.state)}>
              {#if run.summarizer.state === 'running'}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>{/if}
              {runStateLabel(run.summarizer.state)}
            </Badge>
          </div>
          {#if run.summarizer.error}
            <p class="agent-err">{run.summarizer.error}</p>
          {/if}
            {#if run.summarizer.session_id && openTerminals.has(run.summarizer.session_id)}
              <div class="term">
                {#key run.summarizer.session_id}
                  <LazyTerminal sessionId={run.summarizer.session_id} preferDom resumeOnOpen={false} />
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
            <Badge tone={stTone(run.review.state)}>
              {#if run.review.state === 'reviewing' || run.review.state === 'revising'}
                <span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>
              {/if}
              {runStateLabel(run.review.state)}
            </Badge>
          </div>

          {#if run.review.state === 'clean'}
            <div class="review-outcome clean">
              Every reviewer returned a valid clean verdict in round {run.review.current_iteration}.
            </div>
          {:else if run.review.state === 'exhausted'}
            <div class="review-outcome exhausted">
              <strong>Review limit reached.</strong> Findings remain after {plural(run.review.max_iterations, 'iteration')}; the run kept the latest revisions
              and evidence below.
            </div>
          {:else if run.review.outcome}
            <div class="review-outcome">Outcome: {displayState(run.review.outcome)}</div>
          {/if}

          <div class="review-rounds">
            {#each run.review.rounds as round (round.iteration)}
              <article class="review-round" class:current={round.iteration === run.review.current_iteration}>
                <header class="review-round-head">
                  <span class="round-number">{round.iteration}</span>
                  <div>
                    <strong>Round {round.iteration}</strong>
                    <span>
                      {plural(round.reviewers.length, 'reviewer')}
                    </span>
                  </div>
                  <span class="grow"></span>
                  <Badge tone={stTone(round.state)}>{runStateLabel(round.state)}</Badge>
                </header>

                <div class="reviewer-list">
                  {#each round.reviewers as reviewer (reviewer.index)}
                    <div class="reviewer-card">
                      <div class="agent-top">
                        <span class="agent-name">{reviewer.skill}</span>
                        <Badge variant="outline">
                          {reviewer.provider}{reviewer.model ? ' · ' + reviewer.model : ''}
                        </Badge>
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
                        <Badge tone={stTone(reviewer.state)}>
                          {#if reviewer.state === 'running'}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>{/if}
                          {runStateLabel(reviewer.state)}
                        </Badge>
                      </div>
                      {#if reviewer.focus}
                        <p class="review-focus-label">Focus: {reviewer.focus}</p>
                      {/if}
                      {#if reviewer.error}
                        <p class="agent-err">{reviewer.error}</p>
                      {/if}
                      {#if reviewer.findings.length === 0 && reviewer.state === 'done'}
                        <p class="clean-verdict"><Icon name="check" size={12} /> No findings</p>
                      {:else if reviewer.findings.length > 0}
                        <div class="finding-list">
                          {#each reviewer.findings as finding, findingIndex (`${reviewer.index}-${findingIndex}`)}
                            <div class="finding">
                              <div class="finding-head">
                                <span class="severity sev-{finding.severity}">{severityLabel(finding.severity)}</span>
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
                            <LazyTerminal sessionId={reviewer.session_id} preferDom resumeOnOpen={false} />
                          {/key}
                        </div>
                      {/if}
                    </div>
                  {/each}
                </div>

                {#if round.revision.state !== 'skipped'}
                  <div class="revision-card">
                    <div class="agent-top">
                      <span class="revision-mark"><Icon name="edit" size={12} /></span>
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
                      <Badge tone={stTone(round.revision.state)}>
                        {#if round.revision.state === 'running'}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>{/if}
                        {runStateLabel(round.revision.state)}
                      </Badge>
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
                          <LazyTerminal sessionId={round.revision.session_id} preferDom resumeOnOpen={false} />
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
            {plural(run.written.length, 'note')} written
          </span>
          {#each run.written as p (p)}
            <button class="written-link" onclick={() => void vault.open(p)}>{p}</button>
            <button class="written-link" title={`Review edits to ${p} during this run`} onclick={() => void vault.openHistory(p, run.started_at)}>Review edits</button>
          {/each}
        </div>
      {/if}
    {/if}

    <!-- ── runs (current + history, server-persisted) ─────────────────────── -->
    {#if vault.docsRuns.length > 0}
      <div class="runs-section">
        <span class="runs-title">Runs</span>
        {#each vault.docsRuns as r (r.id)}
          <div class="run-row-wrap">
            <button
              class="run-row"
              class:selected={run?.id === r.id}
              onclick={() => selectRun(r)}
              title={r.prompt}
            >
              <Badge tone={stTone(r.state)}
                title={r.state === 'interrupted' ? 'The app/daemon restarted mid-run' : undefined}>
                {#if isActive(r)}<span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>{/if}
                {displayState(r.state)}
              </Badge>
              <span class="kind-chip">{kindLabel(r.kind)}</span>
              <span class="run-row-text">
                {r.kind === 'refine' ? `${r.note_path} — ${r.prompt}` : r.prompt}
              </span>
              <span class="run-row-when">{new Date(r.started_at).toLocaleString()}</span>
            </button>
            {#if !isActive(r)}
              <button
                class="run-del reveal-on-hover"
                title="Delete this run from history" aria-label="Delete this run from history"
                disabled={deleting === r.id}
                onclick={() => void deleteRun(r)}
              >
                <Icon name="trash" size={12} />
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

{#if skillView}
  <Modal title="Skill {skillView.name}" width={760} onclose={() => (skillView = null)}>
    <pre class="skill-body">{skillView.body}</pre>
  </Modal>
{/if}

<style>
  .poll-err {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 8px 10px;
    margin-block-end: 10px;
    border: 1px solid var(--danger);
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    font-size: var(--fs-s);
  }
  .poll-err > span { flex: 1 1 220px; min-width: 0; }
  .docs-agents {
    overflow-y: auto;
    min-height: 0;
  }
  .inner {
    max-width: 760px;
    width: 100%;
    margin: 0 auto;
    padding: 18px 24px 60px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font-size: var(--fs-l);
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
    border: 1px solid var(--accent);
    background: var(--accent-soft);
    color: var(--accent-text);
    border-radius: var(--radius-s);
    padding: 6px 12px;
    cursor: pointer;
    font-size: var(--fs-s);
    white-space: nowrap;
  }
  .tpl-use:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .tpl-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-top: 4px;
    line-height: 1.4;
  }
  .tpl-infra {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-top: 6px;
    cursor: pointer;
    user-select: none;
  }
  .tpl-infra input {
    accent-color: var(--accent);
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
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text);
    background: var(--hover);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 10px;
    cursor: pointer;
  }
  .skill-chip:hover {
    border-color: var(--accent);
    color: var(--accent-text);
  }
  .skill-body {
    margin: 0;
    overflow: auto;
    font-size: var(--fs-s);
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
  }
  /* Shared .field/.input; .field's own bottom margin is dropped — .inner's
     gap spaces the form. Group captions (a span over several controls) read
     like the label of a single-control field. */
  .docs-agents .field {
    margin-bottom: 0;
  }
  .field-caption {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .da-prompt {
    min-height: 72px;
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
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden;
  }
  .review-config.enabled {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .review-config-head {
    min-height: 48px;
    padding: 8px 12px;
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
    font-size: var(--fs-s);
  }
  .review-config-head span {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 1.35;
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
    font-size: var(--fs-xs);
  }
  .iteration-field {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .iteration-input {
    width: 56px;
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
    background: var(--accent-soft);
    color: var(--accent-text);
    font: 600 var(--fs-xs) var(--font-mono);
  }
  .reviewer-fields {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .reviewer-main-fields {
    display: grid;
    grid-template-columns: minmax(90px, 0.8fr) minmax(100px, 0.9fr) minmax(180px, 1.6fr) auto;
    gap: 4px;
  }
  .reviewer-main-fields > select,
  .reviewer-main-fields > input,
  .iteration-input {
    min-width: 0;
  }
  .review-focus {
    width: 100%;
    box-sizing: border-box;
  }
  .review-hint {
    margin-block: 0; margin-inline: 28px 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 1.4;
  }
  .add-agent {
    align-self: flex-start;
    background: none;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font-size: var(--fs-s);
    padding: 4px 12px;
    cursor: pointer;
  }
  .add-agent:hover {
    color: var(--accent-text);
    border-color: var(--accent);
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
  }
  .primary {
    background: var(--accent-solid);
    border: none;
    color: var(--accent-contrast);
    border-radius: var(--radius-m);
    padding: 8px 18px;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .primary:disabled {
    opacity: var(--disabled-opacity);
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
    font-size: var(--fs-s);
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
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 4px 12px;
    cursor: pointer;
    font-size: var(--fs-s);
    white-space: nowrap;
  }
  .ghost.small {
    padding: 2px 10px;
    font-size: var(--fs-xs);
  }
  .ghost:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .ghost:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .agent-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 12px;
  }
  .agent-top {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .agent-name {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .agent-err {
    margin: 6px 0 0;
    font-size: var(--fs-xs);
    color: var(--danger);
    line-height: 1.4;
    word-break: break-word;
  }
  .drafts {
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.4;
    word-break: break-word;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .term {
    height: min(360px, 60vh);
    margin: 8px 0 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    overscroll-behavior: contain;
  }

  /* Review iterations form a compact audit ledger: reviewers first, then the
     author repair, so the quality loop is readable without opening terminals. */
  .review-ledger {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface);
  }
  .review-progress {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .review-progress > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .review-progress strong {
    font-size: var(--fs-m);
  }
  .review-eyebrow {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    text-transform: uppercase;
  }
  .review-outcome {
    margin: 10px 12px 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .review-outcome.clean {
    color: var(--success);
    border-color: color-mix(in srgb, var(--success) 35%, transparent);
    background: color-mix(in srgb, var(--success) 6%, transparent);
  }
  .review-outcome.exhausted {
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: color-mix(in srgb, var(--warning) 6%, transparent);
  }
  .review-rounds {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px 12px;
  }
  .review-round {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface-2);
  }
  .review-round.current {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
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
    font-size: var(--fs-s);
  }
  .review-round-head div span {
    color: var(--text-dim);
    font-size: var(--fs-xs);
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
    border-radius: var(--radius-s);
    background: var(--surface);
    padding: 8px 8px;
  }
  .revision-card {
    margin: 0 8px 8px;
    border-style: dashed;
  }
  .revision-mark {
    width: 20px;
    height: 20px;
    border-radius: var(--radius-s);
    display: inline-grid;
    place-items: center;
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .review-focus-label,
  .clean-verdict {
    margin: 6px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 1.4;
  }
  .clean-verdict {
    color: var(--success);
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .finding-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 6px;
  }
  .finding {
    border-inline-start: 2px solid var(--border);
    padding-block: 2px; padding-inline: 8px 0;
  }
  .finding-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
  }
  .finding > strong {
    font-size: var(--fs-xs);
    line-height: 1.4;
  }
  .finding p {
    margin: 2px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 1.4;
  }
  .finding p span {
    color: var(--text);
    font-weight: 600;
  }
  .severity {
    border-radius: var(--radius-s);
    padding: 1px 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
  }
  .sev-blocking,
  .sev-major {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .sev-minor {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .finding-category {
    color: var(--text-dim);
    font: var(--fs-xs) var(--font-mono);
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
    border-radius: var(--radius-s);
    padding: 2px 6px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
    font: var(--fs-xs) var(--font-mono);
    overflow-wrap: anywhere;
  }


  .err {
    color: var(--danger);
    font-size: var(--fs-s);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    word-break: break-word;
  }

  .written {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 10px 12px;
  }
  .written-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  .written-link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 1px 0;
    font-size: var(--fs-s);
    color: var(--accent-text);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: .06em;
    margin-bottom: 2px;
  }
  .run-row-wrap {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
  }
  .run-row-wrap .run-del {
    display: inline-flex;
    align-items: center;
    background: none;
    border: none;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    padding: 4px;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--dur-fast);
    flex-shrink: 0;
  }
  /* Revealed on row hover / keyboard focus (opacity, not visibility, so it
     stays in the tab order); always shown on a touch screen. */
  .run-row-wrap:hover .run-del,
  .run-row-wrap:focus-within .run-del,
  .run-del:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .run-row-wrap .run-del {
      opacity: 1;
    }
  }
  .run-del:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .run-del:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: 8px;
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    padding: 4px 8px;
    cursor: pointer;
    text-align: start;
    min-width: 0;
    color: var(--text);
    font-size: var(--fs-s);
    flex: 1;
  }
  .run-row:hover {
    background: var(--hover);
  }
  .run-row.selected {
    border-color: var(--accent);
    background: var(--accent-soft);
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .kind-chip {
    flex: none;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 6px;
  }
  .note-link {
    background: none;
    border: none;
    padding: 0;
    font-size: inherit;
    color: var(--accent-text);
    cursor: pointer;
  }
  .note-link:hover {
    text-decoration: underline;
  }

  @media (max-width: 640px) {
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
