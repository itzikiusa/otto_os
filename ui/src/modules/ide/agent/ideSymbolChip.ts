// Pure, zero-Svelte Structure-symbol context chip builder and bounded context
// chip upsert shared between `ideContextBuilders.ts`, `ideAgentState.svelte.ts`,
// and `ui/unit/ideAgentContext.test.ts`.

import {
  detectProposalLanguage,
  MAX_CHIP_SNIPPET_CHARS,
  MAX_CONTEXT_CHIPS,
  offsetsForLineRange,
  truncateSnippet,
  type IdeContextChip,
} from '../store/ideDiffApplier.ts';

export interface IdeSymbolChipInput {
  name: string;
  kind: string;
  detail?: string | null;
  line: number;
  end_line?: number | null;
}

export function buildPureSymbolDraftChip(
  fullPath: string,
  relPath: string,
  buf: string,
  sym: IdeSymbolChipInput,
): Omit<IdeContextChip, 'id'> {
  const startLine = Math.max(1, sym.line || 1);
  const endLine = Math.max(startLine, sym.end_line || startLine);
  const slice = offsetsForLineRange(buf, startLine, endLine);
  return {
    key: `symbol:${fullPath}:${sym.name}:${startLine}`,
    kind: 'symbol',
    label: `@${relPath}#${sym.name}`,
    sublabel: `L${startLine}-L${endLine}`,
    path: fullPath,
    startLine,
    endLine,
    language: detectProposalLanguage(relPath),
    snippet: slice.text || `${sym.kind} ${sym.name} (L${startLine}-L${endLine})`,
  };
}

export function upsertIdeContextChipList(
  contexts: readonly IdeContextChip[],
  draft: Omit<IdeContextChip, 'id'> & { id?: string },
  allocId: () => string,
): { chip: IdeContextChip; next: IdeContextChip[] } {
  const snippet = truncateSnippet(draft.snippet, MAX_CHIP_SNIPPET_CHARS);
  const idx = contexts.findIndex((c) => c.key === draft.key);
  if (idx >= 0) {
    const updated: IdeContextChip = { ...contexts[idx], ...draft, id: contexts[idx].id, snippet };
    const next = contexts.slice();
    next[idx] = updated;
    return { chip: updated, next };
  }
  const created: IdeContextChip = { ...draft, id: draft.id ?? allocId(), snippet };
  const trimmed =
    contexts.length >= MAX_CONTEXT_CHIPS
      ? contexts.slice(contexts.length - (MAX_CONTEXT_CHIPS - 1))
      : contexts;
  return { chip: created, next: [...trimmed, created] };
}
