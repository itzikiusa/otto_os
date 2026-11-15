// What a runtime thread's composer does that is not view state (kept out of
// RuntimeThread.svelte): the slash commands that act on the view (through
// the host's commands, hostCommands.ts), merging an edited queued message
// into the draft, and reading pasted / attached files into attachments.
import type {
  MessageAttachment,
  PlanArtifact,
  ProviderTurn,
  RuntimeDraftContent,
  TurnItem,
} from '../../../lib/api/types';
import type { RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
import type { ComposerSlashCommand, ParsedSlashIntent } from '../../../lib/slashActions';
import { toastError } from '../../../lib/toastError';
import { startMeasurement } from '../../../lib/telemetry';
import type { BannerBlock } from './banners';
import { runHostSlash } from './hostCommands';
import type { RuntimeProjection } from './projection';
import { errorText, type TimelineRow } from './timeline';

/** Per file (images and PDFs). */
const MAX_ATTACHMENT_BYTES = 10 * 1024 * 1024;
let optimisticSeq = 0;

import { runtimeDrafts, type ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
export type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';

export interface OptimisticMessageState {
  item: TurnItem;
  afterOrdinal: number;
}

/** Read the files `takes` accepts as base64 attachments (oversized ones are
 *  refused with a toast). */
export async function readAttachments(files: Iterable<File>, takes: (f: File) => boolean): Promise<ComposerFile[]> {
  const out: ComposerFile[] = [];
  for (const f of files) {
    if (!takes(f)) continue;
    if (f.size > MAX_ATTACHMENT_BYTES) {
      toastError('File too large', new Error(`${f.name} is over 10 MB`));
      continue;
    }
    const data = new Uint8Array(await f.arrayBuffer());
    let bin = '';
    for (let i = 0; i < data.length; i += 0x8000) bin += String.fromCharCode(...data.subarray(i, i + 0x8000));
    out.push({ name: f.name || 'pasted image', mime: f.type, data_base64: btoa(bin) });
  }
  return out;
}

export async function appendDraftAttachments<TDraft extends { images: ComposerFile[] }>(
  ownerId: string,
  draft: TDraft,
  files: Iterable<File>,
  takes: (f: File) => boolean,
): Promise<void> {
  const read = await readAttachments(files, takes);
  if (read.length && runtimeDrafts.owns(ownerId, draft as never)) {
    draft.images = [...draft.images, ...read];
  }
}

export function pasteDraftAttachments<TDraft extends { images: ComposerFile[] }>(
  e: ClipboardEvent,
  canImages: boolean,
  ownerId: string,
  draft: TDraft,
  takes: (f: File) => boolean,
): void {
  const files = [...(e.clipboardData?.files ?? [])].filter(takes);
  if (!canImages || files.length === 0) return;
  e.preventDefault();
  void appendDraftAttachments(ownerId, draft, files, takes);
}

export function handleSlashMenuKey(
  e: KeyboardEvent,
  suggestions: readonly ComposerSlashCommand[],
  cmdIdx: number,
  setIdx: (idx: number) => void,
  dismiss: () => void,
  accept: (cmd: ComposerSlashCommand) => void,
): boolean {
  if (suggestions.length === 0) return false;
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    setIdx((cmdIdx + (e.key === 'ArrowDown' ? 1 : suggestions.length - 1)) % suggestions.length);
    return true;
  }
  if (e.key === 'Escape') {
    e.preventDefault();
    dismiss();
    return true;
  }
  if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
    e.preventDefault();
    accept(suggestions[cmdIdx]);
    return true;
  }
  return false;
}

/** Resolve the composer's display/selected model: `undefined` inherits the
 *  session's pinned model, whereas `null` is an explicit "Default model" choice. */
export function resolveEffectiveDraftModel(
  draftModel: string | null | undefined,
  sessionModel: string | null | undefined,
): string | null {
  return draftModel === undefined ? (sessionModel ?? null) : draftModel;
}

/** Resolve the `model` field sent on `POST /runtime-sessions/{id}/prompt`:
 *  - non-empty string → pin that model
 *  - `null` when `sessionModel` is currently set → `""` to clear the pinned model
 *  - `undefined` (or `null` when already unpinned) → `undefined` (omit from wire) */
export function resolvePromptModelOverride(
  draftModel: string | null | undefined,
  sessionModel: string | null | undefined,
): string | undefined {
  if (typeof draftModel === 'string' && draftModel.trim() !== '') return draftModel.trim();
  if (draftModel === null && typeof sessionModel === 'string' && sessionModel.trim() !== '') return '';
  return undefined;
}

