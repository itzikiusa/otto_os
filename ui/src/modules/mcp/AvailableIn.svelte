<script lang="ts">
  // "Available in": this workspace · selected workspaces · all workspaces
  // (serverScope.ts). The multi-select lists the other workspaces the user can
  // see; the list is capped and scrolls, so many workspaces stay reachable.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { scopeMode, type Scope, type ScopeMode } from './serverScope';

  interface Props {
    /** The server's home workspace (always included). */
    home: string;
    scope: Scope;
    disabled?: boolean;
  }
  let { home, scope = $bindable(), disabled = false }: Props = $props();

  let mode = $state<ScopeMode>(scopeMode(scope));
  const others = $derived(ws.workspaces.filter((w) => w.id !== home && !w.archived));
  // Opened from a workspace the server was shared into: name its home.
  const homeName = $derived(home === ws.currentId ? null : (ws.workspaces.find((w) => w.id === home)?.name ?? 'its home workspace'));

  function setMode(m: ScopeMode): void {
    mode = m;
    scope = { all: m === 'all', shared: m === 'selected' ? scope.shared : [] };
  }

  function toggle(id: string, on: boolean): void {
    scope = { all: false, shared: on ? [...scope.shared, id] : scope.shared.filter((w) => w !== id) };
  }
</script>

<div class="field" data-testid="mcp-available-in">
  <label class="lbl" for="mcp-available-in">Available in</label>
  <select id="mcp-available-in" value={mode} {disabled} onchange={(e) => setMode(e.currentTarget.value as ScopeMode)}>
    <option value="this">{homeName ? `Only ${homeName}` : 'This workspace'}</option>
    <option value="selected" disabled={others.length === 0}>Selected workspaces…</option>
    <option value="all">All workspaces</option>
  </select>
  {#if mode === 'all'}
    <span class="note">Also every workspace added later.</span>
  {:else if mode === 'selected'}
    <div class="picks" role="group" aria-label="Workspaces to share with">
      {#each others as w (w.id)}
        <label class="pick">
          <input type="checkbox" {disabled} checked={scope.shared.includes(w.id)} onchange={(e) => toggle(w.id, e.currentTarget.checked)} />
          <span dir="auto">{w.name}</span>
        </label>
      {/each}
    </div>
    <span class="note">Plus {homeName ?? 'this workspace'}, its home.</span>
  {/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .lbl {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  select {
    width: 100%;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 6px 8px;
    font-size: var(--fs-m);
  }
  .note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .picks {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-block-size: 160px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
    color: var(--text);
  }
  .pick input {
    width: auto;
  }
</style>
