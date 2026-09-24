// Design Hall — the library snapshot the lobby, project, studio and story pages
// share: every visible artifact (via `GET /design/search?q=` so each carries its
// `story_ids` + `reference_count`), the projects, and the product stories those
// artifacts implement. Loads are sequence-guarded (a slow response never
// overwrites a newer one) and live events trigger a debounced refresh.

import { api } from '../../lib/api/client';
import * as design from '../../lib/api/design';
import type { DesignArtifact, DesignProject, DesignSearchHit } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import type { StoryRef } from './model';

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

class DesignLibrary {
  hits: DesignSearchHit[] = $state([]);
  projects: DesignProject[] = $state([]);
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
    return this.hits.map((h) => h.artifact);
  }

  hitOf(id: string): DesignSearchHit | undefined {
    return this.hits.find((h) => h.artifact.id === id);
  }

  projectOf(id: string | null | undefined): DesignProject | undefined {
    return id ? this.projects.find((p) => p.id === id) : undefined;
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
        design.search('', { limit: 500 }),
        design.listProjects(),
      ]);
      if (my !== this.seq) return;
      this.hits = hits;
      this.projects = projects;
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

  /** Coalesce bursts of live events into one reload. */
  refreshSoon(delay = 350): void {
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
