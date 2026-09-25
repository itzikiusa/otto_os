<script lang="ts">
  // The Memory tab: everything Otto knows about you, visible and editable.
  //  • Profile — `profile.md`, facts you own (Otto only proposes additions).
  //  • Waiting for review — memories Otto suggested (memory approval on) or
  //    that a Hermes import queued; nothing is kept until you accept it.
  //  • Memories — short facts Otto saved, each with where it came from;
  //    search, forget one, or forget everything matching a phrase (with Undo).
  //  • Import from Hermes — a one-off, read-only scan of ~/.hermes/memories
  //    that only QUEUES entries for review. Hermes itself is never changed.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { assistant, describeError } from '../../lib/stores/assistant.svelte';
  import { assistantApi } from '../../lib/api/assistant';
  import { ApiError } from '../../lib/api/client';
  import type { AssistantMemory } from '../../lib/api/types';

  interface Props {
    onopenthread: (id: string) => void;
  }
  let { onopenthread }: Props = $props();

  $effect(() => {
    void assistant.loadMemory();
    void assistant.loadHermes();
  });

  const mem = $derived(assistant.memory);
  const memories = $derived(mem.data?.memories ?? []);
  const pendingMem = $derived(mem.data?.pending ?? []);

  // ── profile ────────────────────────────────────────────────────────────────
  let draft = $state<string | null>(null);
  const saved = $derived(mem.data?.profile.content ?? '');
  const profile = $derived(draft ?? saved);
  const dirty = $derived(draft !== null && draft !== saved);
  let saving = $state(false);
  let justSaved = $state(false);
  let profileError = $state('');
  async function saveProfile(): Promise<void> {
    if (!dirty || draft === null) return;
    saving = true;
    profileError = '';
    try {
      await assistant.saveProfile(draft);
      draft = null;
      justSaved = true;
    } catch (e) {
      profileError =
        e instanceof ApiError && e.status === 409
          ? 'Couldn’t save: your profile changed somewhere else. Copy your edits, reload, and apply them again.'
          : `Couldn’t save your profile. ${describeError(e)}`;
    } finally {
      saving = false;
    }
  }

  // ── memories ───────────────────────────────────────────────────────────────
  let query = $state('');
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? memories.filter((m) => m.text.toLowerCase().includes(q) || m.tags.some((t) => t.toLowerCase().includes(q))) : memories;
  });
  const ORIGIN: Record<AssistantMemory['source']['kind'], string> = { agent: 'Otto', user: 'You', hermes: 'Hermes' };
  const threadTitle = (id: string | null): string | null => (id ? (assistant.thread(id)?.title ?? 'a thread') : null);

  // No confirm: forgetting is undoable from the bar right below (patterns.md
  // §7: don't nag on a reversible action).
  async function forgetOne(m: AssistantMemory): Promise<void> {
    try {
      const token = await assistant.forgetMemory(m.id);
      undoBar = { label: `Forgot “${m.text}”`, tokens: [token] };
    } catch (e) {
      toasts.error('Couldn’t forget that memory', describeError(e));
    }
  }

  // The last forget, undoable inline (toasts have no actions).
  let undoBar = $state<{ label: string; tokens: string[] } | null>(null);
  let undoing = $state(false);
  async function undoForget(): Promise<void> {
    if (!undoBar) return;
    undoing = true;
    try {
      await assistant.restoreMemory(undoBar.tokens);
      undoBar = null;
    } catch (e) {
      toasts.error('Couldn’t restore', describeError(e));
    } finally {
      undoing = false;
    }
  }

  async function forgetMatching(): Promise<void> {
    const q = await confirmer.promptText('What should Otto forget? Every memory that matches is removed — you can undo it right after.', {
      title: 'Forget…',
      confirmLabel: 'Forget',
      initial: query.trim(),
    });
    if (!q) return;
    try {
      const res = await assistantApi.forget(q);
      await assistant.loadMemory();
      const n = res.forgotten.length;
      undoBar = n ? { label: `Forgot ${n} ${n === 1 ? 'memory' : 'memories'} matching “${q}”`, tokens: res.undo_tokens } : null;
      if (!n) toasts.info('Nothing matched', `No memory mentions “${q}”.`);
    } catch (e) {
      toasts.error('Couldn’t forget', describeError(e));
    }
  }

  // ── review queue ───────────────────────────────────────────────────────────
  let deciding = $state<string | null>(null);
  async function accept(m: AssistantMemory): Promise<void> {
    deciding = m.id;
    try {
      await assistant.acceptMemory(m.id);
    } catch (e) {
      toasts.error('Couldn’t keep that memory', describeError(e));
    } finally {
      deciding = null;
    }
  }
  async function reject(m: AssistantMemory): Promise<void> {
    deciding = m.id;
    try {
      await assistant.forgetMemory(m.id);
    } catch (e) {
      toasts.error('Couldn’t reject that memory', describeError(e));
    } finally {
      deciding = null;
    }
  }

  // ── Hermes (read-only preview → queue for review) ──────────────────────────
  const hermes = $derived(assistant.hermes);
  const fresh = $derived((hermes.data?.entries ?? []).filter((e) => !e.duplicate));
  let importing = $state(false);
  async function queueHermes(): Promise<void> {
    const ok = await confirmer.ask(
      `Queue ${fresh.length} ${fresh.length === 1 ? 'entry' : 'entries'} from ~/.hermes/memories for review? Nothing is kept until you accept each one, and Hermes isn’t changed.`,
      { title: 'Import from Hermes', confirmLabel: 'Queue for review', danger: false },
    );
    if (!ok) return;
    importing = true;
    try {
      const res = await assistantApi.hermesImport();
      await Promise.all([assistant.loadMemory(), assistant.loadHermes()]);
      toasts.success(`${res.queued} queued for review`, res.duplicates ? `${res.duplicates} already known, skipped.` : undefined);
    } catch (e) {
      toasts.error('Couldn’t read the Hermes memories', describeError(e));
    } finally {
      importing = false;
    }
  }
