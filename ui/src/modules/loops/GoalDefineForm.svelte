<script lang="ts">
  import { ws } from '../../lib/stores/workspace.svelte';
  import { loops } from '../../lib/stores/loops.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { AcceptanceCriterion, GoalLoopDraft } from '../../lib/api/types';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import { confirmer } from '../../lib/confirm.svelte';

  let { oncancel, oncreated }: { oncancel: () => void; oncreated: (id: string) => void } = $props();

  let seed = $state('');
  let mode = $state<'build' | 'research'>('build');
  let allowCommits = $state(false);
  let requireReview = $state(false);
  let sourceLinks = $state('');
  let selectedSkills = $state('');
  let repoPath = $state('');
  let picking = $state(false);
  let feedback = $state('');
  let defining = $state(false);
  let launching = $state(false);

  let draft = $state<GoalLoopDraft | null>(null);
  let name = $state('');
  // Budget (edited as friendlier units).
  let maxIterations = $state(5);
  let maxMinutes = $state(30);
  let perPhaseMinutes = $state(10);
  let executorCount = $state(1);
  // Executor agent: provider from /meta (shell can't take an agent prompt);
  // model is a free-text alias ("" = provider default).
  let execProvider = $state(defaultAgentProvider());
  let execModel = $state('');
  // The agent that DRAFTS the goal (the define call) — its own pick, so
  // changing it doesn't silently change who executes the loop.
  let defProvider = $state(defaultAgentProvider());
  let defModel = $state('');
  let showAdvanced = $state(false);
  const providers = $derived(
    agentProviders(),
  );

  async function define(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !seed.trim() || (mode === 'build' && !repoPath.trim())) return;
    defining = true;
    try {
      const d = await loops.define(wsId, {
        seed, provider: defProvider, model: defModel, mode,
        repo_path: repoPath.trim(),
        context: draft ? JSON.stringify(draft.definition) : undefined,
        feedback: feedback.trim() || undefined,
      });
      draft = d;
      name = d.definition.title;
      maxIterations = d.suggested_limits.max_iterations;
      maxMinutes = Math.round(d.suggested_limits.max_runtime_secs / 60);
      perPhaseMinutes = Math.max(1, Math.round(d.suggested_limits.per_phase_timeout_secs / 60));
      feedback = '';
    } catch (e) {
      toasts.error('Couldn’t draft the goal', e instanceof Error ? e.message : String(e));
    } finally {
      defining = false;
    }
  }

  function addCriterion(): void {
    if (!draft) return;
    // Generate a unique id (ids are the {#each} key; collisions after a remove
    // would mis-map bound inputs).
    const ids = new Set(draft.definition.acceptance_criteria.map((c) => c.id));
    let n = draft.definition.acceptance_criteria.length + 1;
    while (ids.has(`c${n}`)) n++;
    draft.definition.acceptance_criteria.push({
      id: `c${n}`,
      text: '',
      verify: '',
      verify_kind: 'agent',
      verify_cmd: null,
    });
  }
  function removeCriterion(i: number): void {
    if (!draft) return;
    draft.definition.acceptance_criteria.splice(i, 1);
  }

  function canLaunch(): boolean {
    if (!draft || !name.trim()) return false;
    const cs = draft.definition.acceptance_criteria;
    return cs.length > 0 && cs.every((c) => c.text.trim() !== '' && c.verify.trim() !== '');
  }

  async function launch(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !draft || !canLaunch()) return;
    launching = true;
    try {
      const base = draft.suggested_config.executors[0] ?? {
        name: 'Executor',
        provider: defaultAgentProvider(),
        model: '',
        prompt_extra: '',
      };
      const executors = Array.from({ length: Math.max(1, executorCount) }, (_, i) => ({
        ...base,
        provider: execProvider,
        model: execModel.trim(),
        name: executorCount > 1 ? `${base.name} ${i + 1}` : base.name,
      }));
      const loop = await loops.create(wsId, {
        name: name.trim(),
        repo_path: repoPath.trim(),
        definition: draft.definition,
        limits: {
          max_iterations: maxIterations,
          max_runtime_secs: maxMinutes * 60,
          per_phase_timeout_secs: perPhaseMinutes * 60,
          max_cost_usd: null,
          max_attempts_per_executor: 3,
        },
        config: { ...draft.suggested_config, executors, mode, allow_commits: mode === 'build' && allowCommits,
          require_review: requireReview, source_links: sourceLinks.split('\n').map(s => s.trim()).filter(Boolean),
          skills: selectedSkills.split(',').map(s => s.trim()).filter(Boolean) },
        autostart: true,
      });
      oncreated(loop.id);
    } catch (e) {
      toasts.error('Couldn’t launch the goal loop', e instanceof Error ? e.message : String(e));
    } finally {
      launching = false;
    }
  }

  /** Leave the form; ask first once there is work to lose (a draft or a typed goal). */
  async function cancel(): Promise<void> {
    if (draft || seed.trim()) {
      const ok = await confirmer.ask('Discard this goal loop? The draft and your edits are lost.', {
        title: 'Discard goal loop',
        confirmLabel: 'Discard',
      });
      if (!ok) return;
    }
    oncancel();
  }

  function setKind(c: AcceptanceCriterion, kind: AcceptanceCriterion['verify_kind']): void {
    c.verify_kind = kind;
    if (kind === 'command' && c.verify_cmd == null) c.verify_cmd = '';
  }
