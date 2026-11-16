// Private secret requests (`otto.request_secret`, T3 Code #15907): a thread's
// agent asks its owner for a token or password. The card lives here by thread
// id; the value is typed into the card and posted straight to the daemon — it
// is never kept in a store, a draft or the timeline.
import { api } from '../api/client';
import type { Id, OttoEvent } from '../api/types';

export interface SecretRequest {
  request_id: Id;
  session_id: Id;
  prompt: string;
  name?: string | null;
}

class SecretRequests {
  /** Pending requests by thread (one at a time per thread: the newest). */
  bySession: Record<Id, SecretRequest> = $state({});
  #loaded = false;

  /** Pending requests survive a reload: read them once. */
  async load(): Promise<void> {
    if (this.#loaded) return;
    this.#loaded = true;
    try {
      const r = await api.bg.get<{ items: SecretRequest[] }>('/secret-requests');
      for (const it of r.items) this.bySession[it.session_id] = it;
    } catch {
      this.#loaded = false;
    }
  }

  applyEvent(ev: Extract<OttoEvent, { type: 'secret_request_updated' }>): void {
    if (ev.pending) {
      this.bySession[ev.session_id] = { request_id: ev.request_id, session_id: ev.session_id, prompt: ev.prompt ?? '', name: ev.name };
    } else if (this.bySession[ev.session_id]?.request_id === ev.request_id) {
      delete this.bySession[ev.session_id];
    }
  }

  /** Answer with `value`, or decline (`null`). */
  async answer(req: SecretRequest, value: string | null): Promise<void> {
    await api.post(`/secret-requests/${encodeURIComponent(req.request_id)}`, value === null ? { decline: true } : { value });
    if (this.bySession[req.session_id]?.request_id === req.request_id) delete this.bySession[req.session_id];
  }
}

export const secretRequests = new SecretRequests();
