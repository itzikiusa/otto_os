// Product Story Analysis store — workspace-scoped stories, analyses,
// questions, notes, events, test-cases, and learnings.
// Reads `ws.currentId` only (never mutates it), following the same
// singleton-class + Svelte-5-runes pattern as database.svelte.ts.

import { api } from '../api/client';
import { ws } from './workspace.svelte';
import type { Session, ProductLens } from '../api/types';
import type {
  ProductStory,
  ProductStoryVersion,
  ProductAnalysis,
  ProductAnalysisDetail,
  ProductQuestion,
  ProductNote,
  ProductEvent,
  ProductTestcaseRunDetail,
  ProductLearning,
  ProductStoryDetail,
  InjectBundle,
  ImportStoryReq,
  UpdateStoryReq,
  NewQuestionReq,
  UpdateQuestionReq,
  PostQuestionsReq,
  NewNoteReq,
  UpdateNoteReq,
  NewLearningReq,
  UpdateLearningReq,
  UpdateTestcaseReq,
  PublishTestsReq,
  AnalyzeReq,
  RewriteReq,
  GenerateTestsReq,
  GeneratePlanReq,
  InjectSessionReq,
  ProductTranscript,
  NewDraftReq,
  UpdateDraftReq,
  NewTranscriptReq,
  PublishAsRfcReq,
  PublishAsStoryReq,
  ToSwarmReq,
  ToSwarmResp,
  ProductAttachment,
  UploadAttachmentReq,
  DiscoverReq,
  DiscoverResp,
  DiscoveryRunSummary,
  DiscoveryRunDetail,
  MockupAnnotation,
  RefinementThread,
  RefinementThreadDetail,
  CreateThreadReq,
  RefineTurnResp,
  DiscoveryChat,
  DiscoveryChatDetail,
  CreateDiscoveryChatReq,
  DiscoveryChatTurn,
  DiscoveryAction,
  ApplyResult,
} from '../../modules/product/types';

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

class ProductStore {
  // ── Story list ─────────────────────────────────────────────────────────────
  stories: ProductStory[] = $state([]);
  selectedId: string | null = $state(null);

  // ── Story detail + sub-collections ────────────────────────────────────────
  detail: ProductStoryDetail | null = $state(null);
  versions: ProductStoryVersion[] = $state([]);
  analyses: ProductAnalysis[] = $state([]);
  questions: ProductQuestion[] = $state([]);
  notes: ProductNote[] = $state([]);
  events: ProductEvent[] = $state([]);
  testcaseRuns: ProductTestcaseRunDetail[] = $state([]);
  learnings: ProductLearning[] = $state([]);
  transcripts: ProductTranscript[] = $state([]);

  // ── UI state ───────────────────────────────────────────────────────────────
  view: 'stories' | 'learnings' = $state('stories');
  tab: string = $state('overview');

  // ── Loading flags ──────────────────────────────────────────────────────────
  loadingStories = $state(false);
  loadingDetail = $state(false);
  loadingVersions = $state(false);
  loadingAnalyses = $state(false);
  loadingQuestions = $state(false);
  loadingNotes = $state(false);
  loadingEvents = $state(false);
  loadingTestcases = $state(false);
  loadingLearnings = $state(false);
  loadingTranscripts = $state(false);

  // ── Private helpers ────────────────────────────────────────────────────────

  /** Return the current workspace id, or throw if none is active. */
  private wsId(): string {
    const id = ws.currentId;
    if (!id) throw new Error('No workspace selected');
    return id;
  }

  /** Return the currently selected story id, or throw if none is selected. */
  private storyId(): string {
    const id = this.selectedId;
    if (!id) throw new Error('No story selected');
    return id;
  }

  // ── Stories ────────────────────────────────────────────────────────────────

  async loadStories(): Promise<void> {
    const wsId = this.wsId();
    this.loadingStories = true;
    try {
      this.stories = await api.get<ProductStory[]>(`/workspaces/${wsId}/product/stories`);
    } finally {
      this.loadingStories = false;
    }
  }

