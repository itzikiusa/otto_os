<script lang="ts">
  // Lazy recursive schema tree (databases → tables/views → columns; keyspaces →
  // keys; collections → fields). Mirrors CollectionsTree: chevron expand, indent
  // by depth, an icon per node kind, dimmed `detail`. Clicking a leaf object
  // opens its Structure; right-click offers "Explain with agent".
  import Icon from '../../lib/components/Icon.svelte';
  import RedisKeyFilter from './RedisKeyFilter.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { DbNodeKind, SchemaNode } from '../../lib/api/types';

  // Node kinds that, when clicked, open the Structure view (vs. just expanding).
  const OBJECT_KINDS = new Set<DbNodeKind>(['table', 'view', 'collection', 'key']);

  function iconFor(kind: DbNodeKind): string {
    switch (kind) {
      case 'database':
      case 'schema':
        return 'db';
      case 'table':
        return 'grid';
      case 'view':
        return 'eye';
      case 'column':
      case 'field':
        return 'dot';
      case 'index':
        return 'key';
      case 'collection':
        return 'box';
      case 'keyspace':
      case 'key_namespace':
        return 'folder';
      case 'key':
        return 'key';
      default:
        return 'file';
    }
  }

  function onClick(node: SchemaNode): void {
    if (OBJECT_KINDS.has(node.kind)) {
      void database.openObject(node);
    } else if (node.kind === 'database') {
      // Clicking a database makes it the active one (queries scope to it, like
      // Workbench's bold default schema) and expands it.
      database.setActiveDb(node.label);
      if (node.has_children) void database.expand(node);
    } else if (node.has_children) {
      void database.expand(node);
    }
  }

  function explain(node: SchemaNode): void {
    void database.explainWithAgent(
      `Database object: ${node.label} (${node.kind})\nPath: ${node.id}`,
      `Explain this ${node.kind} and how it is typically used.`,
      `Explain ${node.label}`,
    );
  }

  async function copyName(node: SchemaNode): Promise<void> {
    try {
      await navigator.clipboard.writeText(node.label);
    } catch {
      /* clipboard unavailable — ignore */
    }
  }

  // Pretty number for the "Select Rows (Limit N)" label.
  const fmtNum = (n: number): string => n.toLocaleString();

  function showMenu(e: MouseEvent, node: SchemaNode): void {
    const isObject = OBJECT_KINDS.has(node.kind);
    const isSqlTable =
      database.capabilities?.sql === true && (node.kind === 'table' || node.kind === 'view');
    const isMongoCollection = node.kind === 'collection';

    const items = [];

    // Database node: set/clear the active database (queries scope to it). For
    // Mongo this is required so `db.<coll>...` resolves to the right database.
    if (node.kind === 'database') {
      if (database.activeDb === node.label) {
        items.push({ label: 'Clear active database', icon: 'db', action: () => database.setActiveDb(null) });
      } else {
        items.push({ label: 'Set as active database', icon: 'db', action: () => database.setActiveDb(node.label) });
      }
      items.push({ separator: true });
    }

    // Workbench-style data actions for SQL tables/views.
    if (isSqlTable) {
      items.push({
        label: `Select Rows (Limit ${fmtNum(database.rowLimit)})`,
        icon: 'play',
        action: () => void database.selectRows(node),
      });
      items.push({
        label: 'Send to SQL Editor',
        icon: 'send',
        action: () => void database.sendSelectToEditor(node),
      });
      items.push({ separator: true });
    }

    // Data actions for Mongo collections: find({}) capped at the row limit.
    if (isMongoCollection) {
      items.push({
        label: `Find Rows (Limit ${fmtNum(database.rowLimit)})`,
        icon: 'play',
        action: () => void database.findRows(node),
      });
      items.push({
        label: 'Send to Editor',
        icon: 'send',
        action: () => void database.sendFindToEditor(node),
      });
      items.push({ separator: true });
    }

    if (isObject) {
      items.push({ label: 'Open structure', icon: 'eye', action: () => database.openObject(node) });
    }
    items.push({ label: 'Explain with agent', icon: 'zap', action: () => explain(node) });

    items.push({ separator: true });
    items.push({ label: 'Copy name', icon: 'file', action: () => void copyName(node) });
    if (node.has_children) {
      items.push({ label: 'Refresh', icon: 'refresh', action: () => void database.refreshSchema() });
    }

    // Destructive SQL actions — pre-fill a tab (NOT auto-run); user reviews + runs.
    if (isSqlTable) {
      items.push({ separator: true });
      if (node.kind === 'table') {
        items.push({
          label: 'Truncate Table…',
          icon: 'trash',
          danger: true,
          action: () => void database.truncateTable(node),
        });
      }
      items.push({
        label: node.kind === 'view' ? 'Drop View…' : 'Drop Table…',
        icon: 'trash',
        danger: true,
        action: () => void database.dropObject(node),
      });
    }

    ctxMenu.show(e, items);
  }
