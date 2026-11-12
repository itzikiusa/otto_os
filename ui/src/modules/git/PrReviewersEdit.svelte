<script lang="ts">
  // "Edit reviewers" on an open PR: tick people off, add people (typeahead
  // from the forge's collaborators; free text when that lookup fails), then
  // one confirm that names who is added and who is removed before anything is
  // sent — the forge notifies the people added (outward-facing).
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import type { Collaborator } from '../../lib/api/types';
  import { confirmOutward } from '../../lib/confirmOutward';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { focusFirstField } from '../../lib/inlineEdit';
  import { announceShown } from '../../lib/announce.svelte';
  import { reviewerEdit, reviewerEditWhat, reviewerEditWho } from './pr-reviewers-edit';

  interface Props {
    repoId: string;
    number: number;
    /** Names on the PR now. */
    current: string[];
    /** "repo on GitHub · PR #N “title”" */
    where: string;
    providerName: string;
    onsaved: () => void;
    onclose: () => void;
  }
  let { repoId, number, current, where, providerName, onsaved, onclose }: Props = $props();

  let removed = $state<string[]>([]);
  let added = $state<string[]>([]);
  let query = $state('');
  let suggestions = $state.raw<Collaborator[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  const listId = $props.id();

  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  let lookupFailed = false;
  $effect(() => () => clearTimeout(timer));
  function lookup(q: string): void {
    clearTimeout(timer);
    const mine = ++seq;
    if (lookupFailed || q.trim() === '') return void (suggestions = []);
    timer = setTimeout(() => {
      void api.get<Collaborator[]>(`/repos/${repoId}/collaborators?q=${encodeURIComponent(q.trim())}`)
        .then((list) => { if (mine === seq) suggestions = list.slice(0, 8); })
        .catch(() => { lookupFailed = true; suggestions = []; });
    }, 200);
  }
  function add(): void {
    const n = query.trim();
    if (n && !added.includes(n)) added = [...added, n];
    query = '';
    suggestions = [];
  }
  const toggle = (name: string): void => {
    removed = removed.includes(name) ? removed.filter((r) => r !== name) : [...removed, name];
  };
  const edit = $derived(reviewerEdit(current, added, removed));

  async function save(): Promise<void> {
    if (!edit || busy) return;
    const ok = await confirmOutward({
      verb: 'Update reviewers',
      title: `Update the reviewers of PR #${number}?`,
      where,
      what: reviewerEditWhat(edit),
      who: reviewerEditWho(edit, providerName),
    });
    if (!ok) return;
    busy = true;
    error = null;
    try {
      await api.put(`/repos/${repoId}/prs/${number}/reviewers`, edit);
      toasts.success('Reviewers updated', `PR #${number}`);
      onsaved();
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Reviewers of #{number}" width={480} {onclose} dismissable={!busy}>
  <div class="rv">
    {#if current.length > 0}
      <ul class="rv-list" aria-label="Reviewers on this pull request">
        {#each current as name (name)}
          <li class="rv-row" class:gone={removed.includes(name)}>
            <span class="ell">{name}</span>
            <button class="btn small ghost rv-btn" disabled={busy} onclick={() => toggle(name)}
              aria-label={removed.includes(name) ? `Keep ${name}` : `Remove ${name}`} title={removed.includes(name) ? `Keep ${name}` : `Remove ${name}`}>
              {removed.includes(name) ? 'Keep' : 'Remove'}
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="dim none">No reviewers yet.</p>
    {/if}
    <div class="rv-add">
      <input dir="auto" class="input rv-input" aria-label="Add a reviewer" placeholder="Add a reviewer…" list={listId} bind:value={query} disabled={busy}
        use:focusFirstField oninput={() => lookup(query)} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add(); } }} />
      <datalist id={listId}>{#each suggestions as c (c.name)}<option value={c.name}>{c.display_name}</option>{/each}</datalist>
      <button class="btn small rv-btn" disabled={busy || query.trim() === ''} onclick={add}><Icon name="plus" size={12} /> Add</button>
    </div>
    {#if added.length > 0}
      <div class="rv-chips" aria-label="Reviewers to add">
        {#each added as name (name)}
          <span class="chip rv-chip">{name}
            <button class="chip-x" aria-label="Don’t add {name}" title="Don’t add {name}" disabled={busy}
              onclick={() => (added = added.filter((a) => a !== name))}><Icon name="x" size={12} /></button>
          </span>
        {/each}
      </div>
    {/if}
    {#if error}<p class="err" use:announceShown={{ text: `Couldn’t update the reviewers. ${error}`, assertive: true }}>Couldn’t update the reviewers. {error}</p>{/if}
  </div>
  {#snippet footer()}
    <div class="rv-foot">
      <button class="btn ghost" disabled={busy} onclick={onclose}>Cancel</button>
      <button class="btn primary" disabled={busy || !edit} onclick={save} data-testid="pr-reviewers-save">
        {busy ? 'Saving…' : 'Save to ' + providerName}
      </button>
    </div>
  {/snippet}
</Modal>

<style>
  .rv { display: flex; flex-direction: column; gap: 10px; }
  .rv-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .rv-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .rv-row .ell { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rv-row.gone .ell { text-decoration: line-through; color: var(--text-dim); }
  .rv-add { display: flex; gap: 8px; align-items: center; }
  .rv-input { flex: 1; min-width: 0; }
  .rv-chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .rv-chip { display: inline-flex; align-items: center; gap: 4px; }
  .chip-x { background: none; border: none; padding: 0; color: var(--text-dim); cursor: pointer; display: inline-flex; }
  .none { margin: 0; font-size: var(--fs-s); }
  .err { margin: 0; color: var(--danger); font-size: var(--fs-s); }
  .rv-foot { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: 8px; width: 100%; }
  @media (max-width: 1024px) {
    .rv-input { font-size: var(--fs-l); }
    .rv-btn { min-height: 34px; }
  }
</style>
