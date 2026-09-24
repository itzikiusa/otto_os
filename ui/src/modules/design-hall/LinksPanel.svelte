<script lang="ts">
  // Right panel → Links: what this design USES (outgoing) and where it is USED
  // IN (incoming), each row with its relation, version policy and Open. Links a
  // document makes itself (otto://design URIs) are "extracted" and change only
  // by editing the document; explicit links are added and removed here.
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ApiError } from '../../lib/api/client';
  import { createLink, deleteLink, search } from '../../lib/api/design';
  import { openExternal } from '../../lib/external';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { DesignArtifact, DesignLinkDstKind, DesignLinkPolicy, DesignLinkRel, DesignSearchHit } from '../../lib/api/types';
  import StudioBadge from './StudioBadge.svelte';
  import { EXPLICIT_RELS, policyLabel, relLabel, type LinkRow } from './model';
  import { library } from './library.svelte';
  import { openArtifact, openStoryInProduct } from './nav';

  interface Props {
    artifact: DesignArtifact;
    uses: LinkRow[];
    usedIn: LinkRow[];
    loading: boolean;
    error: string | null;
    readonly: boolean;
    seqOf: (versionId: string) => number | null;
    onreload: () => void;
    /** Compare a pinned target's pinned version with its head. */
    oncompare: (target: DesignArtifact, pinnedVersionId: string) => void;
  }
  let { artifact, uses, usedIn, loading, error, readonly, seqOf, onreload, oncompare }: Props = $props();

  function iconFor(kind: DesignLinkDstKind | string): IconName {
    switch (kind) {
      case 'story':
        return 'ticket';
      case 'url':
        return 'globe';
      case 'session':
        return 'terminal';
      case 'pr':
        return 'pr';
      case 'vault_note':
        return 'book';
      default:
        return 'link';
    }
  }

  /** Pinned target whose head moved on: "pinned v9 · v10 available". */
  function newerThanPinned(r: LinkRow): number | null {
    if (r.link.policy !== 'pinned' || !r.other || !r.link.pinned_version_id) return null;
    const pinned = seqOf(r.link.pinned_version_id);
    const head = r.other.head_seq;
    return pinned != null && head != null && head > pinned ? head : null;
  }

  function pinnedSeq(r: LinkRow): number | null {
    if (!r.link.pinned_version_id) return null;
    if (r.other && r.link.pinned_version_id === r.other.head_version_id) return r.other.head_seq;
    return seqOf(r.link.pinned_version_id);
  }

  function open(r: LinkRow, incoming: boolean): void {
    if (incoming || r.link.dst_kind === 'artifact') {
      if (r.other) openArtifact(r.other.id);
      return;
    }
    if (r.link.dst_kind === 'story') openStoryInProduct(r.link.dst_id);
    else if (r.link.dst_kind === 'url') void openExternal(r.link.dst_id);
    else if (r.link.dst_kind === 'session') ws.navigateToSession(r.link.dst_id);
  }
  function canOpen(r: LinkRow, incoming: boolean): boolean {
    if (incoming || r.link.dst_kind === 'artifact') return !!r.other;
    return r.link.dst_kind === 'story' || r.link.dst_kind === 'url' || r.link.dst_kind === 'session';
  }

  async function remove(r: LinkRow): Promise<void> {
    try {
      await deleteLink(artifact.id, r.link.id);
      toasts.success('Link removed');
      onreload();
    } catch (e) {
      toasts.error('Couldn’t remove the link', e instanceof Error ? e.message : String(e));
    }
  }

  // ── Add link ──────────────────────────────────────────────────────────────
  let adding = $state(false);
  let rel = $state<DesignLinkRel>('references');
  let target = $state<'artifact' | 'story' | 'url'>('artifact');
  let policy = $state<'' | DesignLinkPolicy>('');
  let q = $state('');
  let hits = $state<DesignSearchHit[]>([]);
  let picked = $state<DesignArtifact | null>(null);
  let storyId = $state('');
  let url = $state('');
  let busy = $state(false);
  let addError = $state<string | null>(null);
  let searchSeq = 0;

  function openAdd(): void {
    adding = true;
    rel = 'references';
    target = 'artifact';
    policy = '';
    q = '';
    hits = [];
    picked = null;
    storyId = '';
    url = '';
    addError = null;
    void runSearch('');
  }

  async function runSearch(term: string): Promise<void> {
    const my = ++searchSeq;
    try {
      const r = await search(term, { limit: 8 });
      if (my === searchSeq) hits = r.filter((h) => h.artifact.id !== artifact.id);
    } catch {
      if (my === searchSeq) hits = [];
    }
  }
  let t: ReturnType<typeof setTimeout> | null = null;
  function onQuery(): void {
    if (t) clearTimeout(t);
    t = setTimeout(() => void runSearch(q.trim()), 200);
  }

  const stories = $derived(Object.values(library.stories).sort((a, b) => a.source_key.localeCompare(b.source_key)));
  const valid = $derived(
    target === 'artifact' ? !!picked : target === 'story' ? !!storyId : /^https?:\/\/\S+$/i.test(url.trim()),
  );

  async function add(): Promise<void> {
    if (!valid) return;
    busy = true;
    addError = null;
    try {
      await createLink(artifact.id, {
        rel,
        dst_kind: target,
        dst_id: target === 'artifact' ? picked!.id : target === 'story' ? storyId : url.trim(),
        ...(policy ? { policy } : {}),
      });
      adding = false;
      toasts.success('Link added');
      onreload();
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        addError = /cycle/i.test(e.message)
          ? `That link would make a loop — ${picked?.title ?? 'the target'} already ${relLabel(rel)} this design, directly or through another design.`
          : `This link already exists. ${e.message}`;
      } else {
        addError = e instanceof Error ? e.message : String(e);
      }
    } finally {
      busy = false;
    }
  }
