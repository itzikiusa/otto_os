// What a runtime thread's composer does that is not view state (kept out of
// RuntimeThread.svelte): the slash commands that act on the view (through
// the host's commands, hostCommands.ts), merging an edited queued message
// into the draft, and reading pasted / attached files into attachments.
import type {
  MessageAttachment,
  PlanArtifact,
  ProviderTurn,
  RuntimeDraftContent,
  RuntimeSession,
  TurnItem,
} from '../../../lib/api/types';
import { sessionProviderSlug } from '../../../lib/providerRegistry';
import type { LinkedChildThreadSummary } from './agentTasks';
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
/** All of one message's attachments together, raw bytes. Base64 grows them by a
 *  third (16 → ~21.4 MiB), which stays under the daemon's 24 MiB draft ceiling
 *  (`otto-state` agent_runtime/drafts.rs) and its 32 MiB prompt body limit
 *  (`otto-agent-protocol` http.rs) with room for the text and context chips. */
export const MAX_TOTAL_ATTACHMENT_BYTES = 16 * 1024 * 1024;
/** Maximum images/PDFs attached to a single composer draft (mirrors server draft cap). */
export const MAX_DRAFT_IMAGES = 20;
let optimisticSeq = 0;

import { runtimeDrafts, type ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
export type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';

export interface OptimisticMessageState {
  item: TurnItem;
  afterOrdinal: number;
}

const MB = 1024 * 1024;

/** Decoded size of base64 attachments, without decoding them. */
export function attachmentBytes(files: readonly { data_base64: string }[]): number {
  let n = 0;
  for (const f of files) {
    const s = f.data_base64;
    const pad = s.endsWith('==') ? 2 : s.endsWith('=') ? 1 : 0;
    n += Math.max(0, Math.floor((s.length * 3) / 4) - pad);
  }
  return n;
}

/** Why `name` (`size` bytes) can't join attachments already holding `used`
 *  bytes, or null when it fits. Names the limit the person ran into. */
export function attachmentRefusal(name: string, size: number, used: number): string | null {
  if (size > MAX_ATTACHMENT_BYTES) return `${name} is over the ${MAX_ATTACHMENT_BYTES / MB} MB limit for one file.`;
  if (used + size > MAX_TOTAL_ATTACHMENT_BYTES) {
    return `${name} not added: one message carries at most ${MAX_TOTAL_ATTACHMENT_BYTES / MB} MB of attachments. Send it in a separate message.`;
  }
  return null;
}

/** Read the files `takes` accepts as base64 attachments. A file over the
 *  per-file cap, or one that would take the message (with `usedBytes`
 *  already attached) past {@link MAX_TOTAL_ATTACHMENT_BYTES}, is refused
 *  through `refuse` (a toast when none is given); the files after it still
 *  get their chance. */
export async function readAttachments(
  files: Iterable<File>,
  takes: (f: File) => boolean,
  opts: { usedBytes?: number; refuse?: (message: string) => void } = {},
): Promise<ComposerFile[]> {
  const out: ComposerFile[] = [];
  const refuse = opts.refuse ?? ((m: string) => toastError('Attachment not added', new Error(m)));
  let used = opts.usedBytes ?? 0;
  for (const f of files) {
    if (!takes(f)) continue;
    const name = f.name || 'pasted image';
    const why = attachmentRefusal(name, f.size, used);
    if (why) {
      refuse(why);
      continue;
    }
    used += f.size;
    const data = new Uint8Array(await f.arrayBuffer());
    let bin = '';
    for (let i = 0; i < data.length; i += 0x8000) bin += String.fromCharCode(...data.subarray(i, i + 0x8000));
    out.push({ name, mime: f.type, data_base64: btoa(bin) });
  }
  return out;
}

/** Encode raw bytes as base64 in 32 KiB chunks so large screenshots/PDFs never blow the call stack. */
export function uint8ToBase64(data: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < data.length; i += 0x8000) {
    bin += String.fromCharCode(...data.subarray(i, i + 0x8000));
  }
  return btoa(bin);
}

/** Re-hydrate persisted `MessageAttachment[]` from a failed, interrupted, or partial turn
 *  back into `ComposerFile[]` so Edit / Send again preserves screenshots and PDFs. */
export async function hydrateRetryAttachments(
  attachments: readonly MessageAttachment[] | undefined,
  fetchBytes: (attachmentId: string) => Promise<Uint8Array>,
): Promise<ComposerFile[]> {
  if (!attachments || attachments.length === 0) return [];
  const out: ComposerFile[] = [];
  for (let i = 0; i < attachments.length; i++) {
    const a = attachments[i];
    const fallbackName =
      a.name?.trim() ||
      (a.kind === 'image'
        ? attachments.length > 1
          ? `Screenshot ${i + 1}`
          : 'Screenshot'
        : 'Attachment');
    if (a.preview_url?.startsWith('data:')) {
      const comma = a.preview_url.indexOf(',');
      if (comma > 0 && a.preview_url.slice(0, comma).includes(';base64')) {
        out.push({
          name: fallbackName,
          mime: a.mime,
          data_base64: a.preview_url.slice(comma + 1),
        });
        continue;
      }
    }
    const bytes = await fetchBytes(a.id);
    out.push({
      name: fallbackName,
      mime: a.mime,
      data_base64: uint8ToBase64(bytes),
    });
  }
  return out;
}

