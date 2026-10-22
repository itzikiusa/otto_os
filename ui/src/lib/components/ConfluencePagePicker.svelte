<!--
  ConfluencePagePicker — the shared "parent page" typeahead (PC-9).
  A search-as-you-type over `searchConfluence({ space })`: the person picks a
  page by its TITLE, the form stores its id. Content guideline §6 — show the
  label, never the id — so the confirm copy can say `under “{title}”`.
  The result list renders inline under the field (no positioned popup), capped
  in height and scrollable, so long result sets never leave the viewport.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { searchConfluence } from '../api/issues';
  import type { ConfluencePageSummary } from '../api/types/product';
  import { loadErrorText } from '../loadError';

  let {
    accountId, spaceKey, value, title = null, onchange,
    label = 'Parent page', id = 'confluence-parent', disabled = false,
  }: {
    accountId: string;
    spaceKey: string;
    value: string | null;
    title?: string | null;
    onchange: (id: string | null, title: string | null) => void;
    label?: string;
    id?: string;
    disabled?: boolean;
  } = $props();

  let q = $state('');
  let hits = $state<ConfluencePageSummary[]>([]);
  let searching = $state(false);
  let error = $state('');
  let open = $state(false);
  let active = $state(-1);
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let ctl: AbortController | null = null;
  const listId = $derived(`${id}-results`);

  function cancel(): void {
    ++seq;
    if (timer) clearTimeout(timer);
    ctl?.abort();
    ctl = null;
  }

  function schedule(): void {
    cancel();
    error = '';
    const text = q.trim();
    if (!text || !accountId || !spaceKey) { hits = []; searching = false; return; }
    const mine = seq;
    searching = true;
    timer = setTimeout(async () => {
      const c = new AbortController();
      ctl = c;
      try {
        const res = await searchConfluence({ accountId, q: text, space: spaceKey }, c.signal);
        if (mine !== seq) return;
        hits = res;
        active = res.length > 0 ? 0 : -1;
      } catch (e) {
        if (mine === seq && !c.signal.aborted) { error = `Couldn’t search pages. ${loadErrorText(e)}`; hits = []; }
      } finally {
        if (mine === seq) searching = false;
      }
    }, 250);
  }

  // A different space or account invalidates the current pick.
  let lastScope = '';
  $effect(() => {
    const scope = `${accountId}\u0000${spaceKey}`;
    if (lastScope && scope !== lastScope && untrack(() => value)) onchange(null, null);
    lastScope = scope;
    hits = []; q = ''; open = false;
    return cancel;
  });

  // Picking unmounts the focused input and Clear unmounts itself: hand focus
  // to the control that replaces each, or it falls to <body> (outside the
  // Modal's flow).
  let inputEl = $state<HTMLInputElement | null>(null);
  let clearEl = $state<HTMLButtonElement | null>(null);

  async function pick(h: ConfluencePageSummary): Promise<void> {
    onchange(h.id, h.title);
    q = ''; hits = []; open = false; active = -1;
    await tick();
    clearEl?.focus();
  }

  async function clear(): Promise<void> {
    onchange(null, null);
    await tick();
    inputEl?.focus();
  }

  /** Move the highlighted option and keep it inside the capped list. */
  async function moveActive(next: number): Promise<void> {
    open = true;
    active = next;
    await tick();
    document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: 'nearest' });
  }

  function onkey(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown' && hits.length) { e.preventDefault(); void moveActive((active + 1) % hits.length); }
    else if (e.key === 'ArrowUp' && hits.length) { e.preventDefault(); void moveActive((active - 1 + hits.length) % hits.length); }
    else if (e.key === 'Enter' && open && active >= 0 && hits[active]) { e.preventDefault(); void pick(hits[active]); }
    else if (e.key === 'Escape' && open) { e.preventDefault(); e.stopPropagation(); open = false; }
  }
</script>

<div class="cpp">
  <label class="label" for={id}>{label} <span class="dim">(optional)</span></label>
  {#if value}
    <div class="picked">
      <span class="picked-title" title={title ?? undefined}>{title ?? `Page ${value}`}</span>
      <button
        type="button"
        class="btn small"
        {disabled}
        aria-label="Clear {label.toLowerCase()}"
        title="Clear {label.toLowerCase()}"
        bind:this={clearEl}
        onclick={() => void clear()}
      >Clear</button>
    </div>
  {:else}
    <input
      dir="auto"
      {id}
      class="input"
      type="search"
      role="combobox"
      autocomplete="off"
      spellcheck="false"
      aria-autocomplete="list"
      aria-controls={listId}
      aria-expanded={open && hits.length > 0}
      aria-activedescendant={open && active >= 0 ? `${listId}-${active}` : undefined}
      placeholder={spaceKey ? `Search pages in ${spaceKey}…` : 'Pick a space first'}
      disabled={disabled || !spaceKey}
      bind:value={q}
      bind:this={inputEl}
      oninput={() => { open = true; schedule(); }}
      onkeydown={onkey}
    />
    {#if open && hits.length > 0}
      <ul id={listId} class="results" role="listbox" aria-label="{label} results">
        {#each hits as h, i (h.id)}
          <li
            id="{listId}-{i}"
            role="option"
            tabindex="-1"
            aria-selected={i === active}
            class:active={i === active}
            onmousedown={(e) => { e.preventDefault(); void pick(h); }}
          >{h.title}</li>
        {/each}
      </ul>
    {/if}
    <p class="hint" aria-live="polite">
      {#if error}<span class="err">{error}</span>
      {:else if searching}Searching…
      {:else if open && q.trim() && hits.length === 0}No pages match “{q.trim()}”.
      {:else}Leave empty to create the page at the top of the space.{/if}
    </p>
  {/if}
</div>

<style>
  .cpp { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .label { font-size: var(--fs-s); color: var(--text-dim); }
  .dim { color: var(--text-dim); font-weight: normal; }
  .picked { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .picked-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .results {
    list-style: none; margin: 0; padding: 4px 0;
    max-height: min(240px, 40vh); overflow-y: auto;
    border: 1px solid var(--border); border-radius: var(--radius-s);
  }
  .results li { padding: 4px 8px; cursor: pointer; overflow-wrap: anywhere; }
  .results li.active, .results li:hover { background: var(--hover); }
  .hint { margin: 0; color: var(--text-dim); font-size: var(--fs-xs); }
  .err { color: var(--danger); }
</style>
