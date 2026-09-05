<script lang="ts">
  // Outputs — the artifacts an agent session produced (files written, PRs,
  // images, reports, URLs) folded from its transcript, with an inline preview.
  // Rendering discipline (docs/design/conversation-view.md §5.6/§6):
  //   • HTML  → <iframe sandbox=""> via srcdoc (MockupViewer's approach: no
  //     scripts, forms, popups or same-origin — the frame can't reach the daemon)
  //   • Markdown → vault/mdRender (marked + allowlist sanitizer), never lib/md
  //   • images → <img> from an authed blob URL; PDF → sandboxed <iframe>
  //   • anything else → an authed download link
  // Bytes come ONLY from `GET /sessions/{id}/artifacts/{artifact_id}` (opaque id;
  // the server maps it back to the path it folded itself) — so an `on_disk`
  // History entry, which has no session, lists artifacts without a preview.
  //
  // Hosted two ways: as the right-panel **Outputs** tab (no props → the focused
  // agent session) and embedded under the History conversation (`embedded`).
  import { ws } from '../../lib/stores/workspace.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { authedBlobUrl, authedText } from '../../lib/api/client';
  import { renderNote } from '../vault/mdRender';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import type { Artifact } from '../../lib/api/types';

  interface Props {
    /** Session whose artifacts to show; defaults to the focused agent session. */
    sessionId?: string | null;
    /** Pre-folded list (History `on_disk` entries) — no session ⇒ no preview. */
    artifacts?: Artifact[] | null;
    /** Compact chrome when embedded under a conversation. */
    embedded?: boolean;
  }
  let { sessionId = null, artifacts = null, embedded = false }: Props = $props();

  // Resolve the session: explicit prop, else the focused agent session (the
  // right-panel tab is gated on one by the shell, but stay defensive).
  const focused = $derived(ws.activeSession?.kind === 'agent' ? ws.activeSession : null);
  const sid = $derived(sessionId ?? focused?.id ?? null);
  const list = $derived<Artifact[]>(artifacts ?? activity.artifacts(sid));

  $effect(() => {
    if (!artifacts && sid) void activity.loadArtifacts(sid);
  });

  // ── Selection + preview ──────────────────────────────────────────────────────
  let selectedId = $state<string | null>(null);
  const selected = $derived(list.find((a) => a.id === selectedId) ?? null);

  type PreviewKind = 'html' | 'md' | 'image' | 'pdf' | 'text' | 'link' | 'download' | 'none';
  let preview = $state<{ kind: PreviewKind; url?: string; text?: string; html?: string } | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let createdUrls: string[] = [];

  function revokeCreated(): void {
    for (const u of createdUrls.splice(0)) URL.revokeObjectURL(u);
  }
  $effect(() => () => revokeCreated());

  /** Extension of the artifact's path/url (lowercase, no dot). */
  function ext(a: Artifact): string {
    const p = a.path ?? a.url ?? '';
    const base = p.split(/[?#]/)[0].split('/').pop() ?? '';
    const i = base.lastIndexOf('.');
    return i >= 0 ? base.slice(i + 1).toLowerCase() : '';
  }

  /** How to render an artifact — mime first, extension as the fallback. */
  function classify(a: Artifact): PreviewKind {
    if (a.kind === 'pr' || a.kind === 'url') return 'link';
    const m = (a.mime ?? '').toLowerCase();
    const e = ext(a);
    if (m.startsWith('image/') || ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(e)) return 'image';
    if (m === 'text/html' || e === 'html' || e === 'htm') return 'html';
    if (m === 'text/markdown' || e === 'md' || e === 'markdown') return 'md';
    if (m === 'application/pdf' || e === 'pdf') return 'pdf';
    if (m.startsWith('text/') || m === 'application/json' || ['csv', 'json', 'txt', 'log', 'yaml', 'yml', 'toml'].includes(e))
      return 'text';
    return 'download';
  }

  const KIND_ICON: Record<Artifact['kind'], string> = {
    file: 'file',
    pr: 'pr',
    image: 'image',
    report: 'note',
    url: 'link',
  };

  /** Cap for inline text previews — the server already caps at 25 MB; the
   *  DOM shouldn't hold more than a couple hundred KB of <pre>. */
  const TEXT_CAP = 200 * 1024;

  async function select(a: Artifact): Promise<void> {
    selectedId = a.id;
    preview = null;
    error = null;
    revokeCreated();
    const kind = classify(a);
    if (kind === 'link') {
      preview = { kind, url: a.url ?? undefined };
      return;
    }
    if (!sid) {
      // on_disk History entry: the bytes route is per-session, so no preview.
      preview = { kind: 'none' };
      return;
    }
    // Two quick clicks race: every await below re-checks that THIS artifact is
    // still the selection, else the earlier fetch would land its bytes (and
    // the Download link) under the later label.
    const mine = a.id;
    const stale = (): boolean => selectedId !== mine;
    const route = `/sessions/${sid}/artifacts/${encodeURIComponent(a.id)}`;
    loading = true;
    try {
      if (kind === 'image' || kind === 'pdf' || kind === 'download') {
        const url = await authedBlobUrl(route);
        if (stale()) {
          URL.revokeObjectURL(url);
          return;
        }
        createdUrls.push(url);
        preview = { kind, url };
      } else if (kind === 'html') {
        const html = await authedText(route);
        if (stale()) return;
        preview = { kind, html };
      } else if (kind === 'md') {
        const md = await authedText(route);
        if (stale()) return;
        preview = { kind, html: renderNote(md, { resolve: () => null, assetUrl: () => null }) };
      } else {
        let text = await authedText(route);
        if (stale()) return;
        if (text.length > TEXT_CAP) text = text.slice(0, TEXT_CAP) + `\n… (truncated at ${TEXT_CAP / 1024} KB)`;
        preview = { kind: 'text', text };
      }
    } catch (e) {
      if (stale()) return;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      if (!stale()) loading = false;
    }
  }

  // Drop the selection when the session (or the list) changes underneath us.
  $effect(() => {
    void sid;
    void artifacts;
    selectedId = null;
    preview = null;
    error = null;
    revokeCreated();
  });

  async function copyPath(a: Artifact): Promise<void> {
    const v = a.path ?? a.url ?? '';
    try {
      await navigator.clipboard.writeText(v);
      toasts.info('Copied', v);
    } catch {
      toasts.error('Could not copy', v);
    }
  }

  function relTime(iso: string | null): string {
    if (!iso) return '';
    try {
      const secs = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 1000));
      if (secs < 60) return 'now';
      const mins = Math.floor(secs / 60);
      if (mins < 60) return `${mins}m`;
      const hrs = Math.floor(mins / 60);
      if (hrs < 24) return `${hrs}h`;
      return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    } catch {
      return '';
    }
  }

  function downloadName(a: Artifact): string {
    return (a.path ?? a.url ?? a.label).split('/').pop() || a.label;
  }
