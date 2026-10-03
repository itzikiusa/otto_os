<script module lang="ts">
  import { authedBlobUrl as fetchBlobUrl } from '../../lib/api/client';
  import { assetPath as vaultAssetPath } from '../../lib/api/vault';

  /** Rendered reading-view HTML per (vault, path, hash, resolved links). */
  const renderCache = new Map<string, string>();
  const RENDER_CACHE_MAX = 8;

  /** Attachment blob URLs, shared across note switches (LRU; the evicted URL
   *  is revoked — an <img> that already decoded it keeps showing). */
  const assetUrls = new Map<string, Promise<string>>();
  const ASSET_CACHE_MAX = 200;

  function assetBlobUrl(wsId: string, vaultId: number, path: string): Promise<string> {
    const key = `${wsId}:${vaultId}:${path}`;
    const hit = assetUrls.get(key);
    if (hit) {
      assetUrls.delete(key);
      assetUrls.set(key, hit);
      return hit;
    }
    const p = fetchBlobUrl(vaultAssetPath(wsId, vaultId, path));
    p.catch(() => assetUrls.delete(key)); // a failed load retries next render
    assetUrls.set(key, p);
    while (assetUrls.size > ASSET_CACHE_MAX) {
      const [k, old] = assetUrls.entries().next().value!;
      assetUrls.delete(k);
      void old.then((u) => URL.revokeObjectURL(u), () => {});
    }
    return p;
  }
</script>