  async importStory(req: ImportStoryReq): Promise<ProductStory> {
    const wsId = this.wsId();
    // The endpoint returns a ProductStoryDetail wrapper ({ story, source, counts }),
    // not a bare ProductStory — same shape as the drafts endpoint. Unwrap `.story`
    // so callers get a real id (reading `.id` off the wrapper yields undefined,
    // which made the import dialog throw "No story selected" after a successful import).
    const detail = await api.post<ProductStoryDetail>(`/workspaces/${wsId}/product/stories`, req);
    this.stories = [detail.story, ...this.stories];
    return detail.story;
  }

  async select(id: string): Promise<void> {
    this.selectedId = id;
    await this.loadDetail();
  }

  async loadDetail(): Promise<void> {
    const id = this.storyId();
    this.loadingDetail = true;
    try {
      this.detail = await api.get<ProductStoryDetail>(`/product/stories/${id}`);
    } finally {
      this.loadingDetail = false;
    }
  }

  async updateStory(patch: UpdateStoryReq): Promise<ProductStory> {
    const id = this.storyId();
    const updated = await api.patch<ProductStory>(`/product/stories/${id}`, patch);
    this.stories = this.stories.map((s) => (s.id === id ? updated : s));
    if (this.detail) this.detail = { ...this.detail, story: updated };
    return updated;
  }

  async deleteStory(id: string): Promise<void> {
    await api.del(`/product/stories/${id}`);
    this.stories = this.stories.filter((s) => s.id !== id);
    if (this.selectedId === id) {
      this.selectedId = null;
      this.detail = null;
    }
  }

  async refresh(): Promise<void> {
    const id = this.storyId();
    await api.post(`/product/stories/${id}/refresh`);
    await this.loadDetail();
  }

  // ── Drafts ─────────────────────────────────────────────────────────────────

  async createDraft(title?: string | null): Promise<ProductStory> {
    const wsId = this.wsId();
    const req: NewDraftReq = title ? { title } : {};
    const detail = await api.post<ProductStoryDetail>(
      `/workspaces/${wsId}/product/drafts`,
      req,
    );
    this.stories = [detail.story, ...this.stories];
    await this.select(detail.story.id);
    return detail.story;
  }

  async updateDraft(req: UpdateDraftReq): Promise<void> {
    const id = this.storyId();
    const detail = await api.patch<ProductStoryDetail>(
      `/product/stories/${id}/draft`,
      req,
    );
    this.detail = detail;
    this.stories = this.stories.map((s) =>
      s.id === detail.story.id ? detail.story : s,
    );
  }

  // ── Transcripts ────────────────────────────────────────────────────────────

  async loadTranscripts(): Promise<void> {
    const id = this.storyId();
    this.loadingTranscripts = true;
    try {
      this.transcripts = await api.get<ProductTranscript[]>(
        `/product/stories/${id}/transcripts`,
      );
    } finally {
      this.loadingTranscripts = false;
    }
  }

  async addTranscript(req: NewTranscriptReq): Promise<ProductTranscript> {
    const id = this.storyId();
    const t = await api.post<ProductTranscript>(
      `/product/stories/${id}/transcripts`,
      req,
    );
    this.transcripts = [...this.transcripts, t];
    return t;
  }

  async deleteTranscript(trid: string): Promise<void> {
    await api.del(`/product/transcripts/${trid}`);
    this.transcripts = this.transcripts.filter((t) => t.id !== trid);
  }

  // ── Publish ────────────────────────────────────────────────────────────────

  async publishAsRfc(req: PublishAsRfcReq): Promise<ProductStoryDetail> {
    const id = this.storyId();
    const detail = await api.post<ProductStoryDetail>(
      `/product/stories/${id}/publish-as-rfc`,
      req,
    );
    this.detail = detail;
    await this.loadStories();
    return detail;
  }

  async publishAsStory(req: PublishAsStoryReq): Promise<ProductStoryDetail> {
    const id = this.storyId();
    const detail = await api.post<ProductStoryDetail>(
      `/product/stories/${id}/publish-as-story`,
      req,
    );
    this.detail = detail;
    await this.loadStories();
    return detail;
  }

  // ── Versions ───────────────────────────────────────────────────────────────

  async loadVersions(): Promise<void> {
    const id = this.storyId();
    this.loadingVersions = true;
    try {
      this.versions = await api.get<ProductStoryVersion[]>(`/product/stories/${id}/versions`);
    } finally {
      this.loadingVersions = false;
    }
  }

