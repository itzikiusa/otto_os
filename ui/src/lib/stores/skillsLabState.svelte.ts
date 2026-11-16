import { skillReviewApi } from '../api/skillReview';
import { skillsEvalApi } from '../api/skillsEval';
import type {
  BundledSkillView,
  LibrarySkill,
  ProviderSkillInfo,
  SkillEval,
  SkillEvalSummary,
  SkillReview,
  StartSkillEvalReq,
} from '../api/types';
import { toasts } from '../toast.svelte';
import { toastError } from '../toastError';

export type SkillsBrowserSource = 'all' | 'library' | 'bundled' | 'claude' | 'codex' | 'agy' | (string & {});
export type SkillsBrowserVariantSource = 'library' | 'bundled' | 'claude' | 'codex' | 'agy' | (string & {});
export type SkillsBrowserDetailTab = 'overview' | 'edit' | 'evals' | 'usage';
export type SkillsEvalMode = 'form' | 'detail';

export interface WorkspaceEvalState {
  runs: SkillEvalSummary[];
  nextCursor: string | null;
  loaded: boolean;
  mode: SkillsEvalMode;
  selectedId: string | null;
  formSkill: { name: string; source: string } | null;
  compareMode: boolean;
  compareSel: Set<string>;
  compareFull: Record<string, SkillEval>;
}

export interface WorkspaceReviewState {
  reviews: SkillReview[];
  loaded: boolean;
  selected: SkillReview | null;
  fSkill: string;
  fMode: 'static' | 'agents';
  fProviders: Set<string> | null;
  fInstructions: string;
}

function emptyEvalState(): WorkspaceEvalState {
  return {
    runs: [],
    nextCursor: null,
    loaded: false,
    mode: 'form',
    selectedId: null,
    formSkill: null,
    compareMode: false,
    compareSel: new Set(),
    compareFull: {},
  };
}

function emptyReviewState(): WorkspaceReviewState {
  return {
    reviews: [],
    loaded: false,
    selected: null,
    fSkill: '',
    fMode: 'agents',
    fProviders: null,
    fInstructions: '',
  };
}

function evalToSummary(e: SkillEval): SkillEvalSummary {
  const { iterations, config: _config, promoted_at: _pa, promoted_by: _pb, ...rest } = e;
  return { ...rest, iteration_count: iterations.length };
}

class SkillsLabState {
  listCollapsed = $state(false);
  startingEval = $state(false);
  startingReview = $state(false);

  browser = {
    library: [] as LibrarySkill[],
    bundled: [] as BundledSkillView[],
    providerSkills: [] as ProviderSkillInfo[],
    bodies: {} as Record<string, string>,
    loaded: false,
    query: '',
    category: null as string | null,
    source: 'all' as SkillsBrowserSource,
    driftedOnly: false,
    selName: null as string | null,
    selSource: 'library' as SkillsBrowserVariantSource,
    detailTab: 'overview' as SkillsBrowserDetailTab,
  };

  private evalByWs = new Map<string, WorkspaceEvalState>();
  private reviewByWs = new Map<string, WorkspaceReviewState>();

  evalFor(wsId: string | null | undefined): WorkspaceEvalState {
    if (!wsId) return emptyEvalState();
    let entry = this.evalByWs.get(wsId);
    if (!entry) {
      entry = emptyEvalState();
      this.evalByWs.set(wsId, entry);
    }
    return entry;
  }

  syncEval(wsId: string | null | undefined, patch: Partial<WorkspaceEvalState>): void {
    if (!wsId) return;
    const cur = this.evalFor(wsId);
    Object.assign(cur, patch);
  }

  async startEval(wsId: string, req: StartSkillEvalReq): Promise<SkillEval | null> {
    if (!wsId || this.startingEval) return null;
    this.startingEval = true;
    try {
      const created = await skillsEvalApi.start(wsId, req);
      const cur = this.evalFor(wsId);
      cur.runs = [evalToSummary(created), ...cur.runs.filter((r) => r.id !== created.id)];
      cur.loaded = true;
      cur.formSkill = null;
      cur.selectedId = created.id;
      cur.mode = 'detail';
      toasts.success('Evaluation started', 'Watch progress in the report.');
      return created;
    } catch (e) {
      toastError("Couldn’t start the evaluation", e);
      return null;
    } finally {
      this.startingEval = false;
    }
  }

  reviewFor(wsId: string | null | undefined): WorkspaceReviewState {
    if (!wsId) return emptyReviewState();
    let entry = this.reviewByWs.get(wsId);
    if (!entry) {
      entry = emptyReviewState();
      this.reviewByWs.set(wsId, entry);
    }
    return entry;
  }

  syncReview(wsId: string | null | undefined, patch: Partial<WorkspaceReviewState>): void {
    if (!wsId) return;
    const cur = this.reviewFor(wsId);
    Object.assign(cur, patch);
  }

  async startReview(
    wsId: string,
    req: {
      skill_name: string;
      skill_source: string;
      providers: string[];
      agent_mode: 'static' | 'agents';
      instructions: string;
    },
  ): Promise<SkillReview | null> {
    if (!wsId || this.startingReview) return null;
    this.startingReview = true;
    try {
      const rev = await skillReviewApi.start(wsId, req);
      const cur = this.reviewFor(wsId);
      cur.selected = rev;
      cur.reviews = [rev, ...cur.reviews.filter((r) => r.id !== rev.id)];
      cur.loaded = true;
      return rev;
    } catch (e) {
      toastError('Couldn’t start the review', e);
      return null;
    } finally {
      this.startingReview = false;
    }
  }

  resetForTests(): void {
    this.listCollapsed = false;
    this.startingEval = false;
    this.startingReview = false;
    this.browser = {
      library: [],
      bundled: [],
      providerSkills: [],
      bodies: {},
      loaded: false,
      query: '',
      category: null,
      source: 'all',
      driftedOnly: false,
      selName: null,
      selSource: 'library',
      detailTab: 'overview',
    };
    this.evalByWs.clear();
    this.reviewByWs.clear();
  }
}

export const skillsLabState = new SkillsLabState();
