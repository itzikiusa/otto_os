// Header ↔ source switching (`Alt+O`) for C/C++ and Unreal Engine files.
// Prefers live `clangd` (`textDocument/switchSourceHeader`), falling back to
// workspace tree entries + candidate directory probes on disk.

import { api } from '../../../lib/api/client';
import { toasts } from '../../../lib/toast.svelte';
import {
  decodeSwitchSourceHeaderUri,
  headerSourceCandidates,
  isCppHeaderOrSourcePath,
  matchHeaderSourceFromList,
} from './ideHeaderSwitch';
import type { IdeEditorHandle, IdeFsEntry, IdeOpenTab } from '../store/ideTypes';

export interface IdeHeaderSwitchHost {
  activePath: string | null;
  tabs: IdeOpenTab[];
  childrenByDir: Record<string, IdeFsEntry[]>;
  resolvePath(path: string): string;
  editorHandleForPath(path: string): IdeEditorHandle | null;
  targetQuery(extra?: Record<string, string | boolean | null | undefined>): string;
  openFile(path: string, opts?: { preview?: boolean }): Promise<void>;
}

export async function runSwitchHeaderSource(host: IdeHeaderSwitchHost, explicitPath?: string): Promise<boolean> {
  const rawPath = explicitPath ?? host.activePath;
  if (!rawPath) return false;
  const active = host.resolvePath(rawPath);
  if (!isCppHeaderOrSourcePath(active)) {
    toasts.info('Switch header / source', 'Available for C and C++ header/source files.');
    return false;
  }

  // 1. Ask live clangd via `textDocument/switchSourceHeader`.
  const lspUri = await host.editorHandleForPath(active)?.lspSwitchSourceHeader?.().catch(() => null);
  const lspPath = decodeSwitchSourceHeaderUri(lspUri);
  if (lspPath && lspPath !== active) {
    await host.openFile(lspPath, { preview: false });
    return true;
  }

  // 2. Match against already-loaded tree entries and open tabs.
  const knownPaths: string[] = [];
  for (const t of host.tabs) knownPaths.push(t.path);
  for (const entries of Object.values(host.childrenByDir)) {
    for (const e of entries) {
      if (e.kind === 'file') knownPaths.push(e.path);
    }
  }
  const treeHit = matchHeaderSourceFromList(active, knownPaths);
  if (treeHit) {
    await host.openFile(treeHit, { preview: false });
    return true;
  }

  // 3. Probe the unique candidate directories on disk via `/ide/fs/tree`.
  const candidates = headerSourceCandidates(active);
  const candidateDirs: string[] = [];
  for (const c of candidates) {
    const slash = c.lastIndexOf('/');
    const d = slash > 0 ? c.slice(0, slash) : '/';
    if (!candidateDirs.includes(d)) candidateDirs.push(d);
    if (candidateDirs.length >= 6) break;
  }
  const discovered: string[] = [];
  for (const dir of candidateDirs) {
    try {
      const q = host.targetQuery({ path: dir });
      const resp = await api.get<{ entries?: IdeFsEntry[] }>(`/ide/fs/tree?${q}`);
      for (const e of resp.entries ?? []) {
        if (e.kind === 'file') discovered.push(e.path);
      }
    } catch {
      /* directory does not exist — try next candidate */
    }
  }
  const diskHit = matchHeaderSourceFromList(active, discovered);
  if (diskHit) {
    await host.openFile(diskHit, { preview: false });
    return true;
  }

  toasts.info('No counterpart file found', 'No matching header or source file was found.');
  return false;
}
