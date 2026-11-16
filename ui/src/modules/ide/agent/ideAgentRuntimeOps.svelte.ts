// Stateless runtime turn dispatch, inline-edit reply waiting, proposal map
// derivation, and editor-buffer Apply/Undo helpers for `ideAgentController.svelte.ts`.

import type { PromptImage, RuntimeAccessMode, RuntimePromptResp } from '../../../lib/api/types';
import type { EffortLevel } from '../../../lib/providerModels';
import type { RuntimeProjectionView, RuntimeSessionPort } from '../../../lib/runtimeThreadPort';
import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import {
  ide,
  ideBasename,
  type IdeDebugSessionSnapshot,
  type IdeDiagnostic,
  type IdeSymbolNode,
  type IdeTestResult,
  type IdeVariable,
} from '../ideStore.svelte';
import {
  computeLineDiff,
  extractCodeProposals,
  offsetsForLineRange,
  parseIdeContextChipsFromPrompt,
  spliceCodeRange,
  stripIdeContextBlocks,
  type IdeCodeProposal,
  type IdeContextChip,
  type IdeInlinePromptState,
} from '../store/ideDiffApplier';
import { ideAgentState } from './ideAgentState.svelte';
import {
  buildDebugFrameDraftChip,
  buildDiagnosticDraftChip,
  buildFileDraftChip,
  buildSymbolDraftChip,
  buildTestFailureDraftChip,
} from './ideContextBuilders';

const MAX_PROPOSAL_CACHE = 32;
const MAX_UNDO_SNAPSHOTS = 16;

export function readIdeFileBuffer(fullPath: string): string {
  const viaStore = ide.getBuffer?.(fullPath);
  return (
    (typeof viaStore === 'string' && viaStore.length > 0
      ? viaStore
      : ide.tabs.find((t) => t.path === fullPath)?.content) ?? ''
  );
}

export function computeProposalsMap(opts: {
  projection: RuntimeProjectionView | null;
  appliedMap: Readonly<Record<string, string>>;
  appliedPathByProposalId: ReadonlyMap<string, string>;
  proposalCache: Map<string, { sig: string; proposals: IdeCodeProposal[] }>;
}): Record<string, IdeCodeProposal[]> {
  const { projection: proj, appliedMap, appliedPathByProposalId, proposalCache } = opts;
  if (!proj || proj.items.length === 0) return {};
  const activeFallback = ide.activePath;
  const out: Record<string, IdeCodeProposal[]> = {};
  const seenIds = new Set<string>();
  const userByTurn = new Map<string, string>();
  let latestUserText = '';

  const baselineFor = (msgId: string, fullPath: string): string => {
    for (const [propId, snap] of Object.entries(appliedMap)) {
      if (propId.startsWith(`${msgId}:`) && appliedPathByProposalId.get(propId) === fullPath) return snap;
    }
    return readIdeFileBuffer(fullPath);
  };

  for (let i = 0; i < proj.items.length; i++) {
    const it = proj.items[i];
    if (it.type === 'user_message') {
      latestUserText = it.text;
      if (it.turn_id) userByTurn.set(it.turn_id, it.text);
      continue;
    }
    if (it.type !== 'assistant_message') continue;
    seenIds.add(it.id);
    const promptText = (it.turn_id ? userByTurn.get(it.turn_id) : undefined) ?? latestUserText;
    const chips = parseIdeContextChipsFromPrompt(promptText);
    const selChips = chips.filter((c) => c.kind === 'selection' && c.path && c.startLine);
    const firstPathChip = selChips[0] ?? chips.find((c) => Boolean(c.path));
    const fallbackPath = firstPathChip?.path ? ide.resolvePath(firstPathChip.path) : activeFallback;
    const fallbackRange =
      selChips.length === 1 && selChips[0].startLine
        ? { startLine: selChips[0].startLine, endLine: selChips[0].endLine ?? selChips[0].startLine }
        : null;
    const rangeKey = fallbackRange ? `${fallbackRange.startLine}-${fallbackRange.endLine}` : '';
    const fbContent = fallbackPath ? baselineFor(it.id, fallbackPath) : '';
    const sig = `${it.text}\0${fallbackPath ?? ''}\0${rangeKey}\0${fbContent.length}:${fbContent.slice(0, 48)}`;
    let cached = proposalCache.get(it.id);
    if (!cached || cached.sig !== sig) {
      const extracted = extractCodeProposals(it.text, fallbackPath, fallbackRange, {
        messageId: it.id,
        resolvePath: (p) => ide.resolvePath(p),
        relativePath: (p) => ide.relativePath(p),
        getFileContent: (fullPath) => baselineFor(it.id, fullPath),
      });
      cached = { sig, proposals: extracted };
      proposalCache.set(it.id, cached);
    }
    if (cached.proposals.length > 0) {
      const mapped = cached.proposals.map((p) => {
        const isApplied = Object.prototype.hasOwnProperty.call(appliedMap, p.id);
        return p.applied === isApplied ? p : { ...p, applied: isApplied };
      });
      out[it.id] = mapped;
      if (it.message_id && it.message_id !== it.id) out[it.message_id] = mapped;
    }
  }

  if (proposalCache.size > MAX_PROPOSAL_CACHE) {
    for (const k of proposalCache.keys()) {
      if (!seenIds.has(k) || proposalCache.size > MAX_PROPOSAL_CACHE) proposalCache.delete(k);
      if (proposalCache.size <= MAX_PROPOSAL_CACHE) break;
    }
  }
  return out;
}

