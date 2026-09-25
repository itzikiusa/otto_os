<script lang="ts">
  // The "new evaluation" form: pick a skill (library / provider / a path or
  // archive), describe the task, choose the implementation CLI + iterations,
  // add validation dimensions (each fanned across one or more agent CLIs), and
  // pick the improver agent. Prefilled from the saved defaults (/settings/skill-eval).
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type {
    SkillEvalValidationCfg,
    SkillSourceInfo,
    StartSkillEvalReq,
  } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';

  interface Props {
    starting: boolean;
    onstart: (req: StartSkillEvalReq) => void;
    /** Pre-select this skill ("Evaluate <skill>" from the Skills tab). `source`
     *  is "library" | "bundled" | a provider name. */
    initialSkill?: { name: string; source: string } | null;
  }
  let { starting, onstart, initialSkill = null }: Props = $props();

  // The matching source row: the same copy when listed (a provider copy, or
  // the library), else any copy with that name. Bundled skills aren't eval
  // sources, so a bundled hand-off falls back to the library copy if present.
  function matchSource(list: SkillSourceInfo[], want: { name: string; source: string }): number {
    const isProvider = want.source !== 'library' && want.source !== 'bundled';
    let i = list.findIndex((s) =>
      s.name === want.name && (isProvider ? s.kind === 'provider' && s.provider === want.source : s.kind === 'library'),
    );
    if (i < 0) i = list.findIndex((s) => s.name === want.name);
    return i;
  }

  // Agent CLIs available for implementation / validation / improvement.
  const providerOpts = $derived(agentProviders());

  let sources: SkillSourceInfo[] = $state([]);
  // 'custom' = the path/archive branch; otherwise an index into `sources`.
  let sourceSel = $state<'custom' | number>('custom');
  let customPath = $state('');
  let showPicker = $state(false);

  let task = $state('');
  let implCli = $state(defaultAgentProvider());
  let iterations = $state(2);
  let validatorPasses = $state(1);
  let improverProvider = $state(defaultAgentProvider());
  let baseRef = $state('');
  // Repo-specific commands → scored as tests/lint signals + proof-pack evidence.
  let testCmd = $state('');
  let lintCmd = $state('');
  // Saved defaults (Settings → Skill eval). A blank field falls back to these
  // on the daemon, so the placeholder shows what will actually run.
  let testDefault = $state('');
  let lintDefault = $state('');
  let validations: SkillEvalValidationCfg[] = $state([]);
  // Advanced options (commands, passes, improver, base ref) sit behind a
  // disclosure: most runs only need the skill, the task and validations.
  let advancedOpen = $state(false);
  const advancedSummary = $derived.by(() => {
    const parts: string[] = [];
    const t = testCmd.trim() || testDefault;
    parts.push(t ? `Tests: ${t}` : 'No test command');
    parts.push(`${Math.max(1, Math.floor(validatorPasses))} validation pass${Math.floor(validatorPasses) === 1 ? '' : 'es'}`);
    parts.push(`improver ${improverProvider}`);
    if (baseRef.trim()) parts.push(`from ${baseRef.trim()}`);
    return parts.join(' · ');
  });

  let loaded = $state(false);
  // A failed defaults/sources load is shown inline with Retry: the form would
  // otherwise look ready with no skills and no validations.
  let loadError = $state<string | null>(null);
  // The improver model saved in Settings → Skills evaluator (sent with the run
  // so the setting isn't silently dropped).
  let improverModel = $state('');

  $effect(() => {
    if (!loaded) void load();
  });

  async function load(): Promise<void> {
    loadError = null;
    try {
      const wsId = ws.currentId;
      const [cfg, src] = await Promise.all([
        skillsEvalApi.getConfig(),
        wsId ? skillsEvalApi.listSources(wsId) : Promise.resolve({ sources: [] }),
      ]);
      validations = cfg.validations.map((v) => ({ ...v, providers: [...v.providers] }));
      iterations = cfg.iterations ?? 2;
      validatorPasses = cfg.validator_passes ?? 1;
      testDefault = cfg.default_test_cmd ?? '';
      lintDefault = cfg.default_lint_cmd ?? '';
      sources = src.sources;
      implCli = defaultAgentProvider();
      improverProvider = cfg.improver?.provider || implCli;
      improverModel = cfg.improver?.model ?? '';
      const want = initialSkill ? matchSource(sources, initialSkill) : -1;
      if (want >= 0) sourceSel = want;
      else if (sources.length > 0) sourceSel = 0;
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
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
      loaded &&
      task.trim().length > 0 &&
      implCli.length > 0 &&
      (sourceSel === 'custom' ? customPath.trim().length > 0 : sources.length > 0) &&
      validations.length > 0 &&
      validations.every((v) => v.name.trim() && v.criteria.trim()),
  );

  // Why Start is disabled, for its tooltip (a greyed button with no reason
  // leaves the user hunting through the form).
  const blockReason = $derived.by(() => {
    if (starting) return 'Starting…';
    if (!loaded) return 'Loading the evaluator defaults…';
    if (sourceSel === 'custom' ? !customPath.trim() : sources.length === 0) return 'Choose the skill under test';
    if (!task.trim()) return 'Describe the task to implement';
    if (validations.length === 0) return 'Add at least one validation';
    if (!validations.every((v) => v.name.trim() && v.criteria.trim())) return 'Every validation needs a name and criteria';
    return '';
  });

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
      improver: { provider: improverProvider, model: improverProvider === improverDefaultProvider ? improverModel : '' },
      base_ref: baseRef.trim() || null,
      test_cmd: testCmd.trim() || null,
      lint_cmd: lintCmd.trim() || null,
    };
    onstart(req);
  }

  // The saved improver model belongs to the saved improver provider; picking a
  // different agent here falls back to that agent's default model.
  let improverDefaultProvider = $state('');
  $effect(() => {
    if (loaded && !improverDefaultProvider) improverDefaultProvider = improverProvider;
  });

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

  {#if loadError}
    <div class="load-err" role="alert">
      <Icon name="warning" size={14} />
      <div class="grow">
        <strong>Couldn't load the evaluator defaults.</strong>
        <span class="dim">The skill list and saved validations are missing until they load. {loadError}</span>
      </div>
      <button class="btn small" type="button" onclick={() => void load()}>Retry</button>
    </div>
  {/if}

  <!-- What to test: the skill and the task (the fields every run needs). -->
  <section class="card block">
    <div class="fld">
      <label class="field-label" for="se-source">Skill under test</label>
      <select id="se-source" class="input" bind:value={sourceSel} disabled={!loaded}>
        {#if !loaded}<option value="custom">Loading skills…</option>{/if}
        {#each sources as s, i (s.kind + s.name + (s.provider ?? ''))}
          <option value={i}>{sourceLabel(s)}</option>
        {/each}
        <option value="custom">Custom path or archive (.zip / .gz / .tgz)…</option>
      </select>

      {#if sourceSel === 'custom'}
        <div class="row">
          <input
            class="input grow"
            aria-label="Skill path or archive"
            placeholder="/path/to/skill-folder · SKILL.md · skill.zip"
            bind:value={customPath}
          />
          <button class="btn small" onclick={() => (showPicker = true)} type="button">
            <Icon name="folder" size={12} /> Browse…
          </button>
        </div>
      {:else if sources[sourceSel as number]?.description}
        <p class="hint clamp" title={sources[sourceSel as number].description}>{sources[sourceSel as number].description}</p>
      {/if}
    </div>

    <div class="fld">
      <label class="field-label" for="se-task">Task to implement</label>
      <textarea
        id="se-task"
        class="input"
        rows="3"
        placeholder="e.g. Add a new endpoint that returns a player's bonus balance history"
        bind:value={task}
      ></textarea>
    </div>

    <div class="grid2">
      <div class="fld">
        <label class="field-label" for="se-cli">Implementation agent</label>
        <select id="se-cli" class="input" bind:value={implCli}>
          {#each providerOpts as p (p)}<option value={p}>{p}</option>{/each}
        </select>
      </div>
      <div class="fld">
        <label class="field-label" for="se-iter">Iterations</label>
        <input id="se-iter" class="input" type="number" min="1" max="10" bind:value={iterations} />
      </div>
    </div>
  </section>

  <!-- Validations -->
  <section class="card block">
    <div class="block-head">
      <span class="field-label">Validations</span>
      <span class="grow"></span>
      <button class="btn small" onclick={addValidation} type="button">
        <Icon name="plus" size={12} /> Add validation
      </button>
    </div>
    {#if validations.length === 0}
      <p class="hint">Add at least one validation (e.g. logging, docs, naming). Each runs as its own agent.</p>
    {/if}
    {#each validations as v, i (i)}
      <div class="val">
        <div class="row">
          <input class="input grow" placeholder="logging" aria-label="Validation {i + 1} name" bind:value={v.name} />
          <button class="icon-btn" onclick={() => removeValidation(i)} type="button" title="Remove validation" aria-label="Remove validation {v.name.trim() || i + 1}">
            <Icon name="trash" size={14} />
          </button>
        </div>
        <textarea
          class="input"
          rows="2"
          aria-label="Validation {i + 1} criteria"
          placeholder="What to check and how to judge it (passed to the agent)"
          bind:value={v.criteria}
        ></textarea>
        <div class="provider-chips" role="group" aria-label="Agents that run validation {i + 1}">
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

  <!-- Advanced: scoring commands, validation passes, improver, base ref. -->
  <section class="card adv">
    <button class="adv-toggle" type="button" aria-expanded={advancedOpen} aria-controls="se-advanced" onclick={() => (advancedOpen = !advancedOpen)}>
      <Icon name={advancedOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      <span>Advanced</span>
      <span class="adv-sum dim">{advancedSummary}</span>
    </button>
    {#if advancedOpen}
      <div class="block adv-body" id="se-advanced">
        <div class="grid2">
          <div class="fld">
            <label class="field-label" for="se-test">Test command <span class="hint-inline">scored and added to the proof pack</span></label>
            <input id="se-test" class="input" bind:value={testCmd} placeholder={testDefault ? `Default: ${testDefault}` : 'e.g. cargo test  /  npm test'} data-testid="eval-test-cmd" />
          </div>
          <div class="fld">
            <label class="field-label" for="se-lint">Lint command <span class="hint-inline">optional</span></label>
            <input id="se-lint" class="input" bind:value={lintCmd} placeholder={lintDefault ? `Default: ${lintDefault}` : 'e.g. cargo clippy  /  npm run check'} data-testid="eval-lint-cmd" />
          </div>
          <div class="fld">
            <label class="field-label" for="se-passes">Validation passes</label>
            <input id="se-passes" class="input" type="number" min="1" max="3" bind:value={validatorPasses} />
          </div>
          <div class="fld">
            <label class="field-label" for="se-imp">Improver agent</label>
            <select id="se-imp" class="input" bind:value={improverProvider}>
              {#each providerOpts as p (p)}<option value={p}>{p}</option>{/each}
            </select>
          </div>
        </div>
        <div class="fld">
          <label class="field-label" for="se-base">Base git ref</label>
          <input id="se-base" class="input" placeholder="HEAD" bind:value={baseRef} />
          <p class="hint">
            Each iteration's worktree is created from this ref of the workspace's git repo. If the
            workspace root isn't a git repo, Otto uses a scratch repo at <span class="mono">~/Otto/SkillsEvaluator</span>
            (created automatically).
          </p>
        </div>
      </div>
    {/if}
  </section>

  <div class="actions">
    <span class="cost" title="Approximate — improver runs are skipped on a perfect score">
      ≈ {estAgents} agent session{estAgents === 1 ? '' : 's'}
    </span>
    <span class="grow"></span>
    <button class="btn primary" disabled={!canStart} onclick={submit} title={canStart ? undefined : blockReason}>
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
  /* Left-aligned readable column (never a centred island); the scroller spans
     the whole pane so its scrollbar sits at the pane edge. */
  .form-wrap > :global(*) {
    max-width: 760px;
    width: 100%;
    box-sizing: border-box;
  }
  .form-wrap {
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
    font-size: var(--fs-l);
  }
  .lede {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .block {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .grid2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
    /* Bottom-align so the inputs line up even when a label wraps. */
    align-items: end;
  }
  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .adv {
    display: flex;
    flex-direction: column;
  }
  .adv-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 40px;
    padding: 0 14px;
    border: none;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-weight: 500;
    text-align: start;
    cursor: pointer;
  }
  .adv-toggle:hover {
    background: var(--hover);
  }
  .adv-toggle > :global(svg) {
    color: var(--text-dim);
    flex: none;
  }
  .adv-sum {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .adv-body {
    padding-top: 4px;
    border-top: 1px solid var(--border);
  }
  .hint-inline {
    font-weight: 400;
    font-size: var(--fs-xs);
  }
  .hint-inline::before {
    content: '· ';
  }
  .cost {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    align-self: center;
  }
  /* Same field-label style as every other Otto form (app.css .field > label). */
  .field-label {
    font-size: var(--fs-s);
    font-weight: 500;
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
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .provider-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip-toggle {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: var(--fs-xs);
    cursor: pointer;
    user-select: none;
  }
  .chip-toggle.on {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
  }
  /* Visually hidden but still focusable (display:none dropped the chips out
     of the tab order); the label shows the focus ring instead. */
  .chip-toggle input {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    border: 0;
  }
  .chip-toggle:has(input:focus-visible) {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
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
  .load-err {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  .load-err > :global(svg) {
    color: var(--danger);
    flex: none;
    margin-top: 2px;
  }
  .dim {
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .grid2 {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
