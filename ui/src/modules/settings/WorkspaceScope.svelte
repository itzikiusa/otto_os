<script lang="ts">
  // The "which workspace am I editing?" line at the top of every
  // per-workspace Settings page (Channels, MCP servers, Workspace context,
  // Self-improvement). With a dozen workspaces, "for this workspace" alone
  // made it easy to wire Slack into the wrong one. Names the CURRENT workspace
  // and, when there are others, switches it from a menu — the same selection
  // the sidebar's workspace menu makes (`ws.select`), so the page reloads for
  // the chosen workspace. Renders nothing without a workspace: each page
  // shows its own "pick a workspace" empty state.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';

  /** `readonly`: one line saying why this page is read-only for you. */
  let { readonly = '' }: { readonly?: string } = $props();

  const others = $derived(ws.workspaces.filter((w) => w.id !== ws.currentId));

  function pick(e: MouseEvent): void {
    const el = e.currentTarget as HTMLElement;
    const items = [...ws.workspaces]
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((w) => ({
        label: w.name,
        checked: w.id === ws.currentId,
        hint: w.my_role === 'viewer' ? 'view only' : undefined,
        action: () => {
          if (w.id !== ws.currentId) void ws.select(w.id);
        },
      }));
    ctxMenu.showAt(el, items, { filter: items.length > 8, filterPlaceholder: 'Search workspaces…', maxVisible: 12 });
  }
</script>

{#if ws.current}
  <div class="ws-scope" data-testid="settings-workspace-scope">
    <span class="ws-scope-label">Workspace</span>
    {#if others.length > 0}
      <button
        class="ws-scope-name"
        aria-haspopup="menu"
        title={`Editing ${ws.current.name} — choose another workspace`}
        aria-label={`Workspace: ${ws.current.name}. Choose another workspace`}
        onclick={pick}
      >
        <Icon name="folder" size={12} />
        <span class="ws-scope-text">{ws.current.name}</span>
        <Icon name="chevronDown" size={12} />
      </button>
    {:else}
      <span class="ws-scope-name static" title={ws.current.root_path}>
        <Icon name="folder" size={12} />
        <span class="ws-scope-text">{ws.current.name}</span>
      </span>
    {/if}
    <span class="ws-scope-hint">Changes here apply to this workspace only.</span>
  </div>
  {#if readonly}
    <p class="ws-scope-readonly" role="note" data-testid="settings-readonly-note"><Icon name="lock" size={12} /> {readonly}</p>
  {/if}
{/if}

<style>
  .ws-scope {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 8px;
    margin: 0 0 12px;
    max-width: var(--settings-col);
    font-size: var(--fs-s);
  }
  .ws-scope-label {
    color: var(--text-dim);
  }
  .ws-scope-name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 100%;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-weight: 600;
  }
  button.ws-scope-name {
    cursor: pointer;
  }
  button.ws-scope-name:hover {
    background: var(--hover);
  }
  button.ws-scope-name:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .ws-scope-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ws-scope-hint {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .ws-scope-readonly {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: -4px 0 12px;
    max-width: var(--settings-col);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
