import type { IdeScannedRepo } from '../../../lib/api/types/ide-intelligence';
import type { IdeFsEntry } from '../store/ideTypes';

export type RepoTech = 'go' | 'java' | 'rust' | 'node' | 'python';

function pathLeaf(p: string): string {
  const clean = p.replace(/\/+$/, '');
  const idx = clean.lastIndexOf('/');
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}

/**
 * Normalize and deduplicate scanned child repositories by canonical `path` so
 * symlink aliases under `~` (e.g. `support_agent -> supprt_agent`) never
 * collide on Svelte's keyed `{#each ... (entry.path)}` block.
 */
export function buildDedupedBulkRepos(repos: readonly IdeScannedRepo[]): {
  entries: IdeFsEntry[];
  techByPath: Record<string, RepoTech[]>;
} {
  const byPath = new Map<string, IdeScannedRepo>();
  for (let i = 0; i < repos.length; i++) {
    const r = repos[i];
    const cleanPath = (r.path || '').trim().replace(/\/+$/, '');
    if (!cleanPath) continue;
    const prev = byPath.get(cleanPath);
    if (!prev) {
      byPath.set(cleanPath, { ...r, path: cleanPath });
      continue;
    }
    // Prefer the entry whose directory name matches the canonical path's leaf
    // (the real folder rather than a sibling symlink alias).
    const leaf = pathLeaf(cleanPath);
    if (r.name === leaf && prev.name !== leaf) {
      byPath.set(cleanPath, { ...r, path: cleanPath });
    }
  }

  const deduped = [...byPath.values()].sort((a, b) => a.name.localeCompare(b.name));
  const techByPath: Record<string, RepoTech[]> = {};
  const entries: IdeFsEntry[] = deduped.map((r) => {
    const detected: RepoTech[] = [];
    const k = (r.kind || '').toLowerCase();
    if (r.has_go_mod || k === 'go') detected.push('go');
    if (r.has_pom_or_gradle || k === 'java' || k === 'maven' || k === 'gradle') detected.push('java');
    if (k === 'rust') detected.push('rust');
    if (k === 'node' || k === 'ts' || k === 'javascript' || k === 'typescript') detected.push('node');
    if (k === 'python') detected.push('python');
    if (detected.length > 0) techByPath[r.path] = detected;
    return { name: r.name, path: r.path, kind: 'dir' as const };
  });
  return { entries, techByPath };
}

export function dedupeFsDirEntries(entries: readonly IdeFsEntry[]): IdeFsEntry[] {
  const byPath = new Map<string, IdeFsEntry>();
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e.kind !== 'dir' || e.name.startsWith('.')) continue;
    const cleanPath = (e.path || '').trim().replace(/\/+$/, '');
    if (!cleanPath) continue;
    const prev = byPath.get(cleanPath);
    if (!prev || (e.name === pathLeaf(cleanPath) && prev.name !== pathLeaf(cleanPath))) {
      byPath.set(cleanPath, { ...e, path: cleanPath });
    }
  }
  return [...byPath.values()].sort((a, b) => a.name.localeCompare(b.name));
}
