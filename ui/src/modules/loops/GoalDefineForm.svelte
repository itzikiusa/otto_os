<script lang="ts">
  import { ws } from '../../lib/stores/workspace.svelte';
  import { loops } from '../../lib/stores/loops.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import type { AcceptanceCriterion, GoalLoopDraft } from '../../lib/api/types';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';

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
  const providers = $derived(
    agentProviders(),
  );

  async function define(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !seed.trim() || (mode === 'build' && !repoPath.trim())) return;
    defining = true;
    try {
      const d = await loops.define(wsId, {
        seed, provider: execProvider, model: execModel, mode,
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
      toasts.error('Define failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Launch failed', e instanceof Error ? e.message : String(e));
    } finally {
      launching = false;
    }
  }

  function setKind(c: AcceptanceCriterion, kind: AcceptanceCriterion['verify_kind']): void {
    c.verify_kind = kind;
    if (kind === 'command' && c.verify_cmd == null) c.verify_cmd = '';
  }
</script>

<div class="form">
  <header class="head">
    <h1>New goal loop</h1>
    <button class="btn ghost" onclick={oncancel}>Cancel</button>
  </header>

  <section class="block">
    <label class="lbl" for="gl-mode">Mode</label>
    <select id="gl-mode" class="in" bind:value={mode}><option value="build">Build</option><option value="research">Research</option></select>
    <label class="lbl" for="gl-define-provider">Definer provider</label>
    <select id="gl-define-provider" class="in" bind:value={execProvider}>{#each providers as p (p)}<option value={p}>{p}</option>{/each}</select>
    <ModelPicker provider={execProvider} value={execModel} onchange={(m) => (execModel = m)} />
    {#if mode === 'build'}
    <label class="lbl" for="gl-repo">Repository path</label>
    <div class="repo-row">
      <input id="gl-repo" class="in grow" bind:value={repoPath} placeholder="/absolute/path/to/repo" />
      <button type="button" class="btn" onclick={() => (picking = true)}>Browse…</button>
    </div>
    {/if}
    <label class="lbl" for="gl-seed">Goal</label>
    <textarea
      id="gl-seed"
      class="in area"
      bind:value={seed}
      rows="3"
      placeholder="e.g. Make the export endpoint stream instead of buffering, and add a test."
    ></textarea>
    <div class="row">
      <button class="btn primary" onclick={define} disabled={defining || !seed.trim() || (mode === 'build' && !repoPath.trim())}>
        {defining ? 'Defining…' : draft ? 'Re-define' : 'Define with AI'}
      </button>
      {#if draft}
        <input class="in grow" bind:value={feedback} placeholder="Refine: what to change about the draft" />
        <button class="btn" onclick={define} disabled={defining || !feedback.trim()}>Refine</button>
      {/if}
    </div>
  </section>

  {#if draft}
    <section class="block">
      <label class="lbl" for="gl-name">Name</label>
      <input id="gl-name" class="in" bind:value={name} />
      {#if draft.definition.summary}
        <p class="muted">{draft.definition.summary}</p>
      {/if}

      <div class="lbl">Acceptance criteria <span class="hint">(the loop stops only when all are met)</span></div>
      {#each draft.definition.acceptance_criteria as c, i (c.id)}
        <div class="crit">
          <div class="crit-row">
            <input class="in grow" bind:value={c.text} placeholder="Criterion description" />
            <button class="btn ghost small" onclick={() => removeCriterion(i)} aria-label="Remove">✕</button>
          </div>
          <div class="crit-row">
            <select class="in kind" value={c.verify_kind} onchange={(e) => setKind(c, e.currentTarget.value as AcceptanceCriterion['verify_kind'])}>
              <option value="agent">Agent assessment</option>
              <option value="manual">Agent assessment (legacy)</option>
              <option value="human">Human verification</option>
              <option value="command">command</option>
            </select>
            {#if c.verify_kind === 'command'}
              <input class="in grow mono" bind:value={c.verify_cmd} placeholder="shell command (exit 0 = met), e.g. cargo test" />
            {:else}
              <input class="in grow" bind:value={c.verify} placeholder="how to verify (behavior/file)" />
            {/if}
          </div>
          {#if c.verify_kind === 'command'}
            <input class="in grow" bind:value={c.verify} placeholder="what this checks (for humans)" />
          {/if}
        </div>
      {/each}
      <button class="btn small" onclick={addCriterion}>+ Add criterion</button>
    </section>

    <section class="block">
      <div class="lbl">Budget</div>
      <div class="budget">
        <label>Max iterations <input class="in num" type="number" min="1" bind:value={maxIterations} /></label>
        <label>Max minutes <input class="in num" type="number" min="1" bind:value={maxMinutes} /></label>
        <label>Per-phase minutes <input class="in num" type="number" min="1" bind:value={perPhaseMinutes} /></label>
        <label>Executors <input class="in num" type="number" min="1" max="6" bind:value={executorCount} /></label>
      </div>
      <div class="lbl">Executor agent</div>
      <div class="budget">
        <label>Provider
          <select class="in num prov" bind:value={execProvider}>
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
      <label class="lbl"><input type="checkbox" bind:checked={allowCommits} disabled={mode === 'research'} /> Allow local commits</label>
      <p class="muted small">Working files are retained when a loop stops or finishes. Push and publishing require separate authorization.</p>
      <label class="lbl"><input type="checkbox" bind:checked={requireReview} /> Require independent completion review</label>
      <label class="lbl" for="gl-sources">Source, spec and plan links (one per line)</label>
      <textarea id="gl-sources" class="in area" bind:value={sourceLinks}></textarea>
      <label class="lbl" for="gl-skills">Selected skills (comma separated)</label>
      <input id="gl-skills" class="in" bind:value={selectedSkills} />
      {#each ['planner', 'evaluator', 'digester'] as role}
        {@const key = role as 'planner' | 'evaluator' | 'digester'}
        <label class="lbl" for={`gl-${role}`}>{role} provider</label>
        <select id={`gl-${role}`} class="in" bind:value={draft.suggested_config[key].provider} onchange={() => { if (draft) draft.suggested_config[key].model = ''; }}>
          {#each providers as p (p)}<option value={p}>{p}</option>{/each}
        </select>
        <ModelPicker provider={draft.suggested_config[key].provider} value={draft.suggested_config[key].model} onchange={(m) => { if (draft) draft.suggested_config[key].model = m; }} />
      {/each}
    </section>
    <div class="row end">
      <button class="btn primary" onclick={launch} disabled={launching || !canLaunch()}>
        {launching ? 'Launching…' : 'Launch loop'}
      </button>
    </div>
  {/if}
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
  .form {
    padding: 18px 22px;
    max-width: 760px;
    overflow-y: auto;
    height: 100%;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 14px;
  }
  h1 {
    font-size: 18px;
    margin: 0;
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
    font-size: 12px;
    font-weight: 600;
    margin: 10px 0 5px;
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
    font-size: 11.5px;
  }
  .in {
    width: 100%;
    box-sizing: border-box;
    padding: 6px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12.5px;
  }
  .area {
    resize: vertical;
    font-family: inherit;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 10px;
  }
  .row.end {
    justify-content: flex-end;
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
    width: 110px;
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
    font-size: 11.5px;
    color: var(--text-dim);
  }
</style>
