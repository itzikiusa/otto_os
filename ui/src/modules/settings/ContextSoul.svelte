<script lang="ts">
  // Context & Soul settings page: per-workspace context provisioning. Pick which
  // library skills are active, which soul (persona) to use, free-form extra
  // context, and whether to inline the workspace MEMORY.md. A "Materialize now"
  // button per provider (claude / codex) writes the resolved context into the
  // workspace's native CLI files on demand.
  import { contextApi } from '../../lib/api/context';
  import type {
    LibrarySkill,
    LibrarySoul,
    UpdateWorkspaceContextReq,
    WorkspaceContextConfig,
  } from '../../lib/api/types';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let cfg: WorkspaceContextConfig | null = $state(null);
  let skills: LibrarySkill[] = $state([]);
  let souls: LibrarySoul[] = $state([]);
  let loading = $state(false);
  let saving = $state(false);
  let materializing: string | null = $state(null);

  // "All active" vs explicit selection. When true, cfg.skills is null (every
  // library skill is active). When false, cfg.skills is the explicit list.
  let allSkills = $state(true);
  // Explicit selection (a Set of skill names) — only meaningful when !allSkills.
  let selectedSkills: Set<string> = $state(new Set());

  const wsId = $derived(ws.currentId);

  // Agent CLIs offered for materialization (from /meta), restricted to the two
  // providers this feature supports.
  const providers = $derived(
    (auth.meta?.providers ?? ['claude', 'codex']).filter(
      (p) => p === 'claude' || p === 'codex',
    ),
  );

  // ---------------------------------------------------------------------------
  // Load on workspace change
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (wsId) void load(wsId);
  });

  async function load(id: string): Promise<void> {
    loading = true;
    try {
      [cfg, skills, souls] = await Promise.all([
        contextApi.getWorkspaceContext(id),
        contextApi.listSkills(),
        contextApi.listSouls(),
      ]);
      allSkills = cfg.skills === null;
      selectedSkills = new Set(cfg.skills ?? []);
    } catch (e) {
      toasts.error('Could not load context & soul', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Skill selection helpers
  // ---------------------------------------------------------------------------

  function toggleAllSkills(checked: boolean): void {
    allSkills = checked;
    if (!checked && selectedSkills.size === 0) {
      // Switching to explicit selection: pre-select everything so the user
      // starts from "all" and trims down rather than from an empty set.
      selectedSkills = new Set(skills.map((s) => s.name));
    }
  }

  function toggleSkill(name: string, checked: boolean): void {
    const next = new Set(selectedSkills);
    if (checked) next.add(name);
    else next.delete(name);
    selectedSkills = next;
  }

  // ---------------------------------------------------------------------------
  // Save config
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    if (!wsId || !cfg) return;
    saving = true;
    try {
      const body: UpdateWorkspaceContextReq = {
        skills: allSkills ? null : [...selectedSkills],
        soul: cfg.soul,
        extra_context_md: cfg.extra_context_md,
        include_memory: cfg.include_memory,
      };
      cfg = await contextApi.updateWorkspaceContext(wsId, body);
      allSkills = cfg.skills === null;
      selectedSkills = new Set(cfg.skills ?? []);
      toasts.success('Context & soul saved', allSkills ? 'All skills active' : `${selectedSkills.size} skills active`);
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Materialize now
  // ---------------------------------------------------------------------------

  async function materialize(provider: string): Promise<void> {
    if (!wsId) return;
    materializing = provider;
    try {
      const resp = await contextApi.materialize(wsId, provider);
      const result = resp.provider_results.find((r) => r.provider === provider);
      if (!result || result.skipped) {
        toasts.info(`Materialize ${provider} skipped`, 'No files needed updating.');
      } else if (result.files_written.length === 0) {
        toasts.info(`Materialized ${provider}`, 'No files written.');
      } else {
        toasts.success(
          `Materialized ${provider}`,
          result.files_written.join(', '),
        );
      }
    } catch (e) {
      toasts.error(`Materialize ${provider} failed`, e instanceof Error ? e.message : String(e));
    } finally {
      materializing = null;
    }
  }
</script>

<div class="page">
  <!-- Header -->
  <div class="page-header">
    <div>
      <h1>Context &amp; Soul</h1>
      <div class="sub">
        Choose which library skills, soul (persona), and extra context Otto injects into the
        agents it spawns in this workspace. Saved config is materialized at the next session
        spawn, or immediately via “Materialize now”.
      </div>
    </div>
  </div>

  {#if !wsId}
    <!-- No workspace selected -->
    <EmptyState
      icon="gear"
      title="Select a workspace first"
      body="Context & soul are per-workspace. Choose a workspace from the sidebar to configure it."
    />
  {:else if loading && !cfg}
    <Skeleton rows={2} height={88} />
  {:else if cfg}
    <!-- Config form -->
    <div class="card form">
      <!-- Active skills -->
      <div class="field">
        <span class="lbl">Active skills</span>
        <label class="all-row">
          <input
            type="checkbox"
            checked={allSkills}
            onchange={(e) => toggleAllSkills(e.currentTarget.checked)}
          />
          <span>All library skills active</span>
        </label>
        {#if !allSkills}
          {#if skills.length === 0}
            <span class="hint">The library has no skills yet. Add some in the Context Library.</span>
          {:else}
            <div class="skill-grid">
              {#each skills as s (s.name)}
                <label class="skill-row" title={s.description}>
                  <input
                    type="checkbox"
                    checked={selectedSkills.has(s.name)}
                    onchange={(e) => toggleSkill(s.name, e.currentTarget.checked)}
                  />
                  <span class="skill-name mono">{s.name}</span>
                  {#if s.description}
                    <span class="skill-desc dim">{s.description}</span>
                  {/if}
                </label>
              {/each}
            </div>
          {/if}
          <span class="hint">
            Only the checked skills are injected into this workspace's agents.
          </span>
        {/if}
      </div>

      <!-- Soul -->
      <div class="field">
        <label for="cs-soul">Soul</label>
        <select
          id="cs-soul"
          class="input"
          value={cfg.soul ?? ''}
          onchange={(e) => cfg && (cfg.soul = e.currentTarget.value === '' ? null : e.currentTarget.value)}
        >
          <option value="">(global default)</option>
          {#each souls as s (s.name)}
            <option value={s.name}>{s.name}</option>
          {/each}
        </select>
        <span class="hint">
          The persona injected into every interaction here. “(global default)” uses the
          instance-wide default soul set in the Context Library.
        </span>
      </div>

      <!-- Extra context -->
      <div class="field">
        <label for="cs-extra">Extra context</label>
        <textarea
          id="cs-extra"
          class="input mono"
          rows={6}
          bind:value={cfg.extra_context_md}
          spellcheck="false"
          placeholder="Free-form markdown appended to the OTTO context region…"
        ></textarea>
        <span class="hint">Markdown, appended to the Otto-managed region of CLAUDE.md / AGENTS.md.</span>
      </div>

      <!-- Include memory -->
      <div class="field field-row">
        <label for="cs-memory">Inline workspace MEMORY.md</label>
        <input id="cs-memory" type="checkbox" bind:checked={cfg.include_memory} />
      </div>

      <div class="actions">
        <button class="btn primary" disabled={saving} onclick={save}>
          {saving ? 'Saving…' : 'Save'}
        </button>
      </div>
    </div>

    <!-- Materialize -->
    <h2 class="section-title">Materialize now</h2>
    <div class="card-info dim">
      Re-write the Otto-managed context files for this workspace immediately. Normally this happens
      automatically the next time a session spawns.
    </div>
    <div class="actions materialize-actions">
      {#each providers as p (p)}
        <button class="btn" disabled={materializing !== null} onclick={() => materialize(p)}>
          {materializing === p ? 'Materializing…' : `Materialize ${p}`}
        </button>
      {/each}
      {#if providers.length === 0}
        <span class="dim">No supported providers available.</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 18px;
    max-width: 640px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .field-row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }
  .field-row label {
    margin-bottom: 0;
  }

  .field label,
  .lbl {
    font-size: 12.5px;
    font-weight: 600;
  }

  .all-row,
  .skill-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    font-weight: 400;
  }
  .all-row input,
  .skill-row input {
    flex-shrink: 0;
  }

  .skill-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 4px 0 2px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    max-height: 280px;
    overflow: auto;
  }
  .skill-name {
    font-size: 12px;
    font-weight: 500;
  }
  .skill-desc {
    font-size: 11.5px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hint {
    font-size: 11.5px;
    color: var(--text-dim);
  }

  textarea.input {
    resize: vertical;
    line-height: 1.5;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .materialize-actions {
    margin-top: 10px;
  }

  .section-title {
    font-size: 14px;
    font-weight: 600;
    margin: 22px 0 10px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .card-info {
    font-size: 12px;
    max-width: 640px;
  }
</style>
