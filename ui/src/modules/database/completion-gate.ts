// Where the DB query editor should NOT auto-open identifier completion: inside
// literals and comments (typing INSERT … VALUES data, prose). Reads the
// editor's syntax tree, so it is only as right as the tokenizer — the editor
// must run the connection's SQL dialect (`lib/sql-dialects.ts`), or MySQL `\'`
// escapes and `#` comments invert the string/code boundaries.

import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import type { EditorState } from '@codemirror/state';

// Literal / comment syntax nodes (lang-sql, lang-javascript, the redis stream
// tokens).
const INERT_NODE_RE = /^(?:String|Number|LineComment|BlockComment|Comment|TemplateString|RegExp)$/i;

/**
 * True when `pos` sits in a literal/comment where completion is noise. With
 * `mongo`, a quoted object KEY (`{ "na|`) still completes field names — only
 * string VALUES are inert there.
 */
export function isInertAt(state: EditorState, pos: number, mongo: boolean): boolean {
  // Parse up to the cursor if the background parser hasn't reached it yet (a
  // programmatic jump far below a fresh paste), bounded to 20 ms.
  const tree = ensureSyntaxTree(state, pos, 20) ?? syntaxTree(state);
  const node = tree.resolveInner(pos, -1);
  if (!INERT_NODE_RE.test(node.name)) return false;
  if (
    mongo &&
    /^string$/i.test(node.name) &&
    !node.prevSibling &&
    /Property$/.test(node.parent?.name ?? '')
  ) {
    return false;
  }
  return true;
}
