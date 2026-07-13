<script lang="ts">
  // The note pane: breadcrumb + edit⇄read toggle, CodeMirror markdown editor
  // (autosave + wikilink completion) or the sanitized reading view (wikilink
  // nav, note embeds hydrated one level deep, image attachments, tag chips).
  import { untrack } from 'svelte';
  import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { authedBlobUrl } from '../../lib/api/client';
  import { assetPath, vaultNote } from '../../lib/api/vault';
  import { ui } from '../../lib/stores/ui.svelte';
  import { renderMermaid } from '../canvas/mermaid';
  import { renderD2 } from '../canvas/d2';
  import { renderNote, resolverFrom, slugifyHeading, stripFrontmatter } from './mdRender';
  import RefineDrawer from './RefineDrawer.svelte';
  import { vault } from './vault.svelte';

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

  // -- attachments: authed blob URLs, cached per path -------------------------
  let assetUrls = $state<Record<string, string>>({});
  const pendingAssets = new Set<string>();

  function assetUrl(path: string): string | null {
    const hit = assetUrls[path];
    if (hit) return hit;
    if (!pendingAssets.has(path) && vault.current) {
      pendingAssets.add(path);
      void authedBlobUrl(assetPath(vault.wsId, vault.current.id, path))
        .then((u) => (assetUrls = { ...assetUrls, [path]: u }))
        .catch(() => pendingAssets.delete(path));
    }
    return null;
  }

  // -- reading view ------------------------------------------------------------
  const rendered = $derived.by(() => {
    const n = vault.note;
    if (!n || vault.editing) return '';
    // assetUrls is a dependency: images re-render once their blob URL lands.
    void assetUrls;
    return renderNote(stripFrontmatter(vault.editing ? vault.draft : n.raw), {
      resolve: resolverFrom(n.outgoing),
      assetUrl,
    });
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
      if (confirm(`Create "${p}"?`)) void vault.createNote(p, `# ${raw}\n\n`);
    }
  }

  function scrollToHeading(anchor: string): void {
    requestAnimationFrame(() => {
      readEl?.querySelector(`#h-${CSS.escape(slugifyHeading(anchor))}`)?.scrollIntoView({
        behavior: 'smooth',
        block: 'start',
      });
    });
  }

  // Hydrate note embeds (depth 1, no recursion — embedded bodies render plain).
  $effect(() => {
    void rendered;
    const host = readEl;
    if (!host || !vault.current) return;
    const seen = new Set<string>([vault.notePath ?? '']);
    for (const el of Array.from(host.querySelectorAll('div.note-embed[data-embed-path]'))) {
      const p = el.getAttribute('data-embed-path')!;
      if (el.getAttribute('data-hydrated') || seen.has(p)) continue;
      el.setAttribute('data-hydrated', '1');
      untrack(() =>
        vaultNote(vault.wsId, vault.current!.id, p)
          .then((n) => {
            const html = renderNote(stripFrontmatter(n.raw), {
              // Embedded content resolves its own links but never re-embeds.
              resolve: resolverFrom(n.outgoing),
              assetUrl,
            });
            const body = document.createElement('div');
            body.className = 'embed-body md-body';
            body.innerHTML = html;
            // Strip nested embeds inside the embed (depth guard).
            body.querySelectorAll('div.note-embed').forEach((x) => x.removeAttribute('data-embed-path'));
            el.appendChild(body);
          })
          .catch(() => el.classList.add('embed-error')),
      );
    }
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
            ? renderD2(id, src, { dark: ui.resolvedScheme === 'dark' })
            : renderMermaid(id, src);
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
</script>

<svelte:window onkeydown={onKeydown} />

{#if vault.note}
  <div class="note-view">
    <header>
      <nav class="crumbs" aria-label="Note path">
        {#each crumb as part, i (i)}
          {#if i < crumb.length - 1}
            <span class="c dim">{part}</span><span class="sep">/</span>
          {:else}
            <span class="c">{part.replace(/\.md$/i, '')}</span>
          {/if}
        {/each}
      </nav>
      <div class="actions">
        {#if vault.saving}
          <span class="save-state">saving…</span>
        {:else if vault.dirty}
          <span class="save-state">edited</span>
        {/if}
        <button
          class="mode-btn"
          class:refine-on={refineShown}
          title="Refine with AI"
          onclick={toggleRefine}
        >
          <Icon name="zap" size={14} />
        </button>
        <button
          class="mode-btn"
          title={vault.editing ? 'Reading view (⌘E)' : 'Edit (⌘E)'}
          onclick={() => vault.setView(!vault.editing)}
        >
          <Icon name={vault.editing ? 'eye' : 'edit'} size={14} />
        </button>
      </div>
    </header>

    {#if vault.conflict}
      <div class="conflict" role="alert">
        This note changed on disk while you were editing.
        <button onclick={() => void vault.conflictReload()}>Reload disk version</button>
        <button class="danger" onclick={() => void vault.conflictOverwrite()}>Overwrite</button>
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
          completionSource={vaultCompletions}
          onchange={(c: string) => vault.onDraftChange(c)}
        />
      </div>
    {:else}
      <!-- Rendered markdown is sanitized in mdRender (allowlist). -->
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="read md-body" bind:this={readEl} onclick={onReadClick}>
        {@html rendered}
      </div>
    {/if}

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
  .crumbs {
    display: flex;
    gap: 4px;
    font-size: 12.5px;
    overflow: hidden;
    white-space: nowrap;
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
    font-size: 11px;
    color: var(--text-dim);
  }
  .mode-btn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 4px 7px;
    cursor: pointer;
    display: inline-flex;
  }
  .mode-btn:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .mode-btn.refine-on {
    border-color: var(--accent, #7a9cff);
    color: var(--accent, #9ab4ff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
  }
  .conflict {
    display: flex;
    gap: 10px;
    align-items: center;
    margin: 8px 14px 0;
    padding: 8px 12px;
    border: 1px solid #b58a2c;
    background: rgba(181, 138, 44, 0.12);
    border-radius: 8px;
    font-size: 12.5px;
  }
  .conflict button {
    border: 1px solid var(--border);
    background: var(--panel-2, #222);
    color: var(--text);
    border-radius: 6px;
    padding: 3px 10px;
    cursor: pointer;
    font-size: 12px;
  }
  .conflict button.danger {
    border-color: #a33;
    color: #e88;
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
  .read :global(a.internal-link) {
    color: var(--accent, #7a9cff);
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
    background: var(--accent-dim, rgba(90, 120, 255, 0.16));
    color: var(--accent, #9ab4ff);
    border-radius: 999px;
    padding: 1px 8px;
    font-size: 0.85em;
    cursor: pointer;
  }
  .read :global(div.note-embed) {
    border: 1px solid var(--border);
    border-inline-start: 3px solid var(--accent, #7a9cff);
    border-radius: 8px;
    padding: 8px 12px;
    margin: 8px 0;
  }
  .read :global(div.note-embed .embed-title) {
    font-weight: 600;
    font-size: 12px;
    color: var(--text-dim);
  }
  .read :global(div.note-embed.embed-error) {
    opacity: 0.5;
  }
  .read :global(blockquote.callout) {
    border-inline-start: 3px solid var(--accent, #7a9cff);
    background: var(--accent-dim, rgba(90, 120, 255, 0.08));
    border-radius: 6px;
    padding: 8px 12px;
    margin: 8px 0;
  }
  .read :global(blockquote.callout .callout-title) {
    font-weight: 700;
    font-size: 12px;
    text-transform: capitalize;
    margin-bottom: 4px;
  }
  .read :global(blockquote.callout-warning),
  .read :global(blockquote.callout-caution) {
    border-inline-start-color: #d6a548;
    background: rgba(214, 165, 72, 0.08);
  }
  .read :global(blockquote.callout-danger),
  .read :global(blockquote.callout-bug) {
    border-inline-start-color: #d65648;
    background: rgba(214, 86, 72, 0.08);
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
    background: var(--panel, rgba(127, 127, 127, 0.04));
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
    color: #e88;
    font-size: 12px;
    margin-bottom: 8px;
  }
  .read :global(pre.diagram-src) {
    margin: 0;
    font-size: 12px;
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