export async function dispatchPromptToRuntimeSession(
  sid: string,
  text: string,
  opts: {
    model?: string | null;
    effort?: EffortLevel | null;
    files?: ComposerFile[];
    accessMode?: RuntimeAccessMode;
    freshContext?: boolean;
  } = {},
): Promise<RuntimePromptResp> {
  const bridge = ideAgentState.runtimeBridge ?? (await ideAgentState.ensureBridge());
  if (!bridge) throw new Error('Couldn’t load runtime threads in this window.');
  const { runtimeThreads } = bridge;
  const turnBusy = isRuntimeTurnBusy(bridge, sid);
  const attached = opts.files ?? [];
  const pics: PromptImage[] = attached
    .filter((f) => f.mime.startsWith('image/'))
    .map(({ name, mime, data_base64 }) => ({ ...(name ? { name } : {}), mime, data_base64 }));
  const docs = attached.filter((f) => !f.mime.startsWith('image/'));
  return runtimeThreads.prompt(
    sid,
    {
      text,
      ...(opts.model ? { model: opts.model } : {}),
      ...(opts.effort ? { effort: opts.effort } : {}),
      ...(pics.length > 0 ? { images: pics } : {}),
      ...(docs.length > 0 ? { files: docs } : {}),
      ...(opts.accessMode ? { access_mode: opts.accessMode } : {}),
      ...(opts.freshContext ? { fresh_context: true } : {}),
      ...(turnBusy && !opts.freshContext ? { delivery: 'queue' as const } : {}),
    },
    attached,
  );
}

/** Whether `sid` is mid-turn (a new prompt would be queued behind it). */
export function isRuntimeTurnBusy(bridge: RuntimeSessionPort, sid: string): boolean {
  const row = bridge.runtimeThreads.get(sid);
  return (
    row?.status === 'working' ||
    row?.status === 'starting' ||
    Boolean(bridge.runtimeViews.projections[sid]?.turns.some((t) => t.status === 'running' || t.status === 'pending'))
  );
}

