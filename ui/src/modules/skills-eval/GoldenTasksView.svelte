<script lang="ts">
  // Golden tasks: the per-repo evaluation corpus + saved regression cases. Each
  // task is a reusable prompt paired with the commands that decide whether the
  // result is good (test/lint/build) and an optional rubric. From here you can
  // add/edit/delete a task and kick off a single score-only run against the
  // workspace's working tree — the result opens in the run detail via `onopenrun`.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type { GoldenTask, GoldenTaskReq, SkillEval } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';

  let { onopenrun } = $props<{ onopenrun?: (e: SkillEval) => void }>();

  let tasks: GoldenTask[] = $state([]);
  let loading = $state(true);
  let runningId: string | null = $state(null);

  // Inline create/edit form. `editingId === null` while creating.
  let showForm = $state(false);
  let editingId: string | null = $state(null);
  let saving = $state(false);
  let fName = $state('');
  let fPrompt = $state('');
  let fSkill = $state('');
  let fTest = $state('');
  let fLint = $state('');
  let fRubric = $state('');

  // Reload whenever the active workspace changes.
  $effect(() => {
    const wsId = ws.currentId;
    if (wsId) {
      void load(wsId);
    } else {
      tasks = [];
      loading = false;
    }
  });

  async function load(wsId: string): Promise<void> {
    loading = true;
    try {
      tasks = await skillsEvalApi.listGolden(wsId);
    } catch (e) {
      toasts.error('Could not load golden tasks', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // Group by repo so a workspace with several repos reads cleanly.
  const groups = $derived.by(() => {
    const map = new Map<string, GoldenTask[]>();
    for (const t of tasks) {
      const key = t.repo_key || '(unknown)';
      const arr = map.get(key) ?? [];
      arr.push(t);
      map.set(key, arr);
    }
    return [...map.entries()].map(([repoKey, items]) => ({ repoKey, items }));
  });

  function repoLabel(key: string): string {
    return key === ws.currentId ? 'this workspace' : key;
  }

  function truncate(s: string, n = 140): string {
    return s.length > n ? `${s.slice(0, n).trimEnd()}…` : s;
  }

  function resetFields(): void {
    fName = '';
    fPrompt = '';
    fSkill = '';
    fTest = '';
    fLint = '';
    fRubric = '';
  }

  function toggleCreate(): void {
    if (showForm && editingId === null) {
      closeForm();
      return;
    }
    editingId = null;
    resetFields();
    showForm = true;
  }

  function openEdit(t: GoldenTask): void {
    editingId = t.id;
    fName = t.name;
    fPrompt = t.prompt;
    fSkill = t.skill;
    fTest = t.test_cmd;
    fLint = t.lint_cmd;
    fRubric = t.rubric;
    showForm = true;
  }

  function closeForm(): void {
    showForm = false;
    editingId = null;
    resetFields();
  }

  async function submit(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || saving) return;
    if (!fName.trim() || !fPrompt.trim()) {
      toasts.error('Missing fields', 'Name and prompt are both required.');
      return;
    }
    saving = true;
    const body: GoldenTaskReq = {
      name: fName.trim(),
      prompt: fPrompt.trim(),
      skill: fSkill.trim() || undefined,
      test_cmd: fTest.trim() || undefined,
      lint_cmd: fLint.trim() || undefined,
      rubric: fRubric.trim() || undefined,
    };
    try {
      if (editingId) {
        const updated = await skillsEvalApi.updateGolden(editingId, body);
        tasks = tasks.map((t) => (t.id === updated.id ? updated : t));
        toasts.success('Golden task updated', updated.name);
      } else {
        const created = await skillsEvalApi.createGolden(wsId, body);
        tasks = [created, ...tasks];
        toasts.success('Golden task added', created.name);
      }
      closeForm();
    } catch (e) {
      toasts.error('Could not save golden task', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function run(t: GoldenTask): Promise<void> {
    if (runningId) return;
    runningId = t.id;
    try {
      const result = await skillsEvalApi.runGolden(t.id, {
        mode: 'score_only',
        target: { kind: 'working' },
      });
      onopenrun?.(result);
      toasts.success('Evaluation started', `Scoring “${t.name}” against the working tree.`);
    } catch (e) {
      toasts.error('Could not run golden task', e instanceof Error ? e.message : String(e));
    } finally {
      runningId = null;
    }
  }

  async function remove(t: GoldenTask): Promise<void> {
    if (!window.confirm(`Delete golden task “${t.name}”? This cannot be undone.`)) return;
    try {
      await skillsEvalApi.deleteGolden(t.id);
      tasks = tasks.filter((x) => x.id !== t.id);
      if (editingId === t.id) closeForm();
      toasts.success('Golden task deleted', t.name);
    } catch (e) {
      toasts.error('Could not delete golden task', e instanceof Error ? e.message : String(e));
    }
  }

  const canSave = $derived(!saving && fName.trim().length > 0 && fPrompt.trim().length > 0);
</script>

<div class="gt" data-testid="golden-tasks">
  <header class="gt-head">
    <h2>Golden tasks</h2>
    <span class="grow"></span>
    <button
      class="btn small primary"
      data-testid="golden-new-btn"
      onclick={toggleCreate}
      disabled={!ws.currentId}
    >
      <Icon name="plus" size={13} /> New golden task
    </button>
  </header>

  <div class="gt-hint">
    <Icon name="info" size={13} />
    <span>
      Golden tasks are reusable, per-repo evaluation cases — a prompt plus a test command.
      Failed evals can be saved here as regression cases.
    </span>
  </div>

  {#if showForm}
    <form class="gt-form card" onsubmit={(e: SubmitEvent) => { e.preventDefault(); void submit(); }}>
      <div class="gt-form-head">
        <span class="field-label">{editingId ? 'Edit golden task' : 'New golden task'}</span>
        <span class="grow"></span>
        <button type="button" class="btn small ghost" onclick={closeForm} title="Close">
          <Icon name="x" size={12} />
        </button>
      </div>

      <label class="field-label" for="gt-name">Name</label>
      <input
        id="gt-name"
        class="input"
        data-testid="golden-name"
        placeholder="e.g. Add a player bonus-balance endpoint"
        bind:value={fName}
      />

      <label class="field-label" for="gt-prompt">Prompt</label>
      <textarea
        id="gt-prompt"
        class="input"
        rows="3"
        data-testid="golden-prompt"
        placeholder="What the agent should implement…"
        bind:value={fPrompt}
      ></textarea>

      <div class="gt-grid">
        <div>
          <label class="field-label" for="gt-skill">Skill</label>
          <input id="gt-skill" class="input" placeholder="skill name (optional)" bind:value={fSkill} />
        </div>
        <div>
          <label class="field-label" for="gt-test">Test command</label>
          <input
            id="gt-test"
            class="input"
            data-testid="golden-test-cmd"
            placeholder="cargo test · npm test"
            bind:value={fTest}
          />
        </div>
        <div>
          <label class="field-label" for="gt-lint">Lint command</label>
          <input id="gt-lint" class="input" placeholder="cargo clippy · npm run check" bind:value={fLint} />
        </div>
      </div>

      <label class="field-label" for="gt-rubric">Rubric</label>
      <textarea
        id="gt-rubric"
        class="input"
        rows="2"
        placeholder="How to judge a passing result (optional)"
        bind:value={fRubric}
      ></textarea>

      <div class="gt-form-actions">
        <span class="grow"></span>
        <button type="button" class="btn small ghost" onclick={closeForm}>Cancel</button>
        <button class="btn small primary" data-testid="golden-save" disabled={!canSave}>
          {saving ? 'Saving…' : editingId ? 'Save changes' : 'Add golden task'}
        </button>
      </div>
    </form>
  {/if}

  <div class="gt-body">
    {#if !ws.currentId}
      <EmptyState icon="zap" title="No workspace selected" body="Pick a workspace to manage its golden tasks." />
    {:else if loading && tasks.length === 0}
      <div class="gt-muted">Loading…</div>
    {:else if tasks.length === 0}
      <EmptyState
        icon="radar"
        title="No golden tasks"
        body="Add a golden task — a repo-specific prompt + test command — to evaluate skills against it."
        actionLabel="New golden task"
        onaction={toggleCreate}
      />
    {:else}
      {#each groups as g (g.repoKey)}
        <section class="gt-group">
          <h3 class="gt-group-head">
            <Icon name="db" size={12} />
            {repoLabel(g.repoKey)}
            <span class="gt-count">{g.items.length}</span>
          </h3>
          {#each g.items as t (t.id)}
            <article class="gt-card" data-testid="golden-card" class:disabled={!t.enabled}>
              <div class="gt-card-top">
                <span class="gt-name">{t.name}</span>
                {#if t.origin === 'regression'}
                  <span class="gt-badge regression" data-testid="golden-regression-badge">regression</span>
                {/if}
                {#if !t.enabled}<span class="gt-badge muted">disabled</span>{/if}
                <span class="grow"></span>
                <div class="gt-actions">
                  <button
                    class="btn small gt-run"
                    data-testid="golden-run"
                    onclick={() => run(t)}
                    disabled={runningId === t.id}
                  >
                    <Icon name="play" size={12} /> {runningId === t.id ? 'Running…' : 'Run'}
                  </button>
                  <button class="btn small ghost" onclick={() => openEdit(t)} title="Edit">
                    <Icon name="edit" size={12} /> Edit
                  </button>
                  <button class="btn small ghost danger" onclick={() => remove(t)} title="Delete">
                    <Icon name="trash" size={12} />
                  </button>
                </div>
              </div>

              <div class="gt-meta">
                {#if t.skill}<span class="gt-pill"><Icon name="zap" size={11} /> {t.skill}</span>{/if}
                {#if t.test_cmd}<code class="gt-code">{t.test_cmd}</code>{/if}
              </div>

              {#if t.prompt}<p class="gt-prompt">{truncate(t.prompt)}</p>{/if}

              {#if t.tags.length > 0}
                <div class="gt-tags">
                  {#each t.tags as tag (tag)}
                    <span class="gt-tag"><Icon name="tag" size={10} /> {tag}</span>
                  {/each}
                </div>
              {/if}
            </article>
          {/each}
        </section>
      {/each}
    {/if}
  </div>
</div>

<style>
  .gt {
    height: 100%;
    overflow-y: auto;
    padding: 16px 20px 60px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    box-sizing: border-box;
  }
  .gt-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .gt-head h2 {
    margin: 0;
    font-size: 16px;
  }
  .grow {
    flex: 1;
  }
  .gt-hint {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 11px;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .field-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }

  /* Form */
  .gt-form {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .gt-form-head {
    display: flex;
    align-items: center;
    margin-bottom: 2px;
  }
  .gt-grid {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 10px;
  }
  .gt-grid > div {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .gt-form-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  textarea.input {
    resize: vertical;
  }

  /* Groups + cards */
  .gt-body {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .gt-muted {
    padding: 16px 4px;
    color: var(--text-dim);
    font-size: 12px;
  }
  .gt-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .gt-group-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .gt-count {
    font-weight: 600;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    border-radius: 999px;
    padding: 0 6px;
    font-size: 10px;
  }
  .gt-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 7px;
    background: var(--surface);
  }
  .gt-card.disabled {
    opacity: 0.6;
  }
  .gt-card-top {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .gt-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .gt-actions {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }
  /* Run / pass affordance: high-contrast light-green. */
  .btn.gt-run {
    color: #7ee787;
    border-color: color-mix(in srgb, #7ee787 45%, transparent);
  }
  .btn.gt-run:hover {
    background: color-mix(in srgb, #7ee787 16%, transparent);
  }
  .gt-badge {
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 7px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .gt-badge.regression {
    color: #f5a623;
    background: color-mix(in srgb, #f5a623 20%, transparent);
    border: 1px solid color-mix(in srgb, #f5a623 42%, transparent);
  }
  .gt-badge.muted {
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    border: 1px solid var(--border);
  }
  .gt-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
  }
  .gt-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 8px;
  }
  .gt-code {
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    border-radius: var(--radius-s, 5px);
    padding: 1px 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .gt-prompt {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .gt-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .gt-tag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    color: var(--text-dim);
  }
</style>
