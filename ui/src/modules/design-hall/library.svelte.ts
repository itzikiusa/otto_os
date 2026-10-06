// Design Hall — the library snapshot the lobby, project, studio and story pages
// share: every visible artifact (via `GET /design/search?q=` so each carries its
// `story_ids` + `reference_count`), the projects, and the product stories those
// artifacts implement. Loads are sequence-guarded (a slow response never
// overwrites a newer one) and live events trigger a debounced refresh.

import { api } from '../../lib/api/client';
import * as design from '../../lib/api/design';
import type { DesignArtifact, DesignProject, DesignSearchHit } from '../../lib/api/types';
import type { DesignBusEvent } from '../../lib/events.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import type { StoryRef } from './model';

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** More distinct artifacts than this in one burst → one full reload instead. */
const PATCH_MAX = 10;

/** Everything a library card / lookup reads from an artifact. */
function sameArtifactCard(a: DesignArtifact, b: DesignArtifact): boolean {
  return (
    a.updated_at === b.updated_at &&
    a.head_version_id === b.head_version_id &&
    a.approved_version_id === b.approved_version_id &&
    a.thumb_blob === b.thumb_blob &&
    a.title === b.title &&
    a.status === b.status &&
    a.project_id === b.project_id
  );
}

function sameHits(a: DesignSearchHit[], b: DesignSearchHit[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i], y = b[i];
    if (
      x.artifact.id !== y.artifact.id ||
      !sameArtifactCard(x.artifact, y.artifact) ||
      x.reference_count !== y.reference_count ||
      x.story_ids.join() !== y.story_ids.join()
    )
      return false;
  }
  return true;
}

function sameProjects(a: DesignProject[], b: DesignProject[]): boolean {
  return a.length === b.length && JSON.stringify(a) === JSON.stringify(b);
}

/** How many designs the lobby loads (newest first). */
export const LIBRARY_LIMIT = 500;

class DesignLibrary {
  // Replaced wholesale on every load, never mutated → raw (no deep proxy over
  // up to 500 hits), with id-keyed indexes for the per-card lookups.
  hits: DesignSearchHit[] = $state.raw([]);
  /** The lobby reads at most LIBRARY_LIMIT designs (newest first): project
   *  counts, mosaics and the Mine/Shipped filters only see those. True when
   *  the cap was hit, so the lobby can say so (S18-12). */
  truncated = $state(false);
  projects: DesignProject[] = $state.raw([]);
  #hitById = $derived(new Map(this.hits.map((h) => [h.artifact.id, h])));
  #projectById = $derived(new Map(this.projects.map((p) => [p.id, p])));
  #artifacts = $derived(this.hits.map((h) => h.artifact));
  /** story id → the story (key + title + parent), for chips and the epic strip. */
  stories: Record<string, StoryRef> = $state({});
  loading = $state(false);
  loaded = $state(false);
  error = $state<string | null>(null);

  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Story ids we already tried to resolve individually (never retried in a loop). */
  private tried = new Set<string>();

  get artifacts(): DesignArtifact[] {
    return this.#artifacts;
  }

  hitOf(id: string): DesignSearchHit | undefined {
    return this.#hitById.get(id);
  }

  projectOf(id: string | null | undefined): DesignProject | undefined {
    return id ? this.#projectById.get(id) : undefined;
  }

  /** Story keys ("LOY-142") an artifact implements, resolved where known. */
  storyKeys(artifactId: string): string[] {
    const h = this.hitOf(artifactId);
    if (!h) return [];
    return h.story_ids.map((sid) => this.stories[sid]?.source_key).filter((k): k is string => !!k);
  }

  async load(): Promise<void> {
    const my = ++this.seq;
    this.loading = true;
    try {
      const [hits, projects] = await Promise.all([
        design.search('', { limit: LIBRARY_LIMIT }),
        design.listProjects(),
      ]);
      if (my !== this.seq) return;
      // Keep the arrays when nothing a card shows changed, so the id maps,
      // grids and thumbnails see no change at all (SD-16).
      if (!sameHits(this.hits, hits)) this.hits = hits;
      if (!sameProjects(this.projects, projects)) this.projects = projects;
      this.truncated = hits.length >= LIBRARY_LIMIT;
      this.error = null;
      this.loaded = true;
      void this.resolveStories(my);
    } catch (e) {
      if (my !== this.seq) return;
      this.error = errText(e);
    } finally {
      if (my === this.seq) this.loading = false;
    }
  }