</script>

<div class="memory" data-testid="assistant-memory">
  {#if mem.state === 'loading' && !mem.data}
    <div aria-busy="true" aria-label="Loading memory"><Skeleton rows={5} height={40} /></div>
  {:else if mem.state === 'unsupported'}
    <EmptyState icon="book" title="Memory isn’t available yet" body="This daemon doesn’t have the assistant’s memory. Update Otto to see and edit what it remembers about you." />
  {:else if mem.state === 'error' && !mem.data}
    <div class="error" role="alert">
      <Icon name="warning" size={14} />
      <div class="error-t"><strong>Couldn’t load memory.</strong><span class="dim">{mem.error}</span></div>
      <button class="btn small" onclick={() => void assistant.loadMemory()}>Retry</button>
    </div>
  {:else}
    <section class="block">
      <div class="block-head">
        <div>
          <h2 class="h">Profile</h2>
          <p class="help">Facts about you that every thread starts with. You own this file; Otto can only suggest additions.</p>
        </div>
        <span class="status" aria-live="polite">{#if saving}Saving…{:else if dirty}Unsaved changes{:else if justSaved}Saved{/if}</span>
        <button class="btn small" onclick={() => void saveProfile()} disabled={!dirty || saving} title="Save (⌘S)">Save</button>
      </div>
      <label class="sr-only" for="profile-md">Profile (markdown)</label>
      <textarea
        id="profile-md"
        class="input profile mono"
        rows="8"
        value={profile}
        oninput={(e) => {
          draft = e.currentTarget.value;
          justSaved = false;
        }}
        onkeydown={(e) => {
          if (e.key === 's' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void saveProfile();
          }
        }}
        placeholder={'- Prefers aisle seats and quiet hotel rooms\n- Usually free after 17:00 on weekdays'}
        dir="auto"
      ></textarea>
      {#if profileError}<p class="err" role="alert">{profileError}</p>{/if}
    </section>

    {#if pendingMem.length}
      <section class="block" data-testid="memory-review">
        <div class="block-head">
          <div>
            <h2 class="h">Waiting for review <span class="count warn">{pendingMem.length}</span></h2>
            <p class="help">Otto keeps these only if you accept them.</p>
          </div>
        </div>
        <ul class="list">
          {#each pendingMem as m (m.id)}
            <li class="row">
              <div class="main">
                <div class="text">{m.text}</div>
                <div class="meta">
                  <span>{ORIGIN[m.source.kind] ?? m.source.kind}</span>
                  {#if m.source.file}<span class="mono" dir="ltr">· {m.source.file}</span>{/if}
                  <span>· <time datetime={m.created_at} title={new Date(m.created_at).toLocaleString()}>{rel(m.created_at)}</time></span>
                </div>
              </div>
              <div class="acts">
                <button class="btn small ghost" onclick={() => void reject(m)} disabled={deciding === m.id}>Reject</button>
                <button class="btn small" onclick={() => void accept(m)} disabled={deciding === m.id}>Keep</button>
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="block">
      <div class="block-head">
        <div>
          <h2 class="h">Memories <span class="count">{memories.length}</span></h2>
          <p class="help">Short facts Otto saved while you talked. Each links to where it came from.</p>
        </div>
        <button class="btn small" onclick={() => void forgetMatching()} disabled={!memories.length}>Forget…</button>
      </div>
      {#if undoBar}
        <div class="undo-bar" role="status">
          <Icon name="undo" size={14} />
          <span class="grow">{undoBar.label}</span>
          <button class="btn small" onclick={() => void undoForget()} disabled={undoing}>{undoing ? 'Restoring…' : 'Undo'}</button>
          <button class="icon-btn" onclick={() => (undoBar = null)} aria-label="Dismiss" title="Dismiss"><Icon name="x" size={12} /></button>
        </div>
      {/if}
      {#if memories.length}
        <div class="search">
          <Icon name="search" size={14} />
          <input class="search-in" type="search" bind:value={query} placeholder="Search memories" aria-label="Search memories" />
        </div>
      {/if}
      {#if !memories.length}
        <p class="none">Nothing yet. When Otto remembers something, the thread shows a chip with Undo and it’s listed here.</p>
      {:else if !shown.length}
        <p class="none">No memories match “{query.trim()}”. <button class="link" onclick={() => (query = '')}>Clear search</button></p>
      {:else}
        <ul class="list">
          {#each shown as m (m.id)}
            <li class="row">
              <div class="main">
                <div class="text">{m.text}</div>
                <div class="meta">
                  <span>{ORIGIN[m.source.kind] ?? m.source.kind}</span>
                  <span>· <time datetime={m.created_at} title={new Date(m.created_at).toLocaleString()}>{rel(m.created_at)}</time></span>
                  {#if m.source.thread_id}
                    <span>·</span>
                    <button class="link" onclick={() => onopenthread(m.source.thread_id!)} title="Open the conversation it came from">from {threadTitle(m.source.thread_id)}</button>
                  {/if}
                  {#each m.tags as t (t)}<span class="tag">{t}</span>{/each}
                </div>
              </div>
              <button class="icon-btn forget" onclick={() => void forgetOne(m)} aria-label={`Forget “${m.text}”`} title="Forget">
                <Icon name="trash" size={14} />
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if hermes.state === 'ready' && hermes.data?.available}
      <section class="block" data-testid="hermes-import">
        <div class="block-head">
          <div>
            <h2 class="h">Import from Hermes</h2>
            <p class="help">
              Found {hermes.data.entries.length} {hermes.data.entries.length === 1 ? 'entry' : 'entries'} in
              <span class="mono" dir="ltr">~/.hermes/memories</span> ({hermes.data.files.map((f) => f.name).join(', ')}).
              {#if fresh.length < hermes.data.entries.length} {hermes.data.entries.length - fresh.length} already known.{/if}
              Importing only queues them for your review.
            </p>
          </div>
          <button class="btn small" onclick={() => void queueHermes()} disabled={importing || !fresh.length}>
            {importing ? 'Queueing…' : fresh.length ? `Review ${fresh.length}…` : 'All imported'}
          </button>
        </div>
      </section>
    {:else if hermes.state === 'error'}
      <p class="none">Couldn’t read the Hermes memories. <button class="link" onclick={() => void assistant.loadHermes()}>Retry</button></p>
    {/if}
  {/if}
</div>

<style>
  .memory {
    max-width: 820px;
    padding: 18px 20px 32px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .block-head {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin-bottom: 8px;
  }
  .block-head > div {
    flex: 1;
    min-width: 0;
  }
  .h {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .count {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .count.warn {
    color: var(--warning);
  }
  .help {
    margin: 2px 0 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .status {
    font-size: var(--fs-s);
    color: var(--text-dim);
    align-self: center;
  }
  .profile {
    width: 100%;
    height: auto;
    min-height: 140px;
    resize: vertical;
    padding: 8px 10px;
    font-size: var(--fs-s);
    line-height: 1.55;
    box-sizing: border-box;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .err {
    margin: 6px 0 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    margin-bottom: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    max-width: 360px;
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .search-in {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    outline: none; /* the ring is drawn on .search:focus-within */
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 12px;
  }
  .row + .row {
    border-top: 1px solid var(--border);
  }
  .row:hover {
    background: var(--hover);
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .text {
    overflow-wrap: anywhere;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .forget {
    opacity: 0;
  }
  .row:hover .forget,
  .forget:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .forget {
      opacity: 1;
    }
  }
  .tag {
    padding: 0 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
  }
  .undo-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
    padding: 6px 8px 6px 12px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    border: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .undo-bar > :global(svg) {
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .acts {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .none {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  .dim {
    color: var(--text-dim);
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .error-t {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-s);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  @media (max-width: 640px) {
    .memory {
      padding: 12px 12px 24px;
    }
    .block-head {
      flex-wrap: wrap;
    }
  }
</style>