/** Add files to a thread's composer. What doesn't fit (count or size) is
 *  named inline under the composer (`draft.attachError`, RuntimeDraftStatus). */
export async function appendDraftAttachments<TDraft extends { images: ComposerFile[]; attachError?: string | null }>(
  ownerId: string,
  draft: TDraft,
  files: Iterable<File>,
  takes: (f: File) => boolean,
): Promise<void> {
  let refused: string | null = null;
  const read = await readAttachments(files, takes, {
    usedBytes: attachmentBytes(draft.images),
    refuse: (m) => (refused ??= m),
  });
  if (!runtimeDrafts.owns(ownerId, draft as never)) return;
  // Another paste may have landed while these were read: re-check the total.
  let used = attachmentBytes(draft.images);
  const kept: ComposerFile[] = [];
  for (const f of read) {
    const size = attachmentBytes([f]);
    const why = attachmentRefusal(f.name, size, used);
    if (why) {
      refused ??= why;
      continue;
    }
    used += size;
    kept.push(f);
  }
  const combined = [...draft.images, ...kept];
  if (combined.length > MAX_DRAFT_IMAGES) {
    refused ??= `At most ${MAX_DRAFT_IMAGES} images or PDFs per message.`;
    draft.images = combined.slice(0, MAX_DRAFT_IMAGES);
  } else if (kept.length) {
    draft.images = combined;
  }
  draft.attachError = refused;
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

/** Merge attachments popped from an edited queued message back into `draft.images`
 *  without duplicating screenshots/files already sitting in the composer, capped at `MAX_DRAFT_IMAGES`. */
export function mergeQueuedAttachments(
  current: readonly ComposerFile[],
  restored: readonly ComposerFile[],
): ComposerFile[] {
  if (restored.length === 0) return current.slice(0, MAX_DRAFT_IMAGES);
  const kept = current.slice(0, MAX_DRAFT_IMAGES);
  const seen = new Set(kept.map((f) => `${f.mime}:${f.data_base64}`));
  const extra: ComposerFile[] = [];
  let capped = current.length > MAX_DRAFT_IMAGES;
  let oversize = false;
  let used = attachmentBytes(kept);
  for (const item of restored) {
    const key = `${item.mime}:${item.data_base64}`;
    if (seen.has(key)) continue;
    if (kept.length + extra.length >= MAX_DRAFT_IMAGES) {
      capped = true;
      break;
    }
    const size = attachmentBytes([item]);
    if (used + size > MAX_TOTAL_ATTACHMENT_BYTES) {
      oversize = true;
      continue;
    }
    used += size;
    seen.add(key);
    extra.push({ ...item });
  }
  if (capped) {
    toastError('Too many attachments', new Error(`Kept the first ${MAX_DRAFT_IMAGES} attachments in the composer.`));
  } else if (oversize) {
    toastError(
      'Attachments left out',
      new Error(`One message carries at most ${MAX_TOTAL_ATTACHMENT_BYTES / MB} MB of attachments; the rest were not restored.`),
    );
  }
  return extra.length > 0 ? [...kept, ...extra] : kept;
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

export function mapChildThreadSummaries(
  byId: Record<string, RuntimeSession>,
  parentSessionId: string,
): LinkedChildThreadSummary[] {
  const out: LinkedChildThreadSummary[] = [];
  for (const s of Object.values(byId)) {
    if (s.parent_id !== parentSessionId) continue;
    out.push({
      id: s.id,
      title: s.title,
      provider: sessionProviderSlug(s),
      model: s.model ?? null,
      status: s.status,
      createdAt: s.created_at,
      updatedAt: s.last_active_at,
    });
  }
  return out;
}

export function composerPlaceholderText(
  closed: boolean,
  approvals: ReadonlyMap<string, { readonly kind?: string }>,
  running: boolean,
  agentChat?: string,
): string {
  if (closed) return 'Send a message to reopen this thread';
  if (approvals.size > 0) {
    return Array.from(approvals.values()).some((r) => r.kind === 'question')
      ? 'Answer the question above'
      : 'Waiting for your approval above';
  }
  if (running) return 'Queue a follow-up for when this turn ends';
  return agentChat ? `Message ${agentChat}…` : 'Ask anything. @ files or agents, / commands';
}
