import type { RuntimeDraftContent, RuntimeDraftRecord } from '../api/types';

interface ComposerContent {
  text: string;
  images: RuntimeDraftContent['images'];
  context: RuntimeDraftContent['context'];
  model: RuntimeDraftContent['model'];
  effort: RuntimeDraftContent['effort'];
  pickedMode: RuntimeDraftContent['access_mode'];
  freshContext: boolean;
  provider?: string | null;
}

/** Accessors retain Svelte tracking while exposing the wire format to the writer. */
export function composerContent(d: ComposerContent): RuntimeDraftContent {
  return {
    get text() { return d.text; }, set text(v) { d.text = v; },
    get images() { return d.images; }, set images(v) { d.images = v; },
    get model() { return d.model; }, set model(v) { d.model = v; },
    get model_default() { return d.model === null; },
    set model_default(v) {
      if (v === true) d.model = null;
      else if (v === false && d.model === null) d.model = undefined;
    },
    get effort() { return d.effort; }, set effort(v) { d.effort = v; },
    get access_mode() { return d.pickedMode; }, set access_mode(v) { d.pickedMode = v; },
    get fresh_context() { return d.freshContext; }, set fresh_context(v) { d.freshContext = v; },
    get provider() { return d.provider ?? null; }, set provider(v) { d.provider = v ?? null; },
    get context() { return d.context; }, set context(v) { d.context = v; },
  };
}
export function snapshotDraft(d: ComposerContent): RuntimeDraftContent {
  const cloned = JSON.parse(JSON.stringify(composerContent(d))) as RuntimeDraftContent;
  if (d.model === undefined) cloned.model = undefined;
  return cloned;
}
const acknowledged = new WeakSet<RuntimeDraftContent>();
/** Clear what `sent` carried from the composer, once per snapshot: a send
 *  acknowledges at submit and again when the daemon answers, and the second
 *  pass must not strip an identical image pasted while the request ran. */
export function acknowledgeComposer(d: ComposerContent, sent: RuntimeDraftContent): void {
  if (acknowledged.has(sent)) return;
  acknowledged.add(sent);
  acknowledgeDraft(composerContent(d), sent);
}

export function emptyDraft(): RuntimeDraftContent {
  return {
    text: '',
    images: [],
    model: undefined,
    model_default: false,
    effort: null,
    access_mode: null,
    fresh_context: false,
    provider: null,
    context: [],
  };
}
function normalizeRemoteDraftModel(draft: RuntimeDraftContent): string | null | undefined {
  if (typeof draft.model === 'string' && draft.model !== '') return draft.model;
  if (draft.model_default === true) return null;
  return undefined;
}
function isEmptyDraftContent(d: RuntimeDraftContent | null | undefined): boolean {
  return (
    !d ||
    (!d.text &&
      d.images.length === 0 &&
      d.context.length === 0 &&
      !d.fresh_context &&
      !d.provider &&
      !d.model &&
      !d.model_default &&
      !d.effort &&
      !d.access_mode)
  );
}
const DRAFT_KEYS: readonly (keyof RuntimeDraftContent)[] = [
  'text',
  'images',
  'model',
  'effort',
  'access_mode',
  'fresh_context',
  'provider',
  'context',
];
const copy = <T>(v: T): T => {
  if (v === undefined) return undefined as T;
  const cloned = JSON.parse(JSON.stringify(v)) as T;
  if (v && typeof v === 'object' && 'model' in (v as Record<string, unknown>) && (v as Record<string, unknown>).model === undefined) {
    (cloned as Record<string, unknown>).model = undefined;
  }
  return cloned;
};
const equal = (a: unknown, b: unknown): boolean =>
  a === b || (a !== undefined && b !== undefined && JSON.stringify(a) === JSON.stringify(b));

