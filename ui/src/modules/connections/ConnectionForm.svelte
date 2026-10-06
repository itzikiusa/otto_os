<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  import { toastError } from '../../lib/toastError';
  // New/Edit connection sheet — unified form with optional SSH tunnel toggle.
  // Field layout: name / kind / host / port / user / database / password /
  //   [SSH section: jump host + identity file] / first command.
  import { auth } from '../../lib/stores/auth.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { api } from '../../lib/api/client';
  import type {
    Connection,
    ConnectionKind,
    ConnectionSection,
    Environment,
    TestConnectionResp,
    TestUnsavedConnectionReq,
    UpsertConnectionReq,
  } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { untrack } from 'svelte';

  interface Props {
    existing: Connection | null;
    onclose: () => void;
    onsaved: (c: Connection) => void;
    /** Restrict the kind selector (e.g. DB-only in the Database module). */
    kinds?: ConnectionKind[];
  }
  let {
    existing,
    onclose,
    onsaved,
    kinds = ['ssh', 'mysql', 'postgres', 'redis', 'mongodb', 'clickhouse', 'custom'],
  }: Props = $props();

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
  let name = $state(existing?.name ?? '');
  // svelte-ignore state_referenced_locally
  let kind: ConnectionKind = $state(existing?.kind ?? kinds[0] ?? 'ssh');

  // Initialise individual fields from existing params so they survive kind switches.
  // svelte-ignore state_referenced_locally
  let fHost       = $state((existing?.params?.host        as string)  ?? '');
  // svelte-ignore state_referenced_locally
  let fPort       = $state((existing?.params?.port        as string|number|undefined) != null
    ? String(existing!.params!.port) : '');
  // svelte-ignore state_referenced_locally
  let fUser       = $state((existing?.params?.user        as string)  ?? '');
  // svelte-ignore state_referenced_locally
  let fDb         = $state(((existing?.params?.db ?? existing?.params?.database) as string) ?? '');
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
  let environment = $state<Environment>(existing?.environment ?? 'dev');
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
  let tlsMode = $state<TlsMode>(((existingTls?.mode as TlsMode) ?? (existing?.params?.secure === true ? 'required' : 'disabled')));
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
  let showTunnelFilePicker = $state(false);

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
      for (const key of ['port', 'user', 'identity_file']) delete ssh[key];
      if (tunPort !== '') ssh['port'] = Number(tunPort);
      if (tunUser)        ssh['user'] = tunUser;
      if (tunIdentity)    ssh['identity_file'] = tunIdentity;
      p['ssh'] = ssh;
    }

    return p;
  }

  // ── DSN / URI paste import ──────────────────────────────────────────────────
  /** `decodeURIComponent` that never throws: a malformed escape (`%zz`) keeps
   *  the raw text instead of aborting the import with an uncaught error. */
  function safeDecode(v: string): string {
    try {
      return decodeURIComponent(v);
    } catch {
      return v;
    }
  }
  /** Split the password out of a URI's RAW text (scheme://user:PASS@hosts/…):
   *  the template keeps every other byte as pasted. `URL.password` can't be
   *  used to find it — WHATWG re-encodes characters like `^ { } | " < > \` and
   *  space, so a replace on its form missed and left the cleartext password in
   *  the visible connection-string field. */
  function splitUriPassword(text: string): { template: string; password: string } | null {
    const start = text.indexOf('://');
    if (start < 0) return null;
    const authStart = start + 3;
    const rest = text.slice(authStart);
    const end = rest.search(/[/?#]/);
    const authority = end < 0 ? rest : rest.slice(0, end);
    let at = authority.lastIndexOf('@');
    if (at < 0 && end >= 0) {
      // An unencoded `/` in the password (`app:12/ss@h/db` — WHATWG parses
      // it as host `app`, port `12`) ends the authority early. The real
      // separator is then the first `@` past that cut which comes before any
      // query: an `@` inside `?appName=a@b` is an option value, not userinfo.
      const query = rest.search(/[?#]/);
      const later = rest.indexOf('@', end);
      if (later >= 0 && (query < 0 || later < query)) at = later;
    }
    if (at < 0) return null;
    const userinfo = rest.slice(0, at);
    const colon = userinfo.indexOf(':');
    if (colon < 0) return null;
    return {
      template: text.slice(0, authStart) + userinfo.slice(0, colon) + ':{secret}' + text.slice(authStart + at),
      password: safeDecode(userinfo.slice(colon + 1)),
    };
  }
  // Parses a standard connection URI (mysql://, redis://, mongodb://, etc.) and
  // fills the form fields in-place. Unrecognized schemes are ignored with a toast.
  function parseDsn(raw: string): void {
    const s = raw.trim();
    if (!s) return;

    let url: URL;
    try {
      url = new URL(s);
    } catch {
      // The usual cause with credentials: an unencoded `/ ? # @` in the
      // password cuts the authority short (`app:pa/ss@h` → port `pa`).
      toasts.error(
        'Invalid URI',
        s.includes('@')
          ? 'Could not parse as a connection URL — percent-encode / ? # @ in the password (e.g. %2F for /).'
          : 'Could not parse as a connection URL',
      );
      return;
    }

    const scheme = url.protocol.replace(':', '').toLowerCase();
    const kindMap: Record<string, ConnectionKind> = {
      mysql: 'mysql',
      'mysql+tcp': 'mysql',
      postgres: 'postgres',
      postgresql: 'postgres',
      'postgres+tcp': 'postgres',
      redis: 'redis',
      rediss: 'redis',
      mongodb: 'mongodb',
      'mongodb+srv': 'mongodb',
      clickhouse: 'clickhouse',
      'clickhouse+https': 'clickhouse',
      'clickhouse+http': 'clickhouse',
      ssh: 'ssh',
    };
    const mapped = kindMap[scheme];
    if (!mapped) {
      toasts.error('Unknown scheme', `Unrecognized URI scheme: ${scheme}`);
      return;
    }

    setKind(mapped);

    if (mapped === 'mongodb') {
      // Keep topology/options intact but move credentials to the secret field.
      const split = splitUriPassword(s);
      secret = split?.password ?? '';
      fConnString = split ? split.template : s;
    } else {
      if (url.hostname) fHost = url.hostname;
      if (url.port)     fPort = url.port;
      if (url.username) fUser = safeDecode(url.username);
      if (url.password) secret = safeDecode(url.password);
      // The first path segment is the database/db-index.
      const dbPart = url.pathname.replace(/^\//, '').split('/')[0];
      if (dbPart) fDb = safeDecode(dbPart);
    }

    if (scheme === 'rediss' || scheme === 'clickhouse+https') tlsMode = 'required';
    const ssl = url.searchParams.get('sslmode') ?? url.searchParams.get('tls') ?? url.searchParams.get('ssl');
    if (ssl && ['true', '1', 'require', 'verify-ca', 'verify-full'].includes(ssl)) tlsMode = 'required';
    if (ssl && ['false', '0', 'disable'].includes(ssl)) tlsMode = 'disabled';
    if (url.searchParams.get('tlsAllowInvalidCertificates') === 'true') tlsVerify = false;
    if (mapped !== 'mongodb') {
      const supported = new Set(['sslmode', 'tls', 'ssl', 'tlsAllowInvalidCertificates']);
      const ignored = [...url.searchParams.keys()].filter((key) => !supported.has(key));
      if (ignored.length) toasts.warn('URI options need review', `Not imported: ${ignored.join(', ')}`);
    }

    // Auto-fill name from host+db if the name field is still empty.
    if (!name && url.hostname) {
      const dbPart = url.pathname.replace(/^\//, '').split('/')[0];
      name = dbPart ? `${url.hostname}/${dbPart}` : url.hostname;
    }

    toasts.success('URI parsed', `${kindLabels[mapped]} — fields populated`);
  }

  let showDsnInput = $state(false);
  let dsnValue = $state('');

  function commitDsn(): void {
    parseDsn(dsnValue);
    dsnValue = '';
    showDsnInput = false;
  }

  // ── Test-before-save ────────────────────────────────────────────────────
  // Probes the CURRENT form values (unsaved) — DB kinds via
  // /connections/unsaved/db/test (driver), SSH via /connections/unsaved/test
  // (`ssh … exit`). Nothing is written to the DB or Keychain until Save.
  const testableKinds = new Set<ConnectionKind>(['ssh', 'mysql', 'postgres', 'clickhouse', 'redis', 'mongodb']);
  let testing = $state(false);
  /** `detail` is the full outcome line; `hint` says what to change on a
   *  recognised failure; `warn` carries the key-permission warning. */
  let testResult = $state<{ ok: boolean; detail: string; hint?: string; warn?: string } | null>(null);
  let testPanel = $state<HTMLElement | null>(null);

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

  async function testUnsaved(): Promise<void> {
    if (testing || !auth.isRoot) return;
    testing = true;
    testResult = null;
    testedSig = formSig;
    try {
      if (kind === 'ssh') {
        // SSH profiles hold no secret — the probe uses this Mac's keys / agent.
        const req: TestUnsavedConnectionReq = { workspace_id: ws.currentId ?? '', kind: 'ssh', params: buildParams() };
        const res = await api.post<TestConnectionResp>('/connections/unsaved/test', req);
        testResult = {
          ok: res.ok,
          detail: res.ok
            ? `Connected${res.latency_ms != null ? ` (${res.latency_ms}ms)` : ''}`
            : res.message,
          hint: res.hint ?? undefined,
          warn: res.warn_key_perms ?? undefined,
        };
      } else {
        const body: Record<string, unknown> = {
          workspace_id: ws.currentId,
          kind,
          params: buildParams(),
        };
        // The server resolves the saved credential after checking root/configure
        // authority; it never returns the credential to this form.
        if (existing) body.connection_id = existing.id;
        if (auth.isRoot && secret !== '') body.secret = secret;
        const res = await api.post<{ ok: boolean; latency_ms?: number; message: string; server_version?: string }>(
          '/connections/unsaved/db/test',
          body,
        );
        const bits = [res.server_version, res.latency_ms != null ? `${res.latency_ms}ms` : null]
          .filter(Boolean)
          .join(' · ');
        testResult = { ok: res.ok, detail: bits ? `${res.message} (${bits})` : res.message };
      }
    } catch (e) {
      testResult = { ok: false, detail: e instanceof Error ? e.message : String(e) };
    } finally {
      testing = false;
    }
    // A failure's explanation lives at the end of the (scrollable) form body —
    // bring it into view so the user sees what to fix without hunting.
    if (testResult && (!testResult.ok || testResult.warn)) {
      queueMicrotask(() => testPanel?.scrollIntoView({ block: 'nearest' }));
    }
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
  <!-- DSN / URI paste import -->
  {#if !existing}
    {#if showDsnInput}
      <div class="field dsn-row">
        <input dir="ltr" aria-label="Connection string"
          class="input mono"
          bind:value={dsnValue}
          placeholder="mysql://user:pass@host:3306/db"
          spellcheck="false"
          onkeydown={(e) => {
            if (e.key === 'Enter') commitDsn();
            else if (e.key === 'Escape') { showDsnInput = false; dsnValue = ''; }
          }}
        />
        <button class="btn small primary" onclick={commitDsn}>Import</button>
        <button class="btn small" onclick={() => { showDsnInput = false; dsnValue = ''; }}>Cancel</button>
      </div>
    {:else}
      <div class="dsn-hint">
        <button class="btn small ghost dsn-btn" onclick={() => (showDsnInput = true)}>
          Paste connection URI&hellip;
        </button>
      </div>
    {/if}
  {/if}

  <!-- Name -->
  <div class="field">
    <label for="cf-name">Name</label>
    <input dir="auto" id="cf-name" class="input" bind:value={name} placeholder="staging mysql" />
  </div>

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

  {#if !auth.isRoot}<p class="hint">Owner manages credentials, native connection settings and environment.</p>{/if}
  <fieldset class="native-fields" disabled={!auth.isRoot}>
  <!-- Kind -->
  <div class="field">
    <span class="group-label" id="cf-kind-label">Kind</span>
    <div class="kind-row" role="group" aria-labelledby="cf-kind-label">
      {#each kinds as k (k)}
        <button class="kind-chip" class:selected={kind === k} aria-pressed={kind === k} onclick={() => setKind(k)}>{kindLabels[k]}</button>
      {/each}
    </div>
  </div>

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
      Read-only <span class="dim">(refuse all writes / DDL, even outside prod)</span>
    </label>
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
        <div class="field">
          <label for="cf-tls-ca">CA certificate path <span class="dim">(optional)</span></label>
          <PathField bind:value={tlsCaCert} files><input dir="ltr" id="cf-tls-ca" class="input mono" bind:value={tlsCaCert} placeholder="~/certs/ca.pem" spellcheck="false" /></PathField>
        </div>
        <div class="field-row">
          <div class="field grow">
            <label for="cf-tls-cert">Client cert <span class="dim">(optional)</span></label>
            <PathField bind:value={tlsClientCert} files><input dir="ltr" id="cf-tls-cert" class="input mono" bind:value={tlsClientCert} placeholder="~/certs/client.pem" spellcheck="false" /></PathField>
          </div>
          <div class="field grow">
            <label for="cf-tls-key">Client key <span class="dim">(optional)</span></label>
            <PathField bind:value={tlsClientKey} files><input dir="ltr" id="cf-tls-key" class="input mono" bind:value={tlsClientKey} placeholder="~/certs/client-key.pem" spellcheck="false" /></PathField>
          </div>
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
        <div class="field-row">
          <div class="field grow">
            <label for="cf-tun-host">Tunnel host</label>
            <input dir="ltr" id="cf-tun-host" class="input mono" bind:value={tunHost} placeholder="bastion.example.com" spellcheck="false" />
            {#if !tunHost.trim()}
              <span class="hint">Required — without a host the tunnel isn’t saved.</span>
            {/if}
          </div>
          <div class="field tun-port">
            <label for="cf-tun-port">Port <span class="dim">(opt)</span></label>
            <input id="cf-tun-port" class="input mono" type="number" bind:value={tunPort} placeholder="22" />
          </div>
        </div>
        <div class="field">
          <label for="cf-tun-user">Tunnel user <span class="dim">(optional)</span></label>
          <input dir="ltr" id="cf-tun-user" class="input mono" bind:value={tunUser} placeholder="ec2-user" spellcheck="false" />
        </div>
        <div class="field">
          <label for="cf-tun-identity">Identity file <span class="dim">(optional)</span></label>
          <div class="file-input-row">
            <input dir="ltr" id="cf-tun-identity" class="input mono grow" bind:value={tunIdentity} placeholder="~/.ssh/id_rsa" spellcheck="false" />
            <button class="btn browse-btn" onclick={() => (showTunnelFilePicker = true)}>Browse…</button>
          </div>
          <span class="hint">Key file must be private (<span class="mono">chmod 600</span>) or ssh ignores it.</span>
        </div>
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

  <!-- Test outcome: the full message (wrapping, selectable) plus what to fix.
       The footer chip only summarises it. -->
  {#if testResult && (!testResult.ok || testResult.hint || testResult.warn)}
    <div class="test-panel {testResult.ok ? 'ok' : 'err'}" bind:this={testPanel} role="status" aria-live="polite">
      <div class="test-panel-head">
        <Icon name={testResult.ok ? 'check' : 'x'} size={12} />
        <span>{testResult.ok ? 'Connected' : 'Test failed'}</span>
      </div>
      {#if !testResult.ok}<p class="test-panel-msg mono">{testResult.detail}</p>{/if}
      {#if testResult.hint}<p class="test-panel-hint"><strong>What to check:</strong> {testResult.hint}</p>{/if}
      {#if testResult.warn}<p class="test-panel-warn">{testResult.warn}</p>{/if}
    </div>
  {/if}

  {#snippet footer()}
    {#if testResult}
      <span class="test-result {testResult.ok ? 'ok' : 'err'}" title={testResult.detail} role="status">
        <Icon name={testResult.ok ? 'check' : 'x'} size={12} /> {testResult.ok ? testResult.detail : 'Test failed — see details above'}
      </span>
    {/if}
    <button class="btn" onclick={onclose}>Cancel</button>
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

<!-- SSH tunnel identity file picker -->
{#if showTunnelFilePicker}
  <FolderPicker
    title="Choose identity file"
    start={tunIdentity ? tunIdentity.replace(/\/[^/]+$/, '') : ''}
    files={true}
    onpick={(path) => { tunIdentity = path; showTunnelFilePicker = false; }}
    onclose={() => (showTunnelFilePicker = false)}
  />
{/if}

<style>
  .native-fields { border:0; padding:0; margin:0; min-width:0; }
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
  .tun-port {
    flex: 0 0 90px;
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
  .dsn-hint {
    margin-bottom: 6px;
  }
  .dsn-btn {
    color: var(--text-dim);
    font-size: var(--fs-s);
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
