<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { toastError } from '../../lib/toastError';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { loops } from '../../lib/stores/loops.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { AcceptanceCriterion, GoalLoopDraft, GoalLoopGraph } from '../../lib/api/types';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import GoalGraphEditor from './GoalGraphEditor.svelte';
  import IterationDiagram from './IterationDiagram.svelte';
  import { graphError, initialGraph, restoreGraph } from './graph';
  import { goalTemplates, templateDraft, researchDraft, type GoalTemplateId, type PocScope } from './templates';

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
  let selectedTemplate = $state<GoalTemplateId | ''>('');
  let pocScope = $state<PocScope>('both');
  let draftSource = $state<'template' | 'agent'>('agent');
  let execution = $state<'loop' | 'graph'>('loop');
  let savedGraph = $state<GoalLoopGraph | null>(null);
  const template = $derived(goalTemplates.find(item => item.id === selectedTemplate));
  const graphIssue = $derived(draft ? graphError(draft.suggested_config.graph, draft.definition.acceptance_criteria) : null);
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

  let alive = true;
  let formGeneration = 0;
  let initialSnapshot = untrack(formState);
  onDestroy(() => { alive = false; formGeneration++; });
  $effect(() => router.guard(() => canLeaveForm()));

  function formState(): string {
    return JSON.stringify([seed, mode, allowCommits, requireReview, sourceLinks, selectedSkills, repoPath,
      feedback, draft, name, maxIterations, maxMinutes, perPhaseMinutes, executorCount,
      execProvider, execModel, defProvider, defModel, selectedTemplate, pocScope, draftSource, execution, savedGraph]);
  }

  function applyDraft(d: GoalLoopDraft): void {
    draft = mode === 'research' ? researchDraft(d) : d;
    // A new template/agent response owns a new task plan. The cache is only for
    // switching execution type within this exact draft.
    savedGraph = draft.suggested_config.graph ?? null;
    if (execution === 'loop') {
      draft.suggested_config.graph = null;
    } else if (!draft.suggested_config.graph) {
      draft.suggested_config.graph = initialGraph(draft.definition.acceptance_criteria, seed);
    }
    name = d.definition.title;
    maxIterations = d.suggested_limits.max_iterations;
    maxMinutes = Math.round(d.suggested_limits.max_runtime_secs / 60);
    perPhaseMinutes = Math.max(1, Math.round(d.suggested_limits.per_phase_timeout_secs / 60));
    feedback = '';
  }

  async function setExecution(next: 'loop' | 'graph'): Promise<void> {
    if (!alive || next === execution) return;
    const generation = formGeneration;
    const workspace = ws.currentId;
    if (next === 'loop' && draft?.suggested_config.graph && !(await confirmer.ask('Work toward the whole goal each iteration? Your tasks and dependencies stay in this draft so you can switch back.', { title: 'Use a goal loop', confirmLabel: 'Use goal loop' }))) return;
    if (!alive || generation !== formGeneration || workspace !== ws.currentId) return;
    execution = next;
    if (!draft) return;
    if (next === 'loop') {
      savedGraph = draft.suggested_config.graph ?? null;
      draft.suggested_config.graph = null;
    } else {
      draft.suggested_config.graph = savedGraph ? restoreGraph(savedGraph, draft.definition.acceptance_criteria) : initialGraph(draft.definition.acceptance_criteria, seed);
      if (mode === 'research') draft = researchDraft(draft);
    }
  }

  function setMode(next: 'build' | 'research'): void {
    mode = next;
    if (!draft) return;
    if (next === 'research') draft = researchDraft(draft);
    else {
      draft.definition.acceptance_criteria = draft.definition.acceptance_criteria.filter(criterion => criterion.id !== 'otto-research-report');
      if (draft.suggested_config.graph) draft.suggested_config.graph.nodes = draft.suggested_config.graph.nodes.map(node => ({ ...node, criterion_ids: node.criterion_ids.filter(id => id !== 'otto-research-report') }));
      if (savedGraph) savedGraph = { nodes: savedGraph.nodes.map(node => ({ ...node, criterion_ids: node.criterion_ids.filter(id => id !== 'otto-research-report') })) };
    }
  }

  async function chooseTemplate(id: GoalTemplateId, scope: PocScope = pocScope): Promise<void> {
    if (!alive) return;
    const generation = formGeneration;
    const wsId = ws.currentId;
    if (draft && !(await confirmer.ask('Replace the draft, acceptance criteria and graph with this template? Your goal and repository path are kept.', { title: 'Replace goal draft', confirmLabel: 'Use template' }))) return;
    if (!alive || formGeneration !== generation || ws.currentId !== wsId) return;
    // A late AI response belongs to the replaced draft, not this template.
    formGeneration++;
    defining = false;
    selectedTemplate = id;
    pocScope = scope;
    draftSource = 'template';
    const next = templateDraft(id, execProvider, scope);
    savedGraph = next.suggested_config.graph ?? null;
    mode = next.suggested_config.mode ?? 'build';
    applyDraft(next);
    allowCommits = false;
    requireReview = true;
  }

  async function define(): Promise<void> {
    const wsId = ws.currentId;
    if (!alive || !wsId || !seed.trim() || (mode === 'build' && !repoPath.trim())) return;
    const generation = formGeneration;
    const ownsForm = () => alive && generation === formGeneration && ws.currentId === wsId;
    if (draft && (draft.suggested_config.graph || savedGraph) && !(await confirmer.ask('Updating with an agent replaces this draft’s criteria and task plan. Your goal and repository path are kept.', { title: 'Update goal draft', confirmLabel: 'Update draft' }))) return;
    if (!ownsForm()) return;
    defining = true;
    try {
      const d = await loops.define(wsId, {
        seed, provider: defProvider, model: defModel, mode,
        repo_path: repoPath.trim(),
        context: draft ? JSON.stringify(draft.definition) : undefined,
        feedback: feedback.trim() || undefined,
      });
      if (!ownsForm()) return;
      applyDraft(d);
      draftSource = 'agent';
    } catch (e) {
      if (ownsForm()) toastError('Couldn’t draft the goal', e);
    } finally {
      if (ownsForm()) defining = false;
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
    if (!alive || !ws.currentId || !draft || !name.trim() || !seed.trim() || (mode === 'build' && !repoPath.trim())) return false;
    if (graphError(draft.suggested_config.graph, draft.definition.acceptance_criteria)) return false;
    if (maxIterations < (draft.suggested_config.graph?.nodes.length ?? 1)) return false;
    if (![maxIterations, maxMinutes, perPhaseMinutes, executorCount].every(value => Number.isInteger(value) && value > 0) || executorCount > 6) return false;
    const cs = draft.definition.acceptance_criteria;
    return (
      cs.length > 0 &&
      cs.every(
        (c) =>
          c.text.trim() !== '' &&
          c.verify.trim() !== '' &&
          (c.verify_kind !== 'command' || (c.verify_cmd ?? '').trim() !== ''),
      )
    );
  }

  async function launch(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !draft || !canLaunch()) return;
    if (launching) return;
    const generation = formGeneration;
    const ownsForm = () => alive && generation === formGeneration && ws.currentId === wsId;
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
        definition: { ...draft.definition, title: name.trim(), summary: draftSource === 'template' ? `${seed.trim()}\n\n${draft.definition.summary}` : draft.definition.summary },
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
      if (ownsForm()) {
        initialSnapshot = formState();
        oncreated(loop.id);
      }
    } catch (e) {
      if (ownsForm()) toastError('Couldn’t launch the goal loop', e);
    } finally {
      if (ownsForm()) launching = false;
    }
  }

  /** Route/workspace navigation and local Cancel use the same complete draft. */
  async function canLeaveForm(): Promise<boolean> {
    if (!alive) return true;
    if (formState() !== initialSnapshot) {
      const ok = await confirmer.ask('Discard this goal loop? The draft and your edits are lost.', {
        title: 'Discard goal loop',
        danger: true, confirmLabel: 'Discard',
      });
      if (!ok) return false;
    }
    // A workspace switch keeps LoopsPage mounted. Settle its creation state,
    // rather than marking an A draft clean and leaving it launchable under B.
    // Invalidate ownership before the callback; repeated leave/Cancel callbacks
    // during teardown must neither close twice nor publish a pending launch.
    if (!alive) return true;
    alive = false;
    formGeneration++;
    defining = false;
    launching = false;
    oncancel();
    return true;
  }

  async function cancel(): Promise<void> {
    await canLeaveForm();
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
      <Icon name="chevronLeft" size={14} />
    </button>
  {/snippet}
</PageHeader>
<PageBody width="readable">
<div class="form">
  {#if !ws.currentId}
    <!-- Page content (rendered with the page), not a status change: no role. -->
    <section class="block">
      <p>Add a workspace to define and launch this goal. Your draft stays here.</p>
      <button class="btn" onclick={() => (ui.newWorkspaceOpen = true)}>Add workspace</button>
    </section>
  {/if}

  <section class="block">
    <label class="lbl" for="gl-execution">Execution</label>
    <select id="gl-execution" class="input in" value={execution} disabled={launching || defining} onchange={async (e) => {
      const select = e.currentTarget;
      await setExecution(select.value as 'loop' | 'graph');
      select.value = execution;
    }}><option value="loop">Goal loop — iterate on the whole goal</option><option value="graph">Engineering graph — tasks with prerequisites</option></select>
    <label class="lbl" for="gl-template">Start from a template <span class="hint">(optional)</span></label>
    <select id="gl-template" class="input in" value={selectedTemplate} disabled={launching} onchange={async (e) => {
      const select = e.currentTarget;
      const id = select.value as GoalTemplateId;
      if (id) await chooseTemplate(id);
      select.value = selectedTemplate;
    }}>
      <option value="" disabled={!!draft}>Describe your own goal</option>
      {#each goalTemplates as item (item.id)}<option value={item.id}>{item.title}</option>{/each}
    </select>
    {#if selectedTemplate === 'poc'}
      <label class="lbl" for="gl-poc-scope">POC scope</label>
      <select id="gl-poc-scope" class="input in" value={pocScope} disabled={launching} onchange={async (e) => {
        const select = e.currentTarget;
        await chooseTemplate('poc', select.value as PocScope);
        select.value = pocScope;
      }}><option value="backend">Backend</option><option value="frontend">Frontend</option><option value="both">Backend and frontend</option></select>
    {/if}
    <label class="lbl" for="gl-seed">Goal <span class="hint">(what “done” looks like)</span></label>
    <textarea dir="auto"
      id="gl-seed"
      class="input in area"
      bind:value={seed}
      rows="3"
      placeholder={template?.example ?? 'e.g. Make the export endpoint stream instead of buffering, and add a test.'}
    ></textarea>
    <label class="lbl" for="gl-mode">Work type</label>
    <select id="gl-mode" class="input in" value={mode} onchange={(e) => setMode(e.currentTarget.value as 'build' | 'research')}><option value="build">Build</option><option value="research">Research</option></select>
    <p class="muted small">Build changes code on an isolated branch; Research writes findings to a scratch directory.</p>
    {#if mode === 'build'}
    <label class="lbl" for="gl-repo">Repository path</label>
    <div class="repo-row">
      <input dir="ltr" id="gl-repo" class="input in grow" bind:value={repoPath} placeholder="/absolute/path/to/repo" />
      <button type="button" class="btn" onclick={() => (picking = true)}>Browse…</button>
    </div>
    {/if}
    <details open={!draft} class="agent-drafting">
    <summary>{draft ? 'Refine with an agent' : 'Draft with an agent'}</summary>
    <label class="lbl" for="gl-define-provider">Drafting agent <span class="hint">(turns your goal into criteria and a budget)</span></label>
    <select id="gl-define-provider" class="input in" bind:value={defProvider} onchange={() => (defModel = '')}>{#each providers as p (p)}<option value={p}>{p}</option>{/each}</select>
    <ModelPicker provider={defProvider} value={defModel} onchange={(m) => (defModel = m)} />
    <div class="frow">
      {#if draft}
        <input dir="auto" aria-label="Refine the draft" class="input in grow" bind:value={feedback} placeholder="What to change about the draft" />
        <button class="btn" onclick={define} disabled={!ws.currentId || defining || !feedback.trim() || !seed.trim() || (mode === 'build' && !repoPath.trim())}>
          {defining ? 'Updating…' : 'Update draft'}
        </button>
      {:else}
        <button class="btn primary" onclick={define} disabled={!ws.currentId || defining || !seed.trim() || (mode === 'build' && !repoPath.trim())}
          title={!ws.currentId ? 'Add a workspace first' : !seed.trim() ? 'Describe the goal first' : mode === 'build' && !repoPath.trim() ? 'Choose the repository first' : 'Draft criteria and a budget from your goal'}>
          {defining ? 'Drafting…' : 'Draft goal'}
        </button>
      {/if}
    </div>
    </details>
  </section>

  <section class="block">
    {#if execution === 'loop'}
      <IterationDiagram />
    {:else if draft}
      <GoalGraphEditor graph={draft.suggested_config.graph} criteria={draft.definition.acceptance_criteria}
        onchange={(graph) => { if (draft) draft.suggested_config.graph = graph; }} />
    {:else}
      <h2 class="execution-title">Engineering graph</h2>
      <p class="muted small">Choose a template or draft your goal to create its tasks. Connect prerequisites, link evidence criteria, and run one ready task per iteration.</p>
    {/if}
  </section>

  {#if draft}
    <section class="block">
      <label class="lbl" for="gl-name">Name</label>
      <input dir="auto" id="gl-name" class="input in" bind:value={name} />
      <p class="muted small">{draftSource === 'template' ? 'Template draft — edit the scope and criteria before launching.' : 'Agent draft — review the scope and criteria before launching.'}</p>
      <label class="lbl" for="gl-summary">Scope and approach</label>
      <textarea id="gl-summary" class="input in area" dir="auto" rows="3" bind:value={draft.definition.summary}></textarea>

      <div class="lbl">Acceptance criteria <span class="hint">(the loop stops only when all are met)</span></div>
      {#each draft.definition.acceptance_criteria as c, i (c.id)}
        <div class="crit">
          <div class="crit-row">
            <input dir="auto" aria-label="Criterion description" class="input in grow" bind:value={c.text} placeholder="Criterion description" />
            <button class="icon-btn" onclick={() => removeCriterion(i)} disabled={mode === 'research' && c.id === 'otto-research-report'} aria-label="Remove criterion" title={mode === 'research' && c.id === 'otto-research-report' ? 'Research requires a retained report' : 'Remove criterion'}><Icon name="trash" size={13} /></button>
          </div>
          <div class="crit-row">
            <select aria-label="Verification kind" class="input in kind" value={c.verify_kind} onchange={(e) => setKind(c, e.currentTarget.value as AcceptanceCriterion['verify_kind'])}>
              <option value="agent">Agent assessment</option>
              {#if c.verify_kind === 'manual'}<option value="manual">Agent assessment (legacy)</option>{/if}
              <option value="human">Human verification</option>
              <option value="command">Shell command</option>
            </select>
            {#if c.verify_kind === 'command'}
              <input dir="ltr" aria-label="Verification command" class="input in grow mono" bind:value={c.verify_cmd} placeholder="shell command (exit 0 = met), e.g. cargo test" />
            {:else}
              <input dir="auto" aria-label="How to verify" class="input in grow" bind:value={c.verify} placeholder="how to verify (behavior/file)" />
            {/if}
          </div>
          {#if c.verify_kind === 'command'}
            <input dir="auto" aria-label="What this command checks" class="input in grow" bind:value={c.verify} placeholder="what this checks (for humans)" />
          {/if}
        </div>
      {/each}
      <button class="btn small" onclick={addCriterion}><Icon name="plus" size={12} /> Add criterion</button>
      {#if !canLaunch()}
        <!-- A form hint, read in place; an inserted status would be silent anyway. -->
        <p class="muted small" id="goal-criteria-hint">Each criterion needs a description and how it is verified.</p>
      {/if}
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
      {#if draft.suggested_config.graph}
        <p class="muted small">{draft.suggested_config.graph.nodes.length} tasks need at least that many iterations. Leave extra iterations for repairs.</p>
        {#if maxIterations < draft.suggested_config.graph.nodes.length}<p class="form-error" use:announceShown={{ text: 'Increase the iteration budget to cover every graph task.', assertive: true }}>Increase the iteration budget to cover every graph task.</p>{/if}
      {/if}
    </section>

    <section class="block">
      <label class="checkbox-row" title={mode === 'research' ? 'Research loops don’t commit' : undefined}><input type="checkbox" bind:checked={allowCommits} disabled={mode === 'research'} /> Allow local commits</label>
      <p class="muted small indent">Working files are retained when a loop stops or finishes. Push and publishing require separate authorization.</p>
      <label class="checkbox-row"><input type="checkbox" bind:checked={requireReview} /> Require an independent completion review</label>
    </section>

    <section class="block adv">
      <button type="button" class="adv-toggle" aria-expanded={showAdvanced} aria-controls="gl-advanced" onclick={() => (showAdvanced = !showAdvanced)}>
        <Icon name={showAdvanced ? 'chevronDown' : 'chevronRight'} size={12} /> Advanced
        <span class="hint adv-hint">sources, skills, planner / evaluator / digester agents</span>
      </button>
      {#if showAdvanced}
      <div id="gl-advanced">
      <label class="lbl" for="gl-sources">Source, spec and plan links (one per line)</label>
      <textarea dir="ltr" id="gl-sources" class="input in area mono" rows="3" bind:value={sourceLinks} placeholder="https://…"></textarea>
      <label class="lbl" for="gl-skills">Selected skills (comma separated)</label>
      <input dir="auto" id="gl-skills" class="input in" bind:value={selectedSkills} placeholder="e.g. db-mysql, golang-testing" />
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
      {#if graphIssue}<p class="form-error">Fix the execution graph before launching.</p>{/if}
      <button class="btn" onclick={cancel}>Cancel</button>
      <button
        class="btn primary"
        onclick={launch}
        disabled={launching || !canLaunch()}
        title={launching || canLaunch() ? undefined : 'Give the loop a name and at least one criterion, each with a description, how to verify it, and a shell command when verification runs a command'}
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
  .execution-title { font-size: var(--fs-m); font-weight: 600; margin: 0; }
  .agent-drafting { margin-block-start: 12px; }
  .agent-drafting > summary { font-size: var(--fs-s); cursor: pointer; }
  .form-error { color: var(--danger); font-size: var(--fs-s); }
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
    flex-wrap: wrap;
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
  .adv-toggle .hint,
  .adv-hint {
    flex: 1;
    min-width: 0;
    font-weight: 400;
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
    min-width: 0;
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
    flex: 1 1 160px;
    width: auto;
    max-width: 100%;
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
  @media (max-width: 1024px) {
    .crit-row,
    .frow,
    .repo-row {
      flex-wrap: wrap;
    }
    .kind,
    .repo-row .grow,
    .frow .grow,
    .crit-row .grow {
      width: 100%;
      min-width: 0;
      flex: 1 1 100%;
    }
  }
</style>
