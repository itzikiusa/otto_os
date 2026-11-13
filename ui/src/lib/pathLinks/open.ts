// Path links, wired: which folders a session's printed paths are tried
// against, asking the daemon (`GET /fs/stat`), and what a click or the link
// menu does with the answer. The terminal (Terminal.svelte) and the chat /
// Threads markdown (agents/conversation/Markdown.svelte) both come here, so a
// path resolves the same way wherever it was printed. Resolution runs on a
// click / menu / hover only — never while output streams.
import { api, baseUrl } from '../api/client';
import type { FsStat, Repo } from '../api/types';
import { copyWithToast } from '../clipboard';
import { ctxMenu, type MenuItem } from '../contextmenu.svelte';
import { isEmbedded, isTauri } from '../desktop';
import { statPath } from '../folderCheck';
import { isLoopbackHost } from '../prefetchPlan';
import { openFile } from '../stores/openfile.svelte';
import { ws } from '../stores/workspace.svelte';
import { toasts } from '../toast.svelte';
import { toastError } from '../toastError';
import { notFoundDetail, orderRoots, resolvePathRef, type PathRef, type PathResolution, type StatAnswer } from './resolve';

/** Where a link was printed: the session (PTY or runtime thread) and the
 *  folder it started in. Anything unknown is simply skipped. */
export interface PathLinkTarget {
  sessionId?: string | null;
  cwd?: string | null;
  workspaceId?: string | null;
}

/** Svelte context key: a host that renders chat markdown without the
 *  conversation view (a Threads runtime thread) hands in `() => PathLinkTarget`
 *  so its file references resolve against THAT thread's folders. */
export const PATH_LINK_CTX = 'pathLinkTarget';

/** A parsed link (a TerminalLink, a chat file-ref): its path may be absent. */
export type PathLinkRef = { path?: string; line?: number; col?: number };

/** How the host shows a resolved FILE (the chat's in-place preview); the
 *  default is the side panel's Files viewer. */
export interface PathLinkOpts {
  openFile?: (path: string, line?: number, col?: number) => void;
}

/** `/fs/stat` as a resolver answer: 404/400 = missing; 403 (a folder only a
 *  session route may read), 409/502 (busy) or offline = no answer. */
async function statViaApi(path: string): Promise<StatAnswer> {
  try {
    const st = await api.get<FsStat>(statPath(path));
    return { kind: st.is_dir ? 'dir' : 'file', path: st.path };
  } catch (e) {
    const status = (e as { status?: unknown } | null)?.status;
    return status === 404 || status === 400 ? 'missing' : 'unknown';
  }
}

// Registered repos per workspace, fetched on demand (only when the session's
// own folders did not have the path) and kept briefly.
const REPO_TTL_MS = 30_000;
const repoCache = new Map<string, { at: number; paths: Promise<string[]> }>();
function workspaceRepoPaths(workspaceId: string): Promise<string[]> {
  const hit = repoCache.get(workspaceId);
  if (hit && Date.now() - hit.at < REPO_TTL_MS) return hit.paths;
  const paths = api
    .get<Repo[]>(`/workspaces/${encodeURIComponent(workspaceId)}/repos`)
    .then((repos) => repos.map((r) => r.path).filter(Boolean))
    .catch(() => [] as string[]);
  repoCache.set(workspaceId, { at: Date.now(), paths });
  return paths;
}

/** The candidate roots, in order: the session's folder and its extra
 *  folders (`meta.extra_dirs`) now; the repo holding that folder, the
 *  workspace's other repos and the workspace folder on demand. */
export function pathLinkRoots(target: PathLinkTarget): { roots: string[]; more: () => Promise<string[]> } {
  const session = ws.getSession(target.sessionId ?? null);
  const cwd = target.cwd || session?.cwd || '';
  const extra = session?.meta?.extra_dirs;
  const extraDirs = Array.isArray(extra) ? extra.filter((d): d is string => typeof d === 'string') : [];
  const roots = orderRoots([cwd], extraDirs);
  const workspaceId = target.workspaceId ?? session?.workspace_id ?? ws.currentId ?? null;
  const more = async (): Promise<string[]> => {
    const wsRoot = ws.workspaces.find((w) => w.id === workspaceId)?.root_path ?? null;
    const repos = workspaceId ? await workspaceRepoPaths(workspaceId) : [];
    // The repo the session's folder sits in first (its git root), longest match.
    const own = repos.filter((r) => cwd === r || cwd.startsWith(`${r.replace(/\/+$/, '')}/`)).sort((a, b) => b.length - a.length);
    return orderRoots(own, repos, [wsRoot]).filter((r) => !roots.includes(r));
  };
  return { roots, more };
}

