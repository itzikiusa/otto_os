<script lang="ts">
  // Publish sheets for a site. Both targets stay on this Mac:
  //   zip   — the static site (HTML per page + ONE site.css whose first block is
  //           the brand tokens as --brand-* CSS variables + assets) as a download;
  //   local — a loopback preview served by the daemon (bearer-authenticated).
  // Each publish is recorded server-side with the exact versions it rendered
  // (the site version, the brand kit, every 3D embed and image), so a later
  // change to a linked artifact never silently alters what was shipped.
  // Nothing here uploads anywhere; "Publish as claude.ai artifact" is not built.
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import * as design from '../../../lib/api/design';
  import type { DesignArtifact, DesignPublish, DesignSiteLocalResp } from '../../../lib/api/types';
  import type { AuditFinding } from './engine/audit';
  import type { SiteDoc } from './engine/types';
  import { exportZip, listPublishes, previewHtml, publishLocal } from './siteApi';

  interface Props {
    mode: 'zip' | 'local';
    artifact: DesignArtifact;
    doc: SiteDoc;
    /** The working copy as the studio would save it (dirty check vs the head). */
    source: string;
    findings: AuditFinding[];
    embedCount: number;
    onclose: () => void;
    onpreview: (served: Record<string, string>, url: string) => void;
  }
  let { mode, artifact, doc, source, findings, embedCount, onclose, onpreview }: Props = $props();

  let dirty = $state<boolean | null>(null);
  let history = $state<DesignPublish[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let local = $state<DesignSiteLocalResp | null>(null);

  $effect(() => {
    const id = artifact.id;
    void design
      .fetchContent(id, { asText: true })
      .then((c) => (dirty = (c.text ?? '') !== source))
      .catch(() => (dirty = null));
    void listPublishes(id)
      .then((h) => (history = h.slice(0, 5)))
      .catch(() => (history = []));
  });

  const pages = $derived(doc.pages.length);
  const errors = $derived(findings.filter((f) => f.level === 'error'));
  const seq = $derived(artifact.head_seq);

  async function doZip(): Promise<void> {
    busy = true;
    error = null;
    try {
      const r = await exportZip(artifact.id);
      const url = URL.createObjectURL(r.blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = r.filename;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      toasts.success(`Saved ${r.filename}`, `v${r.seq ?? seq} · every linked artifact pinned in the publish record.`);
      onclose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function doLocal(): Promise<void> {
    busy = true;
    error = null;
    try {
      local = await publishLocal(artifact.id);
      history = [local.publish, ...history].slice(0, 5);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function openLocal(): Promise<void> {
    if (!local) return;
    busy = true;
    try {
      const served: Record<string, string> = {};
      for (const [i, p] of local.pages.entries()) {
        served[p.id] = await previewHtml(artifact.id, { page: i === 0 ? undefined : p.slug || p.id, publish: local.publish.id });
      }
      onpreview(served, local.url);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function copyUrl(): Promise<void> {
    if (!local) return;
    try {
      await navigator.clipboard.writeText(local.url);
      toasts.success('Preview address copied', 'It needs your Otto sign-in and only works on this Mac.');
    } catch {
      toasts.warn('Couldn’t copy the address', local.url);
    }
  }
</script>

<Modal title={mode === 'zip' ? 'Export static site' : 'Publish preview (local)'} width={540} {onclose}>
  <div class="body" data-testid="site-publish-modal">
    {#if dirty}
      <p class="banner warn" role="status"><Icon name="warning" size={12} /> You have unsaved edits. {mode === 'zip' ? 'The export' : 'The preview'} uses the last saved version (v{seq}) — press Save (⌘S) first to include them.</p>
    {/if}
    {#if mode === 'zip'}
      <p class="lead">Downloads <strong>{artifact.title}</strong> v{seq} as a static site you can host anywhere.</p>
      <ul class="checks">
        <li><Icon name="check" size={12} /> {pages} page{pages === 1 ? '' : 's'} of semantic HTML — no scripts, motion is pure CSS</li>
        <li><Icon name="check" size={12} /> One <span class="mono">site.css</span>, starting with your brand tokens as <span class="mono">--brand-*</span> CSS variables</li>
        <li><Icon name="check" size={12} /> Images from your library copied into <span class="mono">assets/</span></li>
        <li><Icon name="check" size={12} /> {embedCount ? `${embedCount} 3D embed${embedCount === 1 ? '' : 's'} as poster images (the interactive runtime lands later)` : 'Responsive: desktop, tablet and mobile from the same files'}</li>
        <li><Icon name="shield" size={12} /> The exact version of everything it embeds is recorded with this export</li>
      </ul>
    {:else}
      <p class="lead">Serves <strong>{artifact.title}</strong> v{seq} from your Otto daemon at <span class="mono">127.0.0.1:7700</span>.</p>
      <ul class="checks">
        <li><Icon name="shield" size={12} /> Only reachable from this Mac, and only with your Otto sign-in</li>
        <li><Icon name="check" size={12} /> Rendered by the same engine as the export, with every linked artifact pinned</li>
      </ul>
      {#if local}
        <div class="served" data-testid="site-local-url">
          <span class="mono url">{local.url}</span>
          <button class="btn small" onclick={() => void copyUrl()}><Icon name="copy" size={12} /> Copy</button>
        </div>
        {#if local.warnings.length}
          <ul class="warns">{#each local.warnings as w (w)}<li><Icon name="warning" size={12} /> {w}</li>{/each}</ul>
        {/if}
      {/if}
    {/if}

    {#if errors.length}
      <p class="banner" role="status"><Icon name="info" size={12} /> {errors.length} accessibility issue{errors.length === 1 ? '' : 's'} on this site — see Checks in the Design panel. Publishing still works.</p>
    {/if}
    {#if error}<p class="banner bad" role="alert"><Icon name="warning" size={12} /> {error}</p>{/if}

    {#if history.length}
      <div class="history">
        <span class="k">Recent publishes</span>
        <ul>
          {#each history as h (h.id)}
            {@const set = Array.isArray(h.pinned_set) ? h.pinned_set : []}
            {@const pinnedRefs = set.filter((r) => r.role !== 'site').length}
            <li>
              <span class="chip">{h.target === 'zip' ? '.zip' : 'local'}</span>
              v{set.find((r) => r.role === 'site')?.seq ?? '?'} · {rel(h.created_at)}
              {#if pinnedRefs}<span class="dim"> · {pinnedRefs} pinned</span>{/if}
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    {#if mode === 'zip'}
      <button class="btn primary" disabled={busy || !artifact.head_version_id} onclick={() => void doZip()} data-testid="site-export-zip">
        <Icon name="download" size={12} /> {busy ? 'Exporting…' : 'Export .zip'}
      </button>
    {:else if !local}
      <button class="btn primary" disabled={busy || !artifact.head_version_id} onclick={() => void doLocal()} data-testid="site-publish-local">
        <Icon name="globe" size={12} /> {busy ? 'Publishing…' : 'Publish preview'}
      </button>
    {:else}
      <button class="btn primary" disabled={busy} onclick={() => void openLocal()} data-testid="site-open-local"><Icon name="eye" size={12} /> Open preview</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    font-size: var(--fs-m);
  }
  .lead {
    margin: 0;
  }
  .checks,
  .warns,
  .history ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .checks li,
  .warns li {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: var(--fs-s);
  }
  .checks :global(svg) {
    flex: none;
    margin-block-start: 3px;
    color: var(--success);
  }
  .warns :global(svg) {
    flex: none;
    margin-block-start: 3px;
    color: var(--warning);
  }
  .banner {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--info-soft);
    font-size: var(--fs-s);
  }
  .banner.warn {
    background: var(--warning-soft);
  }
  .banner.bad {
    background: var(--danger-soft);
  }
  .banner :global(svg) {
    flex: none;
    margin-block-start: 2px;
  }
  .served {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .url {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-s);
  }
  .history .k {
    display: block;
    margin-block-end: 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .history li {
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
  }
</style>
