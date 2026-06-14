// Context & Soul API helpers: the Otto-owned library (skills / souls / context
// snippets + global default soul) and the per-workspace context config plus
// on-demand materialization. Library writes are root-only; reads are open to
// any member. See docs/superpowers/specs/2026-06-13-otto-context-provisioning-design.md §6.

import { api } from './client';
import type {
  GlobalSoulResp,
  LibraryContext,
  LibrarySkill,
  LibrarySoul,
  MaterializeResp,
  UpdateWorkspaceContextReq,
  UpsertLibraryEntryReq,
  WorkspaceContextConfig,
} from './types';

export const contextApi = {
  // --- Library: skills ------------------------------------------------------
  listSkills: () => api.get<LibrarySkill[]>('/library/skills'),
  getSkill: (name: string) =>
    api.get<LibrarySkill>(`/library/skills/${encodeURIComponent(name)}`),
  putSkill: (name: string, body: string) =>
    api.put<LibrarySkill>(`/library/skills/${encodeURIComponent(name)}`, {
      body,
    } satisfies UpsertLibraryEntryReq),
  deleteSkill: (name: string) =>
    api.del<void>(`/library/skills/${encodeURIComponent(name)}`),

  // --- Library: souls -------------------------------------------------------
  listSouls: () => api.get<LibrarySoul[]>('/library/souls'),
  getSoul: (name: string) =>
    api.get<LibrarySoul>(`/library/souls/${encodeURIComponent(name)}`),
  putSoul: (name: string, body: string) =>
    api.put<LibrarySoul>(`/library/souls/${encodeURIComponent(name)}`, {
      body,
    } satisfies UpsertLibraryEntryReq),
  deleteSoul: (name: string) =>
    api.del<void>(`/library/souls/${encodeURIComponent(name)}`),

  // --- Library: context snippets --------------------------------------------
  listContext: () => api.get<LibraryContext[]>('/library/context'),
  getContext: (name: string) =>
    api.get<LibraryContext>(`/library/context/${encodeURIComponent(name)}`),
  putContext: (name: string, body: string) =>
    api.put<LibraryContext>(`/library/context/${encodeURIComponent(name)}`, {
      body,
    } satisfies UpsertLibraryEntryReq),
  deleteContext: (name: string) =>
    api.del<void>(`/library/context/${encodeURIComponent(name)}`),

  // --- Global default soul --------------------------------------------------
  getDefaultSoul: () => api.get<GlobalSoulResp>('/library/default-soul'),
  setDefaultSoul: (name: string) =>
    api.put<GlobalSoulResp>('/library/default-soul', { name }),

  // --- Per-workspace context config -----------------------------------------
  getWorkspaceContext: (wsId: string) =>
    api.get<WorkspaceContextConfig>(`/workspaces/${wsId}/context`),
  updateWorkspaceContext: (wsId: string, body: UpdateWorkspaceContextReq) =>
    api.put<WorkspaceContextConfig>(`/workspaces/${wsId}/context`, body),

  // --- Materialize now ------------------------------------------------------
  materialize: (wsId: string, provider?: string) =>
    api.post<MaterializeResp>(
      `/workspaces/${wsId}/context/materialize${
        provider ? `?provider=${encodeURIComponent(provider)}` : ''
      }`,
    ),
};