export function buildOptimisticUserMessage(
  text: string,
  afterOrdinal: number,
  images: readonly ComposerFile[] = [],
): OptimisticMessageState {
  const span = startMeasurement('ui.thread.send', 'threads');
  if (span) {
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => span.finish('ok'));
    else queueMicrotask(() => span.finish('ok'));
  }
  const nowIso = new Date().toISOString();
  const idSeq = ++optimisticSeq;
  const attachments: MessageAttachment[] = images.map((f, idx) => {
    const isImg = f.mime.startsWith('image/');
    return {
      id: `optimistic-att-${idSeq}-${idx}`,
      kind: isImg ? 'image' : 'file',
      mime: f.mime,
      media_type: f.mime,
      name: f.name,
      ...(isImg ? { preview_url: `data:${f.mime};base64,${f.data_base64}` } : {}),
    };
  });
  return {
    afterOrdinal,
    item: {
      id: `optimistic-user-${idSeq}`,
      turn_id: null,
      ordinal: afterOrdinal + 0.5,
      status: 'completed',
      title: null,
      started_at: nowIso,
      completed_at: nowIso,
      updated_at: nowIso,
      type: 'user_message',
      text,
      attachments,
    },
  };
}

export function withOptimisticItem(
  items: readonly TurnItem[],
  optimistic: OptimisticMessageState | null,
): TurnItem[] {
  if (!optimistic) return items as TurnItem[];
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.ordinal <= optimistic.afterOrdinal) break;
    if (it.type === 'user_message') return items as TurnItem[];
  }
  return [...items, optimistic.item];
}

export function restoreOptimisticDraft(
  draft: {
    text: string;
    images: ComposerFile[];
    context: RuntimeDraftContent['context'];
    freshContext: boolean;
  },
  sent: RuntimeDraftContent,
): void {
  if (!draft.text) draft.text = sent.text;
  else if (sent.text && draft.text !== sent.text) draft.text = `${sent.text}\n${draft.text}`;
  if (sent.images.length > 0) draft.images = [...sent.images, ...draft.images];
  if (sent.context.length > 0) {
    const existing = new Set(draft.context.map((c) => c.id));
    draft.context = [...sent.context.filter((c) => !existing.has(c.id)), ...draft.context];
  }
  if (sent.fresh_context) draft.freshContext = true;
}

/** A slash command that acts on the view; `false` = send it to the agent. */
export function runViewSlash(intent: ParsedSlashIntent, sessionId: string, host: RuntimeHostCommands | undefined): boolean {
  if (intent.kind !== 'fresh-context') return runHostSlash(intent, sessionId, host);
  runtimeDrafts.for(sessionId).freshContext = true;
  return true;
}

/** "Edit" on a queued message puts it back in the composer without losing
 *  what is being typed there: unsent text stays, the message goes below it. */
export function mergeQueuedEdit(current: string, queued: string): string {
  const kept = current.replace(/\s+$/, '');
  return kept.trim() ? `${kept}\n${queued}` : queued;
}

export function collectMessageIds(rows: readonly TimelineRow[], prev: string[]): string[] {
  const next: string[] = [];
  for (let i = 0; i < rows.length; i++) {
    const r = rows[i];
    if (r.kind === 'message') next.push(r.item.id);
  }
  return next.length === prev.length && next.every((id, i) => id === prev[i]) ? prev : next;
}

export function deriveSessionError(startError: string | null | undefined, p: RuntimeProjection | null): string | null {
  const e =
    startError ??
    (p?.session && (p.session.status === 'stopped' || p.session.status === 'error') ? p.session.last_error : null);
  const last = p?.items.findLast((i) => !i.parent_item_id);
  const said = last?.type === 'error' ? last.failure.message.trim() : '';
  if (!e || /no conversation found/i.test(e)) return null;
  return said && (e.includes(said) || said.includes(e.trim())) ? null : errorText(e);
}

export function deriveActivePlan(p: RuntimeProjection | null, turn: ProviderTurn | null): PlanArtifact | null {
  if (!p || !turn) return null;
  const pl = p.plans.findLast((x) => x.turn_id === turn.id);
  return pl && pl.steps.some((s) => s.status !== 'completed') ? pl : null;
}

export function buildRuntimeBanners(opts: {
  sessionError: string | null;
  uiAsk: boolean;
  hasQuestions: boolean;
  hasPlan: boolean;
  queueShown: boolean;
  needsRevive: boolean;
  freshContext: boolean;
}): BannerBlock[] {
  return [
    ...(opts.sessionError ? [{ key: 'error', label: 'error', urgent: true }] : []),
    ...(opts.uiAsk ? [{ key: 'ui', label: 'UI control request', urgent: true }] : []),
    ...(opts.hasQuestions ? [{ key: 'questions', label: 'questions', urgent: true }] : []),
    ...(opts.hasPlan ? [{ key: 'plan', label: 'plan' }] : []),
    ...(opts.queueShown ? [{ key: 'queue', label: opts.needsRevive ? 'revive' : 'queue' }] : []),
    ...(opts.freshContext ? [{ key: 'fresh', label: 'fresh context' }] : []),
  ];
}
