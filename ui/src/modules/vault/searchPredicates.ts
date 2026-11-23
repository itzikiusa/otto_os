// Structured OKF/frontmatter search predicates for Vault `search_filtered`.
// Mirrors crates/otto-vault/src/engine.rs operators:
//   status:<val>, type:<val>, tag:<val>, path:<prefix>,
//   stale:true, orphan:true, links-to:<note>.

export interface TogglePredicate {
  id: string;
  label: string;
  token: string;
  title: string;
  /** Mutually exclusive prefix group (e.g. `status:` replaces any prior `status:*`). */
  exclusivePrefix?: string;
}

export interface PrefixOperator {
  id: string;
  label: string;
  prefix: string;
  title: string;
}

export const AUDIT_PREDICATES: readonly TogglePredicate[] = [
  {
    id: 'stale',
    label: 'Stale',
    token: 'stale:true',
    title: 'Show notes flagged stale (stale:true)',
  },
  {
    id: 'orphan',
    label: 'Orphans',
    token: 'orphan:true',
    title: 'Show notes with no incoming links (orphan:true)',
  },
  {
    id: 'stable',
    label: 'Stable',
    token: 'status:stable',
    title: 'Filter to stable OKF notes (status:stable)',
    exclusivePrefix: 'status:',
  },
  {
    id: 'draft',
    label: 'Draft',
    token: 'status:draft',
    title: 'Filter to draft OKF notes (status:draft)',
    exclusivePrefix: 'status:',
  },
];

export const PREFIX_OPERATORS: readonly PrefixOperator[] = [
  { id: 'type', label: 'type:', prefix: 'type:', title: 'Filter by OKF note type (e.g. type:rfc, type:flow)' },
  { id: 'tag', label: 'tag:', prefix: 'tag:', title: 'Filter by tag (e.g. tag:product/rfc)' },
  { id: 'links-to', label: 'links-to:', prefix: 'links-to:', title: 'Find notes linking to a target note (e.g. links-to:index)' },
  { id: 'path', label: 'path:', prefix: 'path:', title: 'Filter by folder prefix (e.g. path:rfcs/)' },
];

const STRUCTURED_TOKEN_RE = /^(?:status|type|tag|path|stale|orphan|links-to):[^\s]+$/i;

function splitTerms(query: string): string[] {
  return query.trim().split(/\s+/).filter(Boolean);
}

/** True when `query` already contains `token` (case-insensitive whole-token match). */
export function hasPredicate(query: string, token: string): boolean {
  const target = token.toLowerCase();
  return splitTerms(query).some((t) => t.toLowerCase() === target);
}

/** Toggle a structured predicate token inside `query`, replacing any token in the same
 *  mutually-exclusive prefix group (e.g. switching `status:draft` → `status:stable`). */
export function togglePredicate(query: string, token: string, exclusivePrefix?: string): string {
  const target = token.toLowerCase();
  const terms = splitTerms(query);
  if (terms.some((t) => t.toLowerCase() === target)) {
    return terms.filter((t) => t.toLowerCase() !== target).join(' ');
  }
  const ex = exclusivePrefix?.toLowerCase();
  const kept = ex ? terms.filter((t) => !t.toLowerCase().startsWith(ex)) : terms;
  return [...kept, token].join(' ');
}

/** Append an operator prefix (e.g. `links-to:`) cleanly so the user can type its value immediately. */
export function insertOperatorPrefix(query: string, prefix: string): string {
  const trimmed = query.trimEnd();
  if (!trimmed) return prefix;
  if (trimmed.toLowerCase().endsWith(prefix.toLowerCase())) return trimmed;
  return `${trimmed} ${prefix}`;
}

/** Return all active structured predicate tokens currently in `query`. */
export function activePredicates(query: string): string[] {
  return splitTerms(query).filter((t) => STRUCTURED_TOKEN_RE.test(t));
}

/** Strip all structured predicate tokens, keeping any free-text search terms. */
export function clearPredicates(query: string): string {
  return splitTerms(query)
    .filter((t) => !STRUCTURED_TOKEN_RE.test(t))
    .join(' ');
}