  async getVersion(vid: string): Promise<ProductStoryVersion> {
    return api.get<ProductStoryVersion>(`/product/versions/${vid}`);
  }

  async publishVersion(vid: string): Promise<void> {
    await api.post(`/product/versions/${vid}/publish`);
  }

  // ── Analyses ───────────────────────────────────────────────────────────────

  async analyze(req: AnalyzeReq): Promise<ProductAnalysis> {
    const wsId = this.wsId();
    const id = this.storyId();
    const analysis = await api.post<ProductAnalysis>(
      `/workspaces/${wsId}/product/stories/${id}/analyze`,
      req,
    );
    this.analyses = [analysis, ...this.analyses];
    return analysis;
  }

  async loadAnalyses(): Promise<void> {
    const id = this.storyId();
    this.loadingAnalyses = true;
    try {
      this.analyses = await api.get<ProductAnalysis[]>(`/product/stories/${id}/analyses`);
    } finally {
      this.loadingAnalyses = false;
    }
  }

  async getAnalysis(aid: string): Promise<ProductAnalysisDetail> {
    return api.get<ProductAnalysisDetail>(`/product/analyses/${aid}`);
  }

  /** Curated analysis-lens catalog for the Configure panel. Workspace-scoped. */
  async loadLenses(): Promise<ProductLens[]> {
    const wsId = this.wsId();
    return api.get<ProductLens[]>(`/workspaces/${wsId}/product/lenses`);
  }

  async retryAgent(analysisId: string, agentId: string): Promise<void> {
    await api.post(`/product/analyses/${analysisId}/agents/${agentId}/retry`);
  }

  async stopAgent(analysisId: string, agentId: string): Promise<void> {
    await api.post(`/product/analyses/${analysisId}/agents/${agentId}/stop`);
  }

  // ── Questions ──────────────────────────────────────────────────────────────

  async loadQuestions(): Promise<void> {
    const id = this.storyId();
    this.loadingQuestions = true;
    try {
      this.questions = await api.get<ProductQuestion[]>(`/product/stories/${id}/questions`);
    } finally {
      this.loadingQuestions = false;
    }
  }

  async addQuestion(req: NewQuestionReq): Promise<ProductQuestion> {
    const id = this.storyId();
    const q = await api.post<ProductQuestion>(`/product/stories/${id}/questions`, req);
    this.questions = [...this.questions, q];
    return q;
  }

  async updateQuestion(qid: string, req: UpdateQuestionReq): Promise<ProductQuestion> {
    const q = await api.patch<ProductQuestion>(`/product/questions/${qid}`, req);
    this.questions = this.questions.map((x) => (x.id === qid ? q : x));
    return q;
  }

  async deleteQuestion(qid: string): Promise<void> {
    await api.del(`/product/questions/${qid}`);
    this.questions = this.questions.filter((q) => q.id !== qid);
  }

  async postQuestions(req: PostQuestionsReq): Promise<void> {
    const id = this.storyId();
    await api.post(`/product/stories/${id}/questions/post`, req);
    await this.loadQuestions();
  }

  // ── Notes ──────────────────────────────────────────────────────────────────

  async loadNotes(): Promise<void> {
    const id = this.storyId();
    this.loadingNotes = true;
    try {
      this.notes = await api.get<ProductNote[]>(`/product/stories/${id}/notes`);
    } finally {
      this.loadingNotes = false;
    }
  }

  async addNote(req: NewNoteReq): Promise<ProductNote> {
    const id = this.storyId();
    const note = await api.post<ProductNote>(`/product/stories/${id}/notes`, req);
    this.notes = [...this.notes, note];
    return note;
  }

  async updateNote(nid: string, body: UpdateNoteReq): Promise<ProductNote> {
    const note = await api.patch<ProductNote>(`/product/notes/${nid}`, body);
    this.notes = this.notes.map((n) => (n.id === nid ? note : n));
    return note;
  }

  async deleteNote(nid: string): Promise<void> {
    await api.del(`/product/notes/${nid}`);
    this.notes = this.notes.filter((n) => n.id !== nid);
  }

