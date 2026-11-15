import type { RuntimeDraftContent, RuntimeDraftRecord } from '../api/types';

interface ComposerContent {
  text: string;
  images: RuntimeDraftContent['images'];
  context: RuntimeDraftContent['context'];
  model: RuntimeDraftContent['model'];
  effort: RuntimeDraftContent['effort'];
  pickedMode: RuntimeDraftContent['access_mode'];
  freshContext: boolean;
}

/** Accessors retain Svelte tracking while exposing the wire format to the writer. */
export function composerContent(d: ComposerContent): RuntimeDraftContent {
  return {
    get text() { return d.text; }, set text(v) { d.text = v; },
    get images() { return d.images; }, set images(v) { d.images = v; },
    get model() { return d.model; }, set model(v) { d.model = v; },
    get effort() { return d.effort; }, set effort(v) { d.effort = v; },
    get access_mode() { return d.pickedMode; }, set access_mode(v) { d.pickedMode = v; },
    get fresh_context() { return d.freshContext; }, set fresh_context(v) { d.freshContext = v; },
    get context() { return d.context; }, set context(v) { d.context = v; },
  };
}
export function snapshotDraft(d: ComposerContent): RuntimeDraftContent {
  return JSON.parse(JSON.stringify(composerContent(d))) as RuntimeDraftContent;
}
export function acknowledgeComposer(d: ComposerContent, sent: RuntimeDraftContent): void {
  acknowledgeDraft(composerContent(d), sent);
}

export function emptyDraft(): RuntimeDraftContent {
  return { text: '', images: [], model: null, effort: null, access_mode: null, fresh_context: false, context: [] };
}
function isEmptyDraftContent(d: RuntimeDraftContent | null | undefined): boolean {
  return !d || (!d.text && d.images.length === 0 && d.context.length === 0 && !d.fresh_context);
}
const copy = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;
const equal = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b);

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
        for (const key of Object.keys(before) as (keyof RuntimeDraftContent)[]) {
          if (equal(this.current[key], before[key])) Object.assign(this.current, { [key]: record.draft[key] });
        }
        if (!equal(addedContext, before.context)) {
          this.current.context = [...record.draft.context.filter(x => !addedContext.some(c => c.id === x.id)), ...addedContext];
        }
        if (!equal(addedImages, before.images)) {
          this.current.images = [...record.draft.images.filter(x => !addedImages.some(c => equal(c, x))), ...addedImages];
        }
      }
      this.loaded = true;
      this.saved = JSON.stringify(record.draft ?? emptyDraft());
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
          for (const key of Object.keys(remoteDraft) as (keyof RuntimeDraftContent)[]) {
            Object.assign(this.current, { [key]: copy(remoteDraft[key]) });
          }
          this.saved = remoteJson;
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
