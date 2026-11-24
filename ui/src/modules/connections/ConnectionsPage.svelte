<script module lang="ts">
  /** Rows hidden while their Delete can still be undone — module-level, so
   *  leaving the page and coming back inside the Undo window keeps them hidden. */
  let deleting = $state<Record<string, true>>({});
</script>

<script lang="ts">
  // Connections — the list IS the page (docs/design/guidelines/patterns.md
  // §10): every saved profile (databases, SSH hosts, custom CLIs) and Kafka
  // cluster as one inbox row — name … state WORD, then kind · host · section ·
  // last opened. A click opens it in the workbench (`#/database/<id>`: a
  // database's query editor, an SSH terminal, the Kafka viewer). Rows say what
  // is already known about their health (lib/stores/connectionList.svelte.ts);
  // nothing is tested because the list rendered — Test is in the row's ⋯.
  // Its own small chunk: forms, dialogs and the workbench load on first use.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import InboxRow from '../../lib/components/InboxRow.svelte';
  import InboxFold from '../../lib/components/InboxFold.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { connectionList } from '../../lib/stores/connectionList.svelte';
  import { brokers } from '../../lib/stores/brokers.svelte';
  import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { engineGlyph } from '../../lib/stores/database/model';
  import { api } from '../../lib/api/client';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { copyWithToast } from '../../lib/clipboard';
  import { popoutItems } from '../../lib/popoutMenu';
  import { lsGet, lsSet } from '../../lib/storage';
  import { plural } from '../../lib/plural';
  import { buildDbCliSnippet, cliLabelForKind } from '../../lib/database/cliSnippet';
  import { nowMinute, rel } from '../../lib/stores/now.svelte';
  import type { ConnectionPrefill } from './ConnectionForm.svelte';
  import type { BrokerCluster, Connection, ConnectionKind, ConnectionSection, DbTestResult, TestConnectionResp } from '../../lib/api/types';
  import {
    KIND_LABEL, KIND_ORDER, fromCluster, fromConnection, groupItems, matches, rowWord, sectionPath,
    type ListItem,
  } from './connectionRows';

  const FormLazy = lazyComponent(() => import('./ConnectionForm.svelte'));
  const ImportLazy = lazyComponent(() => import('./ConnectionImportDialog.svelte'));
  // The Kafka form is the brokers module's; the list creates and edits
  // clusters in place, as the old hub did.
  const ClusterFormLazy = lazyComponent(() => import('../brokers/ClusterForm.svelte')); // ui-guards: allow — Kafka clusters are created from this list
  const SftpLazy = lazyComponent(() => import('./SftpBrowser.svelte'));
  const AccessLazy = lazyComponent(() => import('../../lib/components/ResourceAccess.svelte'));

  const DB_KINDS: ConnectionKind[] = ['mysql', 'postgres', 'clickhouse', 'mongodb', 'redis'];
  const ALL_KINDS: ConnectionKind[] = ['ssh', ...DB_KINDS, 'custom'];

  // ── Load: one shared, event-invalidated read (no per-visit refetch) ──────
  $effect(() => {
    const wid = ws.currentId;
    if (!wid) return;
    untrack(() => {
      void connectionList.ensure();
      void brokers.load(wid, { withSections: false });
    });
  });
  $effect(() => {
    for (const c of connectionList.rows) void resourceAccess.load('connection', c.id);
  });

  // ── Search + one filter menu (kind, section) ─────────────────────────────
  // The kind filter keeps the old hub's key, so a remembered choice survives.
  const FILTER_KEY = 'otto_connhub_filter';
  let q = $state('');
  let kind = $state(lsGet(FILTER_KEY) || 'all');
  let section = $state<string | null>(null);
  function setKind(k: string): void {
    kind = k;
    lsSet(FILTER_KEY, k);
  }

  /** Rows with a Test in flight. */
  let testing = $state<Record<string, true>>({});
  // Minute-grain: a check's freshness and "opened 4m ago" change per minute.
  const now = $derived(nowMinute());

  const allItems = $derived([
    ...connectionList.rows.map(fromConnection),
    ...brokers.clusters.map(fromCluster),
  ].filter((i) => !deleting[i.id]));
  const presentKinds = $derived(KIND_ORDER.filter((k) => allItems.some((i) => i.kind === k)));
  const narrowed = $derived(!!q.trim() || kind !== 'all' || section !== null);
  const shown = $derived(allItems.filter((i) => matches(i, { q, kind, section }, connectionList.sections)));
  const groups = $derived(groupItems(shown, connectionList.sections, narrowed));
  // Kafka clusters are rows too: not "No connections yet" while they load or failed to.
  const isEmpty = $derived(connectionList.loaded && !connectionList.error && !brokers.loading && !brokers.loadError && allItems.length === 0 && connectionList.sections.length === 0);

  function filterMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'All kinds', checked: kind === 'all', action: () => setKind('all') },
      ...presentKinds.map((k) => ({ label: KIND_LABEL[k], checked: kind === k, action: () => setKind(k) })),
    ];
    if (connectionList.sections.length > 0) {
      items.push(
        { separator: true },
        { label: 'All sections', checked: section === null, action: () => (section = null) },
        ...connectionList.sections.map((s) => ({
          label: sectionPath(connectionList.sections, s.id),
          checked: section === s.id,
          action: () => (section = s.id),
        })),
        { label: 'Ungrouped', checked: section === '', action: () => (section = '') },
      );
    }
    ctxMenu.show(e, items);
  }

  // ── Open: everything opens in the workbench, which knows each kind ───────
  function open(item: ListItem): void {
    connectionList.stampOpened(item.id);
    router.go(`database/${item.id}`);
  }

  // ── Forms and dialogs (all lazy) ─────────────────────────────────────────
  let form = $state<{ existing: Connection | null; prefill: ConnectionPrefill | null; kinds: ConnectionKind[] } | null>(null);
  let importOpen = $state(false);
  let clusterForm = $state<{ cluster: BrokerCluster | null } | null>(null);
  let sftpFor = $state<Connection | null>(null);
  let accessFor = $state<Connection | null>(null);
  // A new identity (sign-out, token swap) must not keep the old one's forms or access editor open.
  $effect(() =>
    resourceAccess.subscribe((change) => {
      if (change.type !== 'reset' || !change.identity) return;
      form = clusterForm = null;
      sftpFor = accessFor = null;
      importOpen = false;
    }),
  );

  const ownerOnly = 'Only the owner can create connections';
  function newConnection(kinds: ConnectionKind[] = ALL_KINDS, prefill: ConnectionPrefill | null = null): void {
    if (!auth.isRoot) return;
    form = { existing: null, prefill, kinds };
  }
  function edit(c: Connection): void {
    form = { existing: c, prefill: null, kinds: ALL_KINDS };
  }
  function onSaved(c: Connection): void {
    const wasNew = !form?.existing;
    form = null;
    connectionList.upsert(c, !wasNew);
    void connectionList.refresh();
    // A new database opens straight into the workbench; SSH/custom wait for a click.
    if (wasNew && DB_KINDS.includes(c.kind)) open(fromConnection(c));
  }

  // One-shot prefill handed over by another page (AWS RDS "Open in Connections…").
  $effect(() => {
    if (!ws.currentId) return;
    let raw: string | null = null;
    try {
      raw = sessionStorage.getItem('otto.conn_prefill');
      if (raw) sessionStorage.removeItem('otto.conn_prefill');
    } catch {
      return;
    }
    if (!raw) return;
    if (!auth.isRoot) {
      toasts.info('Connection creation needs admin access', 'Copy the endpoint from AWS Console instead.');
      return;
    }
    try {
      const p = JSON.parse(raw) as ConnectionPrefill;
      if (p && typeof p === 'object' && typeof p.host === 'string' && p.host.trim()) untrack(() => newConnection(ALL_KINDS, p));
    } catch {
      /* ignore a malformed prefill */
    }
  });

  function newMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Database…', icon: 'db', hint: 'MySQL, PostgreSQL, ClickHouse, MongoDB, Redis', action: () => newConnection(DB_KINDS) },
      { label: 'SSH host…', icon: 'terminal', action: () => newConnection(['ssh']) },
      { label: 'Custom CLI…', icon: 'terminal', action: () => newConnection(['custom']) },
      { label: 'Kafka cluster…', icon: engineGlyph('kafka'), action: () => (clusterForm = { cluster: null }) },
      { separator: true },
      { label: 'New section…', icon: 'folder', action: () => void createSection(null) },
      { label: 'Import from another tool…', icon: 'arrowDown', title: 'MySQL Workbench, DBeaver, DataGrip or NoSQLBooster', action: () => (importOpen = true) },
    ]);
  }

  $effect(() =>
    registry.register('connections', [
      { id: 'conn.new', title: 'New connection…', group: 'Connections', keywords: 'database ssh mysql postgres mongo redis clickhouse kafka add', disabled: !auth.isRoot, run: () => newConnection() },
      { id: 'conn.import', title: 'Import connections…', group: 'Connections', keywords: 'workbench dbeaver datagrip nosqlbooster', disabled: !auth.isRoot, run: () => (importOpen = true) },
    ]),
  );

  // ── Row actions ──────────────────────────────────────────────────────────
  const can = (c: Connection, op: string, cap: 'view' | 'edit' | 'admin' = 'edit'): boolean =>
    resourceAccess.can('connection', c.id, op, 'connections', cap);
  const isDb = (c: Connection): boolean => DB_KINDS.includes(c.kind);

  async function test(c: Connection): Promise<void> {
    if (testing[c.id]) return;
    testing = { ...testing, [c.id]: true };
    try {
      if (isDb(c)) {
        const r = await api.post<DbTestResult>(`/connections/${c.id}/db/test`, {});
        connectionList.recordCheck(c.id, { ok: r.ok, message: r.message, latencyMs: r.latency_ms ?? null, serverVersion: r.server_version ?? null });
      } else {
        const r = await api.post<TestConnectionResp>(`/connections/${c.id}/test`, {});
        connectionList.recordCheck(c.id, { ok: r.ok, message: r.message, latencyMs: r.latency_ms });
      }
    } catch (e) {
      toastError(`Couldn’t test ${c.name}`, e);
    } finally {
      const { [c.id]: _, ...rest } = testing;
      testing = rest;
    }
  }

  async function openBesideAgents(c: Connection): Promise<void> {
    const { database } = await import('../../lib/stores/database.svelte');
    await database.loadConnections();
    void database.openConnection(c.id);
    ws.openInSplit(DB_PANE_ID);
    router.go('agents');
  }

  async function duplicate(c: Connection): Promise<void> {
    try {
      const copy = await api.post<Connection>(`/connections/${c.id}/duplicate`, {});
      connectionList.upsert(copy);
      edit(copy);
      toasts.info('Configuration duplicated', 'Set a password for the new connection. Access grants were not copied.');
    } catch (e) {
      toastError('Couldn’t duplicate connection', e);
    }
  }

  /** Delete with undo: the row hides now; the profile (and its Keychain
   *  secret) is deleted when the toast goes away without Undo. */
  function remove(c: Connection): void {
    deleting = { ...deleting, [c.id]: true };
    const restore = (): void => {
      const { [c.id]: _, ...rest } = deleting;
      deleting = rest;
    };
    toasts.push('info', `Deleted “${c.name}”`, 'Its saved password is removed too.', 6000, {
      action: { label: 'Undo', run: restore },
      onClose: (reason) => {
        if (reason === 'action') return;
        void api.del(`/connections/${c.id}`).then(
          () => {
            connectionList.remove(c.id);
            restore();
          },
          (e: unknown) => {
            restore();
            toastError(`Couldn’t delete ${c.name}`, e);
          },
        );
      },
    });
  }

  async function moveTo(c: Connection, sectionId: string | null): Promise<void> {
    if ((c.section_id ?? null) === sectionId) return;
    try {
      const saved = await api.patch<Connection>(`/connections/${c.id}`, {
        name: c.name, kind: c.kind, params: c.params, first_command: c.first_command, section_id: sectionId,
        // Keep the guardrails — omitting them would reset to dev / writable.
        environment: c.environment, read_only: c.read_only,
      });
      connectionList.upsert(saved);
    } catch (e) {
      toastError('Couldn’t move', e);
    }
  }

  function moveItems(current: string | null, move: (id: string | null) => void): MenuItem[] {
    return [
      { label: 'Ungrouped', checked: !current, action: () => move(null) },
      ...connectionList.sections.map((s) => ({ label: sectionPath(connectionList.sections, s.id), checked: current === s.id, action: () => move(s.id) })),
    ];
  }

  function connMenu(e: MouseEvent, c: Connection): void {
    const cli = buildDbCliSnippet(c, { activeDb: null });
    const cliName = cliLabelForKind(c.kind);
    const admin = can(c, 'configure', 'admin');
    ctxMenu.show(e, [
      { label: isDb(c) ? 'Open' : 'Open terminal', icon: isDb(c) ? 'db' : 'terminal', action: () => open(fromConnection(c)) },
      ...(isDb(c) ? [{ label: 'Open beside agents (split)', icon: 'split', action: () => void openBesideAgents(c) }] : []),
      ...(c.kind === 'ssh' && can(c, 'sftp_read', 'view') ? [{ label: 'Browse files (SFTP)', icon: 'folder', action: () => (sftpFor = c) }] : []),
      ...(cli && cliName ? [{ label: `Copy as ${cliName}`, icon: 'copy', action: () => void copyWithToast(cli, `${cliName} command`) }] : []),
      ...(isDb(c) ? popoutItems(`database/${c.id}`, c.name) : []),
      { separator: true },
      { label: 'Test', icon: 'refresh', disabled: !!testing[c.id], action: () => void test(c) },
      ...(admin ? [{ label: 'Edit', icon: 'edit', action: () => edit(c) }] : []),
      ...(admin && auth.isRoot ? [{ label: 'Duplicate without password', icon: 'copy', action: () => void duplicate(c) }] : []),
      ...(admin && connectionList.sections.length > 0 ? [{ label: 'Move to section', icon: 'folder', children: moveItems(c.section_id, (id) => void moveTo(c, id)) }] : []),
      ...(auth.isRoot || can(c, 'manage_access', 'admin') ? [{ label: 'Access…', icon: 'key', action: () => (accessFor = c) }] : []),
      // Undo, not a confirm: the label has no "…".
      ...(admin ? [{ separator: true }, { label: 'Delete', icon: 'trash', danger: true, action: () => remove(c) }] : []), // ui-guards: allow — deletes with Undo, no confirm
    ]);
  }

  function clusterMenu(e: MouseEvent, cl: BrokerCluster): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'split', action: () => open(fromCluster(cl)) },
      { label: 'Open in Message Brokers page', icon: 'split', action: () => { brokers.select(cl.id); router.go('brokers'); } },
      { separator: true },
      { label: 'Edit…', icon: 'edit', action: () => (clusterForm = { cluster: cl }) },
      ...(connectionList.sections.length > 0 ? [{ label: 'Move to section', icon: 'folder', children: moveItems(cl.section_id ?? null, (id) => void brokers.moveCluster(cl.id, id).catch((err: unknown) => toastError('Couldn’t move', err))) }] : []),
      { separator: true },
      { label: 'Remove…', icon: 'trash', danger: true, action: () => void removeCluster(cl) },
    ]);
  }

  async function removeCluster(cl: BrokerCluster): Promise<void> {
    const ok = await confirmer.ask(`Remove cluster “${cl.name}”? Its saved settings and Keychain secrets are removed from Otto; topics on the broker are not touched.`, { title: 'Remove cluster', confirmLabel: 'Remove', danger: true });
    if (!ok) return;
    try {
      await brokers.remove(cl.id);
    } catch (e) {
      toastError('Couldn’t remove', e);
    }
  }

  function itemMenu(e: MouseEvent, item: ListItem): void {
    e.preventDefault();
    if (item.conn) connMenu(e, item.conn);
    else if (item.cluster) clusterMenu(e, item.cluster);
  }

  // ── Sections ─────────────────────────────────────────────────────────────
  async function createSection(parentId: string | null): Promise<void> {
    if (!ws.currentId) return;
    const name = await confirmer.promptText(parentId ? 'Sub-section name' : 'Section name', { title: parentId ? 'New sub-section' : 'New section', confirmLabel: 'Create', placeholder: 'e.g. AWS · STG' });
    if (!name) return;
    try {
      const sec = await api.post<ConnectionSection>(`/workspaces/${ws.currentId}/connection-sections`, { name, parent_id: parentId });
      connectionList.sections = [...connectionList.sections, sec];
    } catch (e) {
      toastError('Couldn’t create the section', e);
    }
  }
  async function renameSection(sec: ConnectionSection): Promise<void> {
    const name = await confirmer.promptText('Rename section', { title: 'Rename section', confirmLabel: 'Rename', initial: sec.name });
    if (!name || name === sec.name) return;
    try {
      const updated = await api.patch<ConnectionSection>(`/connection-sections/${sec.id}`, { name });
      connectionList.sections = connectionList.sections.map((s) => (s.id === sec.id ? updated : s));
    } catch (e) {
      toastError('Couldn’t rename', e);
    }
  }
  async function deleteSection(sec: ConnectionSection): Promise<void> {
    const ok = await confirmer.ask(`Delete section “${sec.name}”? Sub-sections are removed too and their connections become ungrouped.`, { title: 'Delete section', confirmLabel: 'Delete', danger: true });
    if (!ok) return;
    try {
      await api.del(`/connection-sections/${sec.id}`);
      void connectionList.refresh();
      if (ws.currentId) void brokers.load(ws.currentId, { withSections: false });
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }
  function sectionMenu(e: MouseEvent, sec: ConnectionSection): void {
    ctxMenu.show(e, [
      { label: 'Add sub-section…', icon: 'plus', action: () => void createSection(sec.id) },
      { label: 'Rename…', icon: 'edit', action: () => void renameSection(sec) },
      { label: 'Move into', icon: 'folder', children: [
        { label: 'Top level', checked: !sec.parent_id, action: () => void moveSection(sec, null) },
        ...connectionList.sections.filter((s) => s.id !== sec.id).map((s) => ({ label: sectionPath(connectionList.sections, s.id), checked: sec.parent_id === s.id, action: () => void moveSection(sec, s.id) })),
      ] },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteSection(sec) },
    ]);
  }
  async function moveSection(sec: ConnectionSection, parentId: string | null): Promise<void> {
    if ((sec.parent_id ?? null) === parentId) return;
    try {
      const updated = await api.post<ConnectionSection>(`/connection-sections/${sec.id}/move`, { parent_id: parentId });
      connectionList.sections = connectionList.sections.map((s) => (s.id === sec.id ? updated : s));
    } catch (e) {
      toastError('Couldn’t move', e);
    }
  }

  function envWord(i: ListItem): string {
    const env = i.environment === 'prod' ? 'Production' : i.environment === 'staging' ? 'Staging' : '';
    return [env, i.readOnly ? 'read-only' : ''].filter(Boolean).join(' · ');
  }
  const subtitle = $derived(connectionList.loaded && allItems.length > 0 ? `${allItems.length} saved` : undefined);
