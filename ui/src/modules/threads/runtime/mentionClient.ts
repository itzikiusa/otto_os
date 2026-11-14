// Fast client for unified `@` file & folder completion across Threads and IDE.
// Primary endpoint: `POST /ide/fs/complete` (<15ms path/alias/fuzzy engine).
// Graceful fallback: `POST /ide/search` (`mode: 'files'`) + `GET /ide/fs/tree`
// when running against a daemon without `/ide/fs/complete`.

import { api, ApiError } from '../../../lib/api/client';
import { ideProjects } from '../ideProjects.svelte';
import {
  computeRelativeDisplay,
  rankFileMentionItems,
  type RankedFileMention,
  type RawFsCompletionItem,
} from './mentionMatcher';

interface FsCompleteResp {
  items?: RawFsCompletionItem[];
  base_dir?: string;
  elapsed_ms?: number;
}

interface IdeSearchFallbackResp {
  items?: Array<{ path: string; line?: number; preview?: string }>;
}

interface IdeTreeFallbackResp {
  root?: string;
  entries?: Array<{ name: string; path: string; kind: 'file' | 'dir' | 'symlink' }>;
}

let fsCompleteUnsupported = false;

export function instantOpenTabMentions(
  query: string,
  cwd?: string | null,
): RankedFileMention[] {
  const ctx = ideProjects.collectMentionWorkspace(cwd);
  if (ctx.openTabs.length === 0) return [];
  const raw: RawFsCompletionItem[] = ctx.openTabs.map((t) => {
    const rel = computeRelativeDisplay(t.path, ctx.effectiveCwd, ctx.roots);
    return {
      path: t.path,
      name: t.name,
      display: rel.display,
      insert_token: rel.insertToken,
      kind: 'file',
      is_dir: false,
      openTab: true,
      activeTab: t.active,
    };
  });
  return rankFileMentionItems(raw, query, {
    cwd: ctx.effectiveCwd,
    roots: ctx.roots,
    limit: 20,
  });
}

async function fetchFallbackItems(
  query: string,
  effectiveCwd: string,
  roots: readonly string[],
  aliases: Readonly<Record<string, string>>,
  signal?: AbortSignal,
): Promise<RawFsCompletionItem[]> {
  const q = query.trim();
  const targetRoots = roots.length > 0 ? roots.slice(0, 8) : effectiveCwd ? [effectiveCwd] : [];
  if (targetRoots.length === 0 && !q.startsWith('/')) return [];

  // Resolve `@<alias>/...` or directory navigation (`@dir/` or empty `@`).
  const slashIdx = q.lastIndexOf('/');
  const aliasMatch = /^([A-Za-z0-9_.-]+)\/(.*)$/.exec(q);
  const aliasRoot = aliasMatch ? aliases[aliasMatch[1]] : undefined;

  if (!q || q.endsWith('/') || aliasRoot) {
    const baseRoot = aliasRoot ?? targetRoots[0] ?? '';
    const subRel = aliasMatch ? aliasMatch[2] : q;
    const dirPart = slashIdx >= 0 ? subRel.slice(0, subRel.lastIndexOf('/')) : '';
    const dirPath = dirPart
      ? baseRoot
        ? `${baseRoot.replace(/\/+$/, '')}/${dirPart.replace(/^\/+/, '')}`
        : dirPart
      : baseRoot;
    if (!dirPath) return [];
    const qs = new URLSearchParams({
      kind: 'local',
      root: baseRoot || dirPath,
      path: dirPath === baseRoot ? '' : dirPath,
      show_hidden: 'false',
    });
    const tree = await api.get<IdeTreeFallbackResp>(`/ide/fs/tree?${qs.toString()}`, signal);
    return (tree.entries ?? []).map((e) => {
      const isDir = e.kind === 'dir';
      const rel = aliasMatch
        ? `${aliasMatch[1]}/${dirPart ? `${dirPart}/` : ''}${e.name}`
        : computeRelativeDisplay(e.path, effectiveCwd, roots).display;
      return {
        path: e.path,
        name: e.name,
        display: isDir ? `${rel}/` : rel,
        insert_token: isDir ? `${rel}/` : rel,
        kind: isDir ? 'dir' : 'file',
        is_dir: isDir,
      };
    });
  }

  // Fuzzy file search across roots via `POST /ide/search`.
  const primaryRoot = targetRoots[0];
  const additional = targetRoots.slice(1);
  const resp = await api.post<IdeSearchFallbackResp>(
    '/ide/search',
    {
      target: {
        kind: 'local',
        root: primaryRoot,
        ...(additional.length > 0 ? { additional_roots: additional } : {}),
      },
      query: q,
      mode: 'files',
      limit: 30,
    },
    signal,
  );
  return (resp.items ?? []).map((it) => {
    const rel = computeRelativeDisplay(it.path, effectiveCwd, roots);
    return {
      path: it.path,
      display: rel.display,
      insert_token: rel.insertToken,
      kind: 'file',
      is_dir: false,
    };
  });
}

export async function fetchMentionFileItems(opts: {
  query: string;
  cwd?: string | null;
  signal?: AbortSignal;
}): Promise<RankedFileMention[]> {
  const ctx = ideProjects.collectMentionWorkspace(opts.cwd);
  const tabRaw: RawFsCompletionItem[] = ctx.openTabs.map((t) => {
    const rel = computeRelativeDisplay(t.path, ctx.effectiveCwd, ctx.roots);
    return {
      path: t.path,
      name: t.name,
      display: rel.display,
      insert_token: rel.insertToken,
      kind: 'file',
      is_dir: false,
      openTab: true,
      activeTab: t.active,
    };
  });

  let serverItems: RawFsCompletionItem[] = [];
  if (!fsCompleteUnsupported) {
    try {
      const resp = await api.post<FsCompleteResp>(
        '/ide/fs/complete',
        {
          query: opts.query,
          cwd: ctx.effectiveCwd || null,
          roots: ctx.roots,
          aliases: ctx.aliases,
          limit: 40,
        },
        opts.signal,
      );
      serverItems = resp.items ?? [];
    } catch (err) {
      if (opts.signal?.aborted) return [];
      if (err instanceof ApiError && (err.status === 404 || err.status === 405 || err.status === 501)) {
        fsCompleteUnsupported = true;
      } else {
        // Fall through to `/ide/search` / `/ide/fs/tree` fallback on transient error.
      }
    }
  }

  if (serverItems.length === 0 && (fsCompleteUnsupported || !opts.signal?.aborted)) {
    try {
      serverItems = await fetchFallbackItems(
        opts.query,
        ctx.effectiveCwd,
        ctx.roots,
        ctx.aliases,
        opts.signal,
      );
    } catch {
      // Quietly keep open-tab completions if daemon search fails.
    }
  }

  if (opts.signal?.aborted) return [];
  return rankFileMentionItems([...tabRaw, ...serverItems], opts.query, {
    cwd: ctx.effectiveCwd,
    roots: ctx.roots,
    limit: 35,
  });
}
