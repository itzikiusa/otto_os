// Skills Lab — skill catalog + multi-file editor API.
//
// Combines the runtime-writable Otto library (`/library/skills/*`) with the
// bundled catalog (`/library/bundled*`). Library writes are root-only server-side.

import { api, ApiError, baseUrl, getToken } from './client';
import type {
  BundledSkillContent,
  BundledSkillView,
  CreateLibrarySkillReq,
  LibrarySkill,
  Problem,
  SkillFileContentResp,
  SkillFileEntry,
  WriteSkillFileReq,
} from './types';

export const skillLabApi = {
  // --- library (editable) ---------------------------------------------------
  listLibrary: () => api.get<LibrarySkill[]>('/library/skills'),
  getLibrary: (name: string) => api.get<LibrarySkill>(`/library/skills/${encodeURIComponent(name)}`),
  create: (body: CreateLibrarySkillReq) => api.post<LibrarySkill>('/library/skills', body),
  remove: (name: string) => api.del<void>(`/library/skills/${encodeURIComponent(name)}`),

  listFiles: (name: string) =>
    api.get<SkillFileEntry[]>(`/library/skills/${encodeURIComponent(name)}/files`),
  getFile: (name: string, path: string) =>
    api.get<SkillFileContentResp>(
      `/library/skills/${encodeURIComponent(name)}/file?path=${encodeURIComponent(path)}`,
    ),
  putFile: (name: string, body: WriteSkillFileReq) =>
    api.put<SkillFileEntry[]>(`/library/skills/${encodeURIComponent(name)}/file`, body),
  deleteFile: (name: string, path: string) =>
    api.del<void>(`/library/skills/${encodeURIComponent(name)}/file?path=${encodeURIComponent(path)}`),

  // --- bundled catalog (read-only until installed) --------------------------
  listBundled: () => api.get<BundledSkillView[]>('/library/bundled'),
  getBundled: (name: string) =>
    api.get<BundledSkillContent>(`/library/bundled/${encodeURIComponent(name)}`),
  install: (name: string) =>
    api.post<{ name: string; installed: boolean; backed_up: boolean; backup_path: string | null }>(
      `/library/bundled/${encodeURIComponent(name)}/install`,
    ),

  /** Import a skill package from a `.zip` file (raw body upload). */
  importZip: async (file: File | Blob, nameOverride?: string): Promise<LibrarySkill> => {
    const token = getToken();
    const headers: Record<string, string> = { 'Content-Type': 'application/octet-stream' };
    if (token) headers['Authorization'] = `Bearer ${token}`;
    const q = nameOverride ? `?name=${encodeURIComponent(nameOverride)}` : '';
    const resp = await fetch(`${baseUrl()}/api/v1/library/skills/import${q}`, {
      method: 'POST',
      headers,
      body: file,
    });
    if (!resp.ok) {
      let problem: Problem = { code: 'internal', message: resp.statusText };
      try {
        problem = await resp.json();
      } catch {
        /* non-JSON error body */
      }
      throw new ApiError(resp.status, problem);
    }
    return (await resp.json()) as LibrarySkill;
  },
};
