<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  import SectionIntro from './SectionIntro.svelte';
  // MCP Servers settings page: per-workspace, user-managed MCP servers that Otto
  // merges into the workspace's `.mcp.json` when an agent session spawns there
  // (alongside Otto's own managed entries, e.g. the browser server). Nothing is
  // auto-enabled — each server is off until you flip it on, and it's only written
  // to `.mcp.json` the next time a session spawns in the workspace.
  import { auth } from '../../lib/stores/auth.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { mcpApi } from '../../lib/api/mcp';
  import { mcpCpExtraApi } from '../mcp/cp-api';
  import type { McpServer, CreateMcpServerReq } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import SettingToggle from './SettingToggle.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  // The first-party `otto` MCP server (Otto's read-only tools + the read-only DB
  // connection tools), attached to agent sessions per WORKSPACE through the same
  // session-attach endpoint the MCP page uses (`otto_mcp_enabled`, stored as a
  // per-workspace map; unlisted ⇒ default ON). It used to read/write the raw
  // setting as ONE global bool: a workspace switched off on the MCP page still
  // showed "On" here, and flipping it overwrote every workspace's choice.
  let ottoEnabled = $state(true);
  let ottoLoaded = $state(false);
  let ottoSaving = $state(false);
  let ottoError = $state('');

  $effect(() => {
    const id = ws.currentId;
    if (id) void loadOttoSetting(id);
  });

  async function loadOttoSetting(id: string): Promise<void> {
    ottoLoaded = false;
    ottoError = '';
    try {
      const r = await mcpCpExtraApi.sessionAttach(id);
      if (ws.currentId === id) ottoEnabled = r.attached;
    } catch (e) {
      if (ws.currentId === id) ottoError = errMsg(e);
    } finally {
      if (ws.currentId === id) ottoLoaded = true;
    }
  }

  async function toggleOtto(next: boolean): Promise<void> {
    const id = ws.currentId;
    if (!id) return;
    ottoSaving = true;
    try {
      const r = await mcpCpExtraApi.setSessionAttach(id, { enabled: next });
      ottoEnabled = r.attached;
      toasts.success(
        r.attached ? 'Connections MCP attached' : 'Connections MCP detached',
        'For this workspace — applies to agent sessions started from now on.',
      );
    } catch (e) {
      // SettingToggle re-syncs the box to ottoEnabled (unchanged) after this.
      toasts.error("Couldn't change the Connections MCP setting", errMsg(e));
    } finally {
      ottoSaving = false;
    }
  }

  let servers: McpServer[] = $state([]);
  let loading = $state(false);
  let loadError = $state('');
  let loadGeneration = 0;
  const canConfigure = (id: string) => resourceAccess.can('mcp_server', id, 'configure', 'mcp', 'admin');
  $effect(() => { for (const server of servers) void resourceAccess.load('mcp_server', server.id); });
  $effect(() => resourceAccess.subscribe(change => {
    if (change.type === 'reset' || (change.kind === 'mcp_server' && change.before?.operations.configure?.allowed && !change.after?.operations.configure?.allowed)) {
      loadGeneration++; servers = []; closeForm();
      if (ws.currentId) void load(ws.currentId);
    }
  }));
  let busyId: string | null = $state(null);

  // Add/edit form state. `editing` holds the id being edited (null = creating a
  // new one once the form is open).
  let formOpen = $state(false);
  let editing: string | null = $state(null);
  let fName = $state('');
  let fCommand = $state('');
  let fArgs = $state(''); // one arg per line
  let fEnv = $state(''); // KEY=value per line
  // Secret env: values go to the macOS Keychain, never the DB row. On edit the
  // stored keys show as KEY= lines (value blank = keep the stored value).
  let fSecretEnv = $state('');
  let fEnabled = $state(false);
  let saving = $state(false);

  const wsId = $derived(ws.currentId);
  const formValid = $derived(fName.trim() !== '' && (!auth.isRoot || fCommand.trim() !== ''));

  function errMsg(e: unknown): string {
    return loadErrorText(e);
  }

  $effect(() => {
    if (wsId) void load(wsId);
  });

  async function load(id: string): Promise<void> {
    const generation = ++loadGeneration;
    loading = true;
    loadError = '';
    try {
      const result = await mcpApi.list(id);
      if (generation === loadGeneration) servers = result;
    } catch (e) {
      if (generation === loadGeneration) loadError = errMsg(e);
    } finally {
      loading = false;
    }
  }

  function resetForm(): void {
    editing = null;
    fName = '';
    fCommand = '';
    fArgs = '';
    fEnv = '';
    fSecretEnv = '';
    fEnabled = false;
  }

  function openCreate(): void {
    if (!auth.isRoot) return;
    resetForm();
    formOpen = true;
  }

  function openEdit(s: McpServer): void {
    editing = s.id;
    fName = s.name;
    fCommand = s.command;
    fArgs = s.args.join('\n');
    fEnv = Object.entries(s.env)
      .map(([k, v]) => `${k}=${v}`)
      .join('\n');
    // Stored secret values are never returned — surface the key names so a
    // blank value means "keep what's in the Keychain".
    fSecretEnv = s.secret_env_keys.map((k) => `${k}=`).join('\n');
    fEnabled = s.enabled;
    formOpen = true;
  }

  function closeForm(): void {
    formOpen = false;
    resetForm();
  }

  // Split a multi-line textarea into trimmed, non-empty lines.
  function lines(text: string): string[] {
    return text
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);
  }

  // Parse "KEY=value" lines into an object (first '=' splits; later ones kept).
  // `allowEmpty` keeps KEY= lines (used by the secret editor's keep-sentinel).
  function parseEnv(text: string, allowEmpty = false): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of lines(text)) {
      const eq = line.indexOf('=');
      if (eq <= 0) continue; // skip lines without a key
      const v = line.slice(eq + 1).trim();
      if (v === '' && !allowEmpty) continue;
      out[line.slice(0, eq).trim()] = v;
    }
    return out;
  }

  async function save(): Promise<void> {
    if (!wsId || (!editing && !auth.isRoot)) return;
    const name = fName.trim();
    const command = fCommand.trim();
    if (!formValid) return;
    // Secret env: parse KEY=value lines; a KEY= line with an empty value keeps
    // the currently stored Keychain value (edit flow surfaces keys that way).
    const secretPairs = parseEnv(fSecretEnv, true);
    const keptKeys = lines(fSecretEnv)
      .map((l) => l.split('=')[0]?.trim() ?? '')
      .filter(Boolean);
    const existing = editing ? servers.find((x) => x.id === editing) : undefined;
    const secret_env: Record<string, string> = {};
    for (const k of keptKeys) {
      if (secretPairs[k] !== undefined && secretPairs[k] !== '') secret_env[k] = secretPairs[k];
      else if (existing?.secret_env_keys.includes(k)) secret_env[k] = ''; // sentinel: keep stored
    }
    // Drop keep-sentinels for create (nothing stored yet) and warn once.
    if (!editing) for (const k of Object.keys(secret_env)) if (secret_env[k] === '') delete secret_env[k];

    const body: CreateMcpServerReq = {
      name,
      command,
      args: lines(fArgs),
      env: parseEnv(fEnv),
      secret_env,
      enabled: fEnabled,
    };
    saving = true;
    try {
      if (editing) {
        await mcpApi.update(editing, auth.isRoot ? body : { name, enabled: fEnabled });
        toasts.success('MCP server updated', name);
      } else {
        await mcpApi.create(wsId, body);
        toasts.success('MCP server added', name);
      }
      closeForm();
      await load(wsId);
    } catch (e) {
      toasts.error(editing ? "Couldn't save the MCP server" : "Couldn't add the MCP server", errMsg(e));
    } finally {
      saving = false;
    }
  }

  async function toggleEnabled(s: McpServer): Promise<void> {
    if (!wsId) return;
    busyId = s.id;
    try {
      await mcpApi.update(s.id, { enabled: !s.enabled });
      await load(wsId);
    } catch (e) {
      toasts.error(`Couldn't ${s.enabled ? 'disable' : 'enable'} ${s.name}`, errMsg(e));
    } finally {
      busyId = null;
    }
  }

  async function remove(s: McpServer): Promise<void> {
    if (!wsId) return;
    if (
      !(await confirmer.ask(
        `Remove MCP server “${s.name}”? It stops being written to this workspace's .mcp.json for new sessions${s.secret_env_keys.length ? ', and its secret values are removed from the Keychain' : ''}.`,
        { title: 'Remove MCP server', confirmLabel: 'Remove' },
      ))
    )
      return;
    busyId = s.id;
    try {
      await mcpApi.remove(s.id);
      toasts.success('MCP server removed', s.name);
      await load(wsId);
    } catch (e) {
      toasts.error(`Couldn't remove ${s.name}`, errMsg(e));
    } finally {
      busyId = null;
    }
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('mcp-servers')} subtitle="Extra agent tools for this workspace">
    {#snippet actions()}
      {#if wsId && servers.length > 0}
        <button
          class="btn primary"
          disabled={!auth.isRoot}
          title={auth.isRoot ? undefined : 'Only the owner can add MCP servers (they run a command on this Mac)'}
          onclick={openCreate}><Icon name="plus" size={13} /> Add server</button
        >
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <SectionIntro>Enabled servers are merged into this workspace's <code>.mcp.json</code> when an agent session spawns here, alongside Otto's own entries (e.g. the browser). Nothing is auto-enabled — a server is only written once you turn it on.</SectionIntro>

  <div class="section-title">Built in</div>
  <div class="card mcp-card otto" data-testid="connections-mcp">
    <SettingToggle
      label="Attach the Connections MCP to agent sessions"
      checked={ottoEnabled}
      disabled={!wsId || !ottoLoaded || ottoSaving || !!ottoError}
      title={!wsId ? 'Select a workspace first' : ottoError ? "Couldn't read the current value — Retry below" : undefined}
      onchange={toggleOtto}
    >
      Otto's own <code>otto</code> server, <strong>read-only</strong>: agents can list your database
      connections and run read-only queries (<code>otto_list_connections</code>, <code>otto_db_schema</code>,
      <code>otto_db_query</code>, …). Writes and DDL are refused; rows are capped, PII-masked and audited.
      For this workspace; the same switch as “Attach to sessions” on the MCP page.
    </SettingToggle>
    {#if ottoError}
      <div class="otto-error" role="alert">
        <span>Couldn't read this workspace's setting: {ottoError}</span>
        <button class="btn small" onclick={() => ws.currentId && void loadOttoSetting(ws.currentId)}>Retry</button>
      </div>
    {/if}
  </div>

  <div class="section-title">Your servers</div>
  {#if !wsId}
    <EmptyState
      icon="server"
      title="Select a workspace first"
      body="MCP servers are per-workspace. Choose a workspace from the sidebar to configure them."
    />
  {:else}
    <LoadState what="MCP servers" {loading} error={loadError} empty={servers.length === 0} onretry={() => wsId && void load(wsId)} rows={2}>
      {#snippet emptyView()}
        <div class="card mcp-card">
          <EmptyState
            icon="server"
            title="No MCP servers yet"
            body="Add a Model Context Protocol server (a command such as npx @linear/mcp) to give agents extra tools in this workspace."
            actionLabel={auth.isRoot ? 'Add server' : undefined}
            actionIcon="plus"
            onaction={auth.isRoot ? openCreate : undefined}
          >
            {#if !auth.isRoot}<span class="dim">Only the owner can add servers.</span>{/if}
          </EmptyState>
        </div>
      {/snippet}
      <div class="server-list">
        {#each servers as s (s.id)}
          {@const locked = busyId === s.id || !canConfigure(s.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="card server"
            class:off={!s.enabled}
            oncontextmenu={(e) => ctxMenu.show(e, [
              { label: s.enabled ? 'Disable' : 'Enable', icon: s.enabled ? 'eyeOff' : 'eye', disabled: locked, action: () => toggleEnabled(s) },
              { label: 'Edit…', icon: 'edit', disabled: locked, action: () => openEdit(s) },
              { separator: true },
              { label: 'Remove…', icon: 'trash', danger: true, disabled: locked, action: () => remove(s) },
            ])}
          >
            <span class="server-icon"><Icon name="server" size={14} /></span>
            <div class="server-main">
              <span class="server-name mono" title={s.name}>{s.name}</span>
              <div class="server-cmd mono" title={`${s.command} ${s.args.join(' ')}`.trim()}>
                {s.command}{s.args.length ? ' ' + s.args.join(' ') : ''}
              </div>
              {#if Object.keys(s.env).length || s.secret_env_keys.length}
                <div class="server-env">
                  {#if Object.keys(s.env).length}env: {Object.keys(s.env).join(', ')}{/if}
                  {#if s.secret_env_keys.length}
                    {#if Object.keys(s.env).length} · {/if}<Icon name="lock" size={12} /> {s.secret_env_keys.join(', ')}
                  {/if}
                </div>
              {/if}
            </div>
            <div class="server-actions">
              <!-- The same inline "Enabled" switch as a Channels row. -->
              <label class="checkbox-row srv-enabled" title={!canConfigure(s.id) ? "You can't configure this server" : s.enabled ? `Stop writing ${s.name} to .mcp.json` : `Write ${s.name} to .mcp.json for new sessions`}>
                <input
                  type="checkbox"
                  checked={s.enabled}
                  disabled={locked}
                  onchange={async (e) => {
                    const el = e.currentTarget;
                    await toggleEnabled(s);
                    // A failed save leaves the saved value — show it, not the click.
                    el.checked = servers.find((x) => x.id === s.id)?.enabled ?? false;
                  }}
                />
                Enabled
              </label>
              <button
                class="icon-btn srv-tool"
                disabled={locked}
                title={!canConfigure(s.id) ? "You can't configure this server" : `Edit ${s.name}`}
                aria-label="Edit {s.name}"
                onclick={() => openEdit(s)}
              >
                <Icon name="edit" size={14} />
              </button>
              <button
                class="icon-btn srv-tool"
                disabled={locked}
                title="Remove {s.name}"
                aria-label="Remove {s.name}"
                onclick={() => remove(s)}
              >
                <Icon name="trash" size={14} />
              </button>
            </div>
          </div>
        {/each}
      </div>
    </LoadState>
  {/if}
  </PageBody>
</div>

{#if formOpen}
  <Modal title={editing ? 'Edit MCP server' : 'Add MCP server'} width={540} onclose={closeForm}>
    <div class="field">
      <label for="mcp-name">Name</label>
      <input
        id="mcp-name"
        class="input mono"
        bind:value={fName}
        spellcheck="false"
        autocomplete="off"
        placeholder="linear"
      />
      <span class="hint">The key under <code>mcpServers</code> in <code>.mcp.json</code> (unique per workspace).</span>
    </div>
    {#if !auth.isRoot}<p class="hint owner-note">The owner manages credentials and the server command.</p>{/if}
    <div class="field">
      <label for="mcp-command">Command</label>
      <input
        id="mcp-command"
        class="input mono"
        bind:value={fCommand}
        disabled={!auth.isRoot}
        spellcheck="false"
        autocomplete="off"
        placeholder="npx"
      />
    </div>
    <div class="field">
      <label for="mcp-args">Arguments <span class="dim">(one per line)</span></label>
      <textarea
        id="mcp-args"
        class="input mono"
        rows="3"
        bind:value={fArgs}
        disabled={!auth.isRoot}
        spellcheck="false"
        placeholder={'-y\n@linear/mcp'}
      ></textarea>
    </div>
    <div class="field">
      <label for="mcp-env">Environment <span class="dim">(KEY=value, one per line)</span></label>
      <textarea
        id="mcp-env"
        class="input mono"
        rows="3"
        bind:value={fEnv}
        disabled={!auth.isRoot}
        spellcheck="false"
        placeholder={'LOG_LEVEL=info'}
      ></textarea>
      <span class="hint">Non-secret values only — stored in Otto's database. Put tokens and keys below.</span>
    </div>
    <div class="field">
      <label for="mcp-secret-env"><Icon name="lock" size={12} /> Secret environment <span class="dim">(KEY=value, one per line)</span></label>
      <textarea
        id="mcp-secret-env"
        class="input mono"
        rows="3"
        bind:value={fSecretEnv}
        disabled={!auth.isRoot}
        spellcheck="false"
        placeholder={'API_TOKEN=…'}
      ></textarea>
      <span class="hint">
        Stored in the macOS Keychain, never in Otto's database, and written into <code>.mcp.json</code>
        only when a session spawns (the agent CLI needs the real value on disk). When editing, a bare
        <code>KEY=</code> line keeps the stored value.
      </span>
    </div>
    <label class="checkbox-row enable-row">
      <input id="mcp-enabled" type="checkbox" bind:checked={fEnabled} />
      Enabled — write it to <code>.mcp.json</code> for new sessions
    </label>
    {#snippet footer()}
      <button class="btn" disabled={saving} onclick={closeForm}>Cancel</button>
      <button
        class="btn primary"
        disabled={saving || !formValid}
        title={formValid ? undefined : auth.isRoot ? 'Enter a name and a command' : 'Enter a name'}
        onclick={save}
      >
        {saving ? 'Saving…' : editing ? 'Save changes' : 'Add server'}
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
  code {
    font-family: var(--font-mono);
    font-size: 0.92em;
  }
  .mcp-card {
    max-width: var(--settings-col);
  }
  .otto {
    padding: 4px 16px;
  }
  .otto-error {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 10px;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .owner-note {
    margin: 0 0 12px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .enable-row {
    cursor: pointer;
  }
  .enable-row input {
    width: 15px;
    height: 15px;
    margin: 0;
    accent-color: var(--accent);
  }
  .field label :global(svg) {
    vertical-align: -2px;
  }
  .server-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: var(--settings-col);
  }
  .server {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
  }
  .server.off .server-main,
  .server.off .server-icon {
    opacity: 0.6;
  }
  .server-icon {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    display: grid;
    place-items: center;
  }
  .server-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .server-name {
    font-size: var(--fs-m);
    font-weight: 600;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .server-cmd,
  .server-env {
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .server-env :global(svg) {
    vertical-align: -2px;
  }
  .server-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .srv-enabled {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-inline-end: 4px;
    cursor: pointer;
  }
  .srv-enabled input {
    width: 15px;
    height: 15px;
    margin: 0;
    accent-color: var(--accent);
  }
  @media (max-width: 640px) {
    .server {
      flex-wrap: wrap;
    }
    .server-actions {
      width: 100%;
      justify-content: flex-end;
    }
    .srv-tool {
      min-width: 36px;
      min-height: 36px;
    }
  }
</style>
