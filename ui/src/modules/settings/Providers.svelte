<script lang="ts">
  // Custom agent providers (root): add any CLI (opencode, kilo, …) as a
  // session provider. Stored in the `providers` settings key; the daemon
  // reloads its registry live on save.
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Session } from '../../lib/api/types';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  interface ProviderDef {
    cmd: string;
    args?: string[];
    resume_args?: string[] | null;
    update_command?: string | null;
    /** Model-flag template, `{model}` substituted at spawn (e.g. `--model {model}`).
     *  Unset = the CLI takes no model flag; pickers hide the model control. */
    model_args?: string[] | null;
  }

  interface CliAutoUpdate {
    enabled: boolean;
    time_of_day: string; // "HH:MM"
    use_utc: boolean;
    reload_sessions: boolean;
  }

  const AUTO_UPDATE_DEFAULTS: CliAutoUpdate = {
    enabled: true,
    time_of_day: '03:00',
    use_utc: true,
    reload_sessions: true,
  };

  // Built-in providers with their default update commands (None for shell).
  const BUILTINS: { name: string; updateCmd: string | null }[] = [
    { name: 'claude', updateCmd: 'claude update' },
    { name: 'codex',  updateCmd: 'codex update' },
    { name: 'agy',    updateCmd: 'agy update' },
    { name: 'shell',  updateCmd: null },
  ];

  let loading = $state(true);
  let saving = $state(false);
  let updating = $state(false);
  let custom: Record<string, ProviderDef> = $state({});
  let allSettings: Record<string, unknown> = $state({});
  let defaultProvider = $state('');
  /** Model for the PR / commit DRAFT turns (`pr_draft_model`). Deliberately
   *  separate from the default provider model: drafting is a short, mechanical,
   *  user-blocking turn, so it defaults to the fastest model rather than
   *  inheriting a reasoning model and making the modal wait minutes. */
  let draftModel = $state('');
  const DRAFT_MODELS = ['haiku', 'sonnet', 'opus'];
  // Providers the user has EXCLUDED — hidden from every picker (they may have the
  // CLI installed but don't want to use it). Persisted as `disabled_providers`.
  // `shell` is never excludable (plain terminals need it).
  let disabled = $state<Set<string>>(new Set());

  // Daily CLI auto-update config (settings key `cli_auto_update`).
  let autoUpdate: CliAutoUpdate = $state({ ...AUTO_UPDATE_DEFAULTS });
  let lastRun: string | null = $state(null);
  let savingAuto = $state(false);

  // Skip permission prompts (settings key `agent_skip_permissions`, default ON).
  // On = built-in agents launch with their bypass flag (unattended); off = they
  // use their own ask/auto permission mode.
  let skipPermissions = $state(true);
  let savingSkip = $state(false);

  // Dynamic model catalog: per-provider counts + freshness from
  // GET /providers/models; Refresh re-runs the daemon's source chain
  // (CLI probe / docs scrape / models.dev). Loaded independently of the
  // settings blob so a catalog hiccup never blocks this page.
  interface CatalogEntry {
    models: { id: string; label: string; source: string }[];
    fetched_at: string | null;
    stale: boolean;
    last_error?: string;
  }
  let catalog: Record<string, CatalogEntry> = $state({});
  let refreshingModels: string | null = $state(null); // provider slug, or '*' for all

  async function loadCatalog(): Promise<void> {
    try {
      const r = await api.get<{ providers: Record<string, CatalogEntry> }>('/providers/models');
      catalog = r.providers;
    } catch {
      // Non-fatal — the section just renders empty.
    }
  }

  async function refreshModels(provider?: string): Promise<void> {
    refreshingModels = provider ?? '*';
    try {
      const r = await api.post<{ providers: Record<string, CatalogEntry> }>(
        '/providers/models/refresh',
        provider ? { provider } : {},
      );
      catalog = r.providers;
      toasts.info('Model catalog refreshed');
    } catch (e) {
      toasts.error('Refresh failed', e instanceof Error ? e.message : String(e));
    } finally {
      refreshingModels = null;
    }
  }

  $effect(() => {
    void loadCatalog();
  });

  // Providers offered in the default-agent picker: the live registry from
  // /meta (built-ins + custom overrides), falling back to the built-in names.
  const providers = $derived(auth.meta?.providers ?? BUILTINS.map((b) => b.name));

  // form
  let editing: string | null = $state(null);
  let name = $state('');
  let cmd = $state('');
  let args = $state('');
  let resumeArgs = $state('');
  let updateCmd = $state('');
  let modelArgs = $state('');
  let formOpen = $state(false);

  async function updateAllCLIs(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) { toasts.error('No workspace selected'); return; }
    updating = true;
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/providers/update`, {});
      ws.addSession(session); // navigates to the update session
      toasts.info('Updating CLIs…', 'Watch the Update CLIs session for progress');
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      updating = false;
    }
  }

  $effect(() => {
    void (async () => {
      try {
        allSettings = await api.get<Record<string, unknown>>('/settings');
        custom = (allSettings['providers'] as Record<string, ProviderDef> | undefined) ?? {};
        defaultProvider = (allSettings['default_provider'] as string | undefined) ?? '';
        disabled = new Set((allSettings['disabled_providers'] as string[] | undefined) ?? []);
        autoUpdate = {
          ...AUTO_UPDATE_DEFAULTS,
          ...((allSettings['cli_auto_update'] as Partial<CliAutoUpdate> | undefined) ?? {}),
        };
        lastRun = (allSettings['cli_auto_update_last_run'] as string | undefined) ?? null;
        skipPermissions = (allSettings['agent_skip_permissions'] as boolean | undefined) ?? true;
        draftModel = (allSettings['pr_draft_model'] as string | undefined) ?? '';
      } catch {
        toasts.error('Could not load provider settings');
      } finally {
        loading = false;
      }
    })();
  });

  function openNew(): void {
    editing = null;
    name = '';
    cmd = '';
    args = '';
    resumeArgs = '';
    updateCmd = '';
    modelArgs = '';
    formOpen = true;
  }

  function openEdit(n: string): void {
    const p = custom[n];
    editing = n;
    name = n;
    cmd = p.cmd;
    args = (p.args ?? []).join(' ');
    resumeArgs = (p.resume_args ?? []).join(' ');
    updateCmd = p.update_command ?? '';
    modelArgs = (p.model_args ?? []).join(' ');
    formOpen = true;
  }

  async function persist(next: Record<string, ProviderDef>): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
        providers: next,
      });
      custom = (allSettings['providers'] as Record<string, ProviderDef>) ?? {};
      await auth.refreshMeta();
      toasts.success('Providers saved', 'Available immediately for new sessions');
      formOpen = false;
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function saveDraftModel(): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
        pr_draft_model: draftModel,
      });
      draftModel = (allSettings['pr_draft_model'] as string | undefined) ?? '';
      toasts.success(
        'Draft model saved',
        draftModel === ''
          ? 'PR and commit drafts use haiku (fastest)'
          : `PR and commit drafts use ${draftModel}`,
      );
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function saveDefaultProvider(): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
        default_provider: defaultProvider,
      });
      defaultProvider = (allSettings['default_provider'] as string | undefined) ?? '';
      await auth.refreshMeta();
      toasts.success(
        'Default agent saved',
        defaultProvider === ''
          ? 'New sessions use the first available CLI'
          : `New sessions and channel replies default to ${defaultProvider}`,
      );
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  // Exclude / re-include a provider. Optimistic; reverts on failure.
  async function toggleProvider(nameOfProvider: string, enable: boolean): Promise<void> {
    const next = new Set(disabled);
    if (enable) next.delete(nameOfProvider);
    else next.add(nameOfProvider);
    disabled = next;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
        disabled_providers: [...next],
      });
      disabled = new Set((allSettings['disabled_providers'] as string[] | undefined) ?? []);
      await auth.refreshMeta();
      toasts.success(
        enable ? `${nameOfProvider} enabled` : `${nameOfProvider} hidden`,
        enable
          ? 'It reappears in every provider picker'
          : 'Hidden from every picker; existing sessions keep working',
      );
    } catch (e) {
      // revert
      const revert = new Set(disabled);
      if (enable) revert.add(nameOfProvider);
      else revert.delete(nameOfProvider);
      disabled = revert;
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function saveAutoUpdate(): Promise<void> {
    if (!/^\d{1,2}:\d{2}$/.test(autoUpdate.time_of_day)) {
      toasts.error('Invalid time', 'Use HH:MM, e.g. 03:00');
      return;
    }
    savingAuto = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
        cli_auto_update: { ...autoUpdate },
      });
      autoUpdate = {
        ...AUTO_UPDATE_DEFAULTS,
        ...((allSettings['cli_auto_update'] as Partial<CliAutoUpdate> | undefined) ?? {}),
      };
      toasts.success('Auto-update saved', autoUpdate.enabled ? 'Scheduler is on' : 'Scheduler is off');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      savingAuto = false;
    }
  }

  async function saveSkipPermissions(): Promise<void> {
    savingSkip = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
        agent_skip_permissions: skipPermissions,
      });
      skipPermissions = (allSettings['agent_skip_permissions'] as boolean | undefined) ?? true;
      await auth.refreshMeta();
      toasts.success(
        skipPermissions ? 'Permission prompts skipped' : 'Permission prompts enabled',
        'Applies to new sessions; running sessions are unchanged.',
      );
    } catch (e) {
      skipPermissions = !skipPermissions; // revert the optimistic toggle on failure
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      savingSkip = false;
    }
  }

  async function save(): Promise<void> {
    const n = name.trim();
    if (n === '' || cmd.trim() === '') {
      toasts.error('Name and command are required');
      return;
    }
    if (!/^[a-z0-9][a-z0-9_-]*$/.test(n)) {
      toasts.error('Invalid name', 'Use lowercase letters, digits, - or _');
      return;
    }
    const def: ProviderDef = {
      cmd: cmd.trim(),
      args: args.trim() === '' ? [] : args.trim().split(/\s+/),
    };
    const ra = resumeArgs.trim();
    if (ra !== '') def.resume_args = ra.split(/\s+/);
    const uc = updateCmd.trim();
    if (uc !== '') def.update_command = uc;
    const ma = modelArgs.trim();
    if (ma !== '') def.model_args = ma.split(/\s+/);

    const next = { ...custom };
    if (editing && editing !== n) delete next[editing];
    next[n] = def;
    await persist(next);
  }

  async function remove(n: string): Promise<void> {
    const next = { ...custom };
    delete next[n];
    await persist(next);
  }
</script>

<div class="page">
  <div class="page-header">
    <div class="row between">
      <div>
        <h2>Providers</h2>
        <p class="dim">
          Agent CLIs Otto can spawn as sessions. Built-ins are always available;
          add any other CLI (opencode, kilo, …) below. <code>{'{sid}'}</code> and
          <code>{'{cwd}'}</code> expand in arguments; <code>{'{model}'}</code> in the
          model flag template.
        </p>
      </div>
      <button class="btn primary" onclick={updateAllCLIs} disabled={updating || loading}>
        {updating ? 'Updating…' : 'Update all CLIs'}
      </button>
    </div>
  </div>

  {#if loading}
    <Skeleton rows={4} />
  {:else}
    <div class="section">
      <div class="label">Automatic updates</div>
      <label class="toggle-row">
        <input type="checkbox" bind:checked={autoUpdate.enabled} />
        <span>Update all CLIs automatically, every day</span>
      </label>
      {#if autoUpdate.enabled}
        <div class="row wrap">
          <label class="inline">
            <span>At</span>
            <input class="time" type="time" bind:value={autoUpdate.time_of_day} />
          </label>
          <select class="select tz" bind:value={autoUpdate.use_utc}>
            <option value={true}>UTC</option>
            <option value={false}>Local time</option>
          </select>
          <label class="toggle-row">
            <input type="checkbox" bind:checked={autoUpdate.reload_sessions} />
            <span>Reload open sessions onto the new version</span>
          </label>
        </div>
      {/if}
      <p class="dim sm">
        Runs each CLI's update command on a schedule, so you don't have to. Default
        <strong>03:00 UTC</strong> — when new versions are typically published. A missed
        window (machine asleep/off) runs at the next opportunity. Reloaded sessions
        resume their conversation on the new binary.
        {#if lastRun}<br />Last run: {new Date(lastRun).toLocaleString()}.{/if}
      </p>
      <div class="row">
        <button class="btn primary" onclick={saveAutoUpdate} disabled={savingAuto}>
          {savingAuto ? 'Saving…' : 'Save'}
        </button>
      </div>
    </div>

    <div class="section">
      <div class="label">Default agent</div>
      <div class="row">
        <select
          class="select"
          bind:value={defaultProvider}
          onchange={saveDefaultProvider}
          disabled={saving}
        >
          <option value="">Auto (claude)</option>
          {#each providers as p (p)}
            <option value={p}>{p}</option>
          {/each}
        </select>
      </div>
      <p class="dim sm">
        The agent CLI used for new sessions and channel replies unless explicitly
        overridden.
      </p>
    </div>

    <div class="section">
      <div class="label">PR &amp; commit draft model</div>
      <div class="row">
        <select bind:value={draftModel} onchange={saveDraftModel} disabled={saving}>
          <option value="">Fastest (haiku)</option>
          {#each DRAFT_MODELS as m (m)}
            <option value={m}>{m}</option>
          {/each}
        </select>
      </div>
      <p class="dim sm">
        Used only for drafting a PR title/description or a commit message from a
        diff — a short job that blocks the dialog you are looking at, so it does
        not inherit the model above. The turn also skips MCP servers and tools,
        since the diff is already in the prompt. Raise it if drafts come out thin.
      </p>
    </div>

    <div class="section">
      <div class="label">Permissions</div>
      <label class="toggle-row">
        <input
          type="checkbox"
          bind:checked={skipPermissions}
          onchange={saveSkipPermissions}
          disabled={savingSkip}
        />
        <span>Skip permission prompts — run agents unattended</span>
      </label>
      <p class="dim sm">
        On (default): built-in agents launch with their bypass flag
        (<code>--dangerously-skip-permissions</code>; codex
        <code>--dangerously-bypass-approvals-and-sandbox</code>) so tool use never blocks.
        Turn off to use each CLI's own default permission mode (ask / auto) — tool use then
        prompts in the session terminal. Applies to new sessions; running ones are unchanged.
      </p>
    </div>

    <!-- Enable/exclude toggle shared by built-in + custom rows. Excluding a
         provider hides it from every picker (it may be installed but unwanted);
         existing sessions on it keep working. `shell` is never excludable. -->
    {#snippet enableToggle(pname: string)}
      <label
        class="tgl"
        title={disabled.has(pname)
          ? 'Excluded — click to show in pickers'
          : 'Shown in pickers — click to hide'}
      >
        <input
          type="checkbox"
          checked={!disabled.has(pname)}
          onchange={(e) => toggleProvider(pname, e.currentTarget.checked)}
        />
        <span>{disabled.has(pname) ? 'Hidden' : 'Enabled'}</span>
      </label>
    {/snippet}

    <div class="section">
      <div class="label">Built-in</div>
      <div class="list">
        {#each BUILTINS as b (b)}
          <div class="item" class:off={disabled.has(b.name)}>
            <span class="mono">{b.name}</span>
            <span class="grow"></span>
            {#if b.updateCmd}
              <span class="dim sm mono">{b.updateCmd}</span>
            {/if}
            <span class="dim sm">built-in{custom[b.name] ? ' · overridden below' : ''}</span>
            {#if b.name !== 'shell'}
              {@render enableToggle(b.name)}
            {/if}
          </div>
        {/each}
      </div>
      <p class="dim sm">
        Toggle <strong>Enabled</strong> off to hide a provider from every picker
        (Default agent, New session, workflows, …) — useful when a CLI is installed
        but you don't want to use it. Sessions already using it keep working.
      </p>
    </div>

    <div class="section">
      <div class="row between">
        <div class="label">Models catalog</div>
        <button
          class="btn sm"
          onclick={() => refreshModels()}
          disabled={refreshingModels !== null}
        >
          {refreshingModels === '*' ? 'Refreshing…' : 'Refresh all'}
        </button>
      </div>
      <div class="list">
        {#each Object.entries(catalog).sort() as [prov, cat] (prov)}
          <div class="item">
            <span class="mono">{prov}</span>
            <span class="dim sm">{cat.models.length} models</span>
            {#if cat.stale}<span class="chip">stale</span>{/if}
            <span class="grow"></span>
            <span class="dim sm">
              {cat.fetched_at
                ? `fetched ${new Date(cat.fetched_at).toLocaleString()}`
                : 'never fetched'}{#if cat.last_error}&nbsp;· {cat.last_error}{/if}
            </span>
            <button
              class="btn sm"
              onclick={() => refreshModels(prov)}
              disabled={refreshingModels !== null}
            >
              {refreshingModels === prov ? '…' : 'Refresh'}
            </button>
          </div>
        {:else}
          <div class="dim sm empty">No catalog yet — Refresh to discover models.</div>
        {/each}
      </div>
      <p class="dim sm">
        Model ids discovered at runtime from each provider's CLI or public docs
        (no API keys). Model pickers across Otto offer these; a failed refresh
        keeps the last good list and shows <em>stale</em> instead.
      </p>
    </div>

    <div class="section">
      <div class="row between">
        <div class="label">Custom</div>
        <button class="btn" onclick={openNew}>Add provider</button>
      </div>
      <div class="list">
        {#each Object.entries(custom) as [n, p] (n)}
          <div class="item" class:off={disabled.has(n)}>
            <span class="mono">{n}</span>
            <span class="dim sm mono">{p.cmd} {(p.args ?? []).join(' ')}</span>
            <span class="grow"></span>
            {#if p.resume_args?.length}<span class="chip">resume</span>{/if}
            {#if p.update_command}<span class="chip">update</span>{/if}
            {#if p.model_args?.length}<span class="chip">model</span>{/if}
            {@render enableToggle(n)}
            <button class="btn sm" onclick={() => openEdit(n)}>Edit</button>
            <button class="btn sm danger" onclick={() => remove(n)} disabled={saving}>Remove</button>
          </div>
        {:else}
          <div class="dim sm empty">No custom providers yet.</div>
        {/each}
      </div>
    </div>

    {#if formOpen}
      <div class="section form">
        <div class="label">{editing ? `Edit ${editing}` : 'New provider'}</div>
        <div class="grid">
          <label>
            <span>Name</span>
            <input bind:value={name} placeholder="opencode" spellcheck="false" />
          </label>
          <label>
            <span>Command</span>
            <input bind:value={cmd} placeholder="opencode" spellcheck="false" />
          </label>
          <label>
            <span>Arguments (optional)</span>
            <input bind:value={args} placeholder={'--session {sid}'} spellcheck="false" />
          </label>
          <label>
            <span>Resume arguments (optional)</span>
            <input bind:value={resumeArgs} placeholder={'--resume {sid}'} spellcheck="false" />
          </label>
          <label>
            <span>Update command (optional)</span>
            <input bind:value={updateCmd} placeholder={'npm i -g opencode'} spellcheck="false" />
          </label>
          <label>
            <span>Model flag template (optional)</span>
            <input bind:value={modelArgs} placeholder={'--model {model}'} spellcheck="false" />
          </label>
        </div>
        <div class="row end">
          <button class="btn" onclick={() => (formOpen = false)}>Cancel</button>
          <button class="btn primary" onclick={save} disabled={saving}>
            {saving ? 'Saving…' : 'Save provider'}
          </button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .page {
    padding: 24px 28px;
    max-width: min(640px, 92vw);
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .page-header h2 {
    margin: 0 0 4px;
    font-size: 17px;
  }
  .dim {
    color: var(--text-dim);
  }
  .sm {
    font-size: 11.5px;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .list {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    background: var(--surface);
  }
  .item + .item {
    border-top: 1px solid var(--border);
  }
  /* Excluded provider: dim the row (its name/command) so it reads as inactive,
     while leaving the toggle itself fully legible. */
  .item.off > .mono,
  .item.off > .dim {
    opacity: 0.45;
  }
  .tgl {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
  }
  .tgl input {
    cursor: pointer;
  }
  .grow {
    flex: 1;
  }
  .empty {
    padding: 14px;
    text-align: center;
  }
  .chip {
    font-size: 10px;
    padding: 1px 7px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row.between {
    justify-content: space-between;
  }
  .row.wrap {
    flex-wrap: wrap;
  }
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .inline {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .time {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    padding: 5px 8px;
    font-size: 12.5px;
    color: var(--text);
  }
  .select.tz {
    min-width: 120px;
  }
  .row.end {
    justify-content: flex-end;
  }
  .form {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 14px;
    background: var(--surface);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .grid label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .grid input {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    padding: 6px 9px;
    font-size: 12.5px;
    color: var(--text);
    font-family: var(--font-mono);
  }
  .grid input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .select {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    padding: 6px 9px;
    font-size: 12.5px;
    color: var(--text);
    min-width: 220px;
  }
  .select:focus {
    outline: none;
    border-color: var(--accent);
  }
  code {
    font-family: var(--font-mono);
    font-size: 11px;
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: 4px;
  }
</style>
