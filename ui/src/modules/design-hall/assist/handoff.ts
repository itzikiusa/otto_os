// The lobby's Generate: create the draft, then hand the brief to Otto — one
// `generate` turn, or a variants run when the person asked for more than one —
// and open the design on its Otto tab (`#/design/a/<id>/otto`) where the turn
// shows live. The draft exists either way: if Otto can't start (no agent CLI,
// busy, no access) the person still lands on it with the brief kept.

import { search } from '../../../lib/api/design';
import type { DesignSearchHit, DesignStudio } from '../../../lib/api/types';
import { createDesign } from '../create';
import { askError, sendAsk } from './run';
import { promptRequest, salientTerms } from './model';

export interface GenerateInput {
  workspaceId: string;
  studio: DesignStudio;
  format: string;
  title: string;
  projectId?: string | null;
  brief: string;
  /** 1 = one generate turn; 2–4 = a variants run. */
  variants: number;
  /** Artifact ids (or `<id>@v<seq>`) offered first as [R1..Rn]. */
  references: string[];
}

export interface GenerateResult {
  artifactId: string;
  /** Why Otto didn't start (the draft was still created), else null. */
  assistError: string | null;
}

export async function generateFromBrief(input: GenerateInput): Promise<GenerateResult> {
  const artifactId = await createDesign({
    workspaceId: input.workspaceId,
    studio: input.studio,
    format: input.format,
    title: input.title,
    projectId: input.projectId ?? null,
    brief: input.brief,
  });
  try {
    await sendAsk(artifactId, promptRequest(input.brief, { references: input.references }, input.variants, 'generate'));
    return { artifactId, assistError: null };
  } catch (e) {
    return { artifactId, assistError: askError(e) };
  }
}

/**
 * "Use references": the team's best matches for the brief, shipped first —
 * each salient term searched on its own (search ANDs terms), merged by best
 * rank. They go to Otto first, as [R1..Rn].
 */
export async function suggestReferences(
  brief: string,
  workspaceId: string,
  signal?: AbortSignal,
  n = 3,
): Promise<DesignSearchHit[]> {
  const terms = salientTerms(brief, 3);
  if (!terms.length) return [];
  const lists = await Promise.all(
    terms.map((t) => search(t, { workspace_id: workspaceId, limit: 6 }, signal).catch(() => [] as DesignSearchHit[])),
  );
  const best = new Map<string, { hit: DesignSearchHit; rank: number }>();
  for (const list of lists) {
    list.forEach((hit, rank) => {
      if (hit.artifact.status === 'archived') return;
      const cur = best.get(hit.artifact.id);
      if (!cur || rank < cur.rank) best.set(hit.artifact.id, { hit, rank });
    });
  }
  return [...best.values()]
    .sort((a, b) => a.rank - b.rank || b.hit.reference_count - a.hit.reference_count)
    .slice(0, n)
    .map((x) => x.hit);
}

/** The reference string for a hit: the bare id, which the server resolves to
 *  its approved version, else its head (`<id>@v12` would pin one). */
export function referenceOf(hit: DesignSearchHit): string {
  return hit.artifact.id;
}