/** Only acknowledged content is removed; edits made during the request survive. */
export function acknowledgeDraft(current: RuntimeDraftContent, sent: RuntimeDraftContent): void {
  if (current.text === sent.text) current.text = '';
  const remaining = [...sent.images];
  current.images = current.images.filter(image => {
    const index = remaining.findIndex(sentImage => equal(sentImage, image));
    if (index === -1) return true;
    remaining.splice(index, 1);
    return false;
  });
  current.context = current.context.filter(x => !sent.context.some(s => equal(s, x)));
  if (current.fresh_context === sent.fresh_context) current.fresh_context = false;
  if (sent.provider && current.provider === sent.provider) current.provider = null;
}
/** The daemon refuses a stored draft over 24 MiB of JSON (otto-state
 *  agent_runtime/drafts.rs) and its route body over 24 MiB (otto-agent-protocol
 *  http.rs): a bigger one is never sent. */
export const MAX_DRAFT_JSON_BYTES = 24 * 1024 * 1024;
const TOO_LARGE = 'Draft too large to save (24 MB limit). Remove an attachment.';

/** A save the daemon refused for its content (413 size, 400 invalid): the
 *  person-facing reason, or null for a failure worth retrying. */
export function refusedSaveText(e: unknown): string | null {
  const status = (e as { status?: number } | null)?.status;
  if (status !== 413 && status !== 400) return null;
  const msg = e instanceof Error ? e.message.trim() : '';
  // axum's own body-limit rejection has no useful words for a person.
  if (status === 413 && (!msg || /length limit|body/i.test(msg))) return TOO_LARGE;
  return msg || 'The daemon refused this draft.';
}

export interface DraftTransport {
  load(): Promise<RuntimeDraftRecord>;
  save(draft: RuntimeDraftContent, revision: number): Promise<RuntimeDraftRecord>;
}
export type DraftSaveStatus = 'loading' | 'saving' | 'saved' | 'error';