</script>

<div class="form-page">
<PageHeader title="New goal loop">
  {#snippet leading()}
    <button class="icon-btn" title="Back to Goal Loops" aria-label="Back to Goal Loops" onclick={cancel}>
      <Icon name="chevronLeft" size={15} />
    </button>
  {/snippet}
</PageHeader>
<PageBody width="readable">
<div class="form">

  <section class="block">
    <label class="lbl" for="gl-seed">Goal <span class="hint">(what “done” looks like)</span></label>
    <textarea
      id="gl-seed"
      class="input in area"
      bind:value={seed}
      rows="3"
      placeholder="e.g. Make the export endpoint stream instead of buffering, and add a test."
    ></textarea>
    <label class="lbl" for="gl-mode">Mode</label>
    <select id="gl-mode" class="input in" bind:value={mode}><option value="build">Build</option><option value="research">Research</option></select>
    <p class="muted small">Build changes code on an isolated branch; Research writes findings to a scratch directory.</p>
    {#if mode === 'build'}
    <label class="lbl" for="gl-repo">Repository path</label>
    <div class="repo-row">
      <input id="gl-repo" class="input in grow" bind:value={repoPath} placeholder="/absolute/path/to/repo" />
      <button type="button" class="btn" onclick={() => (picking = true)}>Browse…</button>
    </div>
    {/if}
    <label class="lbl" for="gl-define-provider">Drafting agent <span class="hint">(turns your goal into criteria and a budget)</span></label>
    <select id="gl-define-provider" class="input in" bind:value={defProvider} onchange={() => (defModel = '')}>{#each providers as p (p)}<option value={p}>{p}</option>{/each}</select>
    <ModelPicker provider={defProvider} value={defModel} onchange={(m) => (defModel = m)} />
    <div class="frow">
      <button class="btn" class:primary={!draft} onclick={define} disabled={defining || !seed.trim() || (mode === 'build' && !repoPath.trim())}
        title={!seed.trim() ? 'Describe the goal first' : mode === 'build' && !repoPath.trim() ? 'Choose the repository first' : 'Draft criteria and a budget from your goal'}>
        {defining ? 'Defining…' : draft ? 'Re-define' : 'Define with AI'}
      </button>
      {#if draft}
        <input class="input in grow" bind:value={feedback} placeholder="Refine: what to change about the draft" />
        <button class="btn" onclick={define} disabled={defining || !feedback.trim()}>Refine</button>
      {/if}
    </div>
  </section>

  {#if draft}
    <section class="block">
      <label class="lbl" for="gl-name">Name</label>
      <input id="gl-name" class="input in" bind:value={name} />
      {#if draft.definition.summary}
        <p class="muted">{draft.definition.summary}</p>
      {/if}

      <div class="lbl">Acceptance criteria <span class="hint">(the loop stops only when all are met)</span></div>
      {#each draft.definition.acceptance_criteria as c, i (c.id)}
        <div class="crit">
          <div class="crit-row">
            <input class="input in grow" bind:value={c.text} placeholder="Criterion description" />
            <button class="icon-btn" onclick={() => removeCriterion(i)} aria-label="Remove criterion" title="Remove criterion"><Icon name="trash" size={13} /></button>
          </div>
          <div class="crit-row">
            <select class="input in kind" value={c.verify_kind} onchange={(e) => setKind(c, e.currentTarget.value as AcceptanceCriterion['verify_kind'])}>
              <option value="agent">Agent assessment</option>
              {#if c.verify_kind === 'manual'}<option value="manual">Agent assessment (legacy)</option>{/if}
              <option value="human">Human verification</option>
              <option value="command">Shell command</option>
            </select>
            {#if c.verify_kind === 'command'}
              <input class="input in grow mono" bind:value={c.verify_cmd} placeholder="shell command (exit 0 = met), e.g. cargo test" />
            {:else}
              <input class="input in grow" bind:value={c.verify} placeholder="how to verify (behavior/file)" />
            {/if}
          </div>
          {#if c.verify_kind === 'command'}
            <input class="input in grow" bind:value={c.verify} placeholder="what this checks (for humans)" />
          {/if}
        </div>
      {/each}
      <button class="btn small" onclick={addCriterion}><Icon name="plus" size={12} /> Add criterion</button>
    </section>

    <section class="block">
      <div class="lbl">Budget</div>
      <div class="budget">
        <label>Max iterations <input class="input in num" type="number" min="1" bind:value={maxIterations} /></label>
        <label>Max minutes <input class="input in num" type="number" min="1" bind:value={maxMinutes} /></label>
        <label>Per-phase minutes <input class="input in num" type="number" min="1" bind:value={perPhaseMinutes} /></label>
        <label>Executors <input class="input in num" type="number" min="1" max="6" bind:value={executorCount} /></label>
      </div>
      <div class="lbl">Executor agent <span class="hint">(does the work each iteration)</span></div>
      <div class="budget">
        <label>Provider
          <select class="input in num prov" bind:value={execProvider} onchange={() => (execModel = '')}>
            {#each providers as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </label>
        <!-- Catalog-backed model control; hides itself when the provider has
             no model-flag template. Blank = provider default. -->
        <div class="model-ctl">
          <ModelPicker provider={execProvider} value={execModel} onchange={(m) => (execModel = m)} />
        </div>
      </div>
      <p class="muted small">
        Executors run sequentially in an isolated {mode === 'research' ? 'research directory' : 'git worktree'}. Working files remain available after the loop finishes.
      </p>
    </section>

    <section class="block">
      <label class="checkbox-row" title={mode === 'research' ? 'Research loops don’t commit' : undefined}><input type="checkbox" bind:checked={allowCommits} disabled={mode === 'research'} /> Allow local commits</label>
      <p class="muted small indent">Working files are retained when a loop stops or finishes. Push and publishing require separate authorization.</p>
      <label class="checkbox-row"><input type="checkbox" bind:checked={requireReview} /> Require an independent completion review</label>
    </section>

    <section class="block adv">
      <button type="button" class="adv-toggle" aria-expanded={showAdvanced} aria-controls="gl-advanced" onclick={() => (showAdvanced = !showAdvanced)}>
        <Icon name={showAdvanced ? 'chevronDown' : 'chevronRight'} size={12} /> Advanced
        <span class="hint">sources, skills, planner / evaluator / digester agents</span>
      </button>
      {#if showAdvanced}
      <div id="gl-advanced">
      <label class="lbl" for="gl-sources">Source, spec and plan links (one per line)</label>
      <textarea id="gl-sources" class="input in area mono" rows="3" bind:value={sourceLinks} placeholder="https://…"></textarea>
      <label class="lbl" for="gl-skills">Selected skills (comma separated)</label>
      <input id="gl-skills" class="input in" bind:value={selectedSkills} placeholder="e.g. db-mysql, golang-testing" />
      {#each ['planner', 'evaluator', 'digester'] as role}
        {@const key = role as 'planner' | 'evaluator' | 'digester'}
        <label class="lbl" for={`gl-${role}`}>{role[0].toUpperCase() + role.slice(1)} provider</label>
        <select id={`gl-${role}`} class="input in" bind:value={draft.suggested_config[key].provider} onchange={() => { if (draft) draft.suggested_config[key].model = ''; }}>
          {#each providers as p (p)}<option value={p}>{p}</option>{/each}
        </select>
        <ModelPicker provider={draft.suggested_config[key].provider} value={draft.suggested_config[key].model} onchange={(m) => { if (draft) draft.suggested_config[key].model = m; }} />
      {/each}
      </div>
      {/if}
    </section>
    <div class="frow end">
      <button class="btn" onclick={cancel}>Cancel</button>
      <button
        class="btn primary"
        onclick={launch}
        disabled={launching || !canLaunch()}
        title={launching || canLaunch() ? undefined : 'Give the loop a name and at least one criterion, each with a description and how to verify it'}
      >
        {launching ? 'Launching…' : 'Launch loop'}
      </button>
    </div>
  {/if}
</div>
</PageBody>
</div>

{#if picking}
  <FolderPicker
    title="Choose a repository"
    gitOnly
    onpick={(p) => {
      repoPath = p;
      picking = false;
    }}
    onclose={() => (picking = false)}
  />
{/if}

<style>
  .form-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .form {
    max-width: 760px;
  }
  .block {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 14px;
    margin-bottom: 14px;
    background: var(--surface);
  }
  .lbl {
    display: block;
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text);
    margin: 12px 0 4px;
  }
  .lbl:first-child {
    margin-top: 0;
  }
  .hint,
  .muted {
    color: var(--text-dim);
    font-weight: 400;
  }
  .muted.small,
  .small {
    font-size: var(--fs-s);
    margin: 4px 0 0;
  }
  .indent {
    padding-inline-start: 22px;
    margin-block-end: 8px;
  }
  .in {
    width: 100%;
    box-sizing: border-box;
  }
  .area {
    font-family: inherit;
  }
  .in.mono,
  .area.mono {
    font-family: var(--font-mono);
  }
  .frow {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 12px;
  }
  .frow.end {
    justify-content: flex-end;
    margin-bottom: 24px;
  }
  .adv-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    padding: 0;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    font-weight: 600;
    cursor: pointer;
    width: 100%;
    text-align: start;
  }
  .adv-toggle .hint {
    font-weight: 400;
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .crit {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px;
    margin-bottom: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .crit-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .kind {
    width: 190px;
    flex: none;
  }
  .num {
    width: 80px;
  }
  .prov {
    width: 120px;
  }
  .model-ctl {
    min-width: 180px;
    max-width: 280px;
  }
  .repo-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .budget {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
  }
  .budget label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .block > :global(.checkbox-row) + :global(.checkbox-row) {
    margin-top: 8px;
  }
  @media (max-width: 640px) {
    .crit-row {
      flex-wrap: wrap;
    }
    .kind {
      width: 100%;
    }
  }
</style>
