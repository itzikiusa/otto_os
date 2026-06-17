<script lang="ts">
  // Root-only defaults for the Skills Evaluator: the validations, improver
  // agent, iterations, and validation passes pre-filled into the start form.
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type { SkillEvalConfig, SkillEvalValidationCfg } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';

  const providerOpts = $derived.by(() => {
    const ps = (auth.meta?.providers ?? []).filter((p) => p !== 'shell');
    return ps.length > 0 ? ps : ['claude', 'codex', 'agy'];
  });

  let cfg: SkillEvalConfig | null = $state(null);
  let loading = $state(true);
  let saving = $state(false);

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      cfg = await skillsEvalApi.getConfig();
    } catch (e) {
      toasts.error('Could not load settings', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  function addValidation(): void {
    if (!cfg) return;
    cfg.validations = [
      ...cfg.validations,
      { name: '', criteria: '', providers: [providerOpts[0] ?? 'claude'], model: '' },
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
    if (!cfg || saving) return;
    saving = true;
    try {
      const body: SkillEvalConfig = {
        validations: cfg.validations.map((v: SkillEvalValidationCfg) => ({
          name: v.name.trim(),
          criteria: v.criteria.trim(),
          providers: v.providers,
          model: v.model ?? '',
        })),
        improver: { provider: cfg.improver.provider, model: cfg.improver.model ?? '' },
        iterations: Math.max(1, Math.floor(cfg.iterations)),
        validator_passes: Math.max(1, Math.min(3, Math.floor(cfg.validator_passes))),
      };
      cfg = await skillsEvalApi.putConfig(body);
      toasts.success('Skills Evaluator defaults saved');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="page">
  <h2>Skills Evaluator</h2>
  <p class="lede">
    Defaults pre-filled into the start form. Each validation runs as its own agent (one per CLI
    selected); the improver edits the skill between iterations.
  </p>

  {#if loading || !cfg}
    <p class="muted">Loading…</p>
  {:else}
    <section class="card row3">
      <div>
        <label class="field-label" for="sv-iter">Iterations</label>
        <input id="sv-iter" class="input" type="number" min="1" max="10" bind:value={cfg.iterations} />
      </div>
      <div>
        <label class="field-label" for="sv-passes">Validation passes</label>
        <input id="sv-passes" class="input" type="number" min="1" max="3" bind:value={cfg.validator_passes} />
      </div>
      <div>
        <label class="field-label" for="sv-imp">Improver agent</label>
        <select id="sv-imp" class="input" bind:value={cfg.improver.provider}>
          {#each providerOpts as p (p)}<option value={p}>{p}</option>{/each}
        </select>
      </div>
    </section>

    <section class="card block">
      <div class="block-head">
        <span class="field-label">Default validations</span>
        <span class="grow"></span>
        <button class="btn small" onclick={addValidation}><Icon name="plus" size={13} /> Add</button>
      </div>
      {#each cfg.validations as v, i (i)}
        <div class="val card">
          <div class="row">
            <input class="input grow" placeholder="name (e.g. logging)" bind:value={v.name} />
            <button class="btn small ghost danger" onclick={() => removeValidation(i)} title="Remove">
              <Icon name="trash" size={13} />
            </button>
          </div>
          <textarea class="input" rows="2" placeholder="What to check and how to judge it" bind:value={v.criteria}></textarea>
          <div class="chips">
            {#each providerOpts as p (p)}
              <label class="chip-toggle" class:on={v.providers.includes(p)}>
                <input type="checkbox" checked={v.providers.includes(p)} onchange={() => toggleProvider(i, p)} />
                <span class="mono">{p}</span>
              </label>
            {/each}
          </div>
        </div>
      {/each}
    </section>

    <div class="actions">
      <button class="btn primary" disabled={saving} onclick={save}>{saving ? 'Saving…' : 'Save'}</button>
    </div>
  {/if}
</div>

<style>
  .page {
    max-width: 720px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  .lede,
  .muted {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .row3 {
    display: grid;
    grid-template-columns: 110px 110px 1fr;
    gap: 12px;
    padding: 12px 14px;
  }
  .block {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .block-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .field-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
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
  .chips {
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
  textarea.input {
    resize: vertical;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
  }
  .grow {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
</style>
