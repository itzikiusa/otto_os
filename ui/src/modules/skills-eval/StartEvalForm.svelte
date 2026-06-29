<script lang="ts">
  // The "new evaluation" form: pick a skill (library / provider / a path or
  // archive), describe the task, choose the implementation CLI + iterations,
  // add validation dimensions (each fanned across one or more agent CLIs), and
  // pick the improver agent. Prefilled from the saved defaults (/settings/skill-eval).
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type {
    SkillEvalValidationCfg,
    SkillSourceInfo,
    StartSkillEvalReq,
  } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';

  interface Props {
    starting: boolean;
    onstart: (req: StartSkillEvalReq) => void;
  }
  let { starting, onstart }: Props = $props();

  // Agent CLIs available for implementation / validation / improvement.
  const providerOpts = $derived.by(() => {
    const ps = (auth.meta?.providers ?? []).filter((p) => p !== 'shell');
    return ps.length > 0 ? ps : ['claude', 'codex', 'agy'];
  });

  let sources: SkillSourceInfo[] = $state([]);
  // 'custom' = the path/archive branch; otherwise an index into `sources`.
  let sourceSel = $state<'custom' | number>('custom');
  let customPath = $state('');
  let showPicker = $state(false);

  let task = $state('');
  let implCli = $state('claude');
  let iterations = $state(2);
  let validatorPasses = $state(1);
  let improverProvider = $state('claude');
  let baseRef = $state('');
  // Repo-specific commands → scored as tests/lint signals + proof-pack evidence.
  let testCmd = $state('');
  let lintCmd = $state('');
  let validations: SkillEvalValidationCfg[] = $state([]);

  let loaded = $state(false);

  $effect(() => {
    if (!loaded) void load();
  });

  async function load(): Promise<void> {
    try {
      const wsId = ws.currentId;
      const [cfg, src] = await Promise.all([
        skillsEvalApi.getConfig(),
        wsId ? skillsEvalApi.listSources(wsId) : Promise.resolve({ sources: [] }),
      ]);
      validations = cfg.validations.map((v) => ({ ...v, providers: [...v.providers] }));
      iterations = cfg.iterations ?? 2;
      validatorPasses = cfg.validator_passes ?? 1;
      sources = src.sources;
      const def = providerOpts[0] ?? 'claude';
      implCli = providerOpts.includes('claude') ? 'claude' : def;
      improverProvider = cfg.improver?.provider || implCli;
      if (sources.length > 0) sourceSel = 0;
    } catch (e) {
      toasts.error('Could not load evaluator defaults', e instanceof Error ? e.message : String(e));
    } finally {
      loaded = true;
    }
  }

  function addValidation(): void {
    validations = [
      ...validations,
      { name: '', criteria: '', providers: [implCli], model: '' },
    ];
  }
  function removeValidation(i: number): void {
    validations = validations.filter((_, idx) => idx !== i);
  }
  function toggleValProvider(i: number, p: string): void {
    const v = validations[i];
    const has = v.providers.includes(p);
    const next = has ? v.providers.filter((x) => x !== p) : [...v.providers, p];
    validations = validations.map((vv, idx) => (idx === i ? { ...vv, providers: next } : vv));
  }

  const canStart = $derived(
    !starting &&
      task.trim().length > 0 &&
      implCli.length > 0 &&
      (sourceSel === 'custom' ? customPath.trim().length > 0 : sources.length > 0) &&
      validations.length > 0 &&
      validations.every((v) => v.name.trim() && v.criteria.trim()),
  );

  // Rough agent-session count so the user sees the scope before launching.
  // Per iteration: 1 implementation + Σ(validation providers) × passes; plus
  // one improver between iterations (worst case: no early perfect-score exit).
  const estAgents = $derived.by(() => {
    const iters = Math.max(1, Math.floor(iterations));
    const passes = Math.max(1, Math.floor(validatorPasses));
    const valsPerIter = validations.reduce(
      (sum, v) => sum + (v.providers.length > 0 ? v.providers.length : 1) * passes,
      0,
    );
    return iters * (1 + valsPerIter) + Math.max(0, iters - 1);
  });

  function submit(): void {
    let source: StartSkillEvalReq['source'];
    if (sourceSel === 'custom') {
      source = { kind: 'path', reference: customPath.trim() };
    } else {
      const s = sources[sourceSel as number];
      source = { kind: s.kind, reference: s.name, provider: s.provider ?? null };
    }
    const req: StartSkillEvalReq = {
      source,
      task: task.trim(),
      impl_cli: implCli,
      iterations: Math.max(1, Math.floor(iterations)),
      validator_passes: Math.max(1, Math.min(3, Math.floor(validatorPasses))),
      validations: validations.map((v) => ({
        name: v.name.trim(),
        criteria: v.criteria.trim(),
        providers: v.providers.length > 0 ? v.providers : [implCli],
        model: v.model ?? '',
      })),
      improver: { provider: improverProvider, model: '' },
      base_ref: baseRef.trim() || null,
      test_cmd: testCmd.trim() || null,
      lint_cmd: lintCmd.trim() || null,
    };
    onstart(req);
  }

  function sourceLabel(s: SkillSourceInfo): string {
    const origin = s.kind === 'provider' ? s.provider : 'library';
    return `${s.name}  ·  ${origin}`;
  }
