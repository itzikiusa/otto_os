// Design assist API — the unified agent turn, variants and learned rules.
// Mirrors docs/contracts/api.md § "Design assist — agent turns, variants,
// learned rules" (crates/otto-server/src/design_assist.rs). The graph routes
// live in lib/api/design.ts; this module only adds the assist surface the Otto
// panel, the variants tray, the lobby hand-off and the learning page share,
// plus the two neighbours they write through: otto-improve's edit flow
// (approve / reject / rollback — human-only) and the workspace setting
// `design_learning`.

import { api } from '../../../lib/api/client';
import { improveApi } from '../../../lib/api/improve';
import type {
  DesignAssistReq,
  DesignAssistTurn,
  DesignLearnedResp,
  DesignLearnExtractResp,
  DesignVariantAcceptResp,
  DesignVariantRun,
  DesignVariantsReq,
  Id,
  ImprovementEdit,
  Memory,
  Workspace,
} from '../../../lib/api/types';

const enc = encodeURIComponent;

// ── Turns ───────────────────────────────────────────────────────────────────

/** Start one agent turn (202 once the session is live). 409 while busy. */
export function startAssist(artifactId: Id, body: DesignAssistReq) {
  return api.post<DesignAssistTurn>(`/design/artifacts/${enc(artifactId)}/assist`, body);
}

/** Recent turns (in memory on the daemon, ≤ 20, newest first). */
export function listTurns(artifactId: Id) {
  return api.get<DesignAssistTurn[]>(`/design/artifacts/${enc(artifactId)}/assist`);
}

// ── Variants ────────────────────────────────────────────────────────────────

export function startVariants(artifactId: Id, body: DesignVariantsReq) {
  return api.post<DesignVariantRun>(`/design/artifacts/${enc(artifactId)}/variants`, body);
}

export function listVariantRuns(artifactId: Id) {
  return api.get<DesignVariantRun[]>(`/design/artifacts/${enc(artifactId)}/variants`);
}

/** Fast-forward main to a variant (or a conflict draft). 409 when main moved
 *  — pass `force` after the person confirmed applying on top. */
export function acceptVariant(artifactId: Id, versionId: Id, force = false) {
  return api.post<DesignVariantAcceptResp>(
    `/design/artifacts/${enc(artifactId)}/variants/${enc(versionId)}/accept`,
    force ? { force: true } : {},
  );
}

// ── Learning ────────────────────────────────────────────────────────────────

export function getLearned(workspaceId: Id) {
  return api.get<DesignLearnedResp>(`/design/learned?workspace_id=${enc(workspaceId)}`);
}

export function extractRules(workspaceId: Id) {
  return api.post<DesignLearnExtractResp>('/design/learned/extract', { workspace_id: workspaceId });
}

/** The self-improvement edit flow. Human-only: every caller confirms first. */
export const ruleEdits = {
  approve: (editId: Id): Promise<ImprovementEdit> => improveApi.approve(enc(editId)),
  reject: (editId: Id): Promise<ImprovementEdit> => improveApi.reject(enc(editId)),
  rollback: (editId: Id): Promise<ImprovementEdit> => improveApi.rollback(enc(editId)),
};

/** Atomic design memories (`otto-memory` collection `design`). */
export function listDesignMemories(workspaceId: Id) {
  return api.get<Memory[]>(`/workspaces/${enc(workspaceId)}/memories?collection=design&limit=200`);
}

/**
 * Set the workspace setting `design_learning` (`"off"` stops proposals;
 * anything else = suggest). The workspaces PATCH replaces the whole settings
 * object, so merge into a FRESH read — a cached copy could revert keys changed
 * since (the same pattern as `ws.saveNotes`). Workspace-admin gated.
 */
export async function setLearningMode(workspaceId: Id, mode: 'suggest' | 'off'): Promise<Workspace> {
  const fresh = await api.get<Workspace>(`/workspaces/${enc(workspaceId)}`);
  const settings = { ...(fresh.settings ?? {}), design_learning: mode };
  return api.patch<Workspace>(`/workspaces/${enc(workspaceId)}`, { settings });
}
