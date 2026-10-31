// Workspace `@file` mention typeahead + chip helpers for Composer.svelte.
import type { FsEntry } from '../../../lib/api/types';
import { browseWorkspaceFiles } from './api';

export interface MentionQuery {
  /** Path fragment typed after `@` (may be empty right after typing `@`). */
  query: string;
  /** Index of the `@` character in `text`. */
  start: number;
  /** Caret index where the mention token ends. */
  end: number;
}

export interface MentionEntry {
  name: string;
  relPath: string;
  absPath: string;
  isDir: boolean;
}

const MENTION_AT_CARET_RE = /(?:^|\s)@([\w./_-]*)$/;
const MENTION_CHIP_RE = /(?:^|\s)@([\w./_-]*[\w/_-])(?=\s|$|[.,;:!?])/g;
const IGNORED_DIRS = new Set([
  '.git',
  'node_modules',
  'target',
  'dist',
  'build',
  '.svelte-kit',
  '.next',
  '.turbo',
  '__pycache__',
]);

const CACHE_TTL_MS = 30_000;
const dirCache = new Map<string, { at: number; entries: FsEntry[] }>();

function normRoot(root: string): string {
  return root.replace(/\/+$/, '');
}

function toRel(absPath: string, root: string): string {
  const base = normRoot(root);
  if (!base) return absPath;
  if (absPath === base) return '';
  return absPath.startsWith(`${base}/`) ? absPath.slice(base.length + 1) : absPath;
}

async function listDirCached(dirPath: string, signal?: AbortSignal): Promise<FsEntry[]> {
  const clean = normRoot(dirPath) || '/';
  const hit = dirCache.get(clean);
  if (hit && Date.now() - hit.at < CACHE_TTL_MS) return hit.entries;
  const res = await browseWorkspaceFiles(clean, signal);
  const filtered = (res.entries ?? []).filter((e) => !IGNORED_DIRS.has(e.name) && !e.name.startsWith('.DS_Store'));
  dirCache.set(clean, { at: Date.now(), entries: filtered });
  return filtered;
}

/** Detect an active `@path` mention immediately before `caret`. */
export function detectMentionAtCaret(text: string, caret: number): MentionQuery | null {
  if (caret < 1 || caret > text.length) return null;
  const head = text.slice(0, caret);
  const m = MENTION_AT_CARET_RE.exec(head);
  if (!m) return null;
  const query = m[1] ?? '';
  const atOffset = m[0].lastIndexOf('@');
  return {
    query,
    start: m.index + atOffset,
    end: caret,
  };
}

/** Extract unique `@rel/path` file mentions currently present in `text`. */
export function extractMentionedFiles(text: string): string[] {
  if (!text.includes('@')) return [];
  const seen = new Set<string>();
  const out: string[] = [];
  MENTION_CHIP_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = MENTION_CHIP_RE.exec(text)) !== null) {
    const p = m[1]?.trim();
    if (p && !p.endsWith('/') && !seen.has(p)) {
      seen.add(p);
      out.push(p);
    }
  }
  return out;
}

/** Remove all `@relPath` occurrences from `text`, tidying adjacent spaces. */
export function removeMentionFromDraft(text: string, relPath: string): string {
  const escaped = relPath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp(`(^|\\s)@${escaped}(?=\\s|$|[.,;:!?])`, 'g');
  return text
    .replace(re, (_full, lead: string) => (lead === '\n' ? '\n' : ' '))
    .replace(/ {2,}/g, ' ')
    .replace(/^ +/, '');
}

/** Search workspace files/folders under `rootPath` for `@` typeahead. */
export async function searchWorkspaceFiles(
  rootPath: string,
  query: string,
  signal?: AbortSignal,
  limit = 24,
): Promise<MentionEntry[]> {
  const root = normRoot(rootPath);
  if (!root) return [];

  const cleanQuery = query.replace(/^\/+/, '');
  const slashIdx = cleanQuery.lastIndexOf('/');

  // Scoped folder path (`@ui/src/lib/te`) → browse that exact parent folder.
  if (slashIdx >= 0) {
    const dirRel = cleanQuery.slice(0, slashIdx);
    const leaf = cleanQuery.slice(slashIdx + 1).toLowerCase();
    const targetDir = dirRel ? `${root}/${dirRel}` : root;
    const entries = await listDirCached(targetDir, signal);
    const mapped = entries
      .map((e): MentionEntry => ({
        name: e.name,
        relPath: toRel(e.path, root),
        absPath: e.path,
        isDir: e.is_dir,
      }))
      .filter((e) => !leaf || e.name.toLowerCase().includes(leaf));
    mapped.sort((a, b) => {
      const aStarts = a.name.toLowerCase().startsWith(leaf) ? 0 : 1;
      const bStarts = b.name.toLowerCase().startsWith(leaf) ? 0 : 1;
      if (aStarts !== bStarts) return aStarts - bStarts;
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    return mapped.slice(0, limit);
  }

  // Flat query (`@` or `@comp`) → root entries + shallow top-level folders so
  // common files (`src/...`, `ui/...`) match immediately without typing `/`.
  const rootEntries = await listDirCached(root, signal);
  const pool = new Map<string, MentionEntry>();
  const addEntries = (entries: FsEntry[]) => {
    for (const e of entries) {
      const relPath = toRel(e.path, root);
      if (!relPath || pool.has(relPath)) continue;
      pool.set(relPath, { name: e.name, relPath, absPath: e.path, isDir: e.is_dir });
    }
  };
  addEntries(rootEntries);

  if (cleanQuery.length > 0) {
    const subDirs = rootEntries.filter((e) => e.is_dir).slice(0, 8);
    const nested = await Promise.all(
      subDirs.map((d) => listDirCached(d.path, signal).catch(() => [] as FsEntry[])),
    );
    for (const list of nested) addEntries(list);
  }

  const needle = cleanQuery.toLowerCase();
  const candidates = [...pool.values()].filter(
    (e) => !needle || e.name.toLowerCase().includes(needle) || e.relPath.toLowerCase().includes(needle),
  );
  candidates.sort((a, b) => {
    if (needle) {
      const aNameStart = a.name.toLowerCase().startsWith(needle) ? 0 : 1;
      const bNameStart = b.name.toLowerCase().startsWith(needle) ? 0 : 1;
      if (aNameStart !== bNameStart) return aNameStart - bNameStart;
      const aPathStart = a.relPath.toLowerCase().startsWith(needle) ? 0 : 1;
      const bPathStart = b.relPath.toLowerCase().startsWith(needle) ? 0 : 1;
      if (aPathStart !== bPathStart) return aPathStart - bPathStart;
      if (a.isDir !== b.isDir) return a.isDir ? 1 : -1;
    } else if (a.isDir !== b.isDir) {
      return a.isDir ? -1 : 1;
    }
    return a.relPath.localeCompare(b.relPath);
  });
  return candidates.slice(0, limit);
}