  // ── Events ─────────────────────────────────────────────────────────────────

  async loadEvents(section?: string): Promise<void> {
    const id = this.storyId();
    this.loadingEvents = true;
    try {
      const qs = section ? `?section=${encodeURIComponent(section)}` : '';
      this.events = await api.get<ProductEvent[]>(`/product/stories/${id}/events${qs}`);
    } finally {
      this.loadingEvents = false;
    }
  }

  // ── Rewrite ────────────────────────────────────────────────────────────────

  async rewrite(req: RewriteReq): Promise<ProductStoryVersion> {
    const wsId = this.wsId();
    const id = this.storyId();
    const version = await api.post<ProductStoryVersion>(
      `/workspaces/${wsId}/product/stories/${id}/rewrite`,
      req,
    );
    await this.loadDetail();
    return version;
  }

  // ── Plan / Tasks ─────────────────────────────────────────────────────────────

  /** Trigger plan generation (202; PlanTab polls loadVersions for the new plan). */
  async generatePlan(req: GeneratePlanReq): Promise<void> {
    const wsId = this.wsId();
    const id = this.storyId();
    await api.post(`/workspaces/${wsId}/product/stories/${id}/plan/generate`, req);
  }

  /** Persist PO checkbox toggles in place (no new version). */
  async savePlan(body_md: string): Promise<void> {
    const wsId = this.wsId();
    const id = this.storyId();
    await api.post(`/workspaces/${wsId}/product/stories/${id}/plan`, { body_md });
  }

  /**
   * Plan → Swarm: create a swarm project from this story (seeding tasks from its
   * plan) and return the created swarm/project so the caller can navigate to the
   * Kanban board. Refreshes the detail so the linked-project badge appears.
   */
  async sendToSwarm(req: ToSwarmReq = {}): Promise<ToSwarmResp> {
    const id = this.storyId();
    const resp = await api.post<ToSwarmResp>(`/product/stories/${id}/to-swarm`, req);
    await this.loadDetail();
    return resp;
  }

  // ── Attachments ────────────────────────────────────────────────────────────

  async listAttachments(): Promise<ProductAttachment[]> {
    const id = this.storyId();
    return api.get<ProductAttachment[]>(`/product/stories/${id}/attachments`);
  }

  async uploadAttachment(req: UploadAttachmentReq): Promise<ProductAttachment> {
    const id = this.storyId();
    return api.post<ProductAttachment>(`/product/stories/${id}/attachments`, req);
  }

  async deleteAttachment(aid: string): Promise<void> {
    await api.del(`/product/attachments/${aid}`);
  }

  async patchAttachment(
    aid: string,
    patch: { kind?: string; filename?: string },
  ): Promise<ProductAttachment> {
    return api.patch<ProductAttachment>(`/product/attachments/${aid}`, patch);
  }

  // ── Discovery ──────────────────────────────────────────────────────────────

  async discover(req: DiscoverReq = {}): Promise<DiscoverResp> {
    const id = this.storyId();
    return api.post<DiscoverResp>(`/product/stories/${id}/discover`, req);
  }

  async listDiscoveryRuns(): Promise<DiscoveryRunSummary[]> {
    const id = this.storyId();
    return api.get<DiscoveryRunSummary[]>(`/product/stories/${id}/discovery-runs`);
  }

  async getDiscoveryRun(rid: string): Promise<DiscoveryRunDetail> {
    return api.get<DiscoveryRunDetail>(`/product/discovery-runs/${rid}`);
  }

  // ── Refinement threads ─────────────────────────────────────────────────────

  async listRefinementThreads(): Promise<RefinementThread[]> {
    const id = this.storyId();
    return api.get<RefinementThread[]>(`/product/stories/${id}/refinement-threads`);
  }

  async createRefinementThread(req: CreateThreadReq = {}): Promise<RefinementThread> {
    const id = this.storyId();
    return api.post<RefinementThread>(`/product/stories/${id}/refinement-threads`, req);
  }

  async getRefinementThread(tid: string): Promise<RefinementThreadDetail> {
    return api.get<RefinementThreadDetail>(`/product/refinement-threads/${tid}`);
  }

