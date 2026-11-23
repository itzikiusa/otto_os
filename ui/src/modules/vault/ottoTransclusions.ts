// Live hydration for `![[otto://design/<id>]]` and `![[otto://canvas/<id>]]`
// transclusion cards in Vault's reading view. Fetches metadata & thumbnails
// lazily, guards detached DOM / note switches, and revokes blob URLs on teardown.
import { api } from '../../lib/api/client';
import { getArtifact, thumbnailUrl } from '../../lib/api/design';
import type { CanvasScene } from '../../lib/canvas/types';
import { loadErrorText } from '../../lib/loadError';

function formatCanvasLabel(format?: string): string {
  if (format === 'excalidraw') return 'Excalidraw';
  if (format === 'd2') return 'D2 diagram';
  if (format === 'mermaid') return 'Mermaid diagram';
  return 'Canvas';
}

/** Hydrate all `.otto-card[data-otto-card]` elements inside `host`. Returns a
 *  cleanup function that invalidates pending updates and revokes any blob URLs. */
export function hydrateOttoTransclusions(host: HTMLElement): () => void {
  let alive = true;
  const blobUrls: string[] = [];
  const cards = Array.from(host.querySelectorAll<HTMLElement>('.otto-card[data-otto-card]'));

  for (const card of cards) {
    const kind = card.getAttribute('data-otto-card');
    const id = card.getAttribute('data-otto-id');
    const selector = card.getAttribute('data-otto-selector');
    if (!id || (kind !== 'design' && kind !== 'canvas')) continue;

    const titleEl = card.querySelector<HTMLElement>('.otto-card-title');
    const statusEl = card.querySelector<HTMLElement>('.otto-card-status');
    const metaEl = card.querySelector<HTMLElement>('.otto-card-meta');
    const thumbEl = card.querySelector<HTMLElement>('.otto-card-thumb');

    if (kind === 'design') {
      void (async () => {
        try {
          const detail = await getArtifact(id, { version: selector ?? undefined });
          if (!alive || !card.isConnected) return;
          const a = detail.artifact;
          if (titleEl) titleEl.textContent = a.title || id;
          if (statusEl) {
            const approvedTag = a.approved_version_id ? ' · approved' : '';
            const verTag = a.head_seq != null ? `v${a.head_seq}` : '';
            statusEl.textContent = `${a.status}${verTag ? ` · ${verTag}` : ''}${approvedTag}`;
            statusEl.classList.add(`status-${a.status}`);
          }
          if (metaEl) metaEl.textContent = `${a.format} · ${a.studio}${selector ? ` · @${selector}` : ''}`;
          if (thumbEl && a.thumb_blob) {
            try {
              const url = await thumbnailUrl(a.id, a.thumb_blob);
              if (!alive || !card.isConnected) {
                URL.revokeObjectURL(url);
                return;
              }
              blobUrls.push(url);
              const img = card.ownerDocument.createElement('img');
              img.src = url;
              img.alt = a.title || 'Design preview';
              img.loading = 'lazy';
              thumbEl.replaceChildren(img);
            } catch {
              /* no thumbnail available — keep badge tile */
            }
          }
        } catch (e) {
          if (!alive || !card.isConnected) return;
          if (statusEl) statusEl.textContent = 'Unavailable';
          if (metaEl) metaEl.textContent = loadErrorText(e);
        }
      })();
    } else if (kind === 'canvas') {
      void (async () => {
        try {
          const scene = await api.get<CanvasScene>(`/canvas/scenes/${encodeURIComponent(id)}?files=ref`);
          if (!alive || !card.isConnected) return;
          let docFormat = '';
          try {
            const parsed = JSON.parse(scene.doc_json) as { format?: unknown };
            if (typeof parsed?.format === 'string') docFormat = parsed.format;
          } catch {
            /* opaque/invalid doc_json */
          }
          if (titleEl) titleEl.textContent = scene.title || id;
          if (statusEl) statusEl.textContent = formatCanvasLabel(docFormat);
          if (metaEl) {
            const section = scene.section ? `${scene.section} · ` : '';
            const updated = scene.updated_at ? new Date(scene.updated_at).toLocaleDateString() : '';
            metaEl.textContent = `${section}${updated ? `Updated ${updated}` : `otto://canvas/${id}`}`;
          }
          if (thumbEl && scene.thumbnail && /^data:image\/(?:png|jpeg|webp);base64,/i.test(scene.thumbnail)) {
            const img = card.ownerDocument.createElement('img');
            img.src = scene.thumbnail;
            img.alt = scene.title || 'Canvas preview';
            img.loading = 'lazy';
            thumbEl.replaceChildren(img);
          }
        } catch (e) {
          if (!alive || !card.isConnected) return;
          if (statusEl) statusEl.textContent = 'Unavailable';
          if (metaEl) metaEl.textContent = loadErrorText(e);
        }
      })();
    }
  }

  return () => {
    alive = false;
    for (const u of blobUrls) URL.revokeObjectURL(u);
    blobUrls.length = 0;
  };
}
