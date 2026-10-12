<script lang="ts">
  // Settings → Channels: the allowed-users list as a LIST (one row per id,
  // with the Slack member's name when the directory knows it) instead of a
  // comma-separated text box. The comma string stays the value — it is what
  // the API stores — so this is a view + editor over `value`.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import type { SlackDirUser } from '../../lib/api/types';
  import { allowedIds, withAllowedMany, withoutAllowed } from './channelAllow';

  interface Props {
    /** Comma-separated ids (bindable). */
    value: string;
    /** Slack members for names + "Add person…"; empty → ids only. */
    people?: SlackDirUser[];
    placeholder?: string;
    /** Platform word for the empty state ("Slack", "Telegram"). */
    label: string;
  }
  let { value = $bindable(), people = [], placeholder = 'User ID', label }: Props = $props();

  const ids = $derived(allowedIds(value));
  const byId = $derived(new Map(people.map((p) => [p.id, p])));
  let typed = $state('');

  function addTyped(): void {
    if (!typed.trim()) return;
    value = withAllowedMany(value, typed);
    typed = '';
  }

  function pickPerson(e: MouseEvent | KeyboardEvent): void {
    const taken = new Set(ids);
    const items: MenuItem[] = people
      .filter((p) => !taken.has(p.id))
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((p) => ({
        label: p.name,
        hint: p.is_bot ? `@${p.handle} · bot` : `@${p.handle}`,
        title: p.id,
        action: () => (value = withAllowedMany(value, p.id)),
      }));
    if (items.length === 0) items.push({ label: 'Everyone is already allowed', disabled: true });
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Search people…', maxVisible: 12 });
  }
</script>

<div class="allow-list" data-testid="allowed-ids">
  {#if ids.length > 0}
    <ul aria-label="Allowed users">
      {#each ids as id (id)}
        {@const p = byId.get(id)}
        <li>
          <Icon name={p?.is_bot ? 'sparkle' : 'user'} size={13} />
          {#if p}
            <span class="who ellipsis">{p.name}</span>
            <span class="handle ellipsis">@{p.handle}</span>
          {:else}
            <span class="who unknown ellipsis" title="No {label} name found for this id">Unknown user</span>
          {/if}
          <span class="id mono" dir="ltr">{id}</span>
          <button
            type="button"
            class="rm"
            aria-label="Remove {p?.name ?? id}"
            title="Remove from the allowed users"
            onclick={() => (value = withoutAllowed(value, id))}
          >
            <Icon name="x" size={12} />
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="empty">No allowed users yet.</p>
  {/if}
  <div class="add-row">
    {#if people.length > 0}
      <button type="button" class="btn small" onclick={pickPerson} aria-haspopup="menu">
        <Icon name="plus" size={12} /> Add person…
      </button>
    {/if}
    <input
      dir="ltr"
      class="input mono add-id"
      aria-label="Add user id"
      bind:value={typed}
      {placeholder}
      spellcheck="false"
      autocomplete="off"
      onkeydown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          addTyped();
        }
      }}
    />
    <button type="button" class="btn small" disabled={!typed.trim()} onclick={addTyped}>Add</button>
  </div>
</div>

<style>
  .allow-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    max-height: 220px;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 30px;
    padding-inline: 10px 4px;
    font-size: var(--fs-s);
  }
  li + li {
    border-top: 1px solid var(--border);
  }
  li > :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .who {
    font-weight: 500;
    min-width: 0;
  }
  .who.unknown {
    color: var(--text-dim);
    font-weight: 400;
  }
  .handle {
    color: var(--text-dim);
    min-width: 0;
  }
  .id {
    margin-inline-start: auto;
    flex-shrink: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .rm {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .rm:hover {
    background: var(--hover);
    color: var(--danger);
  }
  .rm:focus-visible {
    outline: 2px solid var(--accent-text);
  }
  .empty {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .add-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .add-id {
    flex: 1;
    min-width: 120px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