  async sendRefinementMessage(tid: string, body: string): Promise<RefineTurnResp> {
    return api.post<RefineTurnResp>(`/product/refinement-threads/${tid}/messages`, { body });
  }

  async archiveRefinementThread(tid: string): Promise<RefinementThread> {
    return api.post<RefinementThread>(`/product/refinement-threads/${tid}/archive`, {});
  }

  // ── Discovery chats ────────────────────────────────────────────────────────
  // Conversational agent that works from an EMPTY/Untitled draft to help with
  // early discovery & research, proposing Apply-able action cards. Story-scoped
  // mirror of the refinement-thread methods above.

  async listDiscoveryChats(): Promise<DiscoveryChat[]> {
    const id = this.storyId();
    return api.get<DiscoveryChat[]>(`/product/stories/${id}/discovery-chats`);
  }

  async createDiscoveryChat(req: CreateDiscoveryChatReq = {}): Promise<DiscoveryChat> {
    const id = this.storyId();
    return api.post<DiscoveryChat>(`/product/stories/${id}/discovery-chats`, req);
  }

  async getDiscoveryChat(cid: string): Promise<DiscoveryChatDetail> {
    return api.get<DiscoveryChatDetail>(`/product/discovery-chats/${cid}`);
  }

  async sendDiscoveryMessage(cid: string, body: string): Promise<DiscoveryChatTurn> {
    return api.post<DiscoveryChatTurn>(`/product/discovery-chats/${cid}/messages`, { body });
  }

  async archiveDiscoveryChat(cid: string): Promise<DiscoveryChat> {
    return api.post<DiscoveryChat>(`/product/discovery-chats/${cid}/archive`, {});
  }

  async applyDiscoveryAction(cid: string, action: DiscoveryAction): Promise<ApplyResult> {
    return api.post<ApplyResult>(`/product/discovery-chats/${cid}/apply`, { action });
  }

  // ── Mockup Annotations ─────────────────────────────────────────────────────

  async listAnnotations(aid: string): Promise<MockupAnnotation[]> {
    return api.get<MockupAnnotation[]>(`/product/attachments/${aid}/annotations`);
  }

  async addAnnotation(
    aid: string,
    a: { x_pct: number; y_pct: number; body: string },
  ): Promise<MockupAnnotation> {
    return api.post<MockupAnnotation>(`/product/attachments/${aid}/annotations`, a);
  }

  async patchAnnotation(
    id: string,
    patch: { body?: string; resolved?: boolean },
  ): Promise<MockupAnnotation> {
    return api.patch<MockupAnnotation>(`/product/annotations/${id}`, patch);
  }

  async deleteAnnotation(id: string): Promise<void> {
    await api.del(`/product/annotations/${id}`);
  }

  // ── Test cases ─────────────────────────────────────────────────────────────

  async generateTests(req: GenerateTestsReq): Promise<void> {
    const wsId = this.wsId();
    const id = this.storyId();
    await api.post(`/workspaces/${wsId}/product/stories/${id}/testcases/generate`, req);
    await this.loadTestcases();
  }

  async loadTestcases(): Promise<void> {
    const id = this.storyId();
    this.loadingTestcases = true;
    try {
      this.testcaseRuns = await api.get<ProductTestcaseRunDetail[]>(
        `/product/stories/${id}/testcases`,
      );
    } finally {
      this.loadingTestcases = false;
    }
  }

  async updateTestcase(tid: string, req: UpdateTestcaseReq): Promise<void> {
    await api.patch(`/product/testcases/${tid}`, req);
    await this.loadTestcases();
  }

  async bulkApproveTestcases(rid: string, ids: string[]): Promise<{ approved: number }> {
    const result = await api.post<{ approved: number }>(
      `/product/testcase-runs/${rid}/testcases/bulk-approve`,
      { ids },
    );
    await this.loadTestcases();
    return result;
  }

  async reorderTestcases(rid: string, orderedIds: string[]): Promise<void> {
    await api.post(`/product/testcase-runs/${rid}/testcases/reorder`, {
      ordered_ids: orderedIds,
    });
    await this.loadTestcases();
  }

  async approveRun(rid: string): Promise<void> {
    await api.post(`/product/testcase-runs/${rid}/approve`);
    await this.loadTestcases();
  }

