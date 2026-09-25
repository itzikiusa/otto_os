<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Root-only defaults for the Skills Evaluator: the validations, improver
  // agent, iterations, and validation passes pre-filled into the start form.
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type { SkillEvalConfig, SkillEvalValidationCfg } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';

  const providerOpts = $derived(agentProviders());

  let cfg: SkillEvalConfig | null = $state(null);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  // Last loaded/saved form, so Save stays disabled until something changed.
  let savedKey = $state('');
  function formKey(c: SkillEvalConfig): string {
    return JSON.stringify([c.iterations, c.validator_passes, c.improver.provider, c.validations]);
  }
  const dirty = $derived(!!cfg && formKey(cfg) !== savedKey);
  // Inline validation (not a toast): every validation needs a name, criteria
  // and at least one CLI, and the numbers must be in range.
  const formError = $derived.by(() => {
    if (!cfg) return '';
    const whole = (n: unknown, lo: number, hi: number) => typeof n === 'number' && Number.isInteger(n) && n >= lo && n <= hi;
    if (!whole(cfg.iterations, 1, 10)) return 'Iterations must be a whole number from 1 to 10.';
    if (!whole(cfg.validator_passes, 1, 3)) return 'Validation passes must be a whole number from 1 to 3.';
    const i = cfg.validations.findIndex((v) => !v.name.trim() || !v.criteria.trim());
    if (i >= 0) return `Validation ${i + 1} needs a name and criteria.`;
    const j = cfg.validations.findIndex((v) => v.providers.length === 0);
    if (j >= 0) return `Validation “${cfg.validations[j].name || j + 1}” needs at least one agent CLI.`;
    return '';
  });

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      cfg = await skillsEvalApi.getConfig();
      savedKey = formKey(cfg);
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function addValidation(): void {
    if (!cfg) return;
    cfg.validations = [
      ...cfg.validations,
      { name: '', criteria: '', providers: [defaultAgentProvider()], model: '' },
    ];
  }
  function removeValidation(i: number): void {
    if (!cfg) return;
    cfg.validations = cfg.validations.filter((_, idx) => idx !== i);
  }
  function toggleProvider(i: number, p: string): void {
    if (!cfg) return;
    const v = cfg.validations[i];
    const next = v.providers.includes(p)
      ? v.providers.filter((x) => x !== p)
      : [...v.providers, p];
    cfg.validations = cfg.validations.map((vv, idx) => (idx === i ? { ...vv, providers: next } : vv));
  }

  async function save(): Promise<void> {
    if (!cfg || saving || formError || !dirty) return;
    saving = true;
    try {
      // Spread the loaded config first: PUT stores the body as-is and serde
      // fills anything missing with its default, so sending only the fields
      // on this page silently reset weights, promote_min_score,
      // require_proof_pass and the default test/lint commands on every save.
      const body: SkillEvalConfig = {
        ...$state.snapshot(cfg),
        validations: cfg.validations.map((v: SkillEvalValidationCfg) => ({
          name: v.name.trim(),
          criteria: v.criteria.trim(),
          providers: v.providers,
          model: v.model ?? '',
        })),
        improver: { provider: cfg.improver.provider, model: cfg.improver.model ?? '' },
        iterations: Math.max(1, Math.min(10, Math.floor(cfg.iterations))),
        validator_passes: Math.max(1, Math.min(3, Math.floor(cfg.validator_passes))),
      };
      cfg = await skillsEvalApi.putConfig(body);
      savedKey = formKey(cfg);
      toasts.success('Skills evaluator defaults saved', 'New runs start from these.');
    } catch (e) {
      toasts.error('Couldn’t save the evaluator defaults', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="settings-section">
  <PageHeader
    title={sectionLabel('skill-eval')}
    subtitle="Defaults pre-filled into the start form"
  >
    {#snippet actions()}
      {#if cfg}
        <button
          class="btn small primary"
          disabled={saving || !dirty || !!formError}
          title={formError || (dirty ? 'Save these defaults' : 'No unsaved changes')}
          onclick={save}
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <SectionIntro>Each validation runs as its own agent (one per CLI selected); the improver edits the skill between iterations. The start form in Skills Lab is pre-filled from these, and you can change them per run.</SectionIntro>
  <div class="eval-body">

  {#if !cfg}
    <LoadState what="Skills evaluator defaults" {loading} error={loadError} empty onretry={() => void load()} rows={3} />
  {:else}
    {#if formError}
      <p class="form-note error" role="alert">{formError}</p>
    {:else if dirty}
      <p class="form-note unsaved">Unsaved changes — Save to apply them.</p>
    {/if}
    <h2 class="section-title">Run</h2>
    <section class="card row3">
      <div class="field">
        <label for="sv-iter">Iterations</label>
        <input id="sv-iter" class="input" type="number" min="1" max="10" bind:value={cfg.iterations} />
        <span class="hint">1–10</span>
      </div>
      <div class="field">
        <label for="sv-passes">Validation passes</label>
        <input id="sv-passes" class="input" type="number" min="1" max="3" bind:value={cfg.validator_passes} />
        <span class="hint">1–3</span>
      </div>
      <div class="field">
        <label for="sv-imp">Improver agent</label>
        <select id="sv-imp" class="input" bind:value={cfg.improver.provider}>
          {#each providerOpts as p (p)}<option value={p}>{p}</option>{/each}
        </select>
        <span class="hint">Runs with the provider's default model.</span>
      </div>
    </section>

    <section class="block">
      <div class="block-head">
        <h2 class="section-title">Default validations</h2>
        <span class="count">{cfg.validations.length}</span>
        <span class="grow"></span>
        <button class="btn small" data-icon="plus" onclick={addValidation}><Icon name="plus" size={12} /> Add validation</button>
      </div>
      {#each cfg.validations as v, i (i)}
        <div class="val card">
          <div class="row">
            <input
              class="input grow"
              placeholder="logging"
              aria-label={`Validation ${i + 1} name`}
              bind:value={v.name}
            />
            <button
              class="icon-btn danger-icon"
              onclick={() => removeValidation(i)}
              aria-label={`Remove validation ${v.name || i + 1}`}
              title={`Remove validation ${v.name || i + 1}`}
            >
              <Icon name="trash" size={14} />
            </button>
          </div>
          <textarea
            class="input"
            rows="2"
            placeholder="Logs use the skill's conventions and never leak secrets."
            aria-label={`Validation ${v.name || i + 1} criteria`}
            bind:value={v.criteria}
          ></textarea>
          <div class="chips" role="group" aria-label={`Agent CLIs for ${v.name || `validation ${i + 1}`}`}>
            {#each providerOpts as p (p)}
              <button
                type="button"
                class="pill-toggle"
                class:on={v.providers.includes(p)}
                aria-pressed={v.providers.includes(p)}
                onclick={() => toggleProvider(i, p)}
              >
                {#if v.providers.includes(p)}<Icon name="check" size={12} />{/if}<span class="mono">{p}</span>
              </button>
            {/each}
          </div>
        </div>
      {:else}
        <div class="empty">
          No default validations. Runs will only use the ones you add in the start form.
        </div>
      {/each}
    </section>
  {/if}
  </div>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .eval-body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 880px;
  }
  .section-title {
    margin: 8px 0 0;
  }
  .form-note {
    margin: 0;
    font-size: var(--fs-s);
  }
  .form-note.unsaved {
    color: var(--warning);
  }
  .form-note.error {
    color: var(--danger);
  }
  .row3 {
    display: grid;
    grid-template-columns: 140px 140px minmax(0, 240px);
    gap: 16px;
    padding: 12px 16px;
  }
  .row3 .field {
    margin: 0;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }
  .block-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .block-head .section-title {
    margin: 0;
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .val {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pill-toggle :global(svg) {
    flex-shrink: 0;
  }
  .danger-icon:hover {
    color: var(--danger);
  }
  textarea.input {
    resize: vertical;
    font-size: var(--fs-s);
  }
  .empty {
    padding: 16px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-m);
    font-size: var(--fs-s);
    color: var(--text-dim);
    text-align: center;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .mono {
    font-family: var(--font-mono);
  }
  @media (max-width: 640px) {
    .row3 {
      grid-template-columns: 1fr 1fr;
    }
  }
</style>