</script>

<div class="form-wrap">
  <h2>New skill evaluation</h2>
  <p class="lede">
    A coding agent uses the skill to implement your task in a fresh git worktree, validation agents
    grade the result, and (between iterations) an improver edits a copy of the skill and re-runs —
    each round scored.
  </p>

  <!-- Skill source -->
  <section class="card block">
    <label class="field-label" for="se-source">Skill under test</label>
    <select id="se-source" class="input" bind:value={sourceSel}>
      {#each sources as s, i (s.kind + s.name + (s.provider ?? ''))}
        <option value={i}>{sourceLabel(s)}</option>
      {/each}
      <option value="custom">Custom path or archive (.zip / .gz / .tgz)…</option>
    </select>

    {#if sourceSel === 'custom'}
      <div class="row">
        <input
          class="input grow"
          placeholder="/path/to/skill-folder · SKILL.md · skill.zip"
          bind:value={customPath}
        />
        <button class="btn small" onclick={() => (showPicker = true)} type="button">
          <Icon name="folder" size={13} /> Browse
        </button>
      </div>
    {:else if sources[sourceSel as number]?.description}
      <p class="hint">{sources[sourceSel as number].description}</p>
    {/if}
  </section>

  <!-- Task -->
  <section class="card block">
    <label class="field-label" for="se-task">Task to implement</label>
    <textarea
      id="se-task"
      class="input"
      rows="3"
      placeholder="e.g. Add a new endpoint that returns a player's bonus balance history"
      bind:value={task}
    ></textarea>
  </section>

  <!-- Run knobs -->
  <section class="card block grid4">
    <div>
      <label class="field-label" for="se-cli">Implementation CLI</label>
      <select id="se-cli" class="input" bind:value={implCli}>
        {#each providerOpts as p (p)}<option value={p}>{p}</option>{/each}
      </select>
    </div>
    <div>
      <label class="field-label" for="se-iter">Iterations</label>
      <input id="se-iter" class="input" type="number" min="1" max="10" bind:value={iterations} />
    </div>
    <div>
      <label class="field-label" for="se-passes">Validation passes</label>
      <input id="se-passes" class="input" type="number" min="1" max="3" bind:value={validatorPasses} />
    </div>
    <div>
      <label class="field-label" for="se-imp">Improver agent</label>
      <select id="se-imp" class="input" bind:value={improverProvider}>
        {#each providerOpts as p (p)}<option value={p}>{p}</option>{/each}
      </select>
    </div>
  </section>

  <!-- Repo-specific scoring commands -->
  <section class="card block grid4">
    <div style="grid-column: span 2;">
      <label class="field-label" for="se-test">Test command <span class="hint-inline">(scored + proof)</span></label>
      <input id="se-test" class="input" bind:value={testCmd} placeholder="e.g. cargo test  /  npm test" data-testid="eval-test-cmd" />
    </div>
    <div style="grid-column: span 2;">
      <label class="field-label" for="se-lint">Lint command <span class="hint-inline">(optional)</span></label>
      <input id="se-lint" class="input" bind:value={lintCmd} placeholder="e.g. cargo clippy  /  npm run check" data-testid="eval-lint-cmd" />
    </div>
  </section>

  <!-- Validations -->
  <section class="card block">
    <div class="block-head">
      <span class="field-label">Validations</span>
      <span class="grow"></span>
      <button class="btn small" onclick={addValidation} type="button">
        <Icon name="plus" size={13} /> Add validation
      </button>
    </div>
    {#if validations.length === 0}
      <p class="hint">Add at least one validation (e.g. logging, docs, naming). Each runs as its own agent.</p>
    {/if}
    {#each validations as v, i (i)}
      <div class="val card">
        <div class="row">
          <input class="input grow" placeholder="name (e.g. logging)" bind:value={v.name} />
          <button class="btn small ghost danger" onclick={() => removeValidation(i)} type="button" title="Remove">
            <Icon name="trash" size={13} />
          </button>
        </div>
        <textarea
          class="input"
          rows="2"
          placeholder="What to check and how to judge it (passed to the agent)"
          bind:value={v.criteria}
        ></textarea>
        <div class="provider-chips">
          {#each providerOpts as p (p)}
            <label class="chip-toggle" class:on={v.providers.includes(p)}>
              <input
                type="checkbox"
                checked={v.providers.includes(p)}
                onchange={() => toggleValProvider(i, p)}
              />
              <span class="mono">{p}</span>
            </label>
          {/each}
        </div>
      </div>
    {/each}
  </section>

  <!-- Advanced -->
  <section class="card block">
    <label class="field-label" for="se-base">Base git ref (optional)</label>
    <input id="se-base" class="input" placeholder="HEAD" bind:value={baseRef} />
    <p class="hint">
      Each iteration's worktree is created from this ref of the workspace's git repo. If the
      workspace root isn't a git repo, Otto uses a scratch repo at <span class="mono">~/Otto/SkillsEvaluator</span>
      (created automatically).
    </p>
  </section>

  <div class="actions">
    <span class="cost" title="Approximate — improver runs are skipped on a perfect score">
      ≈ {estAgents} agent session{estAgents === 1 ? '' : 's'}
    </span>
    <span class="grow"></span>
    <button class="btn primary" disabled={!canStart} onclick={submit}>
      {starting ? 'Starting…' : 'Start evaluation'}
    </button>
  </div>
</div>

{#if showPicker}
  <FolderPicker
    title="Choose a skill folder, SKILL.md, or archive"
    files={true}
    start={customPath}
    onpick={(p) => {
      customPath = p;
      showPicker = false;
    }}
    onclose={() => (showPicker = false)}
  />
{/if}

<style>
  .form-wrap {
    max-width: 760px;
    margin: 0 auto;
    padding: 18px 20px 60px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    /* The parent `.se-main` is overflow:hidden (other views self-scroll), so the
       form must self-scroll too — otherwise the lower validations + the Run
       button are clipped and unreachable. Mirrors RunDetail/CompareView roots. */
    height: 100%;
    overflow-y: auto;
    box-sizing: border-box;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  .lede {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .block {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .grid4 {
    display: grid;
    grid-template-columns: 1fr 110px 110px 1fr;
    gap: 12px;
    /* Bottom-align so the inputs line up even when a label wraps to two lines
       (e.g. "Validation passes"). */
    align-items: end;
  }
  .grid4 > div {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .cost {
    font-size: 11.5px;
    color: var(--text-dim);
    align-self: center;
  }
  .field-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .block-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .val {
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .provider-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip-toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 11px;
    cursor: pointer;
    user-select: none;
  }
  .chip-toggle.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--accent);
  }
  .chip-toggle input {
    display: none;
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.45;
  }
  textarea.input {
    resize: vertical;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 4px;
  }
  .grow {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
</style>