<script lang="ts">
  // The note pane: breadcrumb + edit⇄read toggle, CodeMirror markdown editor
  // (autosave + wikilink completion) or the sanitized reading view (wikilink
  // nav, note embeds hydrated one level deep, image attachments, tag chips).
  import { untrack } from 'svelte';
  import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { vaultNote } from '../../lib/api/vault';
  import { ui } from '../../lib/stores/ui.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { renderMermaid } from '../canvas/mermaid';
  import { renderD2 } from '../canvas/d2';
  import { renderNote, resolverFrom, slugifyHeading, stripFrontmatter } from './mdRender';
  import { plainPreview, rendersOffThread } from './noteRenderPlan';
  import { renderNoteOffThread } from './noteRenderAsync';
  import RefineDrawer from './RefineDrawer.svelte';
  import StructuredNote from './StructuredNote.svelte';
  import LinkPreview from './LinkPreview.svelte';
  import { linkPreview } from './previewStore.svelte';
  import { structuredModel } from './structuredNote';
  import { vault, vaultConflictKind } from './vault.svelte';

  // -- "Refine with AI" drawer — open state lives here keyed BY PATH (outside
  // the note data), so reloading the note after the agent edits it does not
  // close the drawer or drop its terminal.
  let refineOpen = $state<Record<string, boolean>>({});
  const refineShown = $derived(!!(vault.notePath && refineOpen[vault.notePath]));

  function toggleRefine(): void {
    const p = vault.notePath;
    if (!p) return;
    refineOpen = { ...refineOpen, [p]: !refineOpen[p] };
  }

  // "Review + fix" (tree context menu) queues a pending refine — force the
  // drawer open for that note; the drawer itself consumes + auto-sends it.
  $effect(() => {
    const pending = vault.pendingRefine;
    if (pending && vault.notePath === pending.path && !refineOpen[pending.path]) {
      refineOpen = { ...refineOpen, [pending.path]: true };
    }
  });

  // -- attachments: authed blob URLs, patched into the live <img> -------------
  // The renderer emits `<img data-asset="path">` with no src (lazyAssets); an
  // effect fills `src` in place once the blob lands. Images used to be a
  // dependency of the whole render: N images = N+1 full re-renders, embed
  // re-fetches and diagram re-renders (and blob: srcs never passed the
  // sanitizer anyway).
  const noAssetUrl = (): null => null;

  function hydrateImages(root: ParentNode): void {
    const v = vault.current;
    if (!v) return;
    const wsId = vault.wsId, vaultId = v.id;
    for (const el of Array.from(root.querySelectorAll<HTMLImageElement>('img[data-asset]:not([src])'))) {
      const path = el.getAttribute('data-asset')!;
      void assetBlobUrl(wsId, vaultId, path).then(
        (u) => { el.src = u; },
        () => el.classList.add('asset-error'),
      );
    }
  }

  // -- reading view ------------------------------------------------------------
  /** Parse + highlight + sanitize once per note content: toggling Edit⇄Read or
   *  an equal poll refresh reuses the HTML instead of re-rendering 300 KB.
   *  Notes over 64 KB (F10) parse in a worker: the pane shows the escaped
   *  plain body at once, and `offThreadDone` re-runs this when the html lands
   *  in the cache. */
  let offThreadDone = $state(0);
  const offThreadInFlight = new Set<string>();
  const renderKey = $derived.by(() => {
    const n = vault.note;
    if (!n || vault.editing) return '';
    return `${vault.current?.id}:${n.meta.path}:${n.meta.hash}:${n.outgoing.map((o) => o.dst_path ?? '').join('|')}`;
  });
  function cacheHtml(key: string, html: string): void {
    renderCache.set(key, html);
    while (renderCache.size > RENDER_CACHE_MAX) renderCache.delete(renderCache.keys().next().value!);
  }
  const rendered = $derived.by(() => {
    void offThreadDone;
    const n = vault.note;
    const key = renderKey;
    if (!n || !key) return '';
    const hit = renderCache.get(key);
    if (hit !== undefined) {
      renderCache.delete(key);
      renderCache.set(key, hit); // LRU touch
      return hit;
    }
    if (rendersOffThread(n.raw)) return plainPreview(stripFrontmatter(n.raw));
    const html = renderNote(stripFrontmatter(n.raw), {
      resolve: resolverFrom(n.outgoing),
      assetUrl: noAssetUrl,
      lazyAssets: true,
    });
    cacheHtml(key, html);
    return html;
  });

  // Large note, not cached: render it in the worker; a superseded result
  // (the note changed or was left meanwhile) is still cached, never shown.
  $effect(() => {
    const key = renderKey;
    const n = vault.note;
    if (!key || !n || !rendersOffThread(n.raw) || renderCache.has(key) || offThreadInFlight.has(key)) return;
    const raw = n.raw, outgoing = n.outgoing;
    offThreadInFlight.add(key);
    untrack(() => {
      void renderNoteOffThread(raw, outgoing).then((html) => {
        offThreadInFlight.delete(key);
        cacheHtml(key, html);
        if (renderKey === key) offThreadDone++;
      });
    });
  });

  $effect(() => {
    void rendered;
    const host = readEl;
    if (host) untrack(() => hydrateImages(host));
  });

  let readEl = $state<HTMLElement | undefined>();

  function onReadClick(e: MouseEvent): void {
    const t = (e.target as HTMLElement).closest('a.internal-link, span.tag, div.note-embed');
    if (!t) return;
    if (t.classList.contains('tag')) {
      const tag = t.getAttribute('data-tag');
      if (tag) vault.searchTag(tag);
      return;
    }
    const path = t.getAttribute('data-path') ?? t.getAttribute('data-embed-path');
    if (path) {
      e.preventDefault();
      if (/\.md$/i.test(path)) {
        const anchor = t.getAttribute('data-anchor');
        void vault.open(path).then(() => {
          if (anchor && !anchor.startsWith('^')) scrollToHeading(anchor);
        });
      }
      return;
    }
    // Unresolved → offer to create the note.
    const raw = t.getAttribute('data-raw');
    if (raw && t.getAttribute('data-unresolved')) {
      e.preventDefault();
      const p = raw.endsWith('.md') ? raw : `${raw}.md`;
      void confirmer.ask(`Create "${p}"?`, { title: 'Create note', confirmLabel: 'Create', danger: false }).then((ok) => {
        if (ok) void vault.createNote(p, `# ${raw}\n\n`);
      });
    }
  }

  // Page previews on inline wikilinks (delegated; same popover as the
  // structured panel's chips). Leaving the link, scrolling or switching
  // notes closes it.
  function onReadOver(e: Event): void {
    const a = (e.target as HTMLElement).closest?.('a.internal-link[data-path]');
    if (a) linkPreview.show(a, a.getAttribute('data-path'));
  }
  function onReadOut(e: Event): void {
    const a = (e.target as HTMLElement).closest?.('a.internal-link[data-path]');
    if (a && !a.contains((e as MouseEvent).relatedTarget as Node | null)) linkPreview.hide();
  }
  $effect(() => {
    void vault.notePath;
    void vault.editing;
    linkPreview.hide();
  });

  function scrollToHeading(anchor: string): void {
    requestAnimationFrame(() => {
      readEl?.querySelector(`#h-${CSS.escape(slugifyHeading(anchor))}`)?.scrollIntoView({
        behavior: 'smooth',
        block: 'start',
      });
    });
  }

  // Hydrate note embeds (depth 1, no recursion — embedded bodies render plain).
  // At most EMBED_WORKERS reads in flight: the daemon admits only a handful of
  // concurrent note preparations and answers the rest "busy" (409), which
  // used to mark every embed past the 4th as broken. A busy answer is
  // retried with a short backoff instead of failing the embed.
  const EMBED_WORKERS = 2;

  async function readEmbed(wsId: string, vaultId: number, path: string) {
    for (let attempt = 0; ; attempt++) {
      try {
        return await vaultNote(wsId, vaultId, path);
      } catch (e) {
        if (attempt >= 4 || vaultConflictKind(e) !== 'busy') throw e;
        await new Promise((r) => setTimeout(r, 250 * 2 ** attempt));
      }
    }
  }

  $effect(() => {
    void rendered;
    const host = readEl;
    if (!host || !vault.current) return;
    const wsId = vault.wsId, vaultId = vault.current.id;
    const seen = new Set<string>([vault.notePath ?? '']);
    const queue: [Element, string][] = [];
    for (const el of Array.from(host.querySelectorAll('div.note-embed[data-embed-path]'))) {
      const p = el.getAttribute('data-embed-path')!;
      if (el.getAttribute('data-hydrated') || seen.has(p)) continue;
      el.setAttribute('data-hydrated', '1');
      queue.push([el, p]);
    }
    const worker = async (): Promise<void> => {
      for (let next = queue.shift(); next; next = queue.shift()) {
        const [el, p] = next;
        try {
          const n = await readEmbed(wsId, vaultId, p);
          const html = renderNote(stripFrontmatter(n.raw), {
            // Embedded content resolves its own links but never re-embeds.
            resolve: resolverFrom(n.outgoing),
            assetUrl: noAssetUrl,
            lazyAssets: true,
          });
          const body = document.createElement('div');
          body.className = 'embed-body md-body';
          body.innerHTML = html;
          // Strip nested embeds inside the embed (depth guard).
          body.querySelectorAll('div.note-embed').forEach((x) => x.removeAttribute('data-embed-path'));
          el.appendChild(body);
          hydrateImages(body);
        } catch {
          el.classList.add('embed-error');
        }
      }
    };
    untrack(() => {
      for (let i = 0; i < Math.min(EMBED_WORKERS, queue.length); i++) void worker();
    });
  });

  // -- diagram blocks: render mermaid / D2 fences to inline SVG -----------------
  // mdRender emits <div class="diagram-block" data-diagram=…><pre>src</pre></div>;
  // swap each for the rendered SVG (lazy-loaded libs). On error keep the source
  // visible with the parse message — never a blank hole in the note.
  let diagramSeq = 0;
  $effect(() => {
    void rendered;
    const host = readEl;
    if (!host) return;
    for (const el of Array.from(host.querySelectorAll('div.diagram-block:not([data-rendered])'))) {
      el.setAttribute('data-rendered', '1');
      const kind = el.getAttribute('data-diagram');
      const src = el.querySelector('pre.diagram-src')?.textContent ?? '';
      const id = `vault-diag-${++diagramSeq}`;
      untrack(() => {
        const render =
          kind === 'd2'
            ? renderD2(id, src, { dark: ui.resolvedScheme === 'dark', isStale: () => !el.isConnected })
            : renderMermaid(id, src, { isStale: () => !el.isConnected });
        void render.then(({ svg, error }) => {
          if (!el.isConnected) return;
          if (svg) {
            el.innerHTML = svg;
            el.classList.add('diagram-ok');
          } else {
            const err = document.createElement('div');
            err.className = 'diagram-error';
            err.textContent = `Diagram error: ${error ?? 'unknown'}`;
            el.prepend(err);
          }
        });
      });
    }
  });

  // -- editor: [[wikilink]] + #tag completion -----------------------------------
  async function vaultCompletions(cx: CompletionContext): Promise<CompletionResult | null> {
    const wiki = cx.matchBefore(/\[\[([^\]\n]*)$/);
    if (wiki) {
      const q = wiki.text.slice(2);
      const hits = await vault.switcherQuery(q);
      const options: Completion[] = hits.slice(0, 30).map((h) => {
        const target = h.path.replace(/\.md$/i, '');
        const insert = h.alias ? `${target}|${h.alias}]]` : `${target}]]`;
        return {
          label: h.alias ?? h.title,
          detail: h.path,
          apply: insert,
          type: 'text',
        };
      });
      return { from: wiki.from + 2, options, filter: false };
    }
    const tag = cx.matchBefore(/(?:^|\s)#([\p{L}\p{N}_\-/]*)$/u);
    if (tag && cx.explicit !== false) {
      const start = tag.text.indexOf('#');
      const q = tag.text.slice(start + 1).toLowerCase();
      const options: Completion[] = vault.tags
        .filter((t) => t.tag.toLowerCase().startsWith(q))
        .slice(0, 20)
        .map((t) => ({ label: `#${t.tag}`, apply: t.tag, detail: `${t.count}`, type: 'keyword' }));
      if (!options.length) return null;
      return { from: tag.from + start + 1, options, filter: false };
    }
    return null;
  }

  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 's') {
      e.preventDefault();
      void vault.saveNow();
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'e') {
      e.preventDefault();
      vault.setView(!vault.editing);
    }
  }

  const crumb = $derived((vault.notePath ?? '').split('/'));

  // -- structured view for typed OKF notes (plain notes: model is null) ---------
  // The toggle is a per-device preference (all typed notes at once).
  const STRUCTURED_KEY = 'otto.vault.structuredView';
  let structuredOn = $state((() => { try { return localStorage.getItem(STRUCTURED_KEY) !== 'off'; } catch { return true; } })());
  const structured = $derived(vault.note && !vault.editing ? structuredModel(vault.note) : null);
  function toggleStructured(): void {
    structuredOn = !structuredOn;
    try { localStorage.setItem(STRUCTURED_KEY, structuredOn ? 'on' : 'off'); } catch { /* private window */ }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if vault.note}
  <div class="note-view">
    <header>
      <nav class="crumbs" aria-label="Note path" title={vault.notePath ?? ''}>
        {#each crumb as part, i (i)}
          {#if i < crumb.length - 1}
            <span class="c dim">{part}</span><span class="sep">/</span>
          {:else}
            <span class="c">{part.replace(/\.md$/i, '')}</span>
          {/if}
        {/each}
      </nav>
      <div class="actions">
        <!-- Save state sits BEFORE the tools, so it appearing/disappearing
             never shifts the buttons under the pointer. -->
        {#if vault.saving}
          <span class="save-state" role="status">Saving…</span>
        {:else if vault.dirty}
          <span class="save-state" title="Autosaves in a moment">Unsaved</span>
        {/if}
        {#if structured}
          <button
            class="mode-btn"
            class:refine-on={structuredOn}
            title={structuredOn ? 'Hide structured panels' : 'Show structured panels'}
            aria-label="Structured view"
            aria-pressed={structuredOn}
            onclick={toggleStructured}
          >
            <Icon name="layout" size={14} />
          </button>
        {/if}
        <button class="mode-btn" title="Note edit history" aria-label="Note edit history" onclick={() => void vault.openHistory(vault.notePath ?? '')}><Icon name="clock" size={14} /></button>
        <button
          class="mode-btn"
          class:refine-on={refineShown}
          title="Refine with AI"
          aria-label="Refine with AI"
          aria-pressed={refineShown}
          onclick={toggleRefine}
        >
          <Icon name="sparkle" size={14} />
        </button>
        <button
          class="mode-btn labelled"
          title={vault.editing ? 'Reading view (⌘E)' : 'Edit (⌘E)'}
          aria-pressed={vault.editing}
          onclick={() => vault.setView(!vault.editing)}
        >
          <Icon name={vault.editing ? 'eye' : 'edit'} size={13} />
          {vault.editing ? 'Read' : 'Edit'}
        </button>
      </div>
    </header>

    {#if vault.conflict}
      <div class="conflict" role="alert">
        This note changed on disk while you were editing. Your edits are kept until you choose.
        <button class="btn small" onclick={() => void vault.conflictOverwrite()} title="Save your version; the disk version stays in History">Keep my edits</button>
        <button class="btn small danger" onclick={() => void vault.conflictReload()}>Discard my edits…</button>
      </div>
    {/if}

    {#if vault.editing}
      <div class="editor-wrap">
        <CodeEditor
          path={vault.notePath ?? 'note.md'}
          content={vault.draft}
          root=""
          language="markdown"
          readOnly={false}
          minimal
          wrap
          completionSource={vaultCompletions}
          onchange={(c: string) => vault.onDraftChange(c)}
        />
      </div>
    {:else}
      <!-- Rendered markdown is sanitized in mdRender (allowlist). -->
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_mouse_events_have_key_events -->
      <div
        class="read md-body"
        bind:this={readEl}
        onclick={onReadClick}
        onmouseover={onReadOver}
        onmouseout={onReadOut}
        onfocusin={onReadOver}
        onfocusout={() => linkPreview.hide()}
        onscroll={() => linkPreview.hide()}
      >
        {#if structured && structuredOn && vault.note}
          <StructuredNote model={structured} note={vault.note} />
        {/if}
        {@html rendered}
      </div>
    {/if}

    <LinkPreview />

    {#if refineShown && vault.notePath}
      <!-- Keyed by path (NOT by note content): reloading the same note after
           the agent's edit keeps the drawer + terminal mounted; opening a
           different note resets the drawer to that note's refine session. -->
      {#key vault.notePath}
        <RefineDrawer path={vault.notePath} />
      {/key}
    {/if}
  </div>
{/if}

<style>
  .note-view {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border);
    gap: 8px;
  }
  /* Long paths ellipsize per segment (folders first, they shrink before the
     note name) instead of being cut mid-glyph at the pane edge. */
  .crumbs {
    display: flex;
    gap: 4px;
    min-width: 0;
    font-size: var(--fs-s);
    overflow: hidden;
    white-space: nowrap;
  }
  .c {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .c.dim {
    flex-shrink: 100;
    min-width: 1.5em;
  }
  .sep,
  .actions {
    flex-shrink: 0;
  }
  .c.dim,
  .sep {
    color: var(--text-dim);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .save-state {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* Same quiet chrome as the header's icon buttons (no boxed outlines); the
     Edit/Read toggle carries a label so the main verb is findable. */
  .mode-btn {
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    height: 28px;
    min-width: 28px;
    padding: 0 7px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    font: inherit;
    font-size: var(--fs-s);
  }
  .mode-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .mode-btn.labelled {
    border-color: var(--border);
    color: var(--text);
  }
  .mode-btn.refine-on {
    border-color: var(--accent);
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .conflict {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
    margin: 8px 14px 0;
    padding: 8px 12px;
    border: 1px solid color-mix(in srgb, var(--warning) 55%, transparent);
    background: var(--warning-soft);
    border-radius: 8px;
    font-size: var(--fs-s);
  }

  .editor-wrap {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .editor-wrap > :global(*) {
    flex: 1;
    min-width: 0;
  }
  .read {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 18px 26px 60px;
    /* Use the space the user gives us: folding the side panes widens the
       measure up to 1240px instead of stranding it at a fixed 860px column.
       96% keeps a breathing gutter at every pane width. */
    max-width: min(1240px, 96%);
    width: 100%;
    margin: 0 auto;
    line-height: 1.6;
  }
  .read :global(pre.note-plain) {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text-dim);
    background: none;
    margin: 0;
  }
  .read :global(a.internal-link) {
    color: var(--accent-text);
    cursor: pointer;
    text-decoration: none;
    border-bottom: 1px solid transparent;
  }
  .read :global(a.internal-link:hover) {
    border-bottom-color: currentColor;
  }
  .read :global(a.internal-link.unresolved) {
    opacity: 0.6;
    border-bottom: 1px dashed currentColor;
  }
  .read :global(span.tag) {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent-text);
    border-radius: 999px;
    padding: 1px 8px;
    font-size: 0.85em;
    cursor: pointer;
  }
  .read :global(div.note-embed) {
    border: 1px solid var(--border);
    border-inline-start: 3px solid var(--accent);
    border-radius: 8px;
    padding: 8px 12px;
    margin: 8px 0;
  }
  .read :global(div.note-embed .embed-title) {
    font-weight: 600;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .read :global(div.note-embed.embed-error) {
    opacity: 0.5;
  }
  .read :global(blockquote.callout) {
    border-inline-start: 3px solid var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border-radius: 6px;
    padding: 8px 12px;
    margin: 8px 0;
  }
  .read :global(blockquote.callout .callout-title) {
    font-weight: 600;
    font-size: var(--fs-s);
    text-transform: capitalize;
    margin-bottom: 4px;
  }
  .read :global(blockquote.callout-warning),
  .read :global(blockquote.callout-caution) {
    border-inline-start-color: var(--warning);
    background: var(--warning-soft);
  }
  .read :global(blockquote.callout-danger),
  .read :global(blockquote.callout-bug) {
    border-inline-start-color: var(--danger);
    background: var(--danger-soft);
  }
  .read :global(img) {
    max-width: 100%;
    border-radius: 8px;
  }
  .read :global(pre) {
    overflow-x: auto;
  }
  .read :global(div.diagram-block) {
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 14px;
    margin: 10px 0;
    background: var(--surface-2);
    overflow-x: auto;
  }
  .read :global(div.diagram-block.diagram-ok) {
    display: flex;
    justify-content: center;
    /* Mermaid's `neutral` theme assumes a light surface — keep the figure
       readable in dark mode too (reads as an embedded light figure). */
    background: #fdfdfd;
  }
  .read :global(div.diagram-block svg) {
    max-width: 100%;
    height: auto;
  }
  .read :global(div.diagram-error) {
    color: var(--status-exited);
    font-size: var(--fs-s);
    margin-bottom: 8px;
  }
  .read :global(pre.diagram-src) {
    margin: 0;
    font-size: var(--fs-s);
  }
  .read :global(table) {
    border-collapse: collapse;
    display: block;
    overflow-x: auto;
    max-width: 100%;
  }
  .read :global(th),
  .read :global(td) {
    border: 1px solid var(--border);
    padding: 4px 10px;
  }
</style>