/** One serialized writer per draft. The ownership predicate fences account changes. */
export class DraftPersistence {
  private revision = 0;
  private loaded = false;
  private loading: Promise<void> | null = null;
  private syncing: Promise<void> | null = null;
  private writing: Promise<void> | null = null;
  private inSaveCall = false;
  private saved = '';
  private conflicted = false;
  /** Length of a draft the daemon refused (or that is over the limit): no
   *  automatic save again until the draft changes (400) or shrinks (413). */
  private refused: { json: number; shrinkOnly: boolean; at: string; msg: string } | null = null;
  private baseline: RuntimeDraftContent;
  private current: RuntimeDraftContent;
  private transport: DraftTransport;
  private owns: () => boolean;
  private status: (status: DraftSaveStatus, message?: string) => void;
  constructor(current: RuntimeDraftContent, transport: DraftTransport, owns: () => boolean,
    status: (status: DraftSaveStatus, message?: string) => void = () => {}) {
    this.baseline = copy(current);
    this.current = current; this.transport = transport; this.owns = owns; this.status = status;
  }
  load(): Promise<void> {
    if (this.loading) return this.loading;
    const before = this.baseline;
    this.status('loading');
    this.loading = this.transport.load().then(record => {
      if (!this.owns()) return;
      this.revision = record.revision;
      if (record.draft) {
        const addedContext = this.current.context;
        const addedImages = this.current.images;
        const remoteModel = normalizeRemoteDraftModel(record.draft);
        for (const key of DRAFT_KEYS) {
          if (!equal(this.current[key], before[key])) continue;
          if (key === 'model') {
            if (!equal(this.current.model_default ?? (this.current.model === null), before.model_default ?? (before.model === null))) {
              continue;
            }
            this.current.model = remoteModel;
            this.current.model_default = remoteModel === null;
            continue;
          }
          Object.assign(this.current, { [key]: record.draft[key] });
        }
        if (!equal(addedContext, before.context)) {
          this.current.context = [...record.draft.context.filter(x => !addedContext.some(c => c.id === x.id)), ...addedContext];
        }
        if (!equal(addedImages, before.images)) {
          this.current.images = [...record.draft.images.filter(x => !addedImages.some(c => equal(c, x))), ...addedImages];
        }
      }
      this.loaded = true;
      this.saved = JSON.stringify(this.current);
      this.status('saved');
    }).catch(e => { if (this.owns()) this.status('error', String(e)); }).finally(() => { this.loading = null; });
    return this.loading;
  }
  /** Refresh `revision` on an idle, clean composer so another surface sending/clearing
   *  the draft never causes a spurious `409` on the next keystroke or image paste. */
  syncIfClean(): Promise<void> {
    if (
      !this.owns() ||
      !this.loaded ||
      this.loading ||
      this.inSaveCall ||
      this.conflicted ||
      JSON.stringify(this.current) !== this.saved
    ) {
      return Promise.resolve();
    }
    if (this.syncing) return this.syncing;
    this.syncing = this.transport
      .load()
      .then(record => {
        if (!this.owns() || this.inSaveCall || this.conflicted) return;
        const remoteDraft = record.draft ?? emptyDraft();
        const remoteJson = JSON.stringify(remoteDraft);
        if (JSON.stringify(this.current) === this.saved) {
          this.revision = record.revision;
          const remoteModel = normalizeRemoteDraftModel(remoteDraft);
          for (const key of DRAFT_KEYS) {
            if (key === 'model') {
              this.current.model = remoteModel;
              this.current.model_default = remoteModel === null;
              continue;
            }
            Object.assign(this.current, { [key]: copy(remoteDraft[key]) });
          }
          this.saved = JSON.stringify(this.current);
          this.status('saved');
        } else if (isEmptyDraftContent(record.draft)) {
          this.revision = record.revision;
          this.saved = remoteJson;
        }
      })
      .catch(() => {
        /* background sync: keep the clean composer undisturbed */
      })
      .finally(() => {
        this.syncing = null;
      });
    return this.syncing;
  }
  save(): Promise<void> {
    if (!this.owns() || this.conflicted) return Promise.resolve();
    if (this.writing) return this.writing;
    this.writing = this.drain().finally(() => { this.writing = null; });
    return this.writing;
  }
  private async drain(): Promise<void> {
    if (this.syncing) await this.syncing;
    if (!this.loaded) await this.load();
    if (!this.loaded || !this.owns()) return;
    while (this.owns()) {
      const currentJson = JSON.stringify(this.current);
      if (currentJson === this.saved) break;
      const r = this.refused;
      if (r && (r.shrinkOnly ? currentJson.length >= r.json : currentJson === r.at)) {
        this.status('error', r.msg);
        return;
      }
      this.refused = null;
      if (currentJson.length > MAX_DRAFT_JSON_BYTES) {
        this.refused = { json: currentJson.length, shrinkOnly: true, at: '', msg: TOO_LARGE };
        this.status('error', TOO_LARGE);
        return;
      }
      const snapshot = JSON.parse(currentJson) as RuntimeDraftContent;
      this.status('saving');
      this.inSaveCall = true;
      try {
        const record = await this.transport.save(snapshot, this.revision);
        if (!this.owns()) return;
        this.revision = record.revision;
        this.saved = currentJson;
      } catch (e) {
        if (!this.owns()) return;
        const refusal = refusedSaveText(e);
        if (refusal) {
          const tooLarge = (e as { status?: number }).status === 413;
          this.refused = { json: currentJson.length, shrinkOnly: tooLarge, at: tooLarge ? '' : currentJson, msg: refusal };
          this.status('error', refusal);
          return;
        }
        this.conflicted = (e as { status?: number }).status === 409;
        this.status('error', this.conflicted ? 'This draft changed in another window. Retry to save this version.' : 'Draft not saved. Retry when connected.');
        return;
      } finally {
        this.inSaveCall = false;
      }
    }
    if (this.owns()) this.status('saved');
  }
  /** Explicit Retry keeps local work, refreshing only the revision after a conflict. */
  async retry(): Promise<void> {
    if (!this.owns()) return;
    this.refused = null;
    if (this.conflicted) {
      try {
        const record = await this.transport.load();
        if (!this.owns()) return;
        this.revision = record.revision; this.conflicted = false;
        this.saved = JSON.stringify(record.draft ?? emptyDraft());
      } catch { this.status('error', 'Draft not saved. Retry when connected.'); return; }
    }
    await this.save();
  }
}
