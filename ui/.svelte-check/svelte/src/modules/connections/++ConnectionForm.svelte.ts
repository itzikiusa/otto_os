///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
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
import type { Connection, ConnectionKind, ConnectionSection,
    Environment, TestConnectionResp, TestUnsavedConnectionReq, UpsertConnectionReq } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { untrack } from 'svelte';
;

  interface Props {
    existing: Connection | null;
    onclose: () => void;
    onsaved: (c: Connection) => void;
    /** Restrict the kind selector (e.g. DB-only in the Database module). */
    kinds?: ConnectionKind[];
  };;
  type TlsMode = 'disabled' | 'preferred' | 'required';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
  // fills the form fields in-place. Returns the inline field error for a URI it
  // can't read (unparseable, unknown scheme) — the caller keeps the pasted
  // value next to it — or null once the fields are filled.
  function parseDsn(raw: string): string | null {
    const s = raw.trim();
    if (!s) return null;

    let url: URL;
    try {
      url = new URL(s);
    } catch {
      // The usual cause with credentials: an unencoded `/ ? # @` in the
      // password cuts the authority short (`app:pa/ss@h` → port `pa`).
      return s.includes('@')
        ? 'Couldn’t read this connection URL. Percent-encode / ? # @ in the password (for example %2F for /).'
        : 'Couldn’t read this connection URL. Start it with a scheme, for example mysql://host:3306/db.';
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
      return `Couldn’t read this connection URL: “${scheme}://” isn’t a scheme Otto knows. Use mysql, postgres, redis, mongodb, clickhouse or ssh.`;
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
    return null;
  }

  let showDsnInput = $state(false);
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
    showDsnInput = false;
  }
  function cancelDsn(): void {
    showDsnInput = false;
    dsnValue = '';
    dsnError = null;
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
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":existing ? `Edit ${existing.name}` : 'New connection',"width":500,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    
     { svelteHTML.createElement("span", {     "class":`test-result ${testResult ? (testResult.ok ? 'ok' : 'err') : ''}`,"title":testResult?.detail,"role":`status`,});
      if(testing){
         { svelteHTML.createElement("span", { "class":`sr-only`,});  }
      } else if (testResult){
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":testResult.ok ? 'check' : 'x',"size":12,}});}
        if(testResult.ok){testResult.detail;}else{  { svelteHTML.createElement("span", { "class":`sr-only`,}); testResult.detail; } { svelteHTML.createElement("span", { "aria-hidden":`true`,});     }}
      }
     }
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
    if(testableKinds.has(kind)){
       { svelteHTML.createElement("button", {         "class":`btn`,"disabled":!auth.isRoot || testing || busy,"title":!auth.isRoot ? 'Only the owner can test connection settings' : 'Test these settings without saving',"onclick":testUnsaved,});
        testing ? 'Testing…' : 'Test';
       }
    }
     { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":(!existing && !auth.isRoot) || busy || name.trim() === '',"title":!existing && !auth.isRoot ? 'Only the owner can create connections' : name.trim() === '' ? 'Enter a name first' : undefined,"onclick":save,});
      busy ? 'Saving…' : existing ? 'Save changes' : 'Create connection';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  
  if(!existing){
    if(showDsnInput){
       { svelteHTML.createElement("div", { "class":`field dsn-row`,});
         { svelteHTML.createElement("input", {                    "dir":`ltr`,"aria-label":`Connection string`,"class":`input mono`,"bind:value":dsnValue,"placeholder":`mysql://user:pass@host:3306/db`,"spellcheck":`false`,"aria-invalid":dsnError ? 'true' : undefined,"aria-describedby":dsnError ? 'dsn-err' : undefined,"oninput":() => (dsnError = null),"onkeydown":(e) => {
            if (e.key === 'Enter') commitDsn();
            else if (e.key === 'Escape') cancelDsn();
          },});/*Ωignore_startΩ*/() => dsnValue = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":commitDsn,});  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":cancelDsn,});  }
       }
      if(dsnError){ { svelteHTML.createElement("p", {   "class":`field-error`,"id":`dsn-err`,});dsnError; }}
    }else{
       { svelteHTML.createElement("div", { "class":`dsn-hint`,});
         { svelteHTML.createElement("button", {   "class":`btn small ghost dsn-btn`,"onclick":() => (showDsnInput = true),});
            
         }
       }
    }
  }

  
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`cf-name`,});  }
     { svelteHTML.createElement("input", {          "dir":`auto`,"id":`cf-name`,"class":`input`,"bind:value":name,"placeholder":`staging mysql`,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }

  
  if(!isGlobal){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`cf-section`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
      if(creatingSection){
         { svelteHTML.createElement("div", { "class":`section-new`,});
           { svelteHTML.createElement("input", {            "dir":`auto`,"aria-label":`New section name`,"class":`input`,"bind:value":newSectionName,"placeholder":`New section name`,"onkeydown":(e) => {
              if (e.key === 'Enter') createSection();
              else if (e.key === 'Escape') {
                creatingSection = false;
                newSectionName = '';
              }
            },});/*Ωignore_startΩ*/() => newSectionName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":createSection,});  }
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => {
              creatingSection = false;
              newSectionName = '';
            },});
            
           }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`section-row`,});
           { svelteHTML.createElement("select", {     "id":`cf-section`,"class":`input`,"bind:value":sectionId,});/*Ωignore_startΩ*/() => sectionId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
               for(let opt of __sveltets_2_ensureArray(sectionOptions)){opt.id;
               { svelteHTML.createElement("option", { "value":opt.id,});opt.label; }
            }
           }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (creatingSection = true),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
         }
      }
     }
  }

  if(!auth.isRoot){ { svelteHTML.createElement("p", { "class":`hint`,});        }}
   { svelteHTML.createElement("fieldset", {   "class":`native-fields`,"disabled":!auth.isRoot,});
  
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("span", {   "class":`group-label`,"id":`cf-kind-label`,});  }
     { svelteHTML.createElement("div", {     "class":`kind-row`,"role":`group`,"aria-labelledby":`cf-kind-label`,});
         for(let k of __sveltets_2_ensureArray(kinds)){k;
         { svelteHTML.createElement("button", {      "class":`kind-chip`,"aria-pressed":kind === k,"onclick":() => setKind(k),});kind === k;kindLabels[k]; }
      }
     }
   }

  
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("span", {   "class":`group-label`,"id":`cf-env-label`,});  }
     { svelteHTML.createElement("div", {     "class":`env-row`,"role":`group`,"aria-labelledby":`cf-env-label`,});
         for(let e of __sveltets_2_ensureArray((['dev', 'staging', 'prod'] as Environment[]))){e;
         { svelteHTML.createElement("button", {         "class":`env-chip`,"aria-pressed":environment === e,"onclick":() => (environment = e),});environment === e;e === 'prod';e; }
      }
     }
    if(environment === 'prod'){
       { svelteHTML.createElement("span", { "class":`hint danger`,});
        if(kind === 'ssh'){                } else if (kind === 'custom'){               }else{            }
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`field ssh-toggle-row`,});
     { svelteHTML.createElement("label", { "class":`toggle-label`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":readOnly,});/*Ωignore_startΩ*/() => readOnly = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        { svelteHTML.createElement("span", { "class":`dim`,});        }
     }
   }

  if(kind === 'clickhouse'){
     { svelteHTML.createElement("div", { "class":`warn-banner`,});
                  
       { svelteHTML.createElement("span", { "class":`mono`,});  }      
     }
  }

  
  if(kind === 'mongodb'){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`cf-conn-string`,});  }
       { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-conn-string`,"class":`input mono`,"bind:value":fConnString,"placeholder":`mongodb://host:27017/db`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fConnString = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

  
  } else if (kind === 'custom'){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`cf-template`,});  }
       { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-template`,"class":`input mono`,"bind:value":fTemplate,"placeholder":`psql -h ${'{host}'} -U ${'{user}'} ${'{db}'}   (${'{secret}'} available)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fTemplate = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

  
  }else{
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`cf-host`,});  }
       { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-host`,"class":`input mono`,"bind:value":fHost,"placeholder":kind === 'ssh' ? 'server.example.com' : 'db.internal',"spellcheck":`false`,});/*Ωignore_startΩ*/() => fHost = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

     { svelteHTML.createElement("div", { "class":`field-row`,});
       { svelteHTML.createElement("div", { "class":`field grow`,});
         { svelteHTML.createElement("label", { "for":`cf-port`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { svelteHTML.createElement("input", {           "id":`cf-port`,"class":`input mono`,"type":`number`,"bind:value":fPort,"placeholder":kind === 'ssh' ? '22' : kind === 'redis' ? '6379' : kind === 'clickhouse' ? '8123' : kind === 'postgres' ? '5432' : '3306',});/*Ωignore_startΩ*/() => fPort = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(kind === 'clickhouse'){
           { svelteHTML.createElement("span", { "class":`hint`,});                      }
        }
       }
       { svelteHTML.createElement("div", { "class":`field grow`,});
         { svelteHTML.createElement("label", { "for":`cf-user`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-user`,"class":`input mono`,"bind:value":fUser,"placeholder":kind === 'ssh' ? 'default (~/.ssh/config or $USER)' : 'root',"spellcheck":`false`,});/*Ωignore_startΩ*/() => fUser = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(kind === 'ssh'){
           { svelteHTML.createElement("span", { "class":`hint`,});          { svelteHTML.createElement("span", { "class":`mono`,});  }  { svelteHTML.createElement("span", { "class":`mono`,});  }    { svelteHTML.createElement("span", { "class":`mono`,});  }     }
        }
       }
     }

    if(hasDatabaseField.has(kind)){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cf-db`,});
          kind === 'redis' ? 'DB index' : 'Database';
           { svelteHTML.createElement("span", { "class":`dim`,});  }
         }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-db`,"class":`input mono`,"bind:value":fDb,"placeholder":kind === 'redis' ? '0' : 'mydb',"spellcheck":`false`,});/*Ωignore_startΩ*/() => fDb = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
    }
  }

  
  if(tzKinds.has(kind)){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`cf-tz`,});  { svelteHTML.createElement("span", { "class":`dim`,});   } }
       { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`cf-tz`,"class":`input mono`,"list":`cf-tz-list`,"bind:value":fTimezone,"placeholder":`UTC`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fTimezone = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("datalist", { "id":`cf-tz-list`,});
         { svelteHTML.createElement("option", { "value":`UTC`,}); }
         { svelteHTML.createElement("option", { "value":`+00:00`,}); }
         { svelteHTML.createElement("option", { "value":`+03:00`,}); }
         { svelteHTML.createElement("option", { "value":`-05:00`,}); }
         { svelteHTML.createElement("option", { "value":`Europe/London`,}); }
         { svelteHTML.createElement("option", { "value":`Europe/Moscow`,}); }
         { svelteHTML.createElement("option", { "value":`America/New_York`,}); }
         { svelteHTML.createElement("option", { "value":`Asia/Jerusalem`,}); }
       }
       { svelteHTML.createElement("span", { "class":`hint`,});                 { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }  }
     }
  }

  
  if(hasPasswordField.has(kind)){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`cf-secret`,});
        kind === 'mongodb' ? 'Secret (credential in the URI)' :
         kind === 'custom'  ? 'Secret ({secret} in template)' : 'Password';
         { svelteHTML.createElement("span", { "class":`dim`,});  }
       }
       { svelteHTML.createElement("input", {             "id":`cf-secret`,"class":`input`,"type":`password`,"bind:value":secret,"placeholder":existing?.secret_ref ? '•••••• (leave blank to keep)' : '',"autocomplete":`new-password`,});/*Ωignore_startΩ*/() => secret = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", { "class":`hint`,});          }
     }
  }

  
  if(tlsKinds.has(kind)){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`cf-tls-mode`,});   }
       { svelteHTML.createElement("select", {     "id":`cf-tls-mode`,"class":`input`,"bind:value":tlsMode,});/*Ωignore_startΩ*/() => tlsMode = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`disabled`,});   }
         { svelteHTML.createElement("option", { "value":`preferred`,});    }
         { svelteHTML.createElement("option", { "value":`required`,});  }
       }
     }
    if(tlsMode !== 'disabled'){
       { svelteHTML.createElement("div", { "class":`ssh-section`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "class":`toggle-label`,});
             { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":tlsVerify,});/*Ωignore_startΩ*/() => tlsVerify = __sveltets_2_any(null);/*Ωignore_endΩ*/}
              
           }
           { svelteHTML.createElement("span", { "class":`hint`,});        }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`cf-tls-ca`,});    { svelteHTML.createElement("span", { "class":`dim`,});  } }
           { const $$_dleiFhtaP4C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP4 = new $$_dleiFhtaP4C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },value:tlsCaCert,"files":true,}});/*Ωignore_startΩ*/() => tlsCaCert = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP4.$$bindings = 'value'; { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-tls-ca`,"class":`input mono`,"bind:value":tlsCaCert,"placeholder":`~/certs/ca.pem`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tlsCaCert = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
         }
         { svelteHTML.createElement("div", { "class":`field-row`,});
           { svelteHTML.createElement("div", { "class":`field grow`,});
             { svelteHTML.createElement("label", { "for":`cf-tls-cert`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
             { const $$_dleiFhtaP5C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP5 = new $$_dleiFhtaP5C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },value:tlsClientCert,"files":true,}});/*Ωignore_startΩ*/() => tlsClientCert = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP5.$$bindings = 'value'; { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-tls-cert`,"class":`input mono`,"bind:value":tlsClientCert,"placeholder":`~/certs/client.pem`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tlsClientCert = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
           }
           { svelteHTML.createElement("div", { "class":`field grow`,});
             { svelteHTML.createElement("label", { "for":`cf-tls-key`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
             { const $$_dleiFhtaP5C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP5 = new $$_dleiFhtaP5C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },value:tlsClientKey,"files":true,}});/*Ωignore_startΩ*/() => tlsClientKey = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP5.$$bindings = 'value'; { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-tls-key`,"class":`input mono`,"bind:value":tlsClientKey,"placeholder":`~/certs/client-key.pem`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tlsClientKey = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
           }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`cf-tls-sni`,});    { svelteHTML.createElement("span", { "class":`dim`,});  } }
           { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-tls-sni`,"class":`input mono`,"bind:value":tlsServerName,"placeholder":`db.example.com`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tlsServerName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
    }

    
     { svelteHTML.createElement("div", { "class":`field ssh-toggle-row`,});
       { svelteHTML.createElement("label", { "class":`toggle-label`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":tunnelOpen,});/*Ωignore_startΩ*/() => tunnelOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`dim`,});        }
       }
     }
    if(tunnelOpen){
       { svelteHTML.createElement("div", { "class":`ssh-section`,});
         { svelteHTML.createElement("div", { "class":`field-row`,});
           { svelteHTML.createElement("div", { "class":`field grow`,});
             { svelteHTML.createElement("label", { "for":`cf-tun-host`,});  }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-tun-host`,"class":`input mono`,"bind:value":tunHost,"placeholder":`bastion.example.com`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tunHost = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(!tunHost.trim()){
               { svelteHTML.createElement("span", { "class":`hint`,});         }
            }
           }
           { svelteHTML.createElement("div", { "class":`field tun-port`,});
             { svelteHTML.createElement("label", { "for":`cf-tun-port`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
             { svelteHTML.createElement("input", {          "id":`cf-tun-port`,"class":`input mono`,"type":`number`,"bind:value":tunPort,"placeholder":`22`,});/*Ωignore_startΩ*/() => tunPort = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`cf-tun-user`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
           { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-tun-user`,"class":`input mono`,"bind:value":tunUser,"placeholder":`ec2-user`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tunUser = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`cf-tun-identity`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
           { svelteHTML.createElement("div", { "class":`file-input-row`,});
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-tun-identity`,"class":`input mono grow`,"bind:value":tunIdentity,"placeholder":`~/.ssh/id_rsa`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tunIdentity = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {   "class":`btn browse-btn`,"onclick":() => (showTunnelFilePicker = true),});  }
           }
           { svelteHTML.createElement("span", { "class":`hint`,});      { svelteHTML.createElement("span", { "class":`mono`,});  }     }
         }
       }
    }
  }

  
  if(kind !== 'ssh' && (jumpKinds.has(kind) || sshEnabled)){
     { svelteHTML.createElement("div", { "class":`field ssh-toggle-row`,});
       { svelteHTML.createElement("label", { "class":`toggle-label`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":sshEnabled,});/*Ωignore_startΩ*/() => sshEnabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            { svelteHTML.createElement("span", { "class":`dim`,});        }
       }
     }
  }

  
  if(sshEnabled){
     { svelteHTML.createElement("div", { "class":`ssh-section`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cf-jump`,});
          kind === 'ssh' ? 'Jump host' : 'SSH bastion / jump host';
           { svelteHTML.createElement("span", { "class":`dim`,});  }
         }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-jump`,"class":`input mono`,"bind:value":fJump,"placeholder":`bastion.example.com`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fJump = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }

       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cf-identity`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { svelteHTML.createElement("div", { "class":`file-input-row`,});
           { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-identity`,"class":`input mono grow`,"bind:value":fIdentity,"placeholder":`~/.ssh/id_rsa`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fIdentity = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {   "class":`btn browse-btn`,"onclick":() => (showFilePicker = true),});  }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});       { svelteHTML.createElement("span", { "class":`mono`,});  }                { svelteHTML.createElement("span", { "class":`mono`,});  }     }
       }
      if(kind !== 'ssh' && fJump && hasPasswordField.has(kind) && kind !== 'clickhouse'){
         { svelteHTML.createElement("span", { "class":`hint`,});
                           
          kind === 'redis' ? 'run AUTH in redis-cli' : 'type it at the prompt';   
                   
         }
      }
     }
  }

  
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`cf-first`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
     { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`cf-first`,"class":`input mono`,"bind:value":firstCommand,"placeholder":`e.g. USE app_db; SHOW TABLES;`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => firstCommand = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("span", { "class":`hint`,});        }
   }

   }

  
  if(testResult && (!testResult.ok || testResult.hint || testResult.warn)){
     { const $$_div1 = svelteHTML.createElement("div", {  "class":`test-panel ${testResult.ok ? 'ok' : 'err'}`,});testPanel = $$_div1;
       { svelteHTML.createElement("div", { "class":`test-panel-head`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":testResult.ok ? 'check' : 'x',"size":12,}});}
         { svelteHTML.createElement("span", {});testResult.ok ? 'Connected' : 'Test failed'; }
       }
      if(!testResult.ok){ { svelteHTML.createElement("p", { "class":`test-panel-msg mono`,});testResult.detail; }}
      if(testResult.hint){ { svelteHTML.createElement("p", { "class":`test-panel-hint`,}); { svelteHTML.createElement("strong", {});   } testResult.hint; }}
      if(testResult.warn){ { svelteHTML.createElement("p", { "class":`test-panel-warn`,});testResult.warn; }}
     }
  }

  
 Modal}


if(showFilePicker){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {           "title":`Choose identity file`,"start":fIdentity ? fIdentity.replace(/\/[^/]+$/, '') : '',"files":true,"onpick":(path) => { fIdentity = path; showFilePicker = false; },"onclose":() => (showFilePicker = false),}});}
}


if(showTunnelFilePicker){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {           "title":`Choose identity file`,"start":tunIdentity ? tunIdentity.replace(/\/[^/]+$/, '') : '',"files":true,"onpick":(path) => { tunIdentity = path; showTunnelFilePicker = false; },"onclose":() => (showTunnelFilePicker = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConnectionForm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConnectionForm__SvelteComponent_ = ReturnType<typeof ConnectionForm__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConnectionForm__SvelteComponent_;