</script>

<div class="schema-tree">
  {#if database.schemaLoading}
    <div class="tree-loading">
      <Icon name="refresh" size={13} />
      <span>Loading schema…</span>
    </div>
  {:else if database.schemaRoot.length === 0}
    <div class="tree-empty">No objects. Test the connection or refresh.</div>
  {:else}
    {#each database.schemaRoot as node (node.id)}
      {@render treeNode(node, 0)}
    {/each}
  {/if}
</div>

{#snippet treeNode(node: SchemaNode, depth: number)}
  {@const open = database.isExpanded(node.id)}
  {@const selected = database.selectedObjectPath === node.id}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div
    class="node"
    class:selected
    class:active-db={node.kind === 'database' && node.label === database.activeDb}
    style="padding-left: {depth * 13 + 4}px"
    oncontextmenu={(e) => showMenu(e, node)}
  >
    {#if node.has_children}
      <button class="caret" onclick={() => database.expand(node)} aria-label="Toggle">
        {#if database.isLoadingNode(node.id)}
          <span class="spin"><Icon name="refresh" size={10} /></span>
        {:else}
          <Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} />
        {/if}
      </button>
    {:else}
      <span class="caret-spacer"></span>
    {/if}
    <span class="node-icon {node.kind}"><Icon name={iconFor(node.kind)} size={12} /></span>
    <button
      class="node-label"
      onclick={() => onClick(node)}
      title={node.detail ? `${node.label} — ${node.detail}` : node.label}
    >
      <span class="nl-text ellipsis">{node.label}</span>
      {#if node.detail}<span class="nl-detail ellipsis">{node.detail}</span>{/if}
    </button>
  </div>
  {#if open}
    {#if node.kind === 'keyspace'}
      <RedisKeyFilter {node} {depth} />
    {/if}
    {@const children = database.childrenOf(node.id)}
    {#if children}
      {#each children as child (child.id)}
        {@render treeNode(child, depth + 1)}
      {:else}
        <div class="node-empty" style="padding-left: {(depth + 1) * 13 + 18}px">empty</div>
      {/each}
    {/if}
  {/if}
{/snippet}

<style>
  .schema-tree {
    display: flex;
    flex-direction: column;
    gap: 0;
    min-width: 0;
  }
  .tree-loading,
  .tree-empty {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 6px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .node {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 25px;
    padding-right: 4px;
    border-radius: var(--radius-s);
    min-width: 0;
  }
  .node:hover {
    background: color-mix(in srgb, var(--text-dim) 9%, transparent);
  }
  .node.selected {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  /* Active database = bold, like Workbench's default schema. */
  .node.active-db .nl-text {
    font-weight: 700;
    color: var(--text);
  }
  .node.active-db .node-icon {
    color: var(--accent);
  }
  .caret {
    display: grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .caret-spacer {
    width: 15px;
    flex-shrink: 0;
  }
  .spin {
    display: grid;
    place-items: center;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .node-icon {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .node-icon.table,
  .node-icon.view,
  .node-icon.collection {
    color: var(--accent);
  }
  .node-icon.database,
  .node-icon.schema {
    color: color-mix(in srgb, var(--accent) 80%, var(--text));
  }
  .node-label {
    display: flex;
    align-items: baseline;
    gap: 7px;
    min-width: 0;
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    height: 100%;
    padding: 0;
  }
  .nl-text {
    font-size: 12px;
    min-width: 0;
    /* Name is the primary value: it only shrinks as a last resort. */
    flex: 0 1 auto;
  }
  .nl-detail {
    font-size: 10.5px;
    color: var(--text-dim);
    min-width: 0;
    /* Engine/detail is secondary: shrinks (and ellipsises away) ~100× faster
       than the name, so a long engine never crowds out the table name. */
    flex: 0 100 auto;
  }
  .node-empty {
    font-size: 10.5px;
    color: var(--text-dim);
    font-style: italic;
    padding-top: 2px;
    padding-bottom: 2px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