</script>

<div class="conn-root">
<PageHeader title="Connections" {subtitle}>
  {#snippet actions()}
    {#if !isEmpty}
      <button class="btn small primary" disabled={!auth.isRoot} title={auth.isRoot ? 'New connection — database, SSH host, custom CLI or Kafka cluster' : ownerOnly} aria-haspopup="menu" onclick={newMenu}>
        <Icon name="plus" size={12} /> New connection <Icon name="chevronDown" size={12} />
      </button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody>
  <div class="list-col conn-page">
    {#if !isEmpty}
      <div class="conn-search" role="search">
        <Icon name="search" size={14} />
        <input class="conn-search-input" type="search" bind:value={q} placeholder="Search connections" aria-label="Search connections" spellcheck="false" dir="auto" />
        <button class="btn small ghost" aria-haspopup="menu" onclick={filterMenu} data-testid="conn-filter">
          <Icon name="filter" size={12} /> Filter
        </button>
      </div>
      {#if kind !== 'all' || section !== null}
        <div class="conn-chips">
          {#if kind !== 'all'}
            <button class="chip" onclick={() => setKind('all')} aria-label="Clear the {KIND_LABEL[kind as keyof typeof KIND_LABEL] ?? kind} filter">{KIND_LABEL[kind as keyof typeof KIND_LABEL] ?? kind} <Icon name="x" size={12} /></button>
          {/if}
          {#if section !== null}
            <button class="chip" onclick={() => (section = null)} aria-label="Clear the section filter">{section === '' ? 'Ungrouped' : sectionPath(connectionList.sections, section)} <Icon name="x" size={12} /></button>
          {/if}
        </div>
      {/if}
    {/if}

    {#if brokers.loadError}
      <LoadState what="Kafka clusters" variant="compact" error={brokers.loadError} loading={brokers.loading} empty={brokers.clusters.length === 0}
        onretry={() => ws.currentId && void brokers.load(ws.currentId, { withSections: false })} />
    {/if}
    <LoadState
      what="connections"
      variant="page"
      loading={connectionList.loading || !connectionList.loaded}
      error={connectionList.error}
      empty={!connectionList.loaded || isEmpty || (!!connectionList.error && allItems.length === 0)}
      onretry={() => void connectionList.refresh()}
    >
      {#snippet emptyView()}
        <EmptyState
          variant="page"
          icon="plug"
          title="No connections yet"
          body="Add a database (MySQL, PostgreSQL, ClickHouse, MongoDB, Redis), an SSH host or a custom CLI, then open it to query, browse its tables or get a terminal."
          actionLabel={auth.isRoot ? 'New connection' : undefined}
          actionIcon="plus"
          onaction={auth.isRoot ? () => newConnection() : undefined}
        >
          {#if auth.isRoot}
            <div class="empty-alt">
              <button class="btn ghost small" onclick={() => (clusterForm = { cluster: null })}><Icon name={engineGlyph('kafka')} size={12} /> Add a Kafka cluster</button>
              <button class="btn ghost small" onclick={() => (importOpen = true)}><Icon name="arrowDown" size={12} /> Import from another tool…</button>
            </div>
          {:else}
            <p class="empty-note">Only the owner can add connections.</p>
          {/if}
        </EmptyState>
      {/snippet}
      {#if shown.length === 0 && narrowed}
        <EmptyState icon="search" title="No matches" body="No connection matches this search or filter." actionLabel="Clear" actionKind="secondary" onaction={() => { q = ''; setKind('all'); section = null; }} />
      {:else}
        <div class="conn-list">
          {#each groups as g (g.key)}
            {#if g.label === null}
              {@render rows(g.items)}
            {:else if g.section}
              {@const sec = g.section}
              <InboxFold label={g.label} count={g.items.length}>
                {#snippet actions()}
                  <button class="icon-btn" aria-label="Section actions for {sec.name}" title="Section actions" onclick={(e) => sectionMenu(e, sec)}><Icon name="more" size={14} /></button>
                {/snippet}
                {#if g.items.length === 0 && !connectionList.sections.some((s) => s.parent_id === sec.id)}<p class="conn-empty-sec">No connections in this section.</p>{/if}
                {@render rows(g.items)}
              </InboxFold>
            {:else}
              <InboxFold label={g.label}>{@render rows(g.items)}</InboxFold>
            {/if}
          {/each}
        </div>
      {/if}
    </LoadState>
  </div>
</PageBody>
</div>

{#snippet rows(items: ListItem[])}
  {#each items as item (item.id)}
    {@const w = testing[item.id] ? null : rowWord(connectionList.checks[item.id], now, (at) => rel(at))}
    {@const env = envWord(item)}
    <div class="conn-row" data-connection-id={item.id}>
      <InboxRow
        twoLine
        onPage
        title={item.name}
        word={testing[item.id] ? 'Testing…' : w?.word}
        tone={w?.tone ?? ''}
        tip={w?.tip}
        state={w?.word}
        onclick={() => open(item)}
        oncontextmenu={(e) => itemMenu(e, item)}
      >
        {#snippet line3()}
          <Icon name={engineGlyph(item.kind)} size={12} />
          <span class="conn-meta ellipsis" dir="auto">
            {KIND_LABEL[item.kind]}{item.host ? ` · ${item.host}` : ''}{env ? ' · ' : ''}<span class:prod={item.environment === 'prod'}>{env}</span> · {w?.reason ?? (item.lastOpened ? `opened ${rel(item.lastOpened)}` : 'never opened')}
          </span>
          {#if w?.latency}<span class="conn-latency">{w.latency}</span>{/if}
        {/snippet}
        {#snippet actions()}
          <button class="icon-btn" aria-label="Actions for {item.name}" title="Actions" onclick={(e) => itemMenu(e, item)}><Icon name="more" size={14} /></button>
        {/snippet}
      </InboxRow>
    </div>
  {/each}
{/snippet}

{#if form}
  <LazyMount lazy={FormLazy} what="the connection form" quiet onerror={() => (form = null)}
    props={{ existing: form.existing, prefill: form.prefill, kinds: form.kinds, onclose: () => (form = null), onsaved: onSaved }} />
{/if}
{#if importOpen && ws.currentId}
  <LazyMount lazy={ImportLazy} what="the import dialog" quiet onerror={() => (importOpen = false)}
    props={{ wsId: ws.currentId, onclose: () => (importOpen = false), onimported: () => void connectionList.refresh() }} />
{/if}
{#if clusterForm}
  <LazyMount lazy={ClusterFormLazy} what="the cluster form" quiet onerror={() => (clusterForm = null)}
    props={{ cluster: clusterForm.cluster, onclose: () => { clusterForm = null; if (ws.currentId) void brokers.load(ws.currentId, { withSections: false }); } }} />
{/if}
{#if sftpFor}
  <LazyMount lazy={SftpLazy} what="the SFTP browser" quiet onerror={() => (sftpFor = null)} props={{ conn: sftpFor, onclose: () => (sftpFor = null) }} />
{/if}
{#if accessFor}
  <Modal title={`Access · ${accessFor.name}`} width={820} onclose={() => (accessFor = null)}>
    <LazyMount lazy={AccessLazy} what="access settings" variant="panel" props={{ kind: 'connection', resourceId: accessFor.id }} />
  </Modal>
{/if}

<style>
  /* The shell's page slot is a plain block: make header + body a column so
     the body (not the window) scrolls a long list. */
  .conn-root {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .conn-page {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding-block: var(--sp-4) var(--sp-8);
  }
  .conn-search {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding-inline: var(--sp-5) var(--sp-2);
    min-block-size: 36px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text-dim);
  }
  .conn-search:focus-within {
    border-color: var(--accent-solid);
  }
  .conn-search-input {
    flex: 1;
    align-self: stretch;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
  }
  .conn-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }
  .conn-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }
  .conn-meta {
    min-width: 0;
  }
  .conn-meta .prod {
    color: var(--danger);
    font-weight: 600;
  }
  .conn-latency {
    flex-shrink: 0;
    margin-inline-start: auto;
    font-variant-numeric: tabular-nums;
  }
  /* The hover ⋯ (24 px + its inset and lead-in) covers line 3's end: line 3
     makes room, so its text ends in an ellipsis beside it, not under it. */
  @media (hover: hover) {
    .conn-row:is(:hover, :focus-within) :global(.tr-l3) {
      padding-inline-end: calc(24px + var(--sp-2) + var(--sp-3) - var(--sp-5));
    }
  }
  .conn-empty-sec {
    margin: 0;
    padding: var(--sp-2) var(--sp-5);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .empty-alt {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--sp-2);
    margin-block-start: var(--sp-3);
  }
  .empty-note {
    margin: var(--sp-3) 0 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  /* Touch: every control is finger-sized (44 px). */
  @media (hover: none), (pointer: coarse) {
    .conn-search {
      min-block-size: 44px;
    }
    .conn-search :global(.btn),
    .conn-chips :global(.chip),
    .conn-root :global(.page-header-bar .btn),
    .conn-root :global(.page-header-bar .icon-btn),
    .conn-list :global(.if-head .icon-btn),
    .conn-list :global(.tr-actions .icon-btn) {
      min-block-size: 44px;
      min-inline-size: 44px;
    }
  }
</style>
