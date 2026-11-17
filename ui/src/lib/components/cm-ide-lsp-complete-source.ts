// How a server's completion list becomes the popup: filtered by what was typed before
// the cap, and the only source while the server is ready (split out of
// `cm-ide-lsp-completion.ts`).

import { completeFromList, type Completion, type CompletionSource } from '@codemirror/autocomplete';
import type { RawLspCompletionItem } from './cm-ide-lsp-completion.ts';

/** Most items one completion popup takes from a server list (tsserver sends ~1000 globals). */
export const LSP_COMPLETION_CAP = 150;

/**
 * The server items matching the typed `prefix` (case-insensitive subsequence of
 * `filterText ?? label`, CodeMirror's own fuzzy rule), capped AFTER filtering, so
 * `Prom` still finds `Promise` past the first 150 globals. `truncated`: the cap cut
 * matches, so the list must be re-queried as the user types instead of reused.
 */
export function filterLspCompletionItems(items: readonly RawLspCompletionItem[], prefix: string): { items: RawLspCompletionItem[]; truncated: boolean } {
  const want = prefix.toLowerCase();
  const matches = want
    ? items.filter((it) => {
        const text = (it.filterText ?? it.label).toLowerCase();
        let i = 0;
        for (const ch of text) if (ch === want[i] && ++i === want.length) return true;
        return false;
      })
    : [...items];
  return { items: matches.slice(0, LSP_COMPLETION_CAP), truncated: matches.length > LSP_COMPLETION_CAP };
}

/**
 * The server's list already has the keywords and locals the language package offers, so
 * merging both listed `const`/`zip` twice. The package's own sources only stand in while
 * the server is not ready.
 */
export function lspFirstCompletionSource(isReady: () => boolean, lspSource: CompletionSource): CompletionSource {
  return async (ctx) => {
    if (isReady()) return lspSource(ctx);
    for (const v of ctx.state.languageDataAt<CompletionSource | readonly (string | Completion)[]>('autocomplete', ctx.pos)) {
      const res = await (typeof v === 'function' ? v : completeFromList(v))(ctx);
      if (res) return res;
    }
    return null;
  };
}