</script>

<div class="links" data-testid="design-links-panel">
  {#if loading && !uses.length && !usedIn.length}
    <p class="dim pad" role="status">Loading links…</p>
  {:else if error}
    <div class="pad err" role="alert">
      <Icon name="warning" size={14} /> Couldn’t load links.
      <button class="btn small ghost" onclick={onreload}>Retry</button>
    </div>
  {:else}
    {#snippet row(r: LinkRow, incoming: boolean)}
      {@const newer = incoming ? null : newerThanPinned(r)}
      <li class="row" data-testid={incoming ? 'design-link-in' : 'design-link-out'}>
        <span class="lead">
          {#if r.other}<StudioBadge studio={r.other.studio} size={20} />{:else}<span class="kind"><Icon name={iconFor(incoming ? 'artifact' : r.link.dst_kind)} size={12} /></span>{/if}
        </span>
        <span class="main">
          <span class="t">
            <span class="name" title={r.label}>{r.label}</span>
            {#if r.other?.head_seq != null}<span class="ver">v{r.other.head_seq}</span>{/if}
          </span>
          <span class="sub">
            {relLabel(r.link.rel)}{r.link.src_node ? ` · ${r.link.src_node}` : ''}
            {#if r.link.origin === 'extracted'}<span class="dim" title="Found in the document (otto://design link) — edit the document to change it"> · in document</span>{/if}
          </span>
          <span class="pills">
            {#if r.link.broken}
              <span class="chip bad" title="The target artifact, version or node is missing">Broken</span>
            {:else if newer != null}
              <span class="chip warnc">pinned v{pinnedSeq(r)} · v{newer} available</span>
            {:else if !incoming && r.link.dst_kind === 'artifact'}
              <span class="chip">{policyLabel(r.link.policy, pinnedSeq(r))}</span>
            {/if}
          </span>
        </span>
        <span class="acts">
          {#if newer != null && r.other}
            <button class="btn small" onclick={() => oncompare(r.other!, r.link.pinned_version_id!)}>Compare…</button>
          {:else if canOpen(r, incoming)}
            <button class="btn small" onclick={() => open(r, incoming)}>Open</button>
          {/if}
          {#if !incoming && !readonly && r.link.origin === 'explicit'}
            <button class="icon-btn" onclick={() => void remove(r)} aria-label={`Remove link to ${r.label}`} title="Remove link">
              <Icon name="x" size={14} />
            </button>
          {/if}
        </span>
      </li>
    {/snippet}

    <section>
      <header>
        <h3>Uses</h3>
        <span class="dim">{uses.length} outgoing</span>
        <span class="grow"></span>
        {#if !readonly}
          <button class="btn small ghost" onclick={openAdd} data-testid="design-add-link"><Icon name="plus" size={12} /> Add link…</button>
        {/if}
      </header>
      {#if uses.length}
        <ul>{#each uses as r (r.link.id)}{@render row(r, false)}{/each}</ul>
      {:else}
        <p class="dim empty">This design doesn’t use other designs yet. Links appear when the document embeds an <span class="mono">otto://design/…</span> reference, or when you add one.</p>
      {/if}
    </section>
    <section>
      <header>
        <h3>Used in</h3>
        <span class="dim">{usedIn.length} incoming</span>
      </header>
      {#if usedIn.length}
        <ul>{#each usedIn as r (r.link.id)}{@render row(r, true)}{/each}</ul>
      {:else}
        <p class="dim empty">Nothing uses this design yet.</p>
      {/if}
    </section>
    <p class="foot dim">Links are version-aware: a design that follows the approved version updates when you approve; a pinned one stays put and shows when something newer exists.</p>
  {/if}
</div>

{#if adding}
  <Modal title="Add link" width={520} onclose={() => (adding = false)}>
    <div class="form">
      <div class="field">
        <label for="dh-link-rel">This design…</label>
        <select id="dh-link-rel" class="input" bind:value={rel}>
          {#each EXPLICIT_RELS as r (r)}<option value={r}>{relLabel(r)}</option>{/each}
        </select>
      </div>
      <div class="field">
        <span class="lbl" id="dh-link-target">Target</span>
        <div class="seg-row" role="radiogroup" aria-labelledby="dh-link-target">
          {#each [['artifact', 'A design'], ['story', 'A story'], ['url', 'A web page']] as [k, l] (k)}
            <button type="button" role="radio" aria-checked={target === k} class="pill-toggle" class:on={target === k}
              onclick={() => (target = k as typeof target)}>{l}</button>
          {/each}
        </div>
      </div>
      {#if target === 'artifact'}
        <div class="field">
          <label for="dh-link-q">Find a design</label>
          <input id="dh-link-q" class="input" bind:value={q} oninput={onQuery} placeholder="Tier card" data-testid="design-link-search" />
          <ul class="pick" role="listbox" aria-label="Designs">
            {#each hits as h (h.artifact.id)}
              <li>
                <button type="button" role="option" aria-selected={picked?.id === h.artifact.id} class:on={picked?.id === h.artifact.id}
                  onclick={() => (picked = h.artifact)}>
                  <StudioBadge studio={h.artifact.studio} />
                  <span class="name">{h.artifact.title}</span>
                  {#if h.artifact.head_seq != null}<span class="ver">v{h.artifact.head_seq}</span>{/if}
                </button>
              </li>
            {:else}
              <li class="dim">No designs match.</li>
            {/each}
          </ul>
        </div>
        <div class="field">
          <label for="dh-link-policy">Version</label>
          <select id="dh-link-policy" class="input" bind:value={policy}>
            <option value="">Default for “{relLabel(rel)}”</option>
            <option value="follow_approved">Follow the approved version</option>
            <option value="follow_latest">Always the latest version</option>
            <option value="pinned">Pin the current version</option>
          </select>
        </div>
      {:else if target === 'story'}
        <div class="field">
          <label for="dh-link-story">Story</label>
          <select id="dh-link-story" class="input" bind:value={storyId} disabled={!stories.length}>
            <option value="">{stories.length ? 'Pick a story' : 'No product stories loaded'}</option>
            {#each stories as s (s.id)}<option value={s.id}>{s.source_key} · {s.title}</option>{/each}
          </select>
        </div>
      {:else}
        <div class="field">
          <label for="dh-link-url">URL</label>
          <input id="dh-link-url" class="input" bind:value={url} placeholder="https://example.com/inspiration" />
          <span class="hint">Stored as a reference; nothing is fetched.</span>
        </div>
      {/if}
      {#if addError}<p class="err-msg" role="alert">{addError}</p>{/if}
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (adding = false)}>Cancel</button>
      <button class="btn primary" onclick={add} disabled={!valid || busy} data-testid="design-add-link-confirm">{busy ? 'Adding…' : 'Add link'}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .links {
    display: flex;
    flex-direction: column;
  }
  .pad {
    padding: 14px;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .err :global(svg) {
    color: var(--danger);
  }
  section {
    padding: 12px 14px;
    border-block-end: 1px solid var(--border);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-block-end: 6px;
  }
  h3 {
    margin: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  header .dim {
    font-size: var(--fs-xs);
  }
  .grow {
    flex: 1;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 10px;
    align-items: start;
    padding: 9px 0;
  }
  .row + .row {
    border-block-start: 1px solid var(--border);
  }
  .kind {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .t {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .name {
    font-size: var(--fs-s);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ver {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pills {
    display: flex;
    gap: 4px;
  }
  .pills:empty {
    display: none;
  }
  .warnc {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .empty {
    margin: 0;
    font-size: var(--fs-s);
  }
  .foot {
    margin: 0;
    padding: 12px 14px;
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .lbl {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .seg-row {
    display: flex;
    gap: 6px;
  }
  .pick {
    margin-block-start: 6px;
    max-height: 220px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .pick li.dim {
    padding: 8px 10px;
    font-size: var(--fs-s);
  }
  .pick button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .pick button:hover {
    background: var(--hover);
  }
  .pick button.on {
    background: var(--accent-soft);
  }
  .err-msg {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
