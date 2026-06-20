// SFTP browser store — one live browse session per open SSH connection.
//
// Drives the daemon's `/connections/{id}/sftp/*` routes (which shell out to the
// system `sftp` binary). State is keyed by connection id so a SftpBrowser
// instance can hold its own cwd / entries while other connections keep theirs.
// Remembers the last-used LOCAL download dir in localStorage so repeat
// downloads default sensibly.

import { api } from '../api/client';
import type {
  SftpDownloadReq,
  SftpDownloadResp,
  SftpEntry,
  SftpListResp,
  SftpReadResp,
} from '../api/types';

const LOCAL_DIR_KEY = 'otto_sftp_local_dir';

/** The default local download dir: the daemon user's ~/Downloads. */
function defaultLocalDir(): string {
  return '~/Downloads';
}

/** Per-connection browse state. */
interface SftpState {
  /** Current remote directory (absolute). */
  cwd: string;
  entries: SftpEntry[];
  loading: boolean;
  error: string;
  /** True once an initial list has resolved (so the UI can distinguish
   *  "never loaded" from "loaded but empty"). */
  loaded: boolean;
}

function blankState(): SftpState {
  return { cwd: '', entries: [], loading: false, error: '', loaded: false };
}

class SftpStore {
  /** connection id → state. */
  private states: Record<string, SftpState> = $state({});

  /** Get (lazily create) the state record for a connection. */
  state(connId: string): SftpState {
    if (!this.states[connId]) {
      this.states = { ...this.states, [connId]: blankState() };
    }
    return this.states[connId];
  }

  /** The remembered local download dir (defaults to ~/Downloads). */
  get localDir(): string {
    return localStorage.getItem(LOCAL_DIR_KEY) ?? defaultLocalDir();
  }
  set localDir(dir: string) {
    if (dir) localStorage.setItem(LOCAL_DIR_KEY, dir);
  }

  /** List a remote path (empty/undefined → the server resolves pwd). */
  async list(connId: string, path?: string): Promise<void> {
    const s = this.state(connId);
    s.loading = true;
    s.error = '';
    try {
      const q = path ? `?path=${encodeURIComponent(path)}` : '';
      const resp = await api.get<SftpListResp>(`/connections/${connId}/sftp/list${q}`);
      s.cwd = resp.path;
      s.entries = resp.entries;
      s.loaded = true;
    } catch (e) {
      s.error = e instanceof Error ? e.message : String(e);
    } finally {
      s.loading = false;
    }
  }

  /** Navigate into a sub-path (or any absolute path). */
  async navigate(connId: string, path: string): Promise<void> {
    await this.list(connId, path);
  }

  /** Go to the parent of the current dir. */
  async up(connId: string): Promise<void> {
    const cwd = this.state(connId).cwd;
    const parent = parentPath(cwd);
    if (parent && parent !== cwd) await this.list(connId, parent);
  }

  /** Re-list the current dir. */
  async refresh(connId: string): Promise<void> {
    const cwd = this.state(connId).cwd;
    await this.list(connId, cwd || undefined);
  }

  /** Join the cwd with a child name into an absolute remote path. */
  childPath(connId: string, name: string): string {
    const cwd = this.state(connId).cwd || '/';
    return cwd.endsWith('/') ? `${cwd}${name}` : `${cwd}/${name}`;
  }

  /** Download a remote file to a chosen local dir (returns the result). */
  async download(connId: string, remotePath: string, localDir: string): Promise<SftpDownloadResp> {
    this.localDir = localDir;
    const body: SftpDownloadReq = { remote_path: remotePath, local_path: localDir };
    return api.post<SftpDownloadResp>(`/connections/${connId}/sftp/download`, body);
  }

  /** Upload a local file into the current remote dir. */
  async upload(connId: string, localPath: string): Promise<void> {
    const name = baseName(localPath);
    const remotePath = this.childPath(connId, name);
    await api.post(`/connections/${connId}/sftp/upload`, {
      local_path: localPath,
      remote_path: remotePath,
    });
  }

  /** Create a directory under the current cwd. */
  async mkdir(connId: string, name: string): Promise<void> {
    await api.post(`/connections/${connId}/sftp/mkdir`, { path: this.childPath(connId, name) });
  }

  /** Remove a remote entry (rmdir for dirs, rm for files). */
  async remove(connId: string, path: string, isDir: boolean): Promise<void> {
    await api.post(`/connections/${connId}/sftp/remove`, { path, dir: isDir });
  }

  /** Rename/move a remote entry within the current dir (new basename). */
  async rename(connId: string, fromPath: string, toName: string): Promise<void> {
    const dir = parentPath(fromPath) || this.state(connId).cwd;
    const to = dir.endsWith('/') ? `${dir}${toName}` : `${dir}/${toName}`;
    await api.post(`/connections/${connId}/sftp/rename`, { from: fromPath, to });
  }

  /** Read a small remote file as text for the in-app viewer. */
  async readText(connId: string, path: string): Promise<SftpReadResp> {
    return api.get<SftpReadResp>(
      `/connections/${connId}/sftp/read?path=${encodeURIComponent(path)}`,
    );
  }

  /** Drop a connection's state (on browser close). */
  reset(connId: string): void {
    const { [connId]: _drop, ...rest } = this.states;
    this.states = rest;
  }
}

/** Parent dir of an absolute POSIX path ("/a/b/c" → "/a/b", "/" → "/"). */
function parentPath(p: string): string {
  if (!p || p === '/') return '/';
  const trimmed = p.replace(/\/+$/, '');
  const idx = trimmed.lastIndexOf('/');
  if (idx <= 0) return '/';
  return trimmed.slice(0, idx);
}

/** Last path component of a local or remote path. */
function baseName(p: string): string {
  const trimmed = p.replace(/[\\/]+$/, '');
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}

export const sftp = new SftpStore();
