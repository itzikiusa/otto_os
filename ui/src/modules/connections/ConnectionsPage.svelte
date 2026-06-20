<script lang="ts">
  // Connections: a tree of user-defined sections (nestable) with connections
  // shown as compact list rows. Drag a connection onto a section to file it;
  // drag a section onto another to nest it. Open / Test / Edit / Delete per row.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type {
    Connection,
    ConnectionKind,
    ConnectionSection,
    Session,
    TestConnectionResp,
  } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ConnectionForm from './ConnectionForm.svelte';
  import SftpBrowser from './SftpBrowser.svelte';

  interface TreeNode {
    sec: ConnectionSection;
    items: Connection[];
    children: TreeNode[];
  }

  // DB engines are managed in the Database section; this page handles the rest.
  const DB_CONN_KINDS = ['mysql', 'redis', 'mongodb', 'clickhouse'];
  const NON_DB_KINDS = ['ssh', 'custom'] as const;

  let conns: Connection[] = $state([]);
  let sections: ConnectionSection[] = $state([]);
  let loading = $state(true);
  let formOpen = $state(false);
  let editing: Connection | null = $state(null);
  // The SSH connection whose SFTP file browser is open (null = none).
  let sftpFor: Connection | null = $state(null);
  let testing: Record<string, boolean> = $state({});
  let testResults: Record<string, TestConnectionResp> = $state({});
  let opening: Record<string, boolean> = $state({});
  let openingSplit: Record<string, boolean> = $state({});
  let collapsed: Record<string, boolean> = $state({});
  let draggedConnId: string | null = $state(null);
  let draggedSectionId: string | null = $state(null);
  // Which connection's "open in workspace…" menu is showing (null = none).
  let openMenuFor: string | null = $state(null);

  const kindIcons: Record<ConnectionKind, string> = {
    ssh: 'key',
    mysql: 'db',
    redis: 'zap',
    mongodb: 'db',
    clickhouse: 'db',
    custom: 'terminal',
  };
  const sortByName = (a: Connection, b: Connection): number => a.name.localeCompare(b.name);

  // Build the section tree from the flat list; `parentId = null` is the root.
  function buildTree(parentId: string | null): TreeNode[] {
    return sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((sec) => ({
        sec,
        items: conns.filter((c) => c.section_id === sec.id).sort(sortByName),
        children: buildTree(sec.id),
      }));
  }
  const tree = $derived(buildTree(null));
  // A connection whose folder is unknown falls back to Ungrouped, so nothing
  // ever vanishes. Connections are global, so all of them can be filed.
  const knownSectionIds = $derived(new Set(sections.map((s) => s.id)));
  const ungrouped = $derived(
    conns.filter((c) => !c.section_id || !knownSectionIds.has(c.section_id)).sort(sortByName),
  );

  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId) return;
    loading = true;
    void api
      .get<Connection[]>(`/workspaces/${wsId}/connections`)
      // DB connections (mysql/redis/mongodb/clickhouse) live in the Database
      // section now — keep this page to SSH / custom clients only.
      .then((c) => (conns = c.filter((x) => !DB_CONN_KINDS.includes(x.kind))))
      .catch((e) => toasts.error('Could not load connections', e instanceof Error ? e.message : ''))
      .finally(() => (loading = false));
    void api
      .get<ConnectionSection[]>(`/workspaces/${wsId}/connection-sections`)
      .then((s) => (sections = s))
      .catch(() => {});
  });

  // --- Section operations ---------------------------------------------------

  async function createSection(parentId: string | null): Promise<void> {
    if (!ws.currentId) return;
    const name = await confirmer.promptText(parentId ? 'Sub-section name' : 'Section name', {
      title: parentId ? 'New sub-section' : 'New section',
      confirmLabel: 'Create',
      placeholder: 'e.g. Production',
    });
    if (!name) return;
    try {
      const sec = await api.post<ConnectionSection>(
        `/workspaces/${ws.currentId}/connection-sections`,
        { name, parent_id: parentId },
      );
      sections = [...sections, sec];
    } catch (e) {
      toasts.error('Create section failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function renameSection(sec: ConnectionSection): Promise<void> {
    const name = await confirmer.promptText('Rename section', {
      title: 'Rename section',
      confirmLabel: 'Rename',
      initial: sec.name,
    });
    if (!name || name === sec.name) return;
    try {
      const updated = await api.patch<ConnectionSection>(`/connection-sections/${sec.id}`, { name });
      sections = sections.map((s) => (s.id === sec.id ? updated : s));
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteSection(sec: ConnectionSection): Promise<void> {
    if (
      !(await confirmer.ask(
        `Delete section “${sec.name}”? Sub-sections are removed too and their connections become ungrouped.`,
        { title: 'Delete section' },
      ))
    )
      return;
    try {
      await api.del(`/connection-sections/${sec.id}`);
      // Drop the section + all descendants locally; clear their connections.
      const removed = new Set<string>();
      const collect = (id: string) => {
        removed.add(id);
        for (const s of sections) if (s.parent_id === id) collect(s.id);
      };
      collect(sec.id);
      sections = sections.filter((s) => !removed.has(s.id));
      conns = conns.map((c) => (c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c));
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  function toggleCollapse(id: string): void {
    collapsed[id] = !collapsed[id];
  }

  // --- Drag & drop ----------------------------------------------------------

  // Move a connection into a section (or null = ungrouped). Global connections
  // are not assignable. Reuses the connection PATCH endpoint.
  async function moveConn(c: Connection, sectionId: string | null): Promise<void> {
    if ((c.section_id ?? null) === sectionId) return;
    try {
      const saved = await api.patch<Connection>(`/connections/${c.id}`, {
        name: c.name,
        kind: c.kind,
        params: c.params,
        first_command: c.first_command,
        section_id: sectionId,
        // Preserve the guardrail flags — omitting them would reset to dev/false.
        environment: c.environment,
        read_only: c.read_only,
      });
      conns = conns.map((x) => (x.id === c.id ? saved : x));
    } catch (e) {
      toasts.error('Move failed', e instanceof Error ? e.message : String(e));
    }
  }

  function isDescendantOf(nodeId: string, ancestorId: string): boolean {
    let cur = sections.find((s) => s.id === nodeId);
    while (cur?.parent_id) {
      if (cur.parent_id === ancestorId) return true;
      cur = sections.find((s) => s.id === cur!.parent_id);
    }
    return false;
  }

  async function reparentSection(id: string, parentId: string | null): Promise<void> {
    const sec = sections.find((s) => s.id === id);
    if (!sec || (sec.parent_id ?? null) === parentId) return;
    if (parentId && (parentId === id || isDescendantOf(parentId, id))) {
      toasts.error('Invalid move', 'Cannot nest a section inside itself');
      return;
    }
    try {
      const updated = await api.post<ConnectionSection>(`/connection-sections/${id}/move`, {
        parent_id: parentId,
      });
      sections = sections.map((s) => (s.id === id ? updated : s));
    } catch (e) {
      toasts.error('Move failed', e instanceof Error ? e.message : String(e));
    }
  }

  // A drop onto a section: a dragged connection files into it; a dragged
  // section nests under it.
  function onSectionDrop(sectionId: string): void {
    if (draggedConnId) {
      const c = conns.find((x) => x.id === draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, sectionId);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, sectionId);
    }
  }

  // A drop onto the "Ungrouped" / top-level zone: connection → no section;
  // section → top-level.
  function onRootDrop(): void {
    if (draggedConnId) {
      const c = conns.find((x) => x.id === draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, null);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, null);
    }
  }

  // --- Connection operations ------------------------------------------------

  function describe(c: Connection): string {
    const p = c.params as Record<string, unknown>;
    switch (c.kind) {
      case 'ssh':
        return `${p.user ?? '?'}@${p.host ?? '?'}${p.port ? `:${p.port}` : ''}`;
      case 'mongodb':
        return String(p.connection_string ?? '');
      case 'custom':
        return String(p.template ?? '');
      default:
        return `${p.host ?? '?'}${p.port ? `:${p.port}` : ''}${p.db !== undefined && p.db !== '' ? ` / ${p.db}` : ''}`;
    }
  }

  async function test(c: Connection): Promise<void> {
    testing[c.id] = true;
    try {
      testResults[c.id] = await api.post<TestConnectionResp>(`/connections/${c.id}/test`);
    } catch (e) {
      testResults[c.id] = {
        ok: false,
        latency_ms: null,
        message: e instanceof Error ? e.message : 'test failed',
        warn_argv: false,
      };
    } finally {
      testing[c.id] = false;
    }
  }

  // Open the connection as a terminal session attached to a workspace. The
  // attachment is per-session and temporary (the connection itself is global):
  // closing the session ends it. `targetWsId` defaults to the current workspace
  // but the row's dropdown can pick any workspace.
  async function open(c: Connection, targetWsId?: string): Promise<void> {
    const wsId = targetWsId ?? ws.currentId;
    openMenuFor = null;
    if (!wsId) {
      toasts.error('No workspace', 'Create or select a workspace to attach the session to');
      return;
    }
    opening[c.id] = true;
    try {
      const session = await api.post<Session>(`/connections/${c.id}/open`, { workspace_id: wsId });
      if (wsId === ws.currentId) {
        ws.addSession(session);
      } else {
        // Switch to the target workspace so its new session is visible.
        await ws.select(wsId);
        ws.navigateToSession(session.id);
      }
      const wsName = ws.workspaces.find((w) => w.id === wsId)?.name;
      toasts.success('Connection opened', wsName ? `${c.name} → ${wsName}` : c.name);
    } catch (e) {
      toasts.error('Open failed', e instanceof Error ? e.message : String(e));
    } finally {
      opening[c.id] = false;
    }
  }

  // Open the connection beside the current session(s): drop its terminal into a
  // new split pane instead of replacing the active tab, so a DB/ssh terminal sits
  // next to an agent. Respects the 1–4 pane cap.
  async function openBeside(c: Connection): Promise<void> {
    openingSplit[c.id] = true;
    if (!ws.currentId) {
      toasts.error('No workspace', 'Select a workspace to attach the session to');
      openingSplit[c.id] = false;
      return;
    }
    try {
      const session = await api.post<Session>(`/connections/${c.id}/open`, {
        workspace_id: ws.currentId,
      });
      // Register + drop into a NEW split pane (not the active tab) in one step,
      // so the connection sits beside the current session rather than replacing it.
      const placed = ws.addSessionInSplit(session);
      router.go('agents');
      if (placed) {
        toasts.success('Opened beside', c.name);
      } else {
        toasts.warn('Up to 4 panes', 'Replaced the focused pane — close one to add more.');
      }
    } catch (e) {
      toasts.error('Open failed', e instanceof Error ? e.message : String(e));
    } finally {
      openingSplit[c.id] = false;
    }
  }

  async function remove(c: Connection): Promise<void> {
    if (!(await confirmer.ask(`Delete connection “${c.name}”? Its Keychain secret is removed too.`, { title: 'Delete connection' }))) return;
    try {
      await api.del(`/connections/${c.id}`);
      conns = conns.filter((x) => x.id !== c.id);
      toasts.info('Connection deleted', c.name);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function togglePin(c: Connection): Promise<void> {
    try {
      const updated = await api.patch<Connection>(`/connections/${c.id}/pin`, {
        pinned: !c.pinned,
      });
      conns = conns.map((x) => (x.id === c.id ? updated : x));
    } catch (e) {
      toasts.error('Pin failed', e instanceof Error ? e.message : String(e));
    }
  }

  function onSaved(saved: Connection): void {
    const idx = conns.findIndex((x) => x.id === saved.id);
    conns = idx >= 0 ? conns.map((x) => (x.id === saved.id ? saved : x)) : [...conns, saved];
    formOpen = false;
    editing = null;
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Connections</h1>
      <div class="sub">SSH, databases and custom clients — opening one creates a terminal session.</div>
    </div>
    <div class="header-actions">
      <button class="btn" onclick={() => createSection(null)}>New Section</button>
      <button
        class="btn primary"
        onclick={() => {
          editing = null;
          formOpen = true;
        }}
      >
        New Connection
      </button>
    </div>
  </div>

  {#if loading}
    <Skeleton rows={4} height={40} />
  {:else if conns.length === 0 && sections.length === 0}
    <EmptyState
      icon="plug"
      title="No connections yet"
      body="Create profiles for ssh, mysql, redis, mongodb, clickhouse or any custom CLI. Secrets go to the Keychain; opening a profile drops you into a live terminal."
      actionLabel="New Connection"
      onaction={() => {
        editing = null;
        formOpen = true;
      }}
    />
  {:else}
    <div class="tree">
      {#each tree as node (node.sec.id)}
        {@render sectionNode(node, 0)}
      {/each}

      <!-- Ungrouped doubles as the top-level / no-section drop target. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="section-head plain"
        class:drop-target={draggedConnId || draggedSectionId}
        ondragover={(e) => {
          if (draggedConnId || draggedSectionId) e.preventDefault();
        }}
        ondrop={(e) => {
          e.preventDefault();
          onRootDrop();
        }}
        title="Connections with no section (drop here to remove from a section / make a section top-level)"
      >
        <span class="caret-spacer"></span>
        <span class="section-name grow">Ungrouped</span>
        {#if ungrouped.length > 0}<span class="count">{ungrouped.length}</span>{/if}
      </div>
      {#each ungrouped as c (c.id)}
        {@render connRow(c, 1)}
      {/each}
    </div>
  {/if}
</div>

{#if openMenuFor}
  <!-- Click-away backdrop for the open-in-workspace menu. -->
  <button class="menu-backdrop" aria-label="Close menu" onclick={() => (openMenuFor = null)}></button>
{/if}

{#snippet sectionNode(node: TreeNode, depth: number)}
  {@const isOpen = !collapsed[node.sec.id]}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="section-head"
    class:drop-target={(draggedSectionId && draggedSectionId !== node.sec.id) || draggedConnId}
    style="padding-left: {depth * 16 + 6}px"
    draggable="true"
    ondragstart={(e) => {
      draggedSectionId = node.sec.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedSectionId = null)}
    ondragover={(e) => {
      if (draggedConnId || (draggedSectionId && draggedSectionId !== node.sec.id)) e.preventDefault();
    }}
    ondrop={(e) => {
      e.preventDefault();
      e.stopPropagation();
      onSectionDrop(node.sec.id);
    }}
  >
    <button class="caret" onclick={() => toggleCollapse(node.sec.id)} aria-label="Toggle section">
      <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
    <Icon name="folder" size={13} />
    <span class="section-name grow ellipsis">{node.sec.name}</span>
    <span class="count">{node.items.length}</span>
    <button class="icon-btn" title="Add sub-section" onclick={() => createSection(node.sec.id)}>
      <Icon name="plus" size={13} />
    </button>
    <button class="icon-btn" title="Rename section" onclick={() => renameSection(node.sec)}>
      <Icon name="edit" size={13} />
    </button>
    <button class="icon-btn" title="Delete section" onclick={() => deleteSection(node.sec)}>
      <Icon name="trash" size={13} />
    </button>
  </div>
  {#if isOpen}
    {#each node.items as c (c.id)}
      {@render connRow(c, depth + 1)}
    {/each}
    {#each node.children as child (child.sec.id)}
      {@render sectionNode(child, depth + 1)}
    {/each}
  {/if}
{/snippet}

{#snippet connRow(c: Connection, depth: number)}
  {@const r = testResults[c.id]}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="conn-row"
    class:dragging={draggedConnId === c.id}
    style="padding-left: {depth * 16 + 8}px"
    draggable="true"
    ondragstart={(e) => {
      draggedConnId = c.id;
      e.stopPropagation();
    }}
    ondragend={() => (draggedConnId = null)}
    ondblclick={(e) => {
      // Double-click the row to open the session — but not when the dblclick
      // lands on one of the action buttons.
      if ((e.target as HTMLElement).closest('button')) return;
      if (!opening[c.id]) void open(c);
    }}
    title={c.first_command ? `${c.name} · ▸ ${c.first_command} — double-click to open` : `${c.name} — double-click to open`}
  >
    <span class="conn-dot"><Icon name={kindIcons[c.kind]} size={13} /></span>
    {#if c.pinned}<span class="pin-glyph" title="Pinned"><Icon name="pin" size={11} /></span>{/if}
    <span class="conn-name ellipsis">{c.name}</span>
    {#if c.environment === 'prod'}
      <span class="env-chip prod" title="Production — writes require typed confirmation">PROD</span>
    {:else if c.read_only}
      <span class="env-chip ro" title="Read-only — writes refused">RO</span>
    {:else if c.environment === 'staging'}
      <span class="env-chip stg" title="Staging">STG</span>
    {/if}
    <span class="conn-desc mono ellipsis">{describe(c)}</span>
    <span class="grow"></span>
    {#if c.kind === 'clickhouse'}
      <span class="chip warn" title="password passes via argv on the host">argv</span>
    {/if}
    {#if r}
      <span class="chip {r.ok ? 'ok' : 'bad'}" title={r.message}>
        {r.ok ? `ok · ${r.latency_ms}ms` : 'failed'}
      </span>
    {/if}
    <!-- Open as a session attached to a workspace; the caret picks which one. -->
    <div class="open-split">
      <button class="btn small primary open-main" disabled={opening[c.id]} onclick={() => open(c)}>
        <Icon name="play" size={11} />
        {opening[c.id] ? 'Opening…' : 'Open'}
      </button>
      <button
        class="btn small primary open-caret"
        title="Open in a specific workspace…"
        aria-label="Open in workspace"
        disabled={opening[c.id]}
        onclick={(e) => {
          e.stopPropagation();
          openMenuFor = openMenuFor === c.id ? null : c.id;
        }}
      >
        <Icon name="chevronDown" size={11} />
      </button>
      {#if openMenuFor === c.id}
        <div class="open-menu">
          <div class="open-menu-title">Attach session to…</div>
          {#each ws.workspaces as w (w.id)}
            <button class="open-menu-item" onclick={() => open(c, w.id)}>
              <span class="ellipsis">{w.name}</span>
              {#if w.id === ws.currentId}<span class="cur">current</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
    </div>
    <button
      class="btn small icon-only"
      title="Open beside — adds this terminal as a split pane next to the current session"
      aria-label="Open beside"
      disabled={openingSplit[c.id]}
      onclick={() => openBeside(c)}
    >
      <Icon name="split" size={12} />
    </button>
    <button class="btn small" disabled={testing[c.id]} onclick={() => test(c)}>
      {testing[c.id] ? 'Testing…' : 'Test'}
    </button>
    {#if c.kind === 'ssh'}
      <button
        class="icon-btn"
        title="Browse files (SFTP)"
        aria-label="Browse files"
        onclick={() => (sftpFor = c)}
      >
        <Icon name="folder" size={13} />
      </button>
    {/if}
    <button
      class="icon-btn"
      title="Edit"
      onclick={() => {
        editing = c;
        formOpen = true;
      }}
    >
      <Icon name="edit" size={13} />
    </button>
    <button
      class="icon-btn"
      title={c.pinned ? 'Unpin' : 'Pin to top'}
      onclick={() => togglePin(c)}
    >
      <Icon name="pin" size={13} />
    </button>
    <button class="icon-btn" title="Delete" onclick={() => remove(c)}>
      <Icon name="trash" size={13} />
    </button>
  </div>
{/snippet}

{#if formOpen}
  <ConnectionForm
    existing={editing}
    kinds={[...NON_DB_KINDS]}
    onclose={() => (formOpen = false)}
    onsaved={onSaved}
  />
{/if}

{#if sftpFor}
  <SftpBrowser conn={sftpFor} onclose={() => (sftpFor = null)} />
{/if}

<style>
  .tree {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .section-head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px 0 6px;
    border-radius: var(--radius-s);
    cursor: grab;
    user-select: none;
  }
  .section-head:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .section-head.plain {
    cursor: default;
    margin-top: 6px;
  }
  .section-head.drop-target {
    outline: 1px dashed color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: -1px;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .section-name {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .caret {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .caret:hover {
    color: var(--text);
  }
  .caret-spacer {
    width: 18px;
    flex-shrink: 0;
  }
  .count {
    font-size: 10px;
    color: var(--text-dim);
    min-width: 16px;
    text-align: center;
  }
  .conn-row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    transition: background 120ms ease-out;
  }
  .conn-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .conn-row.dragging {
    opacity: 0.5;
  }
  .conn-dot {
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .conn-name {
    font-size: 12.5px;
    font-weight: 600;
    display: flex;
    gap: 6px;
    align-items: center;
    flex-shrink: 0;
    max-width: 280px;
  }
  .conn-desc {
    font-size: 11px;
    color: var(--text-dim);
    min-width: 0;
    flex: 0 1 auto;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip.warn {
    color: #b8860b;
    background: color-mix(in srgb, #b8860b 14%, transparent);
  }
  .header-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  /* Square-ish split-pane action sitting next to "Open". */
  .btn.small.icon-only {
    padding: 0 6px;
    color: var(--text-dim);
  }
  .btn.small.icon-only:hover {
    color: var(--text);
  }
  /* Split "Open ▾" — primary button + workspace-picker caret. */
  .open-split {
    position: relative;
    display: inline-flex;
    align-items: stretch;
  }
  .open-main {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
  }
  .open-caret {
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
    padding: 0 5px;
    margin-left: 1px;
  }
  .open-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 50;
    min-width: 180px;
    max-height: 280px;
    overflow-y: auto;
    padding: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28);
  }
  .open-menu-title {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 4px 8px 6px;
  }
  .open-menu-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 30px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    text-align: left;
  }
  .open-menu-item:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .open-menu-item .cur {
    margin-left: auto;
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--accent);
  }
  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    border: none;
    background: transparent;
    padding: 0;
    cursor: default;
  }
  /* Recency / pin indicators */
  .pin-glyph {
    display: inline-flex;
    align-items: center;
    color: var(--accent);
    flex-shrink: 0;
  }
  /* Environment badges */
  .env-chip {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 3px;
    flex-shrink: 0;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .env-chip.prod {
    background: color-mix(in srgb, var(--status-exited) 20%, transparent);
    color: var(--status-exited);
  }
  .env-chip.ro {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .env-chip.stg {
    background: color-mix(in srgb, orange 15%, transparent);
    color: orange;
  }
</style>
