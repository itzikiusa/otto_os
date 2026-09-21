<script lang="ts">
  // Context & Soul settings page: per-workspace context provisioning. Pick which
  // library skills are active, which soul (persona) to use, free-form extra
  // context, and whether to inline the workspace MEMORY.md. A "Materialize now"
  // button per provider (claude / codex / agy) writes the resolved context into
  // the workspace's native CLI files on demand.
  import { onDestroy } from 'svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { contextApi } from '../../lib/api/context';
  import type {
    ContextPreviewReq,
    LibrarySkill,
    LibrarySoul,
    UpdateWorkspaceContextReq,
    WorkspaceContextConfig,
  } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ContextPreview from '../agents/ContextPreview.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let cfg: WorkspaceContextConfig | null = $state(null);
  let skills: LibrarySkill[] = $state([]);
  let souls: LibrarySoul[] = $state([]);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state('');
  let references = $state('');
  let artifacts = $state('');
  let generation = 0;
  const editing = $derived(auth.me?.is_root || ws.current?.my_role === 'admin');
  const canMaterialize = $derived(auth.me?.is_root || ['admin', 'editor'].includes(ws.current?.my_role ?? ''));
  const lines = (text: string) => text.split('\n').map((line) => line.trim()).filter(Boolean);
  onDestroy(() => { generation++; });
  let materializing: string | null = $state(null);

  // "All active" vs explicit selection. When true, cfg.skills is null (every
  // library skill is active). When false, cfg.skills is the explicit list.
  let allSkills = $state(true);
  // Explicit selection (a Set of skill names) — only meaningful when !allSkills.
  let selectedSkills: Set<string> = $state(new Set());

  const wsId = $derived(ws.currentId);

  // The agent CLIs this feature can materialize into. The backend only knows how
  // to write a context bundle for these three (anything else is skipped), so the
  // set is a genuine capability limit — NOT the full provider registry.
  const MATERIALIZE_PROVIDERS = ['claude', 'codex', 'agy'] as const;

  // Which provider the dry-run preview targets. Seed from the configured default
  // agent, constrained to the materializable set (fall back to the first one).
  let previewProvider = $state(
    (MATERIALIZE_PROVIDERS as readonly string[]).includes(defaultAgentProvider())
      ? defaultAgentProvider()
      : MATERIALIZE_PROVIDERS[0],
  );
  // The not-yet-saved selection, fed to the preview so it reflects in-flight
  // edits (skills/soul/extra/memory) rather than only what's persisted.
  function buildPreviewOverrides(c: WorkspaceContextConfig): ContextPreviewReq {
    return {
      skills: allSkills ? null : [...selectedSkills],
      soul: c.soul,
      extra_context_md: c.extra_context_md,
      goal_md: c.goal_md,
      memory_md: c.memory_md,
      decisions_md: c.decisions_md,
      references: lines(references),
      artifacts: lines(artifacts),
      include_memory: c.include_memory,
      include_repo_map: c.include_repo_map ?? false,
    };
  }

  // Agent CLIs offered for materialization: the live registry, restricted to the
  // providers whose context bundle the backend can actually write.
  const providers = $derived(
    agentProviders().filter((p) => (MATERIALIZE_PROVIDERS as readonly string[]).includes(p)),
  );

  // ---------------------------------------------------------------------------
  // Load on workspace change
  // ---------------------------------------------------------------------------

  $effect(() => {
    const id = wsId;
    if (id) void load(id);
    else { generation++; cfg = null; loading = false; error = ''; }
  });

  function accept(value: WorkspaceContextConfig) {
    cfg = { ...value, goal_md: value.goal_md ?? '', memory_md: value.memory_md ?? '', decisions_md: value.decisions_md ?? '', references: value.references ?? [], artifacts: value.artifacts ?? [], context_version: value.context_version ?? 0 };
    references = cfg.references.join('\n');
    artifacts = cfg.artifacts.join('\n');
    allSkills = cfg.skills === null;
    selectedSkills = new Set(cfg.skills ?? []);
  }

  async function load(id: string): Promise<void> {
    const request = ++generation;
    loading = true; saving = false; cfg = null; error = '';
    try {
      const [next, nextSkills, nextSouls] = await Promise.all([
        contextApi.getWorkspaceContext(id), contextApi.listSkills(), contextApi.listSouls(),
      ]);
      if (request !== generation || wsId !== id) return;
      accept(next); skills = nextSkills; souls = nextSouls;
    } catch (e) {
      if (request === generation && wsId === id) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (request === generation && wsId === id) loading = false;
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
    const id = wsId; const current = cfg; const request = generation;
    if (!id || !current || !editing || saving || loading) return;
    saving = true; error = '';
    try {
      const body: UpdateWorkspaceContextReq = {
        skills: allSkills ? null : [...selectedSkills], soul: current.soul,
        extra_context_md: current.extra_context_md,
        goal_md: current.goal_md, memory_md: current.memory_md, decisions_md: current.decisions_md,
        references: lines(references), artifacts: lines(artifacts), context_version: current.context_version,
        include_memory: current.include_memory, include_repo_map: current.include_repo_map ?? false,
      };
      const saved = await contextApi.updateWorkspaceContext(id, body);
      if (request !== generation || wsId !== id) return;
      accept(saved);
      toasts.success('Workspace context saved', 'Used when agent sessions start or restart.');
    } catch (e) {
      if (request === generation && wsId === id) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (request === generation && wsId === id) saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Materialize now
  // ---------------------------------------------------------------------------

  async function materialize(provider: string): Promise<void> {
    if (!wsId || !canMaterialize) return;
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
      <h1>Workspace context</h1>
      <div class="sub">
        {ws.current?.name ?? 'Your workspace'} is the shared project for its sessions.
        Goals, instructions, memory and references apply across agent providers when sessions start or restart.
        Running conversations keep their current context.
      </div>
    </div>
  </div>

  {#if !wsId}
    <!-- No workspace selected -->
    <EmptyState
      icon="gear"
      title="Select a workspace first"
      body="Choose a workspace from the sidebar to edit its shared project context."
    />
  {:else if loading && !cfg}
    <Skeleton rows={2} height={88} />
  {:else if !cfg && error}
    <p role="alert">{error}</p><button class="btn" onclick={() => wsId && load(wsId)}>Retry</button>
  {:else if cfg}
    <p class="dim">{ws.current?.root_path} · Context version {cfg.context_version}</p>
    {#if !editing}<p class="dim">Workspace administrators can edit shared context.</p>{/if}
    <!-- Config form -->
    <fieldset class="card form" disabled={!editing || saving || loading}>
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

      <div class="field">
        <label for="cs-goal">Goal</label>
        <textarea id="cs-goal" class="input mono" rows={3} bind:value={cfg.goal_md} maxlength="32000" placeholder="What this workspace is working toward"></textarea>
      </div>
      <!-- Extra context -->
      <div class="field">
        <label for="cs-extra">Shared instructions</label>
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

      <div class="field">
        <label for="cs-shared-memory">Workspace memory</label>
        <textarea id="cs-shared-memory" class="input mono" rows={4} bind:value={cfg.memory_md} maxlength="32000" placeholder="Curated facts that every session should know"></textarea>
      </div>
      <div class="field">
        <label for="cs-decisions">Decisions</label>
        <textarea id="cs-decisions" class="input mono" rows={3} bind:value={cfg.decisions_md} maxlength="32000" placeholder="Agreed decisions and their reasons"></textarea>
      </div>
      <div class="field">
        <label for="cs-references">References</label>
        <textarea id="cs-references" class="input mono" rows={3} bind:value={references} placeholder="Document, Vault or repository references — one per line"></textarea>
        <span class="hint">References are shared as text. Listing a path does not read its contents.</span>
      </div>
      <div class="field">
        <label for="cs-artifacts">Artifacts</label>
        <textarea id="cs-artifacts" class="input mono" rows={3} bind:value={artifacts} placeholder="Links or paths to outputs — one per line"></textarea>
      </div>
      <!-- Include memory -->
      <div class="field field-row">
        <label for="cs-memory">Inline workspace MEMORY.md</label>
        <input id="cs-memory" type="checkbox" bind:checked={cfg.include_memory} />
      </div>

      <!-- Include repo map -->
      <div class="field field-row">
        <label for="cs-repomap" title="Aider-style tree-sitter + PageRank map of the repo's most-referenced symbols">
          Inject repo map (tree-sitter)
        </label>
        <input
          id="cs-repomap"
          type="checkbox"
          checked={cfg.include_repo_map ?? false}
          onchange={(e) => cfg && (cfg.include_repo_map = e.currentTarget.checked)}
          data-testid="context-repomap"
        />
      </div>

      <div class="actions">
        <button class="btn primary" disabled={saving} onclick={save}>
          {saving ? 'Saving…' : 'Save workspace context'}
        </button>
      </div>
    </fieldset>
    {#if error}
      <p role="alert">{error}</p>
      <p class="dim">Your draft is still here. Reloading replaces it with the saved version.</p>
      <button class="btn" disabled={saving} onclick={() => wsId && load(wsId)}>Reload saved context</button>
    {/if}

    <!-- Materialize -->
    <h2 class="section-title">Materialize now</h2>
    <div class="card-info dim">
      Re-write the Otto-managed context files for this workspace immediately. Normally this happens
      automatically the next time a session spawns.
    </div>
    <div class="actions materialize-actions">
      {#each providers as p (p)}
        <button class="btn" disabled={!canMaterialize || materializing !== null} onclick={() => materialize(p)}>
          {materializing === p ? 'Materializing…' : `Materialize ${p}`}
        </button>
      {/each}
      {#if providers.length === 0}
        <span class="dim">No supported providers available.</span>
      {/if}
    </div>

    <!-- Preview (dry-run) -->
    {#if providers.length > 0 && wsId}
      <h2 class="section-title">Preview</h2>
      <div class="card-info dim">
        See exactly what a spawn would write — the skill files, soul, generated
        instruction file and runtime hooks — for the current selection above,
        before saving or materializing.
      </div>
      <div class="actions materialize-actions">
        <label class="preview-prov">
          <span class="hint">Provider</span>
          <select class="input" bind:value={previewProvider}>
            {#each providers as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </label>
      </div>
      <div class="preview-box">
        {#if cfg}
          <ContextPreview
            {wsId}
            provider={previewProvider}
            overrides={buildPreviewOverrides(cfg)}
          />
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .form {
    margin: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 18px;
    max-width: min(640px, 92vw);
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
    max-width: min(640px, 92vw);
  }

  .preview-prov {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .preview-prov .input {
    width: auto;
  }
  .preview-box {
    margin-top: 10px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    max-width: min(720px, 92vw);
  }
</style>
