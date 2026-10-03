// Workbench placeholders — the SAME syntax as the DB Explorer's query
// variables / "Run on…" multi-run (`:name`, `{name}`, `{{name}}`).
//
// SQL docs reuse the DB editor's parser (`substituteVars` / `extractVars` in
// modules/database/sql-util.ts): quote- and comment-aware, skips `::casts`,
// `user:123`-style keys, and anything inside a string literal or comment —
// so a script sent to "Run on…" sees exactly the variables shown here.
//
// Every OTHER language only recognises `{name}` / `{{name}}`, and does so
// everywhere (inside JSON strings too, which is where they usually live):
// `:name` is far too noisy outside SQL (YAML keys, URLs, times, CSS…).

import { substituteVars } from '../../database/sql-util.ts';

export type WbPlaceholderSyntax = ':name' | '{name}' | '{{name}}';

export interface WbPlaceholder {
  name: string;
  /** Syntax of the FIRST occurrence (a name may appear in several forms). */
  syntax: WbPlaceholderSyntax;
  count: number;
  /** Offset of the first occurrence in the text. */
  firstIndex: number;
}

interface Hit {
  name: string;
  syntax: WbPlaceholderSyntax;
  start: number;
  end: number;
}

const SQL_LANGS = new Set(['sql']);

/** Languages whose placeholders follow the SQL (DB editor) rules. */
export function isSqlLike(lang?: string): boolean {
  return !!lang && SQL_LANGS.has(lang);
}

function syntaxAt(text: string, i: number): WbPlaceholderSyntax {
  if (text[i] === ':') return ':name';
  return text.startsWith('{{', i) ? '{{name}}' : '{name}';
}

/** SQL hits via the DB parser: substitute every name with a marker, then walk
 *  the result alongside the original to recover each token's span. */
function sqlHits(text: string): Hit[] {
  // Gather candidate names with a permissive regex, then let the DB parser
  // decide which positions are real code-position variables.
  const names = new Set<string>();
  for (const m of text.matchAll(/(?::|\{\{?)([A-Za-z_]\w*)/g)) names.add(m[1]);
  if (!names.size) return [];
  const OPEN = '\u0001';
  const CLOSE = '\u0002';
  const values: Record<string, string> = {};
  for (const n of names) values[n] = `${OPEN}${n}${CLOSE}`;
  const marked = substituteVars(text, values, 'sql');
  const hits: Hit[] = [];
  let o = 0; // offset in the original
  let k = 0; // offset in the marked text
  while (k < marked.length) {
    if (marked[k] !== OPEN) {
      k++;
      o++;
      continue;
    }
    const end = marked.indexOf(CLOSE, k);
    const name = marked.slice(k + 1, end);
    const syntax = syntaxAt(text, o);
    const len = syntax === ':name' ? name.length + 1 : syntax === '{name}' ? name.length + 2 : name.length + 4;
    hits.push({ name, syntax, start: o, end: o + len });
    o += len;
    k = end + 1;
  }
  return hits;
}

/** Non-SQL hits: `{{name}}` and `{name}` anywhere (mustache matched whole). */
function braceHits(text: string): Hit[] {
  const hits: Hit[] = [];
  const re = /\{\{([A-Za-z_]\w*)\}\}|\{([A-Za-z_]\w*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    if (m[1] !== undefined) hits.push({ name: m[1], syntax: '{{name}}', start: m.index, end: m.index + m[0].length });
    else hits.push({ name: m[2], syntax: '{name}', start: m.index, end: m.index + m[0].length });
  }
  return hits;
}

function hitsFor(text: string, lang?: string): Hit[] {
  return isSqlLike(lang) ? sqlHits(text) : braceHits(text);
}

/** Unique placeholders in first-seen order, with occurrence counts. */
export function detectPlaceholders(text: string, lang?: string): WbPlaceholder[] {
  const byName = new Map<string, WbPlaceholder>();
  for (const h of hitsFor(text, lang)) {
    const p = byName.get(h.name);
    if (p) p.count++;
    else byName.set(h.name, { name: h.name, syntax: h.syntax, count: 1, firstIndex: h.start });
  }
  return [...byName.values()].sort((a, b) => a.firstIndex - b.firstIndex);
}

/** Replace placeholders that have a NON-EMPTY value (raw text, the caller
 *  controls quoting); unknown or empty ones are left as-is. */
export function fillPlaceholders(text: string, values: Record<string, string>, lang?: string): string {
  const reps = hitsFor(text, lang)
    .filter((h) => Object.prototype.hasOwnProperty.call(values, h.name) && values[h.name] !== '')
    .sort((a, b) => b.start - a.start);
  let out = text;
  for (const r of reps) out = out.slice(0, r.start) + values[r.name] + out.slice(r.end);
  return out;
}
