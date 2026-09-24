// HTTP surface of the Otto Assistant (docs/contracts/api.md → "Otto Assistant").
// One place for every path so the store and pages never build URLs.
import { api } from './client';
import type {
  AssistantAttachment,
  AssistantDecisionReq,
  AssistantForgetResp,
  AssistantHermesImportResp,
  AssistantHermesPreview,
  AssistantLimitState,
  AssistantMemory,
  AssistantMemoryView,
  AssistantProfileDoc,
  AssistantRouteDecision,
  AssistantRouteReq,
  AssistantRoutingSettings,
  AssistantSendReq,
  AssistantSendResp,
  AssistantTask,
  AssistantTaskAction,
  AssistantThread,
  AssistantTurn,
  CreateAssistantThreadReq,
  UpdateAssistantThreadReq,
} from './types';

const enc = encodeURIComponent;

export const assistantApi = {
  threads: () => api.get<AssistantThread[]>('/assistant/threads'),
  thread: (id: string) => api.get<AssistantThread>(`/assistant/threads/${enc(id)}`),
  createThread: (body: CreateAssistantThreadReq) => api.post<AssistantThread>('/assistant/threads', body),
  updateThread: (id: string, body: UpdateAssistantThreadReq) => api.patch<AssistantThread>(`/assistant/threads/${enc(id)}`, body),
  deleteThread: (id: string) => api.del<{ ok: boolean }>(`/assistant/threads/${enc(id)}`),
  /** Oldest first; `before` pages further back. */
  turns: (id: string, before?: string, limit = 200) =>
    api.get<AssistantTurn[]>(`/assistant/threads/${enc(id)}/turns?limit=${limit}${before ? `&before=${enc(before)}` : ''}`),
  send: (id: string, body: AssistantSendReq) => api.post<AssistantSendResp>(`/assistant/threads/${enc(id)}/turns`, body),
  attach: (id: string, body: { name: string; content_base64: string; mime?: string }) =>
    api.post<AssistantAttachment>(`/assistant/threads/${enc(id)}/attachments`, body),
  /** Pin provider/model for the thread; `provider: null` goes back to the rules. */
  route: (id: string, body: AssistantRouteReq) => api.post<AssistantThread>(`/assistant/threads/${enc(id)}/route`, body),
  routePreview: (text: string, threadId?: string) =>
    api.post<AssistantRouteDecision>('/assistant/route/preview', { text, thread_id: threadId }),

  needsYou: () => api.get<AssistantTask[]>('/assistant/needs-you'),
  tasks: (q: { state?: string; thread_id?: string; limit?: number } = {}) => {
    const p = new URLSearchParams();
    if (q.state) p.set('state', q.state);
    if (q.thread_id) p.set('thread_id', q.thread_id);
    p.set('limit', String(q.limit ?? 200));
    return api.get<AssistantTask[]>(`/assistant/tasks?${p}`);
  },
  task: (id: string) => api.get<AssistantTask>(`/assistant/tasks/${enc(id)}`),
  act: (id: string, action: AssistantTaskAction, body: AssistantDecisionReq = {}) =>
    api.post<AssistantTask>(`/assistant/tasks/${enc(id)}/${action}`, body),

  memory: (q?: string) => api.get<AssistantMemoryView>(`/assistant/memory${q ? `?q=${enc(q)}` : ''}`),
  saveProfile: (content: string, version: string) => api.put<AssistantProfileDoc>('/assistant/memory', { profile: { content, version } }),
  forgetMemory: (id: string) => api.del<{ ok: boolean; undo_token: string }>(`/assistant/memory/${enc(id)}`),
  acceptMemory: (id: string) => api.post<AssistantMemory>(`/assistant/memory/${enc(id)}/accept`, {}),
  undoForget: (undo_token: string) => api.post<AssistantMemory>('/assistant/memory/undo', { undo_token }),
  forget: (query: string, threadId?: string) => api.post<AssistantForgetResp>('/assistant/forget', { query, thread_id: threadId }),
  hermesPreview: () => api.get<AssistantHermesPreview>('/assistant/memory/import/hermes'),
  hermesImport: () => api.post<AssistantHermesImportResp>('/assistant/memory/import/hermes', {}),

  routing: () => api.get<AssistantRoutingSettings>('/assistant/routing'),
  saveRouting: (body: Partial<Omit<AssistantRoutingSettings, 'updated_at'>>) => api.put<AssistantRoutingSettings>('/assistant/routing', body),
  limits: () => api.get<AssistantLimitState[]>('/assistant/limits'),
};
