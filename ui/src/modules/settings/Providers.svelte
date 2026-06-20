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
  }

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
          <code>{'{cwd}'}</code> expand in arguments.
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
      <div class="label">Built-in</div>
      <div class="list">
        {#each BUILTINS as b (b)}
          <div class="item">
            <span class="mono">{b.name}</span>
            <span class="grow"></span>
            {#if b.updateCmd}
              <span class="dim sm mono">{b.updateCmd}</span>
            {/if}
            <span class="dim sm">built-in{custom[b.name] ? ' · overridden below' : ''}</span>
          </div>
        {/each}
      </div>
    </div>

    <div class="section">
      <div class="row between">
        <div class="label">Custom</div>
        <button class="btn" onclick={openNew}>Add provider</button>
      </div>
      <div class="list">
        {#each Object.entries(custom) as [n, p] (n)}
          <div class="item">
            <span class="mono">{n}</span>
            <span class="dim sm mono">{p.cmd} {(p.args ?? []).join(' ')}</span>
            <span class="grow"></span>
            {#if p.resume_args?.length}<span class="chip">resume</span>{/if}
            {#if p.update_command}<span class="chip">update</span>{/if}
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
