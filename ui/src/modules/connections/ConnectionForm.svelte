<script lang="ts" module>
  import type { ConnectionKind as _CK, Environment as _Env } from '../../lib/api/types';
  export interface ConnectionPrefill { name?: string; kind?: _CK; host?: string; port?: number; user?: string; db?: string; environment?: _Env; tlsMode?: 'disabled' | 'preferred' | 'required'; source?: string; }
</script>

<script lang="ts">
  import { announce } from '../../lib/announce.svelte';
  import PemField from './PemField.svelte';
  import TunnelFields from './TunnelFields.svelte';
  import { argvWarningFor } from './connection-warnings';
  import { parseConnectionUri } from './connectionUri';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  // New/Edit connection sheet — unified form with optional SSH tunnel toggle.
  // Field layout: name / kind / host / port / user / database / password /
  //   [SSH section: jump host + identity file] / first command.
  import { auth } from '../../lib/stores/auth.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { api } from '../../lib/api/client';
  import type { Connection, ConnectionKind, ConnectionSection,
    Environment, TestConnectionResp, TestUnsavedConnectionReq, UpsertConnectionReq } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { connectionList } from '../../lib/stores/connectionList.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { untrack } from 'svelte';

  interface Props {
    existing: Connection | null;
    /** Optional initial values when creating a new connection (e.g. from AWS RDS). */
    prefill?: ConnectionPrefill | null;
    onclose: () => void;
    onsaved: (c: Connection) => void;
    /** Restrict the kind selector (e.g. DB-only in the Database module). */
    kinds?: ConnectionKind[];
    /** "Diagnose with an agent…" on a saved profile's failed Test (the host closes the form and asks consent). */
    ondiagnose?: (probe: { message: string; hint?: string | null; detail?: string | null }) => void;
  }
  let { existing, prefill = null, onclose, onsaved, ondiagnose, kinds = ['ssh', 'mysql', 'postgres', 'redis', 'mongodb', 'clickhouse', 'custom'] }: Props = $props();

  // Display names for the kind chips / toasts (the raw enum is lowercase).
  const kindLabels: Record<ConnectionKind, string> = {
    ssh: 'SSH', mysql: 'MySQL', postgres: 'PostgreSQL', redis: 'Redis',
    mongodb: 'MongoDB', clickhouse: 'ClickHouse', custom: 'Custom',
  };

  // Which kinds support the db field
  const hasDatabaseField = new Set<ConnectionKind>(['mysql', 'postgres', 'clickhouse', 'redis', 'mongodb']);
  // Which kinds need host/port/user (not mongodb / custom)
  const hasHostFields = new Set<ConnectionKind>(['ssh', 'mysql', 'postgres', 'redis', 'clickhouse']);
  // Which kinds can have a password (all except ssh by default)
  const hasPasswordField = new Set<ConnectionKind>(['mysql', 'postgres', 'redis', 'mongodb', 'clickhouse', 'custom']);
  // Kinds whose terminal client can run ON a jump host (`params.jump`). MongoDB
  // and Custom terminals ignore it, so they don't offer the toggle.
  const jumpKinds = new Set<ConnectionKind>(['mysql', 'postgres', 'redis', 'clickhouse']);

  // svelte-ignore state_referenced_locally
  let name = $state(existing?.name ?? prefill?.name ?? '');
  // svelte-ignore state_referenced_locally
  let kind: ConnectionKind = $state(existing?.kind ?? (prefill?.kind && kinds.includes(prefill.kind) ? prefill.kind : undefined) ?? kinds[0] ?? 'ssh');

  // Initialise individual fields from existing params (or prefill) so they survive kind switches.
  // svelte-ignore state_referenced_locally
  let fHost       = $state((existing?.params?.host        as string)  ?? prefill?.host ?? '');
  // svelte-ignore state_referenced_locally
  let fPort       = $state((existing?.params?.port        as string|number|undefined) != null
    ? String(existing!.params!.port) : (prefill?.port != null ? String(prefill.port) : ''));
  // svelte-ignore state_referenced_locally
  let fUser       = $state((existing?.params?.user        as string)  ?? prefill?.user ?? '');
  // svelte-ignore state_referenced_locally
  let fDb         = $state(((existing?.params?.db ?? existing?.params?.database) as string) ?? prefill?.db ?? '');
  // svelte-ignore state_referenced_locally
  let fTimezone   = $state((existing?.params?.timezone    as string)  ?? 'UTC');
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

  // ── Environment + write guardrail ──────────────────────────────────────────
  // `prod` connections (and any read-only profile) refuse writes/DDL in the DB
  // Explorer unless the user types a confirmation — see the database module.
  // svelte-ignore state_referenced_locally
  let environment = $state<Environment>(existing?.environment ?? prefill?.environment ?? 'dev');
  // svelte-ignore state_referenced_locally
  let readOnly = $state(existing?.read_only ?? false);

  // ── TLS + SSH-tunnel (DB engines) ──────────────────────────────────────────
  // These write structured objects into params.tls / params.ssh for the four DB
  // kinds, on top of the existing flat jump/identity_file fields (left intact).
  const tlsKinds = new Set<ConnectionKind>(['mysql', 'postgres', 'redis', 'mongodb', 'clickhouse']);
  // Engines with a meaningful session time zone (renders datetimes in this zone).
  const tzKinds = new Set<ConnectionKind>(['mysql', 'postgres', 'clickhouse']);
  type TlsMode = 'disabled' | 'preferred' | 'required';
  // svelte-ignore state_referenced_locally
  const existingTls = (existing?.params?.tls as Record<string, unknown> | undefined) ?? undefined;
  // svelte-ignore state_referenced_locally
  const existingSsh = (existing?.params?.ssh as Record<string, unknown> | undefined) ?? undefined;

  // svelte-ignore state_referenced_locally
  let tlsMode = $state<TlsMode>(((existingTls?.mode as TlsMode) ?? (existing?.params?.secure === true ? 'required' : (prefill?.tlsMode ?? 'disabled'))));
  // svelte-ignore state_referenced_locally
  let tlsVerify = $state(existingTls?.verify !== undefined ? !!existingTls.verify : true);
  // svelte-ignore state_referenced_locally
  let tlsCaCert = $state((existingTls?.ca_cert as string) ?? '');
  // svelte-ignore state_referenced_locally
  let tlsClientCert = $state((existingTls?.client_cert as string) ?? '');
  // svelte-ignore state_referenced_locally
  let tlsClientKey = $state((existingTls?.client_key as string) ?? '');
  // svelte-ignore state_referenced_locally
  let tlsServerName = $state((existingTls?.server_name as string) ?? '');

  // svelte-ignore state_referenced_locally
  let tunnelOpen = $state(!!existingSsh);
  // svelte-ignore state_referenced_locally
  let tunHost = $state((existingSsh?.host as string) ?? '');
  // svelte-ignore state_referenced_locally
  let tunPort = $state((existingSsh?.port as string | number | undefined) != null ? String(existingSsh!.port) : '');
  // svelte-ignore state_referenced_locally
  let tunUser = $state((existingSsh?.user as string) ?? '');
  // svelte-ignore state_referenced_locally
  let tunIdentity = $state((existingSsh?.identity_file as string) ?? '');
  // svelte-ignore state_referenced_locally
  let tunJump = $state((existingSsh?.proxy_jump as string) ?? '');

  // Section assignment (workspace connections only). `''` = ungrouped.
  // svelte-ignore state_referenced_locally
  let sectionId = $state<string>(existing?.section_id ?? '');
  // The shared section tree (one cached read for the whole app).
  const sections: ConnectionSection[] = $derived(connectionList.sections);
  let creatingSection = $state(false);
  let newSectionName = $state('');
  // Global (root-managed) connections are not assignable to a workspace section.
  // svelte-ignore state_referenced_locally
  const isGlobal = existing != null && existing.workspace_id === null;

  $effect(() => {
    if (!ws.currentId || isGlobal) return;
    void connectionList.ensure();
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
      connectionList.sections = [...connectionList.sections, sec];
      sectionId = sec.id;
      newSectionName = '';
      creatingSection = false;
    } catch (e) {
      toastError('Couldn’t create section', e);
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
    // A test outcome belongs to the kind it probed — don't leave a stale ✓/✗.
    if (k !== kind) testResult = null;
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
    const p: Record<string, unknown> = existing?.kind === kind ? { ...existing.params } : {};
    // Preserve fields this form does not own (custom placeholders, driver
    // options). Only explicitly controlled fields are replaced or cleared.
    const controlled = kind === 'mongodb' ? ['conn_string'] : kind === 'custom' ? ['command_template'] : ['host', 'port', 'user', 'db', 'database'];
    for (const key of controlled) delete p[key];
    for (const key of ['jump', 'identity_file']) delete p[key];
    if (tzKinds.has(kind)) delete p.timezone;
    if (tlsKinds.has(kind)) { delete p.tls; delete p.ssh; if (tlsMode === 'disabled') delete p.secure; }

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

    // Session time zone (default UTC) for engines that honor it.
    if (tzKinds.has(kind)) {
      const tz = fTimezone.trim();
      if (tz && tz.toUpperCase() !== 'UTC') p['timezone'] = tz;
    }

    // SSH section fields (only when SSH toggle is on)
    if (sshEnabled) {
      if (fJump)     p['jump']           = fJump;
      if (fIdentity) p['identity_file']  = fIdentity;
    }

    // TLS (DB engines only). Persist when explicitly enabled with a mode.
    if (tlsKinds.has(kind) && tlsMode !== 'disabled') {
      const tls: Record<string, unknown> = { ...(existing?.params?.tls as Record<string, unknown> ?? {}), mode: tlsMode, verify: tlsVerify };
      for (const key of ['ca_cert', 'client_cert', 'client_key', 'server_name']) delete tls[key];
      if (tlsCaCert)     tls['ca_cert']     = tlsCaCert;
      if (tlsClientCert) tls['client_cert'] = tlsClientCert;
      if (tlsClientKey)  tls['client_key']  = tlsClientKey;
      if (tlsServerName) tls['server_name'] = tlsServerName;
      p['tls'] = tls;
    }

    // SSH tunnel (DB engines only). Requires at least a host.
    if (tlsKinds.has(kind) && tunnelOpen && tunHost.trim()) {
      const ssh: Record<string, unknown> = { ...(existing?.params?.ssh as Record<string, unknown> ?? {}), host: tunHost.trim() };
      for (const key of ['port', 'user', 'identity_file', 'proxy_jump']) delete ssh[key];
      if (tunJump.trim()) ssh['proxy_jump'] = tunJump.trim();
      if (tunPort !== '') ssh['port'] = Number(tunPort);
      if (tunUser)        ssh['user'] = tunUser;
      if (tunIdentity)    ssh['identity_file'] = tunIdentity;
      p['ssh'] = ssh;
    }

    return p;
  }

  // ── DSN / URI paste import (parsing: connectionUri.ts) ─────────────────────
  // Fills the form fields in place. Returns the inline field error for a URI it
  // can't read (unparseable, unknown scheme) — the caller keeps the pasted
  // value next to it — or null once the fields are filled.
  function parseDsn(raw: string): string | null {
    const r = parseConnectionUri(raw);
    if (!r) return null;
    if ('error' in r) return r.error;
    setKind(r.kind);
    if (r.connString !== undefined) {
      secret = r.secret ?? '';
      fConnString = r.connString;
    } else {
      fHost = r.host ?? '';
      fPort = r.port ?? '';
      fUser = r.user ?? '';
      secret = r.secret ?? '';
      fDb = r.db ?? '';
    }
    if (r.tlsMode) tlsMode = r.tlsMode;
    if (r.tlsVerify === false) tlsVerify = false;
    if (r.ignored.length) toasts.warn('URI options need review', `Not imported: ${r.ignored.join(', ')}`);
    if (!name && r.name) name = r.name;
    toasts.success('URI parsed', `${kindLabels[r.kind]} — fields populated`);
    return null;
  }

  let dsnValue = $state('');
  /** Inline error for the pasted URI (validation is inline, never a toast). */
  let dsnError: string | null = $state(null);

  /** Clear and close only on success; on an error the field stays open with
   *  the pasted value beside the reason, so it can be fixed in place. */
  function commitDsn(): void {
    dsnError = parseDsn(dsnValue);
    // Spoken at the edge: the inline error below is plain text (an alert
    // inserted with its text is not read by WebKit/VoiceOver).
    if (dsnError) {
      announce(dsnError, { assertive: true });
      return;
    }
    dsnValue = '';
    step = 'fields';
  }

  // ── Two steps for a new connection ──────────────────────────────────────
  // 1. paste a URI or pick a kind; 2. that kind's few fields + Test. An edit,
  // a prefill (AWS RDS) or a single offered kind start on step 2.
  // svelte-ignore state_referenced_locally
  let step = $state<'pick' | 'fields'>(existing || prefill?.host || kinds.length === 1 ? 'fields' : 'pick');
  function pickKind(k: ConnectionKind): void {
    setKind(k);
    step = 'fields';
  }
  // "More options" (section, environment, time zone, TLS, SSH, first command)
  // opens by itself when the profile being edited already uses any of them.
  // svelte-ignore state_referenced_locally
  let moreOpen = $state(
    !!existing &&
      (tlsMode !== 'disabled' || !!existingSsh || !!(existing.params?.jump || existing.params?.identity_file) ||
        !!existing.first_command || (tzKinds.has(existing.kind) && !!fTimezone && fTimezone.toUpperCase() !== 'UTC') ||
        existing.environment !== 'dev' || existing.read_only || !!existing.section_id) ||
      (!existing && (!!prefill?.environment && prefill.environment !== 'dev')),
  );
  /** What is set under the folded More options, in words. */
  const moreSummary = $derived(
    [
      environment === 'prod' ? 'Production' : environment === 'staging' ? 'Staging' : '',
      readOnly ? 'read-only' : '',
      tlsKinds.has(kind) && tlsMode !== 'disabled' ? 'TLS' : '',
      tlsKinds.has(kind) && tunnelOpen && tunHost.trim() ? 'SSH tunnel' : '',
      sectionId ? (sections.find((s) => s.id === sectionId)?.name ?? '') : '',
    ].filter(Boolean).join(' · '),
  );

  // ── Test-before-save ────────────────────────────────────────────────────
  // Probes the CURRENT form values (unsaved) — DB kinds via
  // /connections/unsaved/db/test (driver), SSH via /connections/unsaved/test
  // (`ssh … exit`). Nothing is written to the DB or Keychain until Save.
  const testableKinds = new Set<ConnectionKind>(['ssh', 'mysql', 'postgres', 'clickhouse', 'redis', 'mongodb']);
  let testing = $state(false);
  /** `detail` is the full outcome line; `hint` says what to change on a
   *  recognised failure; `warn` carries the key-permission warning. */
  let testResult = $state<{ ok: boolean; detail: string; hint?: string; warn?: string; stderr?: string; argv?: boolean } | null>(null);
  let testPanel = $state<HTMLElement | null>(null);
  // Password on the client's command line: the last Test's `warn_argv` (the daemon's verdict), else the same rule locally.
  const argvWarn = $derived.by(() => argvWarningFor(kind, testResult?.argv, fConnString, secret !== '' || !!existing?.secret_ref));

  /** The settings the shown result was measured against — editing any field
   *  afterwards clears it, so a green "Connected" never vouches for values
   *  that were never tested. */
  let testedSig = '';
  const formSig = $derived(JSON.stringify([kind, buildParams(), secret]));
  $effect(() => {
    const sig = formSig;
    untrack(() => {
      if (testResult && !testing && sig !== testedSig) testResult = null;
    });
  });

  let testSeq = 0;
  $effect(() => () => { testSeq++; });
  async function testUnsaved(): Promise<void> {
    if (testing || !auth.isRoot) return;
    const seq = ++testSeq;
    testing = true; testResult = null; testedSig = formSig;
    try {
      if (kind === 'ssh') {
        const req: TestUnsavedConnectionReq = { workspace_id: ws.currentId ?? '', kind: 'ssh', params: buildParams() };
        const res = await api.post<TestConnectionResp>('/connections/unsaved/test', req);
        if (seq !== testSeq) return;
        testResult = {
          ok: res.ok, detail: res.ok ? `Connected${res.latency_ms != null ? ` (${res.latency_ms}ms)` : ''}` : res.message,
          hint: res.hint ?? undefined, warn: res.warn_key_perms ?? undefined, stderr: res.detail ?? undefined, argv: typeof res.warn_argv === 'boolean' ? res.warn_argv : undefined,
        };
      } else {
        const body: Record<string, unknown> = { workspace_id: ws.currentId, kind, params: buildParams() };
        if (existing) body.connection_id = existing.id;
        if (auth.isRoot && secret !== '') body.secret = secret;
        const res = await api.post<{ ok: boolean; latency_ms?: number; message: string; server_version?: string; warn_argv?: boolean }>('/connections/unsaved/db/test', body);
        if (seq !== testSeq) return;
        const bits = [res.server_version, res.latency_ms != null ? `${res.latency_ms}ms` : null].filter(Boolean).join(' · ');
        testResult = { ok: res.ok, detail: bits ? `${res.message} (${bits})` : res.message, argv: res.warn_argv };
      }
    } catch (e) {
      if (seq === testSeq) testResult = { ok: false, detail: loadErrorText(e) };
    } finally {
      if (seq === testSeq) testing = false;
    }
    if (seq === testSeq && formSig !== testedSig) testResult = null;
    if (seq === testSeq && testResult && (!testResult.ok || testResult.warn)) queueMicrotask(() => testPanel?.scrollIntoView({ block: 'nearest' }));
  }

  async function save(): Promise<void> {
    if (busy || (!existing && !auth.isRoot)) return;
    busy = true;
    const body: UpsertConnectionReq = {
      name: name.trim(),
      kind: !auth.isRoot && existing ? existing.kind : kind,
      params: !auth.isRoot && existing ? existing.params : buildParams(),
      first_command: !auth.isRoot && existing ? existing.first_command : firstCommand.trim() === '' ? null : firstCommand.trim(),
      section_id: sectionId === '' ? null : sectionId,
      environment: !auth.isRoot && existing ? existing.environment : environment,
      read_only: !auth.isRoot && existing ? existing.read_only : readOnly,
    };
    if (auth.isRoot && secret !== '') body.secret = secret;
    try {
      const saved = existing
        ? await api.patch<Connection>(`/connections/${existing.id}`, body)
        : await api.post<Connection>(`/workspaces/${ws.currentId}/connections`, body);
      toasts.success(existing ? 'Connection updated' : 'Connection created', saved.name);
      onsaved(saved);
    } catch (e) {
      toastError('Couldn’t save the connection', e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={existing ? `Edit ${existing.name}` : 'New connection'} width={500} {onclose}>
  {#if step === 'pick'}
    <!-- Step 1: paste a URI (the fastest path) or pick a kind. -->
    <div class="field">
      <label for="cf-dsn">Paste a connection URI</label>
      <div class="dsn-row">
        <input dir="ltr" id="cf-dsn"
          class="input mono"
          bind:value={dsnValue}
          placeholder="mysql://user:pass@host:3306/db"
          spellcheck="false"
          aria-invalid={dsnError ? 'true' : undefined}
          aria-describedby={dsnError ? 'dsn-err' : undefined}
          oninput={() => (dsnError = null)}
          onkeydown={(e) => {
            if (e.key === 'Enter') commitDsn();
          }}
        />
        <button class="btn small primary" disabled={!dsnValue.trim()} onclick={commitDsn}>Continue</button>
      </div>
      {#if dsnError}<p class="field-error" id="dsn-err">{dsnError}</p>{/if}
    </div>
    <div class="field">
      <span class="group-label" id="cf-pick-label">…or pick a kind</span>
      <div class="kind-tiles" role="group" aria-labelledby="cf-pick-label">
        {#each kinds as k (k)}
          <button class="kind-tile" onclick={() => pickKind(k)}>{kindLabels[k]}</button>
        {/each}
      </div>
    </div>
  {:else}
  {#if !existing && kinds.length > 1}
    <p class="kind-line"><span>{kindLabels[kind]}</span> <button class="link-btn" onclick={() => (step = 'pick')}>Change</button></p>
  {/if}

  <!-- Name -->
  <div class="field">
    <label for="cf-name">Name</label>
    <input dir="auto" id="cf-name" class="input" bind:value={name} placeholder="e.g. orders staging" />
  </div>

  {#if !auth.isRoot}<p class="hint">Owner manages credentials, native connection settings and environment.</p>{/if}
  <fieldset class="native-fields" disabled={!auth.isRoot}>
  {#if argvWarn}<div class="warn-banner" data-testid="argv-warning">{argvWarn}</div>{/if}

  <!-- MongoDB: connection string only -->
  {#if kind === 'mongodb'}
    <div class="field">
      <label for="cf-conn-string">Connection string</label>
      <input dir="ltr"
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
      <input dir="ltr"
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
      <input dir="ltr"
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
          placeholder={kind === 'ssh' ? '22' : kind === 'redis' ? '6379' : kind === 'clickhouse' ? '8123' : kind === 'postgres' ? '5432' : '3306'}
        />
        {#if kind === 'clickhouse'}
          <span class="hint">Use the HTTP interface — 8123 (plain, the default when empty) or 8443 (TLS). The native ports 9000 / 9440 aren’t supported.</span>
        {/if}
      </div>
      <div class="field grow">
        <label for="cf-user">User <span class="dim">(optional)</span></label>
        <input dir="ltr"
          id="cf-user"
          class="input mono"
          bind:value={fUser}
          placeholder={kind === 'ssh' ? 'default (~/.ssh/config or $USER)' : 'root'}
          spellcheck="false"
        />
        {#if kind === 'ssh'}
          <span class="hint">Leave empty to let ssh resolve it — a <span class="mono">Host</span>/<span class="mono">User</span> entry in <span class="mono">~/.ssh/config</span>, else your local username.</span>
        {/if}
      </div>
    </div>

    {#if hasDatabaseField.has(kind)}
      <div class="field">
        <label for="cf-db">
          {kind === 'redis' ? 'DB index' : 'Database'}
          <span class="dim">(optional)</span>
        </label>
        <input dir="ltr"
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
  </fieldset>

  <!-- Everything a first connection doesn't need: open on demand, and on its
       own when editing a profile that already uses any of it. -->
  <button class="more-toggle" aria-expanded={moreOpen} aria-controls="cf-more" onclick={() => (moreOpen = !moreOpen)}>
    <Icon name={moreOpen ? 'chevronDown' : 'chevronRight'} size={12} /> More options
    {#if !moreOpen && moreSummary}<span class="dim">· {moreSummary}</span>{/if}
  </button>
  {#if moreOpen}
  <div id="cf-more" class="more-body">
  <!-- Section (workspace connections only) -->
  {#if !isGlobal}
    <div class="field">
      <label for="cf-section">Section <span class="dim">(optional)</span></label>
      {#if creatingSection}
        <div class="section-new">
          <input dir="auto" aria-label="New section name"
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
          <button class="btn small" onclick={() => (creatingSection = true)}><Icon name="plus" size={12} /> New</button>
        </div>
      {/if}
    </div>
  {/if}
  <fieldset class="native-fields" disabled={!auth.isRoot}>
  {#if existing}
  <!-- Kind -->
  <div class="field">
    <span class="group-label" id="cf-kind-label">Kind</span>
    <div class="kind-row" role="group" aria-labelledby="cf-kind-label">
      {#each kinds as k (k)}
        <button class="kind-chip" class:selected={kind === k} aria-pressed={kind === k} onclick={() => setKind(k)}>{kindLabels[k]}</button>
      {/each}
    </div>
  </div>
  {/if}
  <!-- Environment + write guardrail -->
  <div class="field">
    <span class="group-label" id="cf-env-label">Environment</span>
    <div class="env-row" role="group" aria-labelledby="cf-env-label">
      {#each (['dev', 'staging', 'prod'] as Environment[]) as e (e)}
        <button
          class="env-chip"
          class:selected={environment === e}
          class:prod={e === 'prod'}
          aria-pressed={environment === e}
          onclick={() => (environment = e)}
        >{e}</button>
      {/each}
    </div>
    {#if environment === 'prod'}
      <span class="hint danger">
        {#if kind === 'ssh'}
          Production — shown with a prod badge; file uploads, renames and deletes ask you to confirm first.
        {:else if kind === 'custom'}
          Production — shown with a prod badge so it can’t be mistaken for a dev target.
        {:else}
          Production — writes &amp; schema changes are blocked until you type a confirmation.
        {/if}
      </span>
    {/if}
  </div>

  <div class="field ssh-toggle-row">
    <label class="toggle-label">
      <input type="checkbox" bind:checked={readOnly} />
      Read-only <span class="dim">(writes / DDL need the connection name typed to confirm, even outside prod)</span>
    </label>
  </div>

  <!-- Session time zone (mysql / clickhouse) -->
  {#if tzKinds.has(kind)}
    <div class="field">
      <label for="cf-tz">Timezone <span class="dim">(session; default UTC)</span></label>
      <input dir="ltr"
        id="cf-tz"
        class="input mono"
        list="cf-tz-list"
        bind:value={fTimezone}
        placeholder="UTC"
        spellcheck="false"
      />
      <datalist id="cf-tz-list">
        <option value="UTC"></option>
        <option value="+00:00"></option>
        <option value="+03:00"></option>
        <option value="-05:00"></option>
        <option value="Europe/London"></option>
        <option value="Europe/Moscow"></option>
        <option value="America/New_York"></option>
        <option value="Asia/Jerusalem"></option>
      </datalist>
      <span class="hint">View datetimes as the DB treats them — offset (e.g. +03:00) or a named zone. MySQL: <code>SET time_zone</code>; ClickHouse: <code>session_timezone</code>.</span>
    </div>
  {/if}

  <!-- TLS / SSL (DB engines) — single control; "Disabled" = no encryption -->
  {#if tlsKinds.has(kind)}
    <div class="field">
      <label for="cf-tls-mode">TLS / SSL</label>
      <select id="cf-tls-mode" class="input" bind:value={tlsMode}>
        <option value="disabled">Disabled (no encryption)</option>
        <option value="preferred">Preferred (use if available)</option>
        <option value="required">Required</option>
      </select>
    </div>
    {#if tlsMode !== 'disabled'}
      <div class="ssh-section">
        <div class="field">
          <label class="toggle-label">
            <input type="checkbox" bind:checked={tlsVerify} />
            Verify server certificate
          </label>
          <span class="hint">Turn off to accept self-signed / invalid certificates.</span>
        </div>
        <div class="field"><PemField id="cf-tls-ca" label="CA certificate — file path or pasted PEM" bind:value={tlsCaCert} placeholder="~/certs/ca.pem or -----BEGIN CERTIFICATE-----" /></div>
        <div class="field-row">
          <div class="field grow"><PemField id="cf-tls-cert" label="Client cert — path or PEM" bind:value={tlsClientCert} placeholder="~/certs/client.pem or pasted PEM" /></div>
          <div class="field grow"><PemField id="cf-tls-key" label="Client key — path or PEM" bind:value={tlsClientKey} placeholder="~/certs/client-key.pem or pasted PEM" /></div>
        </div>
        <div class="field">
          <label for="cf-tls-sni">Server name (SNI) <span class="dim">(optional)</span></label>
          <input dir="ltr" id="cf-tls-sni" class="input mono" bind:value={tlsServerName} placeholder="db.example.com" spellcheck="false" />
        </div>
      </div>
    {/if}

    <!-- SSH tunnel (structured) -->
    <div class="field ssh-toggle-row">
      <label class="toggle-label">
        <input type="checkbox" bind:checked={tunnelOpen} />
        SSH tunnel <span class="dim">(Database Explorer reaches the DB through a bastion)</span>
      </label>
    </div>
    {#if tunnelOpen}
      <div class="ssh-section">
        <TunnelFields bind:host={tunHost} bind:port={tunPort} bind:user={tunUser} bind:jump={tunJump} bind:identity={tunIdentity} />
      </div>
    {/if}
  {/if}

  <!-- SSH toggle (kinds whose terminal can run on a jump host; an existing
       jump on another kind stays visible so it can be cleared) -->
  {#if kind !== 'ssh' && (jumpKinds.has(kind) || sshEnabled)}
    <div class="field ssh-toggle-row">
      <label class="toggle-label">
        <input type="checkbox" bind:checked={sshEnabled} />
        Connect via SSH <span class="dim">(terminal runs the client on the jump host)</span>
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
        <input dir="ltr"
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
          <input dir="ltr"
            id="cf-identity"
            class="input mono grow"
            bind:value={fIdentity}
            placeholder="~/.ssh/id_rsa"
            spellcheck="false"
          />
          <button class="btn browse-btn" onclick={() => (showFilePicker = true)}>Browse…</button>
        </div>
        <span class="hint">Leave empty to use ssh-agent / <span class="mono">~/.ssh/config</span>; otherwise ssh asks for a password in the terminal. Key file must be private (<span class="mono">chmod 600</span>) or ssh ignores it.</span>
      </div>
      {#if kind !== 'ssh' && fJump && hasPasswordField.has(kind) && kind !== 'clickhouse'}
        <span class="hint">
          The saved password stays on this Mac — the terminal client on the jump host asks for it
          ({kind === 'redis' ? 'run AUTH in redis-cli' : 'type it at the prompt'}). The Database Explorer
          uses the SSH tunnel above instead and signs in automatically.
        </span>
      {/if}
    </div>
  {/if}

  <!-- First command -->
  <div class="field">
    <label for="cf-first">First command <span class="dim">(optional)</span></label>
    <input dir="ltr"
      id="cf-first"
      class="input mono"
      bind:value={firstCommand}
      placeholder="e.g. USE app_db; SHOW TABLES;"
      spellcheck="false"
    />
    <span class="hint">Sent to the terminal once the client connects.</span>
  </div>
  </fieldset>
  </div>
  {/if}
  {/if}


  <!-- Test outcome: the full message (wrapping, selectable) plus what to fix.
       The footer chip only summarises it — and the chip is the ONE live
       region, so this panel is plain content (no second announcement). -->
  {#if testResult && (!testResult.ok || testResult.hint || testResult.warn)}
    <div class="test-panel {testResult.ok ? 'ok' : 'err'}" bind:this={testPanel}>
      <div class="test-panel-head">
        <Icon name={testResult.ok ? 'check' : 'x'} size={12} />
        <span>{testResult.ok ? 'Connected' : 'Test failed'}</span>
      </div>
      {#if !testResult.ok}<p class="test-panel-msg mono">{testResult.detail}</p>{/if}
      {#if testResult.hint}<p class="test-panel-hint"><strong>What to check:</strong> {testResult.hint}</p>{/if}
      {#if testResult.warn}<p class="test-panel-warn">{testResult.warn}</p>{/if}
      {#if !testResult.ok && existing && ondiagnose && (kind === 'ssh' || tlsKinds.has(kind))}<div><button class="btn small" onclick={() => testResult && ondiagnose?.({ message: testResult.detail, hint: testResult.hint, detail: testResult.stderr })} data-testid="conn-diagnose"><Icon name="assistant" size={12} />Diagnose with an agent…</button></div>{/if}
    </div>
  {/if}

  {#snippet footer()}
    <!-- Always mounted: a live region inserted together with its text is
         usually silent in VoiceOver/WebKit, so this one stays in the DOM and
         its text changes in place (Testing… → Connected / Test failed: …). -->
    <span class="test-result {testResult ? (testResult.ok ? 'ok' : 'err') : ''}" title={testResult?.detail} role="status">
      {#if testing}
        <span class="sr-only">Testing…</span>
      {:else if testResult}
        <Icon name={testResult.ok ? 'check' : 'x'} size={12} />
        {#if testResult.ok}{testResult.detail}{:else}Test failed<span class="sr-only">: {testResult.detail}</span><span aria-hidden="true"> — see details above</span>{/if}
      {/if}
    </span>
    <button class="btn" onclick={onclose}>Cancel</button>
    {#if step === 'fields'}
    {#if testableKinds.has(kind)}
      <button
        class="btn"
        disabled={!auth.isRoot || testing || busy}
        title={!auth.isRoot ? 'Only the owner can test connection settings' : 'Test these settings without saving'}
        onclick={testUnsaved}
      >
        {testing ? 'Testing…' : 'Test'}
      </button>
    {/if}
    <button
      class="btn primary"
      disabled={(!existing && !auth.isRoot) || busy || name.trim() === ''}
      title={!existing && !auth.isRoot ? 'Only the owner can create connections' : name.trim() === '' ? 'Enter a name first' : undefined}
      onclick={save}
    >
      {busy ? 'Saving…' : existing ? 'Save changes' : 'Create connection'}
    </button>
    {/if}
  {/snippet}
</Modal>

<!-- Identity file picker (file-pick mode) -->
{#if showFilePicker}
  <FolderPicker
    title="Choose identity file"
    start={fIdentity ? fIdentity.replace(/\/[^/]+$/, '') : ''}
    files={true}
    onpick={(path) => { fIdentity = path; showFilePicker = false; }}
    onclose={() => (showFilePicker = false)}
  />
{/if}

<style>
  .native-fields { border:0; padding:0; margin:0; min-width:0; }
  .kind-tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: var(--sp-2);
  }
  .kind-tile {
    min-block-size: var(--hit-min);
    padding: var(--sp-3) var(--sp-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .kind-tile:hover {
    background: var(--hover);
  }
  .kind-line {
    margin: 0 0 var(--sp-3);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .link-btn {
    border: 0;
    padding: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .more-toggle {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-block-size: var(--hit-min);
    margin-block: var(--sp-2);
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .more-body {
    display: flex;
    flex-direction: column;
  }
  /* Same look as `.field > label` for a group caption that labels a chip row
     (a <label> must point at a single form control). */
  .group-label { font-size: var(--fs-s); font-weight: 500; color: var(--text-dim); }
  .kind-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .kind-chip {
    height: 24px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
    transition: background var(--dur-fast) ease-out, border-color var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
  }
  .kind-chip.selected {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
    font-weight: 500;
  }
  .env-row {
    display: flex;
    gap: 6px;
  }
  .env-chip {
    height: 24px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
    text-transform: capitalize;
    transition: background var(--dur-fast) ease-out, border-color var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
  }
  .env-chip.selected {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
    font-weight: 500;
  }
  /* Production selected → red danger styling. */
  .env-chip.prod.selected {
    background: var(--danger-soft);
    border-color: color-mix(in srgb, var(--danger) 55%, transparent);
    color: var(--danger);
  }
  .hint.danger {
    color: var(--danger);
  }
  .warn-banner {
    font-size: var(--fs-s);
    line-height: 1.5;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--warning) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
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
    font-size: var(--fs-m);
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
    border: 1px solid var(--accent-line);
    background: var(--accent-faint);
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
  /* DSN / URI paste import */
  .field-error {
    margin: -4px 0 8px;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .dsn-row {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-bottom: 8px;
  }
  .dsn-row .input {
    flex: 1;
    min-width: 0;
  }
  /* Inline unsaved-config test outcome (footer, before the buttons). */
  .test-result {
    font-size: var(--fs-s);
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-inline-end: auto;
  }
  .test-result :global(svg) {
    vertical-align: -2px;
  }
  .test-result.ok {
    color: var(--success);
  }
  .test-result.err {
    color: var(--danger);
  }
  .test-panel {
    margin-top: 12px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    background: var(--danger-soft);
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  .test-panel.ok {
    border-color: color-mix(in srgb, var(--success) 40%, transparent);
    background: var(--success-soft);
  }
  .test-panel-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    color: var(--danger);
  }
  .test-panel.ok .test-panel-head {
    color: var(--success);
  }
  .test-panel p {
    margin: 6px 0 0;
    overflow-wrap: anywhere;
  }
  .test-panel-msg {
    user-select: text;
    white-space: pre-wrap;
  }
  .test-panel-warn {
    color: var(--warning);
  }
</style>