// One answer per (link, roots) for a few seconds: a hover's lookup serves the
// click that follows it, and a file created a moment later is still found.
const RESOLVE_TTL_MS = 4_000;
const resolved = new Map<string, { at: number; res: Promise<PathResolution> }>();
export function resolvePathLink(link: PathLinkRef, target: PathLinkTarget): Promise<PathResolution> {
  const ref: PathRef = { path: link.path ?? '', line: link.line, col: link.col };
  if (!ref.path) return Promise.resolve({ found: false, path: '', tried: [] });
  const { roots, more } = pathLinkRoots(target);
  const key = `${ref.path}\0${ref.line ?? ''}\0${ref.col ?? ''}\0${target.workspaceId ?? ''}\0${roots.join('\0')}`;
  const hit = resolved.get(key);
  if (hit && Date.now() - hit.at < RESOLVE_TTL_MS) return hit.res;
  if (resolved.size > 64) resolved.clear();
  const res = resolvePathRef(ref, roots, statViaApi, more);
  resolved.set(key, { at: Date.now(), res });
  return res;
}

/** Finder exists only in the desktop app talking to THIS Mac's daemon (paths
 *  are on the daemon host); a browser, a phone or a remote daemon hides it. */
export function canRevealInFinder(): boolean {
  if (!isTauri || isEmbedded) return false;
  try {
    return isLoopbackHost(new URL(baseUrl(), location.href).hostname);
  } catch {
    return false;
  }
}

export async function revealInFinder(path: string): Promise<void> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('plugin:opener|reveal_item_in_dir', { paths: [path] });
  } catch (e) {
    toastError('Couldn’t show it in Finder', e);
  }
}

function notFound(res: { path: string; tried: string[] }): void {
  toasts.push('warn', `Not found: ${res.path}`, notFoundDetail(res), 8000, {
    action: { label: 'Copy path', run: () => void copyWithToast(res.path, 'the path') },
  });
}

/** Show a resolved link: a file in the viewer, a folder in the side panel. */
export function openResolved(res: PathResolution, opts: PathLinkOpts = {}): void {
  if (!res.found) notFound(res);
  else if (res.dir) openFile.openDir(res.path);
  else if (opts.openFile) opts.openFile(res.path, res.line, res.col);
  else openFile.open(res.path, res.line, res.col);
}

/** A plain click: resolve against the session's folders, then open. */
export async function activatePathLink(ref: PathLinkRef, target: PathLinkTarget, opts: PathLinkOpts = {}): Promise<void> {
  openResolved(await resolvePathLink(ref, target), opts);
}

/** The menu rows for a resolved link (exported for tests). `viewer` = the
 *  host has its own file viewer, so "Reveal in side panel" is a distinct
 *  action there (in the terminal, opening a file IS the side panel). */
export function pathLinkMenuItems(res: PathResolution, opts: PathLinkOpts, finder: boolean): MenuItem[] {
  const copy: MenuItem = { label: 'Copy path', icon: 'copy', action: () => void copyWithToast(res.path, 'the path') };
  if (!res.found) {
    return [{ label: `Not found · ${notFoundDetail(res)}`, icon: 'warning', disabled: true, title: res.tried.join('\n') }, copy];
  }
  const items: MenuItem[] = res.dir
    ? [{ label: 'Open in side panel', icon: 'folder', action: () => openFile.openDir(res.path) }]
    : [{ label: opts.openFile ? 'Preview' : 'Open in side panel', icon: opts.openFile ? 'eye' : 'file', action: () => openResolved(res, opts) }];
  if (!res.dir && opts.openFile) items.push({ label: 'Reveal in side panel', icon: 'folder', action: () => openFile.open(res.path, res.line, res.col) });
  if (finder) items.push({ label: 'Show in Finder', icon: 'external', action: () => void revealInFinder(res.path) });
  items.push({ separator: true }, copy);
  return items;
}

/** Right-click on a link: the menu, opened where the click was once the
 *  link is resolved (a local stat takes milliseconds). From the keyboard
 *  (⇧F10 / the context-menu key) it hangs under `anchor`, the link itself. */
export function showPathLinkMenu(e: MouseEvent, ref: PathLinkRef, target: PathLinkTarget, opts: PathLinkOpts = {}, anchor?: Element | null): void {
  e.preventDefault();
  e.stopPropagation();
  const keyboard = e.detail === 0 && e.clientX === 0 && e.clientY === 0;
  const at = { clientX: e.clientX, clientY: e.clientY };
  void resolvePathLink(ref, target).then((res) => {
    const items = pathLinkMenuItems(res, opts, canRevealInFinder());
    if (keyboard && anchor?.isConnected) ctxMenu.showAt(anchor, items);
    else ctxMenu.show(new MouseEvent('contextmenu', at), items);
  });
}

/** Hover text: the printed path at once, the resolved one when known. */
export function hoverPathLink(ref: PathLinkRef, target: PathLinkTarget, apply: (title: string) => void): void {
  if (!ref.path) return;
  apply(ref.path);
  void resolvePathLink(ref, target).then((res) => {
    apply(res.found ? `${res.path}${res.dir ? ' (folder)' : ''} — right-click for more` : `${ref.path} — not found`);
  });
}
