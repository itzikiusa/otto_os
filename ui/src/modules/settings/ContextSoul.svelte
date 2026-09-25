<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import SettingToggle from './SettingToggle.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
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
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';

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

  // What Save would send, as a string — compared against the last loaded /
  // saved value so Save stays disabled until something actually changed.
  function draftKey(c: WorkspaceContextConfig): string {
    return JSON.stringify({
      skills: allSkills ? null : [...selectedSkills].sort(),
      soul: c.soul ?? null,
      extra: c.extra_context_md ?? '',
      goal: c.goal_md ?? '',
      memory: c.memory_md ?? '',
      decisions: c.decisions_md ?? '',
      references: lines(references),
      artifacts: lines(artifacts),
      include_memory: !!c.include_memory,
      include_repo_map: !!c.include_repo_map,
    });
  }
  let savedKey = $state('');
  const dirty = $derived(!!cfg && draftKey(cfg) !== savedKey);
  let loadError = $state('');

  function accept(value: WorkspaceContextConfig) {
    cfg = { ...value, goal_md: value.goal_md ?? '', memory_md: value.memory_md ?? '', decisions_md: value.decisions_md ?? '', references: value.references ?? [], artifacts: value.artifacts ?? [], context_version: value.context_version ?? 0 };
    references = cfg.references.join('\n');
    artifacts = cfg.artifacts.join('\n');
    allSkills = cfg.skills === null;
    selectedSkills = new Set(cfg.skills ?? []);
    savedKey = draftKey(cfg);
  }

  async function load(id: string): Promise<void> {
    const request = ++generation;
    loading = true; saving = false; cfg = null; error = ''; loadError = '';
    try {
      const [next, nextSkills, nextSouls] = await Promise.all([
        contextApi.getWorkspaceContext(id), contextApi.listSkills(), contextApi.listSouls(),
      ]);
      if (request !== generation || wsId !== id) return;
      accept(next); skills = nextSkills; souls = nextSouls;
    } catch (e) {
      if (request === generation && wsId === id) loadError = loadErrorText(e);
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
    if (!id || !current || !editing || saving || loading || !dirty) return;
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
      if (request === generation && wsId === id) {
        error = e instanceof Error ? e.message : String(e);
        toasts.error('Couldn’t save workspace context', error);
      }
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
        toasts.info(`${provider}: nothing to write`, 'Its context files were already up to date.');
      } else if (result.files_written.length === 0) {
        toasts.info(`Materialized ${provider}`, 'No files written.');
      } else {
        toasts.success(
          `Materialized ${provider}`,
          result.files_written.join(', '),
        );
      }
    } catch (e) {
      toasts.error(`Couldn’t materialize ${provider}`, e instanceof Error ? e.message : String(e));
    } finally {
      materializing = null;
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('context-soul')} subtitle="Shared goals, instructions and memory for sessions">
    {#snippet actions()}
      {#if cfg && editing}
        <button
          class="btn small primary"
          disabled={saving || loading || !dirty}
          title={dirty ? 'Save workspace context' : 'No unsaved changes'}
          onclick={save}
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <SectionIntro><strong>{ws.current?.name ?? 'Your workspace'}</strong> is the shared project for its sessions. Goals, instructions, memory and references apply across agent providers when sessions start or restart. Running conversations keep their current context.</SectionIntro>

  {#if !wsId}
    <!-- No workspace selected -->
    <EmptyState
      variant="page"
      icon="folder"
      title="No workspace selected"
      body="Workspace context belongs to a workspace. Pick one from the workspace menu at the top of the sidebar to edit its shared project context."
    />
  {:else if (loading && !cfg) || (!cfg && loadError)}
    <LoadState what="workspace context" loading={loading} error={loadError} empty rows={4} onretry={() => wsId && load(wsId)} />
  {:else if cfg}
    <p class="meta">
      <span class="path mono" title={ws.current?.root_path}>{ws.current?.root_path}</span>
      <span aria-hidden="true">·</span>
      <span>Context version {cfg.context_version}</span>
      {#if dirty && editing}<span aria-hidden="true">·</span><span class="unsaved">Unsaved changes</span>{/if}
    </p>
    {#if !editing}<p class="readonly"><strong>Read-only.</strong> Workspace administrators can edit shared context.</p>{/if}
    {#if error}
      <div class="save-err" role="alert">
        <p><strong>Couldn’t save workspace context.</strong> <span class="dim">{error}</span></p>
        <p class="dim">Your draft is still here. Save again, or reload to replace it with the saved version.</p>
        <button class="btn small" disabled={saving} onclick={() => wsId && load(wsId)}>Reload saved context</button>
      </div>
    {/if}
    <!-- Config form -->
    <fieldset class="card form" disabled={!editing || saving || loading}>
      <legend class="sr-only">Workspace context</legend>
      <!-- Active skills -->
      <div class="field">
        <span class="lbl" id="cs-skills-lbl">Active skills</span>
        <SettingToggle label="All library skills active" hint="Uncheck to pick the skills this workspace's agents get." checked={allSkills} onchange={(v) => toggleAllSkills(v)} />
        {#if !allSkills}
          {#if skills.length === 0}
            <span class="hint">The library has no skills yet. Add some in Settings → Context library.</span>
          {:else}
            <div class="skill-grid" role="group" aria-labelledby="cs-skills-lbl">
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
          <option value="">Global default</option>
          {#each souls as s (s.name)}
            <option value={s.name}>{s.name}</option>
          {/each}
        </select>
        <span class="hint">
          The persona injected into every interaction here. “Global default” uses the default soul set in
          Settings → Context library.
        </span>
      </div>

      <div class="field">
        <label for="cs-goal">Goal</label>
        <textarea id="cs-goal" class="input mono" rows={3} bind:value={cfg.goal_md} maxlength="32000" placeholder="Ship the v2 billing API by March"></textarea>
        <span class="hint">What this workspace is working toward.</span>
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
          placeholder="Always run the tests before committing."
        ></textarea>
        <span class="hint">Markdown, added to the context Otto hands each new or restarted agent session (kept outside the repo — your CLAUDE.md / AGENTS.md aren't edited).</span>
      </div>

      <div class="field">
        <label for="cs-shared-memory">Workspace memory</label>
        <textarea id="cs-shared-memory" class="input mono" rows={4} bind:value={cfg.memory_md} maxlength="32000" placeholder="The staging database is read-only."></textarea>
        <span class="hint">Curated facts every session should know.</span>
      </div>
      <div class="field">
        <label for="cs-decisions">Decisions</label>
        <textarea id="cs-decisions" class="input mono" rows={3} bind:value={cfg.decisions_md} maxlength="32000" placeholder="Use Postgres, not MySQL — the team already runs it."></textarea>
        <span class="hint">Agreed decisions and their reasons.</span>
      </div>
      <div class="field">
        <label for="cs-references">References</label>
        <textarea id="cs-references" class="input mono" rows={3} bind:value={references} placeholder="docs/architecture.md"></textarea>
        <span class="hint">Documents, Vault notes or repositories — one per line. Shared as text; listing a path does not read its contents.</span>
      </div>
      <div class="field">
        <label for="cs-artifacts">Artifacts</label>
        <textarea id="cs-artifacts" class="input mono" rows={3} bind:value={artifacts} placeholder="https://example.com/design-doc"></textarea>
        <span class="hint">Links or paths to outputs — one per line.</span>
      </div>
      <!-- Include memory / repo map -->
      <div class="field toggles">
        <SettingToggle
          label="Inline the workspace MEMORY.md"
          hint="Adds the workspace's MEMORY.md file to the context, not just the curated memory above."
          checked={cfg.include_memory}
          onchange={(v) => { if (cfg) cfg.include_memory = v; }}
        />
        <SettingToggle
          label="Inject a repo map (tree-sitter)"
          hint="A map of the repo's most-referenced symbols (tree-sitter + PageRank), so agents find their way faster."
          checked={cfg.include_repo_map ?? false}
          testid="context-repomap"
          onchange={(v) => { if (cfg) cfg.include_repo_map = v; }}
        />
      </div>
    </fieldset>

    <!-- Materialize -->
    <h2 class="section-title">Materialize now</h2>
    <p class="card-info dim">
      Re-write the Otto-managed context files for this workspace immediately. Normally this happens
      automatically the next time a session spawns. Uses the saved context, not unsaved edits.
    </p>
    <div class="actions">
      {#each providers as p (p)}
        <button
          class="btn small"
          disabled={!canMaterialize || materializing !== null}
          title={canMaterialize ? `Write ${p}'s context files now` : 'Workspace editors and admins can materialize'}
          onclick={() => materialize(p)}
        >
          {materializing === p ? 'Materializing…' : `Materialize ${p}`}
        </button>
      {/each}
      {#if providers.length === 0}
        <span class="dim">No supported agent CLIs are enabled (claude, codex or agy).</span>
      {/if}
    </div>

    <!-- Preview (dry-run) -->
    {#if providers.length > 0 && wsId}
      <h2 class="section-title">Preview</h2>
      <p class="card-info dim">
        See exactly what a spawn would write — the skill files, soul, generated
        instruction file and runtime hooks — for the current selection above,
        before saving or materializing.
      </p>
      <div class="actions">
        <label class="preview-prov" for="cs-preview-prov">
          <span class="hint">Provider</span>
        </label>
        <select id="cs-preview-prov" class="input" bind:value={previewProvider}>
          {#each providers as p (p)}
            <option value={p}>{p}</option>
          {/each}
        </select>
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
  .meta {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin: 0 0 12px;
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .meta .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: ltr;
  }
  .meta > span {
    flex-shrink: 0;
  }
  .meta > .path {
    flex-shrink: 1;
  }
  .unsaved {
    color: var(--warning);
  }
  .readonly {
    margin: 0 0 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .save-err {
    max-width: 760px;
    margin: 0 0 12px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    font-size: var(--fs-s);
  }
  .save-err p {
    margin: 0 0 6px;
  }
  .form {
    margin: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 18px;
    max-width: 760px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .form .field {
    margin: 0;
  }
  .lbl {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .toggles {
    gap: 0;
  }
  .skill-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-size: var(--fs-s);
  }
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
    flex-shrink: 0;
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .skill-desc {
    font-size: var(--fs-xs);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  textarea.input {
    resize: vertical;
    line-height: 1.5;
    font-size: var(--fs-s);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .section-title {
    margin: 24px 0 6px;
  }
  .card-info {
    margin: 0 0 10px;
    font-size: var(--fs-s);
    max-width: 760px;
  }
  .dim {
    color: var(--text-dim);
  }
  .preview-prov {
    display: flex;
    align-items: center;
  }
  .actions .input {
    width: auto;
  }
  .preview-box {
    margin-top: 10px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    max-width: 760px;
    min-width: 0;
    overflow-x: auto;
  }
</style>
