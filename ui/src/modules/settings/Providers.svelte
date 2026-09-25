<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Custom agent providers (root): add any CLI (opencode, kilo, …) as a
  // session provider. Stored in the `providers` settings key; the daemon
  // reloads its registry live on save.
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Session } from '../../lib/api/types';
  import SectionIntro from './SectionIntro.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import SettingToggle from './SettingToggle.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';

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
  // A failed load shows inline with Retry. Rendering the form instead showed
  // the defaults as if they were saved — and every save here writes whole
  // objects (`providers`, `cli_auto_update`), so one click would have replaced
  // the real custom providers / update schedule with those defaults.
  let loadError = $state('');
  let saving = $state(false);
  let updating = $state(false);
  let custom: Record<string, ProviderDef> = $state({});
  // Latest full settings object (the PUT response). Saves send ONLY the keys
  // they change: PUT /settings upserts exactly the keys in the body, so
  // spreading this page-load snapshot reverted keys written since (auto-update
  // last-run, MCP/PR-review settings, another window) and wrote false
  // skip-permissions / network-listener audit entries on every save.
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
  // Inline validation for the provider sheet (not a toast — components.md §2).
  let formError = $state('');
  // Inline validation for the auto-update time field.
  let timeError = $state('');

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
    void loadSettings();
  });

  async function loadSettings(): Promise<void> {
    loading = true;
    loadError = '';
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
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openNew(): void {
    editing = null;
    name = '';
    cmd = '';
    args = '';
    resumeArgs = '';
    updateCmd = '';
    modelArgs = '';
    formError = '';
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
    formError = '';
    formOpen = true;
  }

  async function persist(next: Record<string, ProviderDef>): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        providers: next,
      });
      custom = (allSettings['providers'] as Record<string, ProviderDef>) ?? {};
      await auth.refreshMeta();
      toasts.success('Providers saved', 'Available immediately for new sessions');
      formOpen = false;
    } catch (e) {
      toasts.error('Couldn’t save providers', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function saveDraftModel(): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
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
      toasts.error('Couldn’t save the draft model', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function saveDefaultProvider(): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
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
      toasts.error('Couldn’t save the default agent', e instanceof Error ? e.message : String(e));
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
      toasts.error(`Couldn’t ${enable ? 'enable' : 'hide'} ${nameOfProvider}`, e instanceof Error ? e.message : String(e));
    }
  }

  // Applies on change (like every other control on this page — no separate
  // Save for one section). The time field commits on `change` (blur / Enter),
  // so half-typed values never save; an invalid one is flagged inline.
  async function saveAutoUpdate(): Promise<void> {
    if (!/^\d{1,2}:\d{2}$/.test(autoUpdate.time_of_day)) {
      timeError = 'Use HH:MM, for example 03:00.';
      return;
    }
    timeError = '';
    savingAuto = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        cli_auto_update: { ...autoUpdate },
      });
      autoUpdate = {
        ...AUTO_UPDATE_DEFAULTS,
        ...((allSettings['cli_auto_update'] as Partial<CliAutoUpdate> | undefined) ?? {}),
      };
      toasts.success(
        autoUpdate.enabled ? 'Automatic updates on' : 'Automatic updates off',
        autoUpdate.enabled ? `Daily at ${autoUpdate.time_of_day} ${autoUpdate.use_utc ? 'UTC' : 'local time'}` : undefined,
      );
    } catch (e) {
      toasts.error('Couldn’t save automatic updates', e instanceof Error ? e.message : String(e));
      // Re-read so the controls never show a schedule that isn't saved.
      autoUpdate = {
        ...AUTO_UPDATE_DEFAULTS,
        ...((allSettings['cli_auto_update'] as Partial<CliAutoUpdate> | undefined) ?? {}),
      };
    } finally {
      savingAuto = false;
    }
  }

  async function saveSkipPermissions(): Promise<void> {
    // Turning the bypass back ON removes a safety prompt from every new
    // session — ask first. Turning it off (safer) needs no confirmation.
    if (
      skipPermissions &&
      !(await confirmer.ask(
        'New agent sessions will run tools, edit files and execute commands without asking you first.',
        { title: 'Skip permission prompts?', confirmLabel: 'Skip prompts', danger: false },
      ))
    ) {
      skipPermissions = false;
      return;
    }
    savingSkip = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
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
      toasts.error('Couldn’t save the permission setting', e instanceof Error ? e.message : String(e));
    } finally {
      savingSkip = false;
    }
  }

  async function save(): Promise<void> {
    const n = name.trim();
    if (n === '' || cmd.trim() === '') {
      formError = 'Name and command are required.';
      return;
    }
    if (!/^[a-z0-9][a-z0-9_-]*$/.test(n)) {
      formError = 'Use lowercase letters, digits, - or _ in the name (for example opencode).';
      return;
    }
    if (editing !== n && custom[n]) {
      formError = `A custom provider named “${n}” already exists. Choose another name or edit it.`;
      return;
    }
    formError = '';
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
    if (
      !(await confirmer.ask(
        `Remove the custom provider “${n}”? It disappears from every picker; sessions already running on it keep working. You'd have to re-enter its command to add it back.`,
        { title: 'Remove provider', confirmLabel: 'Remove' },
      ))
    )
      return;
    const next = { ...custom };
    delete next[n];
    await persist(next);
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('providers')} subtitle="Agent CLIs Otto can spawn as sessions">
    {#snippet actions()}
      <button
        class="btn small primary"
        data-icon="download"
        onclick={updateAllCLIs}
        disabled={updating || loading}
        title="Run every CLI's update command now, in a new session"
      >
        <Icon name="download" size={12} /> {updating ? 'Updating…' : 'Update all CLIs'}
      </button>
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <SectionIntro>Built-ins are always available; add any other CLI (opencode, kilo, …) under Custom. Every change here applies immediately to new sessions.</SectionIntro>
  <div class="providers-body">

  {#if loading || loadError}
    <LoadState what="provider settings" {loading} error={loadError || null} empty rows={4} onretry={() => void loadSettings()} />
  {:else}
    <section class="section">
      <h2 class="section-title">Default agent</h2>
      <div class="card set-card">
        <div class="field">
          <label for="prov-default">Agent for new sessions and channel replies</label>
          <select
            id="prov-default"
            class="input select"
            bind:value={defaultProvider}
            onchange={saveDefaultProvider}
            disabled={saving}
          >
            <option value="">Auto (claude)</option>
            {#each providers as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
          <span class="hint">Used unless a session, workflow or channel picks another agent.</span>
        </div>
        <div class="field">
          <label for="prov-draft">PR &amp; commit draft model</label>
          <select id="prov-draft" class="input select" bind:value={draftModel} onchange={saveDraftModel} disabled={saving}>
            <option value="">Fastest (haiku)</option>
            {#each DRAFT_MODELS as m (m)}
              <option value={m}>{m}</option>
            {/each}
          </select>
          <span class="hint">
            Only for drafting a PR title/description or a commit message from a diff — a short job that blocks the
            dialog you're looking at, so it doesn't inherit the agent's model and skips MCP servers and tools. Raise
            it if drafts come out thin.
          </span>
        </div>
      </div>
    </section>

    <section class="section">
      <h2 class="section-title">Automatic updates</h2>
      <div class="card set-card tfirst">
        <SettingToggle
          label="Update all CLIs automatically, every day"
          checked={autoUpdate.enabled}
          disabled={savingAuto}
          onchange={(v) => { autoUpdate.enabled = v; return saveAutoUpdate(); }}
        >
          Runs each CLI's update command on a schedule. Default 03:00 UTC, when new versions are usually
          published; a missed window (Mac asleep or off) runs at the next opportunity.
          {#if lastRun}<span title={new Date(lastRun).toLocaleString()}>Last run {rel(lastRun)}.</span>{/if}
        </SettingToggle>
        {#if autoUpdate.enabled}
          <div class="prow wrap indent">
            <label class="inline" for="prov-time">At</label>
            <input
              id="prov-time"
              class="input time"
              type="time"
              bind:value={autoUpdate.time_of_day}
              onchange={saveAutoUpdate}
              disabled={savingAuto}
              aria-invalid={timeError ? 'true' : undefined}
              aria-describedby={timeError ? 'prov-time-err' : undefined}
            />
            <select class="input tz" bind:value={autoUpdate.use_utc} onchange={saveAutoUpdate} disabled={savingAuto} aria-label="Time zone">
              <option value={true}>UTC</option>
              <option value={false}>Local time</option>
            </select>
          </div>
          {#if timeError}<p class="field-err indent" id="prov-time-err" role="alert">{timeError}</p>{/if}
          <div class="indent">
            <SettingToggle
              label="Reload open sessions onto the new version"
              hint="Reloaded sessions resume their conversation on the new binary."
              checked={autoUpdate.reload_sessions}
              disabled={savingAuto}
              onchange={(v) => { autoUpdate.reload_sessions = v; return saveAutoUpdate(); }}
            />
          </div>
        {/if}
      </div>
    </section>

    <section class="section">
      <h2 class="section-title">Permissions</h2>
      <div class="card set-card tfirst">
        <SettingToggle
          label="Skip permission prompts — run agents unattended"
          checked={skipPermissions}
          disabled={savingSkip}
          onchange={(v) => { skipPermissions = v; return saveSkipPermissions(); }}
        >
          On (default): built-in agents launch with their bypass flag
          (<code class="flag">--dangerously-skip-permissions</code>; codex
          <code class="flag">--dangerously-bypass-approvals-and-sandbox</code>) so tool use never blocks.
          Off: each CLI's own permission mode (ask / auto) — tool use prompts in the session terminal.
          Applies to new sessions; running ones are unchanged. Background agent runs (workflow steps,
          scheduled tasks, swarms, self-improvement) still skip prompts — nobody is at their terminal to answer.
        </SettingToggle>
      </div>
    </section>

    <!-- Enable/exclude toggle shared by built-in + custom rows. Excluding a
         provider hides it from every picker (it may be installed but unwanted);
         existing sessions on it keep working. `shell` is never excludable. -->
    {#snippet enableToggle(pname: string)}
      <label
        class="checkbox-row tgl"
        title={disabled.has(pname)
          ? `${pname} is hidden from every picker — check to show it`
          : `${pname} is shown in pickers — uncheck to hide it`}
      >
        <input
          type="checkbox"
          checked={!disabled.has(pname)}
          aria-label={`Show ${pname} in pickers`}
          onchange={(e) => toggleProvider(pname, e.currentTarget.checked)}
        />
        {disabled.has(pname) ? 'Hidden' : 'Enabled'}
      </label>
    {/snippet}

    <section class="section">
      <h2 class="section-title">Built-in</h2>
      <div class="list">
        {#each BUILTINS as b (b)}
          <div class="item" class:off={disabled.has(b.name)}>
            <span class="mono name">{b.name}</span>
            {#if b.updateCmd}
              <span class="dim sm mono meta" title="Update command">{b.updateCmd}</span>
            {/if}
            <span class="grow"></span>
            {#if custom[b.name]}<span class="chip">Overridden below</span>{/if}
            {#if b.name !== 'shell'}
              {@render enableToggle(b.name)}
            {:else}
              <span class="dim sm" title="Plain terminals need the shell provider">Always on</span>
            {/if}
          </div>
        {/each}
      </div>
      <p class="hint">
        Uncheck <strong>Enabled</strong> to hide a provider from every picker (Default agent, New session,
        workflows, …) — useful when a CLI is installed but you don't want to use it. Sessions already using it
        keep working.
      </p>
    </section>

    <section class="section">
      <div class="prow between">
        <h2 class="section-title">Custom</h2>
        <button class="btn small" data-icon="plus" onclick={openNew}><Icon name="plus" size={12} /> Add provider…</button>
      </div>
      <div class="list">
        {#each Object.entries(custom) as [n, p] (n)}
          <div class="item" class:off={disabled.has(n)}>
            <span class="mono name">{n}</span>
            <span class="dim sm mono meta" title={`${p.cmd} ${(p.args ?? []).join(' ')}`}>{p.cmd} {(p.args ?? []).join(' ')}</span>
            <span class="grow"></span>
            {#if p.resume_args?.length}<span class="chip" title="Resume arguments set">resume</span>{/if}
            {#if p.update_command}<span class="chip" title={`Update command: ${p.update_command}`}>update</span>{/if}
            {#if p.model_args?.length}<span class="chip" title={`Model flag: ${(p.model_args ?? []).join(' ')}`}>model</span>{/if}
            {@render enableToggle(n)}
            <button class="btn small ghost" onclick={() => openEdit(n)}>Edit…</button>
            <button class="btn small danger" onclick={() => remove(n)} disabled={saving}>Remove…</button>
          </div>
        {:else}
          <div class="empty">No custom providers yet — Add provider… registers any other agent CLI.</div>
        {/each}
      </div>
      <p class="hint">
        <code>{'{sid}'}</code> and <code>{'{cwd}'}</code> expand in arguments; <code>{'{model}'}</code> in the
        model flag template.
      </p>
    </section>

    <section class="section">
      <div class="prow between">
        <h2 class="section-title">Models catalog</h2>
        <button
          class="btn small"
          data-icon="refresh"
          onclick={() => refreshModels()}
          disabled={refreshingModels !== null}
        >
          <Icon name="refresh" size={12} /> {refreshingModels === '*' ? 'Refreshing…' : 'Refresh all'}
        </button>
      </div>
      <div class="list">
        {#each Object.entries(catalog).sort() as [prov, cat] (prov)}
          <div class="item">
            <span class="mono name">{prov}</span>
            <span class="dim sm">{cat.models.length} models</span>
            {#if cat.stale}<span class="chip warn" title={cat.last_error ?? 'The last refresh failed; showing the last good list'}>Stale</span>{/if}
            <span class="grow"></span>
            <span
              class="dim sm meta"
              title={cat.fetched_at ? `Fetched ${new Date(cat.fetched_at).toLocaleString()}${cat.last_error ? ` · ${cat.last_error}` : ''}` : cat.last_error}
            >
              {cat.fetched_at ? `Fetched ${rel(cat.fetched_at)}` : 'Never fetched'}{#if cat.last_error}&nbsp;· {cat.last_error}{/if}
            </span>
            <button
              class="btn small ghost"
              onclick={() => refreshModels(prov)}
              disabled={refreshingModels !== null}
              aria-label={`Refresh ${prov} models`}
            >
              {refreshingModels === prov ? 'Refreshing…' : 'Refresh'}
            </button>
          </div>
        {:else}
          <div class="empty">No catalog yet — Refresh all to discover models.</div>
        {/each}
      </div>
      <p class="hint">
        Model ids discovered at runtime from each provider's CLI or public docs (no API keys). Model pickers
        across Otto offer these; a failed refresh keeps the last good list and marks it Stale.
      </p>
    </section>
  {/if}
  </div>
  </PageBody>
</div>

{#if formOpen}
  <Modal title={editing ? `Edit ${editing}` : 'New provider'} width={560} onclose={() => (formOpen = false)}>
    <form
      class="grid"
      id="provider-form"
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div class="field">
        <label for="pv-name">Name</label>
        <input id="pv-name" class="input mono-in" bind:value={name} placeholder="opencode" spellcheck="false" autocomplete="off" />
      </div>
      <div class="field">
        <label for="pv-cmd">Command</label>
        <input id="pv-cmd" class="input mono-in" bind:value={cmd} placeholder="opencode" spellcheck="false" autocomplete="off" />
      </div>
      <div class="field">
        <label for="pv-args">Arguments (optional)</label>
        <input id="pv-args" class="input mono-in" bind:value={args} placeholder={'--session {sid}'} spellcheck="false" />
      </div>
      <div class="field">
        <label for="pv-resume">Resume arguments (optional)</label>
        <input id="pv-resume" class="input mono-in" bind:value={resumeArgs} placeholder={'--resume {sid}'} spellcheck="false" />
      </div>
      <div class="field">
        <label for="pv-update">Update command (optional)</label>
        <input id="pv-update" class="input mono-in" bind:value={updateCmd} placeholder={'npm i -g opencode'} spellcheck="false" />
      </div>
      <div class="field">
        <label for="pv-model">Model flag template (optional)</label>
        <input id="pv-model" class="input mono-in" bind:value={modelArgs} placeholder={'--model {model}'} spellcheck="false" />
      </div>
    </form>
    <p class="hint">
      <code>{'{sid}'}</code> and <code>{'{cwd}'}</code> expand in arguments; <code>{'{model}'}</code> in the model flag
      template. Leave the model template empty if the CLI takes no model flag.
    </p>
    {#if formError}<p class="field-err" role="alert">{formError}</p>{/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (formOpen = false)}>Cancel</button>
      <button class="btn primary" type="submit" form="provider-form" disabled={saving}>
        {saving ? 'Saving…' : editing ? 'Save provider' : 'Add provider'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .providers-body {
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-width: var(--settings-col);
  }
  .dim {
    color: var(--text-dim);
  }
  .sm {
    font-size: var(--fs-s);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .section .section-title {
    margin: 0;
  }
  .section .field {
    margin: 0;
  }
  /* Same card as Daemon / Insights / Self-improvement. */
  .set-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 16px 14px;
  }
  /* A card that opens on a SettingToggle: the row brings its own 8px. */
  .set-card.tfirst {
    padding-top: 4px;
    gap: 8px;
  }
  .hint {
    margin: 0;
    max-width: 78ch;
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text-dim);
  }
  /* Hints and sub-controls under a toggle line up with its label text
     (SettingToggle: 15px box + 10px gap). */
  .indent {
    padding-inline-start: 25px;
  }
  .field-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .list {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface);
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 36px;
    padding: 4px 12px;
  }
  .item + .item {
    border-top: 1px solid var(--border);
  }
  .item .name {
    flex-shrink: 0;
    color: var(--text);
  }
  .item .meta {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Excluded provider: dim the row (its name/command) so it reads as inactive,
     while leaving the toggle itself fully legible. */
  .item.off > .mono,
  .item.off > .dim {
    opacity: 0.55;
  }
  .tgl {
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .tgl input {
    cursor: pointer;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .prow {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .prow.between {
    justify-content: space-between;
  }
  .prow.wrap {
    flex-wrap: wrap;
  }
  .inline {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .time {
    width: 110px;
  }
  .tz {
    min-width: 120px;
  }
  .select {
    max-width: 320px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0 12px;
  }
  .mono-in {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  /* A flag is one token — never break it mid-word across lines. */
  code.flag {
    white-space: nowrap;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: var(--radius-s);
  }
  @media (max-width: 640px) {
    .grid {
      grid-template-columns: 1fr;
    }
    .item {
      flex-wrap: wrap;
    }
  }
</style>