  async publishTests(rid: string, req: PublishTestsReq): Promise<void> {
    await api.post(`/product/testcase-runs/${rid}/publish`, req);
    await this.loadTestcases();
  }

  // ── Inject ─────────────────────────────────────────────────────────────────

  async loadInject(): Promise<InjectBundle> {
    const id = this.storyId();
    return api.get<InjectBundle>(`/product/stories/${id}/inject`);
  }

  async injectSession(req: InjectSessionReq): Promise<Session> {
    const wsId = this.wsId();
    const id = this.storyId();
    return api.post<Session>(`/workspaces/${wsId}/product/stories/${id}/inject-session`, req);
  }

  // ── Learnings ──────────────────────────────────────────────────────────────

  async loadLearnings(activeOnly?: boolean): Promise<void> {
    const wsId = this.wsId();
    this.loadingLearnings = true;
    try {
      const qs = activeOnly ? '?active=true' : '';
      this.learnings = await api.get<ProductLearning[]>(
        `/workspaces/${wsId}/product/learnings${qs}`,
      );
    } finally {
      this.loadingLearnings = false;
    }
  }

  async addLearning(req: NewLearningReq): Promise<ProductLearning> {
    const wsId = this.wsId();
    const l = await api.post<ProductLearning>(`/workspaces/${wsId}/product/learnings`, req);
    this.learnings = [l, ...this.learnings];
    return l;
  }

  async updateLearning(lid: string, req: UpdateLearningReq): Promise<ProductLearning> {
    const l = await api.patch<ProductLearning>(`/product/learnings/${lid}`, req);
    this.learnings = this.learnings.map((x) => (x.id === lid ? l : x));
    return l;
  }

  async deleteLearning(lid: string): Promise<void> {
    await api.del(`/product/learnings/${lid}`);
    this.learnings = this.learnings.filter((l) => l.id !== lid);
  }

  async acceptLearning(lid: string): Promise<ProductLearning> {
    const l = await api.post<ProductLearning>(`/product/learnings/${lid}/accept`);
    this.learnings = this.learnings.map((x) => (x.id === lid ? l : x));
    return l;
  }

  // ── WS event integration ───────────────────────────────────────────────────
  // Registered callbacks that the AnalysisTab/RewriteTab/PlanTab/TestCasesTab
  // subscribe to so they can refresh on `product_changed` without waiting for
  // the next 3-second poll tick.
  private sectionListeners: Map<string, Set<(status: string) => void>> = new Map();

  onSectionChange(section: string, cb: (status: string) => void): () => void {
    if (!this.sectionListeners.has(section)) {
      this.sectionListeners.set(section, new Set());
    }
    this.sectionListeners.get(section)!.add(cb);
    return () => {
      this.sectionListeners.get(section)?.delete(cb);
    };
  }

  // `plan_run` subscribers (the Plan tab tiles the live planning sessions).
  private planRunListeners: Set<(sessionIds: string[], interactive: boolean) => void> =
    new Set();

  onPlanRun(cb: (sessionIds: string[], interactive: boolean) => void): () => void {
    this.planRunListeners.add(cb);
    return () => {
      this.planRunListeners.delete(cb);
    };
  }

  /** Dispatch a `plan_run` WS event (only for the selected story). Returns true
   *  when handled (always — the event is product-owned), like {@link applyEvent}. */
  applyPlanRun(ev: import('../api/types').OttoEvent): boolean {
    if (ev.type !== 'plan_run') return false;
    if (ev.story_id !== this.selectedId) return true;
    for (const cb of this.planRunListeners) cb(ev.session_ids, ev.interactive);
    return true;
  }

  applyEvent(ev: import('../api/types').OttoEvent): boolean {
    if (ev.type !== 'product_changed') return false;
    // Only fire if the event is for the currently-selected story.
    if (ev.story_id !== this.selectedId) return true;
    const listeners = this.sectionListeners.get(ev.section);
    if (listeners) {
      for (const cb of listeners) cb(ev.status);
    }
    return true;
  }

  // ── Error helper (convenience for callers) ─────────────────────────────────
  errMsg = errMsg;
}

export const product = new ProductStore();