export function waitForInlineTurnCompletion(
  sid: string,
  resp: RuntimePromptResp,
  knownIds: ReadonlySet<string>,
  isCancelled: () => boolean,
): Promise<{ messageId: string; text: string }> {
  if (!ideAgentState.runtimeBridge) void ideAgentState.ensureBridge();
  return new Promise((resolve, reject) => {
    let settled = false;
    let stopEffect: (() => void) | null = null;
    const finish = (fn: () => void): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timeoutId);
      if (stopEffect) {
        stopEffect();
        stopEffect = null;
      }
      fn();
    };
    const timeoutId = setTimeout(() => {
      finish(() => reject(new Error('Timed out waiting for inline edit response.')));
    }, 60_000);

    stopEffect = $effect.root(() => {
      $effect(() => {
        if (isCancelled()) {
          finish(() => resolve({ messageId: 'cancelled', text: '' }));
          return;
        }
        const bridge = ideAgentState.runtimeBridge;
        if (!bridge) return;
        const proj = bridge.runtimeViews.projections[sid];
        const rowStatus = bridge.runtimeThreads.byId[sid]?.status;
        if (!proj) return;
        const errItem = proj.items.find(
          (i) => !knownIds.has(i.id) && i.type === 'error' && (!resp.turn_id || i.turn_id === resp.turn_id),
        );
        if (errItem && errItem.type === 'error') {
          finish(() => reject(new Error(errItem.failure.message || 'Inline edit turn returned an error.')));
          return;
        }
        const assistants = proj.items.filter(
          (i): i is Extract<(typeof proj.items)[number], { type: 'assistant_message' }> =>
            i.type === 'assistant_message' &&
            (!knownIds.has(i.id) || (Boolean(resp.turn_id) && i.turn_id === resp.turn_id)),
        );
        const turn = resp.turn_id
          ? proj.turns.find((t) => t.id === resp.turn_id)
          : proj.turns[proj.turns.length - 1];
        const turnDone = turn
          ? turn.status === 'completed' ||
            turn.status === 'failed' ||
            turn.status === 'interrupted' ||
            turn.status === 'cancelled'
          : assistants.length > 0 && assistants.every((a) => !a.streaming) && rowStatus === 'idle';
        if (turnDone && assistants.length > 0) {
          finish(() =>
            resolve({
              messageId: assistants[assistants.length - 1].id,
              text: assistants.map((a) => a.text).join('\n\n'),
            }),
          );
          return;
        }
        if (turn && (turn.status === 'failed' || turn.status === 'cancelled' || turn.status === 'interrupted')) {
          finish(() => reject(new Error(`Turn ${turn.status}.`)));
        }
      });
    });
    if (settled && stopEffect) {
      stopEffect();
      stopEffect = null;
    }
  });
}

export function extractInlineEditProposal(
  st: IdeInlinePromptState,
  query: string,
  reply: { messageId: string; text: string },
): IdeCodeProposal | null {
  const range = { from: st.from, to: st.to, startLine: st.startLine, endLine: st.endLine };
  const extracted = extractCodeProposals(reply.text, st.path, range, {
    messageId: reply.messageId,
    preferRange: true,
    resolvePath: (p) => ide.resolvePath(p),
    relativePath: (p) => ide.relativePath(p),
    getFileContent: (fullPath) => readIdeFileBuffer(fullPath),
  });
  if (extracted[0]) return extracted[0];
  const cleaned = stripIdeContextBlocks(reply.text).replace(/^echo:\s*/i, '').trim();
  if (!cleaned || cleaned === query) return null;
  const diffLines = computeLineDiff(st.selectedText, cleaned);
  return {
    id: `${reply.messageId}:inline`,
    messageId: reply.messageId,
    path: st.path,
    relPath: st.relPath,
    language: st.language,
    code: cleaned,
    range,
    diffLines,
    addedCount: diffLines.filter((d) => d.kind === 'add').length,
    removedCount: diffLines.filter((d) => d.kind === 'del').length,
    applied: false,
  };
}

async function writeIdeEditorBuffer(
  fullPath: string,
  content: string,
  range?: { from: number; to: number },
): Promise<boolean> {
  if (ide.applyAgentEdit) return ide.applyAgentEdit(fullPath, content, range);
  await ide.openFile(fullPath, { preview: false });
  ide.updateBuffer(fullPath, range ? spliceCodeRange(readIdeFileBuffer(fullPath), content, range) : content);
  return true;
}

