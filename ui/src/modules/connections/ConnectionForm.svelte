<script lang="ts">
  // New/Edit connection sheet — unified form with optional SSH tunnel toggle.
  // Field layout: name / kind / host / port / user / database / password /
  //   [SSH section: jump host + identity file] / first command.
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { api } from '../../lib/api/client';
  import type {
    Connection,
    ConnectionKind,
    ConnectionSection,
    UpsertConnectionReq,
  } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    existing: Connection | null;
    onclose: () => void;
    onsaved: (c: Connection) => void;
  }
  let { existing, onclose, onsaved }: Props = $props();

  const kinds: ConnectionKind[] = ['ssh', 'mysql', 'redis', 'mongodb', 'clickhouse', 'custom'];

  // Which kinds support the db field
  const hasDatabaseField = new Set<ConnectionKind>(['mysql', 'clickhouse', 'redis', 'mongodb']);
  // Which kinds need host/port/user (not mongodb / custom)
  const hasHostFields = new Set<ConnectionKind>(['ssh', 'mysql', 'redis', 'clickhouse']);
  // Which kinds can have a password (all except ssh by default)
  const hasPasswordField = new Set<ConnectionKind>(['mysql', 'redis', 'mongodb', 'clickhouse', 'custom']);

  // svelte-ignore state_referenced_locally
  let name = $state(existing?.name ?? '');
  // svelte-ignore state_referenced_locally
  let kind: ConnectionKind = $state(existing?.kind ?? 'ssh');

  // Initialise individual fields from existing params so they survive kind switches.
  // svelte-ignore state_referenced_locally
  let fHost       = $state((existing?.params?.host        as string)  ?? '');
  // svelte-ignore state_referenced_locally
  let fPort       = $state((existing?.params?.port        as string|number|undefined) != null
    ? String(existing!.params!.port) : '');
  // svelte-ignore state_referenced_locally
  let fUser       = $state((existing?.params?.user        as string)  ?? '');
  // svelte-ignore state_referenced_locally
  let fDb         = $state((existing?.params?.db          as string)  ?? '');
  // svelte-ignore state_referenced_locally
  let fConnString = $state((existing?.params?.conn_string as string)  ?? '');
  // svelte-ignore state_referenced_locally
  let fTemplate   = $state((existing?.params?.command_template as string) ?? '');
  // svelte-ignore state_referenced_locally
  let fJump       = $state((existing?.params?.jump        as string)  ?? '');
  // svelte-ignore state_referenced_locally
  let fIdentity   = $state((existing?.params?.identity_file as string) ?? '');
  let secret      = $state('');
  // svelte-ignore state_referenced_locally
  let firstCommand = $state(existing?.first_command ?? '');
  let busy         = $state(false);

  // Section assignment (workspace connections only). `''` = ungrouped.
  // svelte-ignore state_referenced_locally
  let sectionId = $state<string>(existing?.section_id ?? '');
  let sections: ConnectionSection[] = $state([]);
  let creatingSection = $state(false);
  let newSectionName = $state('');
  // Global (root-managed) connections are not assignable to a workspace section.
  // svelte-ignore state_referenced_locally
  const isGlobal = existing != null && existing.workspace_id === null;

  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || isGlobal) return;
    void api
      .get<ConnectionSection[]>(`/workspaces/${wsId}/connection-sections`)
      .then((s) => (sections = s.sort((a, b) => a.position - b.position)))
      .catch(() => {});
  });

  // Flatten the section tree into indented <option>s ("Platform / AWS").
  function buildOptions(parentId: string | null, depth: number): { id: string; label: string }[] {
    return sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .flatMap((s) => [
        { id: s.id, label: `${'   '.repeat(depth)}${depth > 0 ? '↳ ' : ''}${s.name}` },
        ...buildOptions(s.id, depth + 1),
      ]);
  }
  const sectionOptions = $derived(buildOptions(null, 0));

  async function createSection(): Promise<void> {
    const nm = newSectionName.trim();
    if (!nm || !ws.currentId) return;
    try {
      const sec = await api.post<ConnectionSection>(
        `/workspaces/${ws.currentId}/connection-sections`,
        { name: nm },
      );
      sections = [...sections, sec];
      sectionId = sec.id;
      newSectionName = '';
      creatingSection = false;
    } catch (e) {
      toasts.error('Could not create section', e instanceof Error ? e.message : String(e));
    }
  }

  // SSH toggle: ON by default for 'ssh' kind, OFF for others.
  // When editing, turn it on if jump or identity_file is already set.
  // svelte-ignore state_referenced_locally
  let sshEnabled = $state(
    kind === 'ssh' ||
    !!(existing?.params?.jump || existing?.params?.identity_file)
  );

  // File picker state
  let showFilePicker = $state(false);

  function setKind(k: ConnectionKind): void {
    kind = k;
    // Auto-enable SSH for the ssh kind; auto-disable when switching away
    // (unless jump/identity already filled).
    if (k === 'ssh') {
      sshEnabled = true;
    } else if (!fJump && !fIdentity) {
      sshEnabled = false;
    }
  }

  function buildParams(): Record<string, unknown> {
    const p: Record<string, unknown> = {};

    if (kind === 'mongodb') {
      if (fConnString) p['conn_string'] = fConnString;
    } else if (kind === 'custom') {
      if (fTemplate) p['command_template'] = fTemplate;
    } else {
      if (fHost)                       p['host'] = fHost;
      if (fPort !== '')                p['port'] = Number(fPort);
      if (fUser)                       p['user'] = fUser;
      if (hasDatabaseField.has(kind) && fDb) p['db'] = fDb;
    }

    // SSH section fields (only when SSH toggle is on)
    if (sshEnabled) {
      if (fJump)     p['jump']           = fJump;
      if (fIdentity) p['identity_file']  = fIdentity;
    }

    return p;
  }

  async function save(): Promise<void> {
    if (busy) return;
    busy = true;
    const body: UpsertConnectionReq = {
      name: name.trim(),
      kind,
      params: buildParams(),
      first_command: firstCommand.trim() === '' ? null : firstCommand.trim(),
      section_id: sectionId === '' ? null : sectionId,
    };
    if (secret !== '') body.secret = secret;
    try {
      const saved = existing
        ? await api.patch<Connection>(`/connections/${existing.id}`, body)
        : await api.post<Connection>(`/workspaces/${ws.currentId}/connections`, body);
      toasts.success(existing ? 'Connection updated' : 'Connection created', saved.name);
      onsaved(saved);
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={existing ? `Edit ${existing.name}` : 'New Connection'} width={500} {onclose}>
  <!-- Name -->
  <div class="field">
    <label for="cf-name">Name</label>
    <input id="cf-name" class="input" bind:value={name} placeholder="staging mysql" />
  </div>

  <!-- Section (workspace connections only) -->
  {#if !isGlobal}
    <div class="field">
      <label for="cf-section">Section <span class="dim">(optional)</span></label>
      {#if creatingSection}
        <div class="section-new">
          <input
            class="input"
            bind:value={newSectionName}
            placeholder="New section name"
            onkeydown={(e) => {
              if (e.key === 'Enter') createSection();
              else if (e.key === 'Escape') {
                creatingSection = false;
                newSectionName = '';
              }
            }}
          />
          <button class="btn small primary" onclick={createSection}>Add</button>
          <button
            class="btn small"
            onclick={() => {
              creatingSection = false;
              newSectionName = '';
            }}
          >
            Cancel
          </button>
        </div>
      {:else}
        <div class="section-row">
          <select id="cf-section" class="input" bind:value={sectionId}>
            <option value="">Ungrouped</option>
            {#each sectionOptions as opt (opt.id)}
              <option value={opt.id}>{opt.label}</option>
            {/each}
          </select>
          <button class="btn small" onclick={() => (creatingSection = true)}>＋ New</button>
        </div>
      {/if}
    </div>
  {/if}

  <!-- Kind -->
  <div class="field">
    <label for="cf-kind">Kind</label>
    <div class="kind-row" id="cf-kind">
      {#each kinds as k (k)}
        <button class="kind-chip" class:selected={kind === k} onclick={() => setKind(k)}>{k}</button>
      {/each}
    </div>
  </div>

  {#if kind === 'clickhouse'}
    <div class="warn-banner">
      clickhouse-client only accepts the password via argv — it may be visible in
      <span class="mono">ps</span> output on the host while connected.
    </div>
  {/if}

  <!-- MongoDB: connection string only -->
  {#if kind === 'mongodb'}
    <div class="field">
      <label for="cf-conn-string">Connection string</label>
      <input
        id="cf-conn-string"
        class="input mono"
        bind:value={fConnString}
        placeholder="mongodb://host:27017/db"
        spellcheck="false"
      />
    </div>

  <!-- Custom: command template only -->
  {:else if kind === 'custom'}
    <div class="field">
      <label for="cf-template">Command template</label>
      <input
        id="cf-template"
        class="input mono"
        bind:value={fTemplate}
        placeholder="psql -h {'{host}'} -U {'{user}'} {'{db}'}   ({'{secret}'} available)"
        spellcheck="false"
      />
    </div>

  <!-- All other kinds: host / port / user / db -->
  {:else}
    <div class="field">
      <label for="cf-host">Host</label>
      <input
        id="cf-host"
        class="input mono"
        bind:value={fHost}
        placeholder={kind === 'ssh' ? 'server.example.com' : 'db.internal'}
        spellcheck="false"
      />
    </div>

    <div class="field-row">
      <div class="field grow">
        <label for="cf-port">Port <span class="dim">(optional)</span></label>
        <input
          id="cf-port"
          class="input mono"
          type="number"
          bind:value={fPort}
          placeholder={kind === 'ssh' ? '22' : kind === 'redis' ? '6379' : kind === 'clickhouse' ? '9000' : '3306'}
        />
      </div>
      <div class="field grow">
        <label for="cf-user">User <span class="dim">(optional)</span></label>
        <input
          id="cf-user"
          class="input mono"
          bind:value={fUser}
          placeholder="root"
          spellcheck="false"
        />
      </div>
    </div>

    {#if hasDatabaseField.has(kind)}
      <div class="field">
        <label for="cf-db">
          {kind === 'redis' ? 'DB index' : 'Database'}
          <span class="dim">(optional)</span>
        </label>
        <input
          id="cf-db"
          class="input mono"
          bind:value={fDb}
          placeholder={kind === 'redis' ? '0' : 'mydb'}
          spellcheck="false"
        />
      </div>
    {/if}
  {/if}

  <!-- Password (not for ssh kind) -->
  {#if hasPasswordField.has(kind)}
    <div class="field">
      <label for="cf-secret">
        {kind === 'mongodb' ? 'Secret (credential in the URI)' :
         kind === 'custom'  ? 'Secret ({secret} in template)' : 'Password'}
        <span class="dim">(optional)</span>
      </label>
      <input
        id="cf-secret"
        class="input"
        type="password"
        bind:value={secret}
        placeholder={existing?.secret_ref ? '•••••• (leave blank to keep)' : ''}
        autocomplete="new-password"
      />
      <span class="hint">Stored in the macOS Keychain — never in the database.</span>
    </div>
  {/if}

  <!-- SSH toggle -->
  {#if kind !== 'ssh'}
    <div class="field ssh-toggle-row">
      <label class="toggle-label">
        <input type="checkbox" bind:checked={sshEnabled} />
        Connect via SSH <span class="dim">(jump host + identity)</span>
      </label>
    </div>
  {/if}

  <!-- SSH section -->
  {#if sshEnabled}
    <div class="ssh-section">
      <div class="field">
        <label for="cf-jump">
          {kind === 'ssh' ? 'Jump host' : 'SSH bastion / jump host'}
          <span class="dim">(optional)</span>
        </label>
        <input
          id="cf-jump"
          class="input mono"
          bind:value={fJump}
          placeholder="bastion.example.com"
          spellcheck="false"
        />
      </div>

      <div class="field">
        <label for="cf-identity">Identity file <span class="dim">(optional)</span></label>
        <div class="file-input-row">
          <input
            id="cf-identity"
            class="input mono grow"
            bind:value={fIdentity}
            placeholder="~/.ssh/id_rsa"
            spellcheck="false"
          />
          <button class="btn browse-btn" onclick={() => (showFilePicker = true)}>Browse…</button>
        </div>
        <span class="hint">Identity OR password — both optional.</span>
      </div>
    </div>
  {/if}

  <!-- First command -->
  <div class="field">
    <label for="cf-first">First command <span class="dim">(optional)</span></label>
    <input
      id="cf-first"
      class="input mono"
      bind:value={firstCommand}
      placeholder="e.g. USE app_db; SHOW TABLES;"
      spellcheck="false"
    />
    <span class="hint">Sent to the terminal once the client connects.</span>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy || name.trim() === ''} onclick={save}>
      {busy ? 'Saving…' : existing ? 'Save Changes' : 'Create Connection'}
    </button>
  {/snippet}
</Modal>

<!-- Identity file picker (file-pick mode) -->
{#if showFilePicker}
  <FolderPicker
    title="Choose Identity File"
    start={fIdentity ? fIdentity.replace(/\/[^/]+$/, '') : ''}
    files={true}
    onpick={(path) => { fIdentity = path; showFilePicker = false; }}
    onclose={() => (showFilePicker = false)}
  />
{/if}

<style>
  .kind-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .kind-chip {
    height: 24px;
    padding: 0 11px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    transition: all 130ms ease-out;
  }
  .kind-chip.selected {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
    font-weight: 500;
  }
  .warn-banner {
    font-size: 11.5px;
    line-height: 1.5;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, #febc2e 12%, transparent);
    border: 1px solid color-mix(in srgb, #febc2e 40%, transparent);
    margin-bottom: 12px;
  }
  .field-row {
    display: flex;
    gap: 12px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .ssh-toggle-row {
    margin-top: 4px;
  }
  .toggle-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
    user-select: none;
  }
  .toggle-label input[type='checkbox'] {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .ssh-section {
    margin-top: 2px;
    padding: 10px 12px 4px;
    border-radius: var(--radius-m);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    background: color-mix(in srgb, var(--accent) 5%, transparent);
  }
  .file-input-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .browse-btn {
    flex-shrink: 0;
    white-space: nowrap;
  }
  .section-row,
  .section-new {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .section-row select {
    flex: 1;
    min-width: 0;
  }
  .section-new .input {
    flex: 1;
    min-width: 0;
  }
</style>
