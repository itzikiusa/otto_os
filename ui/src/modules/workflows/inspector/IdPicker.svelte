<script lang="ts" generics="T">
  // A node param that holds another object's id (a DB connection, a broker
  // cluster or topic, a product story, an issue account) as a picker over the
  // workspace's objects — the person never types an id the UI doesn't show.
  // A saved value that isn't in the list stays selectable ("Unknown … (id)") so
  // opening the node never silently rewrites it, and while the list is loading
  // or after it failed the field falls back to free text (with Retry).
  import { untrack } from 'svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { OptionCache } from './options.svelte';
  import type { IdOption } from './util';

  interface Props {
    id: string;
    value: string;
    /** The page's list cache and the key it's fetched by (workspace, cluster…). */
    cache: OptionCache<T>;
    key: string;
    toOptions: (rows: T[] | null) => IdOption[] | null;
    /** "connection", "cluster", "story"… — for "Choose a …" / "Unknown … (id)". */
    noun: string;
    /** The free-text fallback's placeholder. */
    placeholder: string;
    /** First option's label; defaults to "Choose a {noun}…". */
    emptyLabel?: string;
    /** What the list is, for "Couldn’t load {what}". */
    what: string;
    onchange: (value: string) => void;
    dir?: 'ltr' | 'auto';
  }
  let { id, value, cache, key, toOptions, noun, placeholder, emptyLabel, what, onchange, dir = 'auto' }: Props = $props();
  $effect(() => {
    const k = key;
    untrack(() => cache.ensure(k));
  });
  const options = $derived(toOptions(cache.rows(key)));
  const error = $derived(cache.error(key));
</script>

{#if options}
  <select {id} {value} onchange={(e) => onchange(e.currentTarget.value)}>
    <option value="">{emptyLabel ?? `Choose a ${noun}…`}</option>
    {#each options as o (o.id)}<option value={o.id} title={o.title}>{o.label}</option>{/each}
    {#if value && !options.some((o) => o.id === value)}<option {value}>Unknown {noun} ({value})</option>{/if}
  </select>
{:else}
  <input {dir} {id} type="text" {placeholder} {value} oninput={(e) => onchange(e.currentTarget.value)} />
  {#if error}<LoadState {what} variant="compact" {error} empty onretry={() => cache.retry(key)} />{/if}
{/if}
