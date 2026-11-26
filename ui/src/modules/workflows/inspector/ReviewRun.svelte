<script lang="ts">
  // review_run: repository / base (inherited by default), threshold, execution
  // mode, and the rich reviewer config — per-lens reviewers (each its own
  // providers + instructions), a summarizer and a per-severity scoring guideline.
  import type { WorkflowNode } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
  import RepoPicker from './RepoPicker.svelte';
  import StepModel from './StepModel.svelte';
  import { providerName } from '../editor/nodeSummary';
  import { reviewerPick, withNodePick, withReviewerPick, type ModelPick, type ReviewerParams } from '../editor/stepRules';
  import type { InspectorOptions } from './options.svelte';
  import {
    linesValue, paramBool, paramLines, paramNum, paramStr, reviewers as reviewersOf, scoringField as scoringOf,
    summarizerField as summarizerOf, toggled, withScoring, withSummarizer, type ReviewerRow, type SetParam,
  } from './util';

  let { node, set, wsId, pickers, pickModels = false }: {
    node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions;
    /** The editor's form: per-reviewer model and effort, the summarizer's pick, speed (ux.md §9.3). */
    pickModels?: boolean;
  } = $props();
  const p = $derived((node.params ?? {}) as { picks?: Record<string, ModelPick>; summarizer?: Record<string, string>; reviewer_time_box_secs?: number; scope_assessment?: string });
  /** Providers any reviewer runs on: one review-wide default each. */
  const usedProviders = $derived([...new Set(reviewers().flatMap((r) => r.providers ?? []))]);
  const pickOf = (v: ModelPick): { provider: string; model: string; effort: string } => ({ provider: '', model: v.model ?? '', effort: v.effort ?? '' });
  function setRowPick(i: number, prov: string, v: { model: string; effort: string }): void {
    setReviewers(reviewers().map((r, k) => (k === i ? (withReviewerPick(r as ReviewerParams, prov, v) as ReviewerRow) : r)));
  }
  function setSummarizer(v: { provider: string; model: string; effort: string }): void {
    const next = { ...(p.summarizer ?? {}) };
    for (const k of ['provider', 'model', 'effort'] as const) {
      if (v[k]) next[k] = v[k];
      else delete next[k];
    }
    set('summarizer', Object.keys(next).length ? next : undefined);
  }
  // Reviewer providers come from the live registry (built-ins + custom).
  const REVIEW_PROVIDERS = $derived(agentProviders());
  function reviewers(): ReviewerRow[] {
    return reviewersOf(node);
  }
  function setReviewers(rows: ReviewerRow[]): void {
    set('reviewers', rows.length ? rows : undefined);
  }
  function addReviewer(): void {
    setReviewers([...reviewers(), { lens: '', providers: [defaultAgentProvider()] }]);
  }
  function removeReviewer(i: number): void {
    setReviewers(reviewers().filter((_, idx) => idx !== i));
  }
  function updateReviewer(i: number, patch: Partial<ReviewerRow>): void {
    setReviewers(reviewers().map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  }
  function toggleReviewerProvider(i: number, prov: string): void {
    const next = toggled(reviewers()[i]?.providers ?? [], prov);
    updateReviewer(i, { providers: next.length ? next : [defaultAgentProvider()] });
  }
  const summarizerField = (field: 'provider' | 'instructions'): string => summarizerOf(node, field);
  const updateSummarizer = (field: string, value: string): void => set('summarizer', withSummarizer(node, field, value));
  const scoringField = (sev: 'bug' | 'warn' | 'info', def: number): number => scoringOf(node, sev, def);
  const updateScoring = (sev: string, value: number): void => set('scoring', withScoring(node, sev, value));
</script>

<p class="insp-note">
  Leave Repository and Base on “inherit” to review exactly where the implementer worked
  (the run’s working folder + base). Set them only to override.
</p>
<label for="np-repo">Repository (optional — inherits from the implementer)</label>
<RepoPicker id="np-repo" emptyLabel="Inherit from the working folder" {node} {set} {wsId} {pickers} />
<label for="np-base">Base branch (optional — inherits, else main)</label>
<input dir="auto"
  id="np-base"
  type="text"
  placeholder="inherits from the run, else main"
  value={paramStr(node, 'base')}
  oninput={(e) => set('base', e.currentTarget.value)}
/>
<label for="np-threshold">Pass threshold (0–100)</label>
<input
  id="np-threshold"
  type="number"
  min="0"
  max="100"
  value={paramNum(node, 'threshold', 80)}
  oninput={(e) => set('threshold', Number(e.currentTarget.value))}
/>
<!-- How the lenses are spread over sessions. Empty value = no
     `mode` key at all, so the run override / stored config wins. -->
<label for="np-review-mode">Execution mode</label>
<select
  id="np-review-mode"
  data-testid="review-mode-select"
  value={paramStr(node, 'mode')}
  onchange={(e) => set('mode', e.currentTarget.value || undefined)}
>
  <option value="">Default (fan-out unless the run overrides it)</option>
  <option value="fan_out">Fan-out — one reviewer agent per lens × provider</option>
  <option value="orchestrator">Orchestrator — one agent per provider runs every lens as its own sub-agents</option>
</select>
<p class="insp-note">
  Both modes end with one summarizer. Orchestrator opens N sessions instead of
  N × lenses; fan-out finishes sooner on a fast machine, orchestrator is easier on
  file descriptors and CPU. A run started with an explicit review mode overrides
  this setting.
</p>
<div class="rv-h np-sec">
  <span class="np-label">Reviewers — one per lens, each its own agents (like PR review)</span>
  <button class="btn small ghost" type="button" onclick={addReviewer}>
    <Icon name="plus" size={12} /> Add
  </button>
</div>
{#if reviewers().length === 0}
  <p class="insp-note">
    No reviewers — leave empty to use the default PR-review config, or add one
    (e.g. <code>correctness-review</code> on claude + codex).
  </p>
{/if}
<div class="rv-list">
{#each reviewers() as r, i (i)}
  <div class="rv-row">
    <div class="rv-top">
      <input dir="auto" aria-label="Reviewer lens / skill"
        class="rv-lens"
        type="text"
        placeholder="lens / skill (e.g. correctness-review)"
        value={r.lens ?? ''}
        oninput={(e) => updateReviewer(i, { lens: e.currentTarget.value })}
      />
      <button class="rv-del" type="button" title="Remove reviewer" aria-label="Remove reviewer" onclick={() => removeReviewer(i)}>
        <Icon name="trash" size={12} />
      </button>
    </div>
    <div class="rv-provs">
      {#each REVIEW_PROVIDERS as prov (prov)}
        <label class="rv-chip" class:on={(r.providers ?? []).includes(prov)}>
          <input
            type="checkbox"
            checked={(r.providers ?? []).includes(prov)}
            onchange={() => toggleReviewerProvider(i, prov)}
          />
          {prov}
        </label>
      {/each}
    </div>
    {#if pickModels}
      {#each r.providers ?? [] as prov (prov)}
        <div class="rv-pick">
          <span class="node-hint">{providerName(prov)}</span>
          <StepModel compact lockProvider label="{r.lens || 'Reviewer'} · {providerName(prov)}" value={pickOf(reviewerPick(r as ReviewerParams, prov))} defaultProvider={prov}
            onchange={(v) => setRowPick(i, prov, v)} />
        </div>
      {/each}
    {/if}
    <textarea dir="auto" aria-label="Reviewer instructions"
      class="rv-instr np-prompt"
      rows="5"
      placeholder="custom instructions for this reviewer (optional)"
      value={r.instructions ?? ''}
      oninput={(e) => updateReviewer(i, { instructions: e.currentTarget.value })}
    ></textarea>
  </div>
{/each}
</div>

{#if pickModels && usedProviders.length}
  <details class="np-sec">
    <summary class="node-hint">Default for every reviewer of a provider</summary>
    {#each usedProviders as prov (prov)}
      <div class="rv-pick">
        <span class="node-hint">Every {providerName(prov)} reviewer</span>
        <StepModel compact lockProvider label="Every {providerName(prov)} reviewer" value={pickOf(p.picks?.[prov] ?? {})} defaultProvider={prov}
          onchange={(v) => set('picks', withNodePick(p.picks, prov, v))} />
      </div>
    {/each}
  </details>
{/if}
{#if pickModels}
  <span class="np-label np-sec">Summarizer (consolidates + scores; the scope check runs on its pick)</span>
  <StepModel label="Summarizer" value={{ provider: p.summarizer?.provider ?? '', model: p.summarizer?.model ?? '', effort: p.summarizer?.effort ?? '' }}
    defaultProvider={defaultAgentProvider()} onchange={setSummarizer} />
{:else}
<label class="np-sec" for="np-sum-prov">Summarizer (consolidates + scores)</label>
<input dir="auto"
  id="np-sum-prov"
  type="text"
  placeholder="provider (e.g. claude)"
  value={summarizerField('provider')}
  oninput={(e) => updateSummarizer('provider', e.currentTarget.value)}
/>
{/if}
<textarea dir="auto" aria-label="Summarizer instructions"
  class="np-prompt np-prompt-lg"
  rows="14"
  placeholder="summarizer instructions (optional)"
  value={summarizerField('instructions')}
  oninput={(e) => updateSummarizer('instructions', e.currentTarget.value)}
></textarea>

<span class="np-label np-sec">Scoring guideline — % deducted per open finding</span>
<div class="rv-score">
  <label class="rv-sc">
    Critical
    <input type="number" min="0" max="100" value={scoringField('bug', 20)}
      oninput={(e) => updateScoring('bug', Number(e.currentTarget.value))} />
  </label>
  <label class="rv-sc">
    High
    <input type="number" min="0" max="100" value={scoringField('warn', 5)}
      oninput={(e) => updateScoring('warn', Number(e.currentTarget.value))} />
  </label>
  <label class="rv-sc">
    Optional
    <input type="number" min="0" max="100" value={scoringField('info', 0)}
      oninput={(e) => updateScoring('info', Number(e.currentTarget.value))} />
  </label>
</div>
<label class="np-sec" for="np-goals">Goals (one per line, optional)</label>
<textarea dir="auto"
  id="np-goals"
  class="np-prompt"
  rows="7"
  placeholder={'No N+1 queries\nAll inputs validated'}
  value={paramLines(node, 'goals')}
  oninput={(e) => set('goals', linesValue(e.currentTarget.value))}
></textarea>
<label class="np-sec" for="np-checks">Checks — commands the reviewer runs (one per line, optional)</label>
<textarea dir="ltr"
  id="np-checks"
  class="np-prompt"
  rows="7"
  placeholder={'go test -tags=component ./...\ngo test -tags=integration ./...'}
  value={paramLines(node, 'checks')}
  oninput={(e) => set('checks', linesValue(e.currentTarget.value))}
></textarea>
<p class="insp-note">
  The reviewer agent runs these in the repo and reports any failure as a blocking
  finding — a safety net for a check the implementer may have skipped.
</p>
{#if pickModels}
  <span class="np-label np-sec">Speed</span>
  <label for="np-timebox">Time-box each reviewer (minutes)</label>
  <input id="np-timebox" type="number" min="1" max="240" placeholder="Off" value={p.reviewer_time_box_secs ? Math.round(p.reviewer_time_box_secs / 60) : ''}
    oninput={(e) => { const m = Number(e.currentTarget.value); set('reviewer_time_box_secs', m > 0 ? Math.round(m * 60) : undefined); }} />
  <p class="insp-note">A reviewer that runs out keeps the findings it wrote; its lens counts as missing otherwise. Empty: off.</p>
  <label for="np-scope">Which reviewers run</label>
  <select id="np-scope" value={p.scope_assessment === 'configured' ? 'configured' : ''} onchange={(e) => set('scope_assessment', e.currentTarget.value || undefined)}>
    <option value="">Chosen by the scope check</option>
    <option value="configured">Only the reviewers above (the scope check may drop one, never add one)</option>
  </select>
{/if}
<label class="np-chk">
  <input
    type="checkbox"
    checked={paramBool(node, 'await', true)}
    onchange={(e) => set('await', e.currentTarget.checked)}
  /> Wait for the review to finish
</label>
<label class="np-chk">
  <input
    type="checkbox"
    checked={paramBool(node, 'require_pass')}
    onchange={(e) => set('require_pass', e.currentTarget.checked)}
  /> Require pass (fail this step if below threshold)
</label>

<style>
  .rv-pick {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }
</style>