export async function applyIdeCodeProposal(
  proposal: IdeCodeProposal,
  appliedUndoByProposalId: Readonly<Record<string, string>>,
  appliedPathByProposalId: Map<string, string>,
): Promise<Record<string, string> | null> {
  const fullPath = ide.resolvePath(proposal.path);
  if (!ide.tabs.some((t) => t.path === fullPath && !t.loading)) await ide.openFile(fullPath, { preview: false });
  const tab = ide.tabs.find((t) => t.path === fullPath);
  if (!tab || tab.readOnly || !ide.canEditCollab) return null;
  const before = readIdeFileBuffer(fullPath);
  const r = proposal.range;
  const resolved = r
    ? r.from >= 0 && r.to >= r.from && r.to <= before.length
      ? { from: r.from, to: r.to }
      : offsetsForLineRange(before, r.startLine, r.endLine)
    : null;
  const fullNext = r ? spliceCodeRange(before, proposal.code, resolved ?? r) : proposal.code;
  const chunk = resolved ? fullNext.slice(resolved.from, fullNext.length - (before.length - resolved.to)) : fullNext;
  if (!(await writeIdeEditorBuffer(fullPath, chunk, resolved ?? undefined))) return null;
  appliedPathByProposalId.set(proposal.id, fullPath);
  const nextUndo: Record<string, string> = { ...appliedUndoByProposalId, [proposal.id]: before };
  const keys = Object.keys(nextUndo);
  while (keys.length > MAX_UNDO_SNAPSHOTS) {
    const evict = keys.shift()!;
    delete nextUndo[evict];
    appliedPathByProposalId.delete(evict);
  }
  return nextUndo;
}

export async function undoIdeCodeProposal(
  proposalId: string,
  appliedUndoByProposalId: Readonly<Record<string, string>>,
  appliedPathByProposalId: Map<string, string>,
  proposalsByMessageId: Readonly<Record<string, IdeCodeProposal[]>>,
): Promise<Record<string, string> | null> {
  if (!Object.prototype.hasOwnProperty.call(appliedUndoByProposalId, proposalId)) return null;
  const before = appliedUndoByProposalId[proposalId];
  let fullPath = appliedPathByProposalId.get(proposalId) ?? null;
  if (!fullPath) {
    for (const list of Object.values(proposalsByMessageId)) {
      const hit = list.find((p) => p.id === proposalId);
      if (hit) {
        fullPath = ide.resolvePath(hit.path);
        break;
      }
    }
  }
  if (!fullPath || !(await writeIdeEditorBuffer(fullPath, before))) return null;
  appliedPathByProposalId.delete(proposalId);
  const rest = { ...appliedUndoByProposalId };
  delete rest[proposalId];
  return rest;
}

export async function createFileChipFromPath(path: string): Promise<Omit<IdeContextChip, 'id'>> {
  return buildFileDraftChip(path, readIdeFileBuffer);
}

export function createSymbolChipFromPath(path: string, sym: IdeSymbolNode): Omit<IdeContextChip, 'id'> {
  const fullPath = ide.resolvePath(path);
  const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
  return buildSymbolDraftChip(fullPath, relPath, readIdeFileBuffer(fullPath), sym);
}

export function createDiagnosticChipEntry(diag: IdeDiagnostic): { chip: Omit<IdeContextChip, 'id'>; prompt: string } {
  const fullPath = ide.resolvePath(diag.path || ide.activePath || '');
  const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
  return {
    chip: buildDiagnosticDraftChip(fullPath, relPath, readIdeFileBuffer(fullPath), diag),
    prompt: `Fix this ${diag.severity} in ${relPath}:${diag.line}: ${diag.message}`,
  };
}

export function createTestFailureChipEntry(
  test: IdeTestResult,
  outputLines: string[],
): { chip: Omit<IdeContextChip, 'id'>; prompt: string } {
  const fullPath = test.file ? ide.resolvePath(test.file) : undefined;
  const relPath = fullPath ? ide.relativePath(fullPath) : undefined;
  return {
    chip: buildTestFailureDraftChip(test, outputLines, fullPath, relPath),
    prompt: `Investigate and fix the failing test ${test.name}.`,
  };
}

export function createDebugFrameChipEntry(
  dbg: IdeDebugSessionSnapshot,
  expandedVars?: Record<number, IdeVariable[]>,
): { chip: Omit<IdeContextChip, 'id'>; prompt: string } {
  const { chip, shortFile, line } = buildDebugFrameDraftChip(
    dbg,
    expandedVars,
    (p) => ide.resolvePath(p),
    (p) => ide.relativePath(p),
  );
  return {
    chip,
    prompt: `Inspect the paused debugger frame at ${shortFile}:${line} and help fix the issue.`,
  };
}
