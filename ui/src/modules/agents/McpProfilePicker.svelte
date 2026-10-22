<script lang="ts">
  // "MCP tools" — which Otto MCP profiles an agent session runs under. Multi-
  // select: the session sees the UNION of the picked profiles' tools. The
  // default profile is preselected; picking exactly the default stores
  // nothing, so the session follows a later default change (like sessions
  // created before profiles existed). Narrowing only — permissions and
  // approvals still apply to every tool the agent can see.
  import LoadState from '../../lib/components/LoadState.svelte';
  import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
  import { selectionHint, storedProfileIds, toggleProfile } from '../../lib/mcpProfiles';

  interface Props {
    /** The stored ids ([] = the default profile). */
    value: string[];
    onchange: (ids: string[]) => void;
    disabled?: boolean;
    /** Field id prefix (two pickers can share a page). */
    idPrefix?: string;
  }
  let { value, onchange, disabled = false, idPrefix = 'mcp-tools' }: Props = $props();

  $effect(() => {
    void mcpProfiles.load();
  });

  const selected = $derived(value.length ? value : [mcpProfiles.defaultId]);
  const hint = $derived(selectionHint(selected, mcpProfiles.profiles));

  function toggle(id: string): void {
    const next = toggleProfile(selected, id, mcpProfiles.defaultId);
    onchange(storedProfileIds(next, mcpProfiles.profiles, mcpProfiles.defaultId));
  }
</script>

<div class="field" data-testid="mcp-profile-picker">
  <span class="field-label" id="{idPrefix}-label">MCP tools</span>
  <LoadState what="MCP profiles" variant="compact" loading={!mcpProfiles.loaded && !mcpProfiles.error} error={mcpProfiles.error} rows={1} onretry={() => void mcpProfiles.load(true)}>
    <div class="chips" role="group" aria-labelledby="{idPrefix}-label">
      {#each mcpProfiles.profiles as p (p.id)}
        <button
          type="button"
          class="pick"
          aria-pressed={selected.includes(p.id)}
          title={p.description || p.name}
          data-testid="mcp-profile-option-{p.id}"
          {disabled}
          onclick={() => toggle(p.id)}
        >
          {p.name}{#if p.id === mcpProfiles.defaultId}<span class="dflt">default</span>{/if}
        </button>
      {/each}
    </div>
    <span class="hint">
      The agent sees Otto’s <strong>{mcpProfiles.labelFor(selected)}</strong> tools{selected.length > 1 ? ' (all picked profiles combined)' : ''}. Permissions and approvals still apply. {hint}
    </span>
  </LoadState>
  {#if mcpProfiles.error}<span class="hint">Until they load, the session gets the default profile.</span>{/if}
</div>

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pick {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 26px;
    padding: 2px 10px;
    font: inherit;
    font-size: var(--fs-s);
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    cursor: pointer;
  }
  .pick[aria-pressed='true'] {
    color: var(--accent-text);
    background: var(--accent-soft);
    border-color: var(--accent-solid);
  }
  .pick:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .pick:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .dflt {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