</script>

{#if !sid && !artifacts}
  <EmptyState
    icon="layers"
    title="No session selected"
    body="Open or focus an agent session to see the files, PRs and images it produced."
  />
{:else}
  <div class="outputs" class:embedded data-testid="outputs-panel">
    <div class="list-head">
      <span class="section-title">Outputs</span>
      {#if list.length > 0}<span class="count">{list.length}</span>{/if}
    </div>

    {#if list.length === 0}
      <p class="empty-line dim">
        Nothing produced yet. Files the agent writes, PRs it opens and images it captures show up here.
      </p>
    {:else}
      <ul class="alist" role="listbox" aria-label="Artifacts">
        {#each list as a (a.id)}
          <li>
            <button
              class="arow"
              class:on={selectedId === a.id}
              role="option"
              aria-selected={selectedId === a.id}
              onclick={() => void select(a)}
              title={a.path ?? a.url ?? a.label}
            >
              <span class="aicon"><Icon name={KIND_ICON[a.kind] ?? 'file'} size={12} /></span>
              <span class="alabel">{a.label}</span>
              <span class="ameta mono">{ext(a) || a.kind}</span>
              <span class="ameta mono">{relTime(a.produced_at)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if selected}
      <div class="preview" data-testid="outputs-preview">
        <div class="phead">
          <span class="ptitle" title={selected.path ?? selected.url ?? ''}>{selected.label}</span>
          <span class="pactions">
            <button class="icon-btn" title="Copy path" aria-label="Copy path" onclick={() => void copyPath(selected!)}>
              <Icon name="copy" size={12} />
            </button>
            {#if preview?.kind === 'download' || preview?.kind === 'image' || preview?.kind === 'pdf'}
              <a class="icon-btn" href={preview.url} download={downloadName(selected)} title="Download" aria-label="Download">
                <Icon name="arrowDown" size={12} />
              </a>
            {/if}
            <button class="icon-btn" title="Close preview" aria-label="Close preview" onclick={() => (selectedId = null)}>
              <Icon name="x" size={12} />
            </button>
          </span>
        </div>
        {#if loading}
          <div class="pbody dim">Loading…</div>
        {:else if error}
          <div class="pbody err">{error}</div>
        {:else if preview?.kind === 'link'}
          <div class="pbody">
            <a class="ext-link" href={preview.url} target="_blank" rel="noopener noreferrer">
              <Icon name="external" size={12} /> {preview.url}
            </a>
          </div>
        {:else if preview?.kind === 'none'}
          <div class="pbody dim">
            <div class="mono path">{selected.path ?? selected.url}</div>
            Preview is available once this conversation is resumed in Otto.
          </div>
        {:else if preview?.kind === 'image'}
          <div class="pbody media"><img src={preview.url} alt={selected.label} /></div>
        {:else if preview?.kind === 'html'}
          <!-- sandbox="" — most restrictive: no scripts/forms/popups/same-origin. -->
          <iframe class="frame" title={selected.label} sandbox="" srcdoc={preview.html}></iframe>
        {:else if preview?.kind === 'pdf'}
          <iframe class="frame" title={selected.label} sandbox="" src={preview.url}></iframe>
        {:else if preview?.kind === 'md'}
          <!-- eslint-disable-next-line svelte/no-at-html-tags — sanitized by mdRender -->
          <div class="pbody md">{@html preview.html}</div>
        {:else if preview?.kind === 'text'}
          <pre class="pbody mono text">{preview.text}</pre>
        {:else if preview?.kind === 'download'}
          <div class="pbody dim">
            No inline preview for this type.
            <a href={preview.url} download={downloadName(selected)}>Download {downloadName(selected)}</a>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .outputs {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .outputs.embedded {
    height: auto;
  }
  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 10px 6px;
  }
  .section-title {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
  }
  .count {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 1px 7px;
  }
  .empty-line {
    font-size: 11.5px;
    line-height: 1.4;
    margin: 2px 10px 10px;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .alist {
    list-style: none;
    margin: 0;
    padding: 0 6px 6px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    max-height: 40%;
    overflow-y: auto;
    flex-shrink: 0;
  }
  .outputs.embedded .alist {
    max-height: 220px;
  }
  .arow {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    text-align: start;
    cursor: pointer;
  }
  .arow:hover {
    background: var(--surface-2);
  }
  .arow.on {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .aicon {
    flex-shrink: 0;
    display: inline-flex;
    color: var(--text-dim);
  }
  .alabel {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ameta {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--text-dim);
  }
  .preview {
    flex: 1;
    min-height: 160px;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
  }
  .phead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    font-weight: 600;
  }
  .ptitle {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pactions {
    display: inline-flex;
    gap: 2px;
    flex-shrink: 0;
  }
  .pactions .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .pactions .icon-btn:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .pbody {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 10px;
    font-size: 12px;
    line-height: 1.5;
  }
  .pbody.media {
    display: grid;
    place-items: start center;
    background: var(--surface);
  }
  .pbody.media img {
    max-width: 100%;
    height: auto;
  }
  .pbody.text {
    margin: 0;
    font-size: 11px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .pbody.err {
    color: var(--status-exited, #e5534b);
  }
  .path {
    font-size: 11px;
    word-break: break-all;
    margin-bottom: 6px;
  }
  .frame {
    flex: 1;
    min-height: 200px;
    width: 100%;
    border: none;
    background: #fff;
  }
  .ext-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--accent);
    word-break: break-all;
  }
  .md :global(pre) {
    overflow-x: auto;
  }
  .md :global(img) {
    max-width: 100%;
  }
</style>