  /**
   * Live events → the cheapest refresh (SD-16). A content / meta / approval /
   * archive change of an artifact the library already shows re-reads THAT
   * artifact (one small GET, no content) and patches its card; anything that
   * can change the set, the story links or reference counts (created,
   * deleted, link events, unknown artifacts, big bursts) falls back to the
   * debounced full reload of up to 500 hits.
   */
  applyEvents(evs: readonly DesignBusEvent[]): void {
    let full = false;
    for (const ev of evs) {
      if (ev.type === 'design_learning_update') continue;
      if (
        ev.type === 'design_artifact_updated' &&
        ev.change !== 'created' &&
        ev.change !== 'deleted' &&
        this.#hitById.has(ev.artifact_id)
      ) {
        this.#patchIds.add(ev.artifact_id);
      } else {
        full = true;
      }
    }
    if (full || this.#patchIds.size > PATCH_MAX) {
      this.#patchIds.clear();
      this.refreshSoon();
    } else if (this.#patchIds.size) {
      this.#patchSoon();
    }
  }

  #patchIds = new Set<string>();
  #patchTimer: ReturnType<typeof setTimeout> | null = null;
  #patchSoon(): void {
    if (this.#patchTimer || this.timer) return; // a full reload is already due
    this.#patchTimer = setTimeout(() => {
      this.#patchTimer = null;
      void this.#patch();
    }, 350);
  }

  async #patch(): Promise<void> {
    const ids = [...this.#patchIds];
    this.#patchIds.clear();
    const seq = this.seq;
    const fresh = await Promise.all(
      ids.map((id) => design.getArtifact(id).then((d) => d.artifact, () => null)),
    );
    if (seq !== this.seq) return; // a full load landed meanwhile
    const byId = new Map(fresh.filter((a): a is DesignArtifact => !!a).map((a) => [a.id, a]));
    if (byId.size < ids.length) {
      this.refreshSoon(); // gone or unreadable → let the full list decide
      return;
    }
    let changed = false;
    const next = this.hits.map((h) => {
      const a = byId.get(h.artifact.id);
      if (!a || sameArtifactCard(h.artifact, a)) return h;
      changed = true;
      return { ...h, artifact: a };
    });
    if (changed) this.hits = next;
  }

  /** Coalesce bursts of live events into one reload. */
  refreshSoon(delay = 350): void {
    if (this.#patchTimer) {
      clearTimeout(this.#patchTimer);
      this.#patchTimer = null;
      this.#patchIds.clear();
    }
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.load();
    }, delay);
  }

  /**
   * Resolve the stories the library points at: the current workspace's story
   * list first (one request, includes epics + children), then the few ids from
   * other workspaces one by one. Product may not be granted — then chips just
   * stay unlabeled.
   */
  private async resolveStories(my: number): Promise<void> {
    const wanted = new Set<string>();
    for (const h of this.hits) for (const s of h.story_ids) wanted.add(s);
    for (const p of this.projects) if (p.epic_story_id) wanted.add(p.epic_story_id);
    if (!wanted.size || !auth.can('product', 'view')) return;
    const next: Record<string, StoryRef> = { ...this.stories };
    const wsId = ws.currentId;
    if (wsId && !this.tried.has(`ws:${wsId}`)) {
      this.tried.add(`ws:${wsId}`);
      try {
        const list = await api.get<StoryRef[]>(`/workspaces/${wsId}/product/stories`);
        for (const s of list) next[s.id] = s;
      } catch {
        /* no product access in this workspace */
      }
    }
    const missing = [...wanted].filter((id) => !next[id] && !this.tried.has(id)).slice(0, 20);
    await Promise.all(
      missing.map(async (id) => {
        this.tried.add(id);
        try {
          const d = await api.get<{ story: StoryRef }>(`/product/stories/${encodeURIComponent(id)}`);
          if (d.story) next[id] = d.story;
        } catch {
          /* gone or not visible */
        }
      }),
    );
    // Parents of resolved children (the epic strip needs the epic row).
    const parents = Object.values(next)
      .map((s) => s.parent_id)
      .filter((p): p is string => !!p && !next[p] && !this.tried.has(p))
      .slice(0, 20);
    await Promise.all(
      parents.map(async (id) => {
        this.tried.add(id);
        try {
          const d = await api.get<{ story: StoryRef }>(`/product/stories/${encodeURIComponent(id)}`);
          if (d.story) next[id] = d.story;
        } catch {
          /* not visible */
        }
      }),
    );
    if (my === this.seq || this.loaded) this.stories = next;
  }
}

export const library = new DesignLibrary();
