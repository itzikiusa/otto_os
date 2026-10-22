<!--
  ConfluenceSpaceSelect — the shared Confluence space `<select>` (PC-9).
  Loads the account's spaces itself, shows the space NAME (key in brackets),
  and picks the first space when `value` is empty or no longer listed so a
  form is never left pointing at a space the account can't see.
-->
<script lang="ts">
  import Skeleton from './Skeleton.svelte';
  import { listConfluenceSpaces } from '../api/issues';
  import type { ConfluenceSpace } from '../api/types/product';
  import { loadErrorText } from '../loadError';
  import { truncatedNote } from '../listingPage';

  let { accountId, value, onchange, id = 'confluence-space', disabled = false, emptyOption = undefined, ariaLabel = undefined }: {
    accountId: string;
    value: string;
    onchange: (key: string) => void;
    id?: string;
    disabled?: boolean;
    /** When set, the space is optional: a first `""` option with this label,
     *  and no auto-pick of the first space. */
    emptyOption?: string;
    /** Accessible name when no `<label for={id}>` names the select. */
    ariaLabel?: string;
  } = $props();

  let spaces = $state<ConfluenceSpace[]>([]);
  let loading = $state(false);
  let error = $state('');
  let truncated = $state(false);
  let seq = 0;

  async function load(): Promise<void> {
    const account = accountId;
    const mine = ++seq;
    spaces = []; error = ''; truncated = false;
    if (!account) return;
    loading = true;
    const ctl = new AbortController();
    abort = () => ctl.abort();
    try {
      const page = await listConfluenceSpaces(account, ctl.signal);
      if (mine !== seq) return;
      spaces = page.items;
      truncated = page.truncated;
      const listed = spaces.some((s) => s.key === value);
      if (emptyOption !== undefined) { if (value && !listed) onchange(''); }
      else if (spaces.length > 0 && !listed) onchange(spaces[0].key);
    } catch (e) {
      if (mine === seq && !ctl.signal.aborted) error = `Couldn’t load Confluence spaces. ${loadErrorText(e)}`;
    } finally {
      if (mine === seq) loading = false;
    }
  }
  let abort: () => void = () => {};

  $effect(() => {
    void accountId;
    void load();
    return () => { ++seq; abort(); };
  });
</script>

{#if loading}
  <Skeleton rows={1} height={27} label="spaces" />
{:else if error}
  <div role="alert" class="field-error">{error}</div>
  <button type="button" class="btn small" onclick={() => void load()}>Retry spaces</button>
{:else}
  <select
    {id}
    aria-label={ariaLabel}
    class="input"
    {value}
    disabled={disabled || (spaces.length === 0 && emptyOption === undefined)}
    onchange={(e) => onchange((e.currentTarget as HTMLSelectElement).value)}
  >
    {#if emptyOption !== undefined}
      <option value="">{emptyOption}</option>
    {/if}
    {#if spaces.length === 0 && emptyOption === undefined}
      <option value="">No spaces found</option>
    {/if}
    {#if spaces.length > 0}
      {#each spaces as sp (sp.key)}
        <option value={sp.key}>{sp.name} ({sp.key})</option>
      {/each}
    {/if}
  </select>
  {#if truncated}
    <p class="trunc-note" role="note">{truncatedNote('spaces', spaces.length)}</p>
  {/if}
{/if}

<style>
  .field-error { color: var(--danger); font-size: var(--fs-s); }
  .trunc-note { margin: 4px 0 0; color: var(--text-dim); font-size: var(--fs-xs); }
</style>
