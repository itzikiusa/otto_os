<script lang="ts">
  // Clipboard history panel (`#/clipboard`, the shell's `otto-clipboard`
  // non-activating panel, summoned by its global shortcut — ⌘⌃⇧V default).
  // Keyboard-first, Raycast-style: type to filter, ↑↓ to move, Return pastes
  // into the app that was in front, ⌘K for Actions, Esc (or a click away)
  // closes. Everything comes from the desktop shell (lib/clipboardHistory) — this page
  // never signs in to the daemon, so nothing here reaches it.
  import { onMount, tick } from 'svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import ContextMenu from '../../lib/components/ContextMenu.svelte';
  import ConfirmDialog from '../../lib/components/ConfirmDialog.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { announceShown } from '../../lib/announce.svelte';
  import { onDesktopEvent, useGlassWindow, type DesktopEvent } from '../../lib/desktop';
  import { clip, type ClipEntry, type ClipKind, type ClipStatus } from '../../lib/clipboardHistory';
  import {
    KIND_LABEL, RENDER_CAP, dayLabel, formatSize, groupByDay, moveSelection, pasteLabel, title,
  } from './clipboardList';

  const KIND_ICON: Record<ClipKind, IconName> = { text: 'text', link: 'link', file: 'file', image: 'image' };

  let status = $state<ClipStatus | null>(null);
  let loadError = $state('');
  let query = $state('');
  let kind = $state<ClipKind | ''>('');
  let entries = $state<ClipEntry[]>([]);
  let sel = $state(0);
  /** Inline line in the bottom bar: the paste fallback, or a failed action. */
  let note = $state<'accessibility' | string>('');
  let previewText = $state('');
  /** Blob URLs: `t:<id>` thumbnails, `f:<id>` full images, `i:<bundle>` app icons. */
  let urls = $state<Record<string, string>>({});
  const requested = new Set<string>();
  let filterEl: HTMLInputElement | undefined = $state();
  let actionsBtn: HTMLButtonElement | undefined = $state();

  const groups = $derived(groupByDay(entries, Date.now()));
  const flat = $derived(groups.flatMap((g) => g.items));
  const current = $derived(flat[sel] ?? null);

  let seq = 0;
  async function refresh(): Promise<void> {
    const mine = ++seq;
    try {
      const s = await clip.status();
      const list = s.enabled ? await clip.list(query, kind || null) : [];
      if (mine !== seq) return;
      status = s;
      entries = list;
      loadError = '';
      sel = moveSelection(sel, 0, Math.min(list.length, RENDER_CAP));
    } catch (e) {
      if (mine === seq) loadError = loadErrorText(e);
    }
  }

  // Re-list as the filter or the type changes (the shell searches full text).
  $effect(() => {
    void query;
    void kind;
    sel = 0;
    void refresh();
  });

  function blob(key: string, load: () => Promise<ArrayBuffer>): void {
    if (requested.has(key)) return;
    requested.add(key);
    void load()
      .then((buf) => (urls = { ...urls, [key]: URL.createObjectURL(new Blob([buf], { type: 'image/png' })) }))
      .catch(() => {});
  }

  function forgetBlobs(): void {
    for (const u of Object.values(urls)) URL.revokeObjectURL(u);
    urls = {};
    requested.clear();
  }

  $effect(() => {
    for (const e of flat) {
      if (e.kind === 'image') blob(`t:${e.id}`, () => clip.image(e.id, true));
      const b = e.source?.bundle_id;
      if (b) blob(`i:${b}`, () => clip.icon(b));
    }
  });

  $effect(() => {
    const e = current;
    if (!e) return;
    if (e.kind === 'image') return blob(`f:${e.id}`, () => clip.image(e.id, false));
    previewText = e.text;
    void clip.text(e.id).then((t) => { if (current?.id === e.id) previewText = t; }).catch(() => {});
  });

  async function act(how: 'paste' | 'copy'): Promise<void> {
    const e = current;
    if (!e) return;
    note = '';
    try {
      const pasted = await clip.paste(e.id, how === 'paste');
      if (how === 'copy') void clip.hide();
      else if (!pasted) note = 'accessibility';
    } catch (err) {
      note = `Couldn’t ${how}: ${loadErrorText(err)}`;
    }
  }

  async function run(what: () => Promise<unknown>): Promise<void> {
    try {
      await what();
      await refresh();
    } catch (err) {
      note = loadErrorText(err);
    }
  }

  async function clearHistory(): Promise<void> {
    const ok = await confirmer.ask('Every entry, pinned ones too, is deleted from this Mac.', {
      title: 'Clear clipboard history?',
      confirmLabel: 'Clear history',
      danger: true,
    });
    if (!ok) return;
    forgetBlobs();
    await run(() => clip.clear());
  }

  function openActions(): void {
    if (!actionsBtn) return;
    const e = current;
    const items: MenuItem[] = [
      ...(e
        ? [
            { label: pasteLabel(status?.target ?? null), icon: 'send', hint: '↵', action: () => void act('paste') },
            { label: 'Copy to Clipboard', icon: 'copy', action: () => void act('copy') },
            { label: e.pinned ? 'Unpin' : 'Pin', icon: 'pin', action: () => void run(() => clip.pin(e.id, !e.pinned)) },
            { label: 'Delete Entry', icon: 'trash', action: () => void run(() => clip.remove(e.id)) },
            { separator: true },
          ]
        : []),
      status?.paused
        ? { label: 'Resume Capture', icon: 'play', action: () => void run(() => clip.set({ paused: false })) }
        : { label: 'Pause Capture', icon: 'pause', action: () => void run(() => clip.set({ paused: true })) },
      { label: 'Clear History…', icon: 'trash', danger: true, action: () => void clearHistory() },
    ];
    ctxMenu.showAt(actionsBtn, items, { align: 'end' });
  }

  function onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || ctxMenu.open || confirmer.open) return;
    const t = e.target as HTMLElement | null;
    const inList = t === filterEl || t === document.body || !!t?.closest('.list');
    if (e.key === 'Escape') {
      e.preventDefault();
      void clip.hide();
    } else if (e.key === 'k' && e.metaKey) {
      e.preventDefault();
      openActions();
    } else if (inList && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault();
      sel = moveSelection(sel, e.key === 'ArrowDown' ? 1 : -1, flat.length);
      void tick().then(() => document.getElementById(`clip-${current?.id}`)?.scrollIntoView({ block: 'nearest' }));
    } else if (inList && e.key === 'Enter' && !e.isComposing) {
      e.preventDefault();
      void act('paste');
    }
  }

  async function shown(): Promise<void> {
    query = '';
    kind = '';
    note = '';
    sel = 0;
    await refresh();
    filterEl?.focus();
  }

  function fullTime(ms: number): string {
    const time = new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    return `${dayLabel(ms, Date.now())} at ${time}`;
  }

  onMount(() => {
    useGlassWindow();
    const offs: (() => void)[] = [];
    let disposed = false;
    const on = (name: DesktopEvent, cb: () => void) =>
      void onDesktopEvent(name, cb).then((off) => (disposed ? off() : offs.push(off)));
    on('otto://clipboard-shown', () => void shown());
    on('otto://clipboard-changed', () => void refresh());
    filterEl?.focus();
    return () => {
      disposed = true;
      offs.forEach((off) => off());
      forgetBlobs();
    };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="clip" role="dialog" aria-label="Clipboard history">
  <header class="top">
    <Icon name="search" size={14} />
    <input
      bind:this={filterEl}
      bind:value={query}
      class="filter"
      dir="auto"
      placeholder="Type to filter entries…"
      aria-label="Filter entries"
      role="combobox"
      aria-expanded="true"
      aria-controls="clip-list"
      aria-activedescendant={current ? `clip-${current.id}` : undefined}
      disabled={!status?.enabled}
    />
    <select class="input kind" bind:value={kind} aria-label="Type" disabled={!status?.enabled}>
      <option value="">All Types</option>
      <option value="text">Text</option>
      <option value="image">Images</option>
      <option value="file">Files</option>
      <option value="link">Links</option>
    </select>
  </header>

  {#if loadError}
    <EmptyState tone="error" icon="warning" title="Couldn’t load clipboard history" body={loadError} actionLabel="Retry" onaction={() => void refresh()} />
  {:else if !status}
    <div class="pad"><Skeleton rows={6} height={28} label="clipboard history" /></div>
  {:else if !status.enabled}
    <EmptyState
      icon="copy"
      title="Turn on clipboard history?"
      body="Otto will keep what you copy (text, links, file paths and images) on this Mac only, and never copies marked as passwords."
      actionLabel="Turn on"
      onaction={() => void run(() => clip.set({ enabled: true }))}
    />
  {:else}
    {#if status.access !== 'allowed'}
      {@const why = `macOS ${status.access === 'ask' ? 'asks before' : 'stops'} Otto reading the clipboard, so new copies aren’t kept. Set Otto to Allow in Paste from Other Apps.`}
      <div class="banner" use:announceShown={{ text: why }}>
        <Icon name="warning" size={13} />
        <span>{why}</span>
        <button class="btn small" onclick={() => void clip.openSettings('paste')}>Open System Settings</button>
      </div>
    {:else if status.paused}
      <div class="banner" use:announceShown={{ text: 'Capture is paused.' }}>
        <Icon name="pause" size={13} />
        <span>Capture is paused. New copies aren’t kept.</span>
        <button class="btn small" onclick={() => void run(() => clip.set({ paused: false }))}>Resume</button>
      </div>
    {/if}
    {#if entries.length === 0}
      {#if query || kind}
        <EmptyState icon="search" title="No matching entries" body="Try another word or All Types." />
      {:else}
        <EmptyState icon="copy" title="Nothing copied yet" body="Copy something in any app and it shows up here." />
      {/if}
    {:else}
      <div class="main">
        <div class="list" id="clip-list" role="listbox" aria-label="Entries">
          {#each groups as g (g.label)}
            <div class="group" role="group" aria-label={g.label}>
              <div class="group-label" aria-hidden="true">{g.label}</div>
              {#each g.items as e (e.id)}
                {@const i = flat.indexOf(e)}
                <button
                  id="clip-{e.id}"
                  class="clip-entry"
                  class:sel={i === sel}
                  role="option"
                  aria-selected={i === sel}
                  tabindex="-1"
                  onclick={() => (sel = i)}
                  ondblclick={() => void act('paste')}
                >
                  {#if e.kind === 'image' && urls[`t:${e.id}`]}
                    <img class="thumb" src={urls[`t:${e.id}`]} alt="" />
                  {:else}
                    <span class="kind-icon"><Icon name={KIND_ICON[e.kind]} size={13} /></span>
                  {/if}
                  <span class="row-title">{title(e)}</span>
                  {#if e.pinned}<Icon name="pin" size={12} />{/if}
                </button>
              {/each}
            </div>
          {/each}
          {#if entries.length > RENDER_CAP}
            <p class="more">Showing {RENDER_CAP} of {entries.length}. Type to narrow it down.</p>
          {/if}
        </div>

        <section class="preview" aria-label="Preview">
          {#if current}
            <div class="content">
              {#if current.kind === 'image'}
                {#if urls[`f:${current.id}`]}
                  <img src={urls[`f:${current.id}`]} alt="Copied image, {current.width} by {current.height} pixels" />
                {/if}
              {:else}
                <pre dir="auto">{previewText}</pre>
              {/if}
            </div>
            <h2 class="info-title">Information</h2>
            <dl class="info">
              <dt>Source</dt>
              <dd>
                {#if current.source?.bundle_id && urls[`i:${current.source.bundle_id}`]}
                  <img class="app-icon" src={urls[`i:${current.source.bundle_id}`]} alt="" />
                {/if}
                {current.source?.name ?? 'Unknown'}
              </dd>
              <dt>Type</dt><dd>{KIND_LABEL[current.kind]}</dd>
              {#if current.kind === 'image'}<dt>Dimensions</dt><dd>{current.width}×{current.height}</dd>{/if}
              <dt>Size</dt><dd>{formatSize(current.size)}</dd>
              <dt>Times copied</dt><dd>{current.count}</dd>
              <dt>Last copied</dt><dd>{fullTime(current.last_ms)}</dd>
            </dl>
          {/if}
        </section>
      </div>
    {/if}
  {/if}

  <footer class="bar">
    {#if note === 'accessibility'}
      <span class="note" use:announceShown={{ text: 'Copied. To paste for you, Otto needs Accessibility.' }}>Copied. To paste for you, Otto needs Accessibility.</span>
      <button class="btn small" onclick={() => void clip.openSettings('accessibility')}>Open System Settings</button>
    {:else if note}
      <span class="note error" use:announceShown={{ text: note, assertive: true }}>{note}</span>
    {:else}
      <span class="brand"><Icon name="copy" size={13} /> Clipboard History</span>
    {/if}
    <span class="spacer"></span>
    <button class="btn small primary" disabled={!current} onclick={() => void act('paste')}>
      {pasteLabel(status?.target ?? null)} <kbd>↵</kbd>
    </button>
    <button class="btn small ghost" bind:this={actionsBtn} aria-haspopup="menu" disabled={!status?.enabled} onclick={openActions}>
      Actions <kbd>⌘K</kbd>
    </button>
  </footer>
</div>

<ContextMenu />
<ConfirmDialog />

<style>
  .clip {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-m);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    overflow: hidden;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 10px;
    padding-inline: 14px;
    border-block-end: 1px solid var(--separator);
    color: var(--text-dim);
  }
  .filter {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-l);
    outline: none;
  }
  .kind {
    width: auto;
  }
  .pad {
    padding: 14px;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 8px;
    padding-inline: 14px;
    background: var(--warning-soft);
    font-size: var(--fs-s);
  }
  .banner span {
    flex: 1;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .list {
    width: 40%;
    overflow-y: auto;
    padding: 6px;
    border-inline-end: 1px solid var(--separator);
  }
  .group-label {
    padding-block: 8px 4px;
    padding-inline: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .clip-entry {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 32px;
    padding-inline: 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: default;
  }
  .clip-entry:hover {
    background: var(--hover);
  }
  .clip-entry.sel {
    background: var(--accent-soft);
  }
  .kind-icon {
    display: inline-flex;
    width: 20px;
    justify-content: center;
    color: var(--text-dim);
  }
  .thumb {
    width: 20px;
    height: 20px;
    object-fit: cover;
    border-radius: var(--radius-s);
  }
  .row-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .more {
    margin: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .preview {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 14px;
  }
  .content img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  pre {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .info-title {
    margin: 0;
    padding-block: 8px 4px;
    padding-inline: 14px;
    border-block-start: 1px solid var(--separator);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .info {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 16px;
    margin: 0;
    padding-block: 0 12px;
    padding-inline: 14px;
    font-size: var(--fs-s);
  }
  .info dt {
    color: var(--text-dim);
  }
  .info dd {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    justify-content: flex-end;
  }
  .app-icon {
    width: 16px;
    height: 16px;
  }
  .bar {
    margin-block-start: auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 8px;
    padding-inline: 14px;
    border-block-start: 1px solid var(--separator);
    background: var(--surface);
  }
  .brand,
  .note {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note.error {
    color: var(--danger);
  }
  .spacer {
    flex: 1;
  }
  kbd {
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    opacity: 0.8;
  }
  .clip-entry:focus-visible,
  .filter:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
</style>
