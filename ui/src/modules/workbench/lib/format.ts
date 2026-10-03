// Workbench "Format" (⇧⌥F) + inline validation, per language.
//
// No new dependencies: JSON is built in; YAML (`yaml`) and SQL
// (`sql-formatter`) are already app deps and load lazily on first use; XML /
// HTML / SVG use the small pretty-printer in ./xml.ts; CSV/TSV round-trip
// through ./csv.ts; everything else gets conservative whitespace
// normalisation (trailing spaces, tab indentation, blank-line runs, final
// newline) that can never change meaning. A formatter never loses content,
// and formatting its own output again is a no-op (unit-tested).

import { maskQueryPlaceholders, unmaskQueryPlaceholders } from '../../database/sql-util.ts';
import { parseCsv, stringifyCsv } from './csv.ts';
import { formatMarkup, MarkupError, validateMarkup } from './xml.ts';

export type FormatResult =
  | { ok: true; text: string; changed: boolean }
  | { ok: false; error: string; line?: number; col?: number };

export interface ValidationIssue {
  error: string;
  line?: number;
  col?: number;
}

const FORMATTABLE = new Set([
  'json', 'yaml', 'toml', 'csv', 'tsv', 'sql', 'html', 'xml', 'svg', 'md', 'css', 'js', 'ts', 'py', 'sh', 'go',
  'rs', 'java', 'mermaid', 'd2', 'txt', 'http',
]);

export function canFormat(lang: string): boolean {
  return FORMATTABLE.has(lang);
}

/** sql-formatter dialects we route to (anything else → generic `sql`). */
const SQL_DIALECTS = new Set([
  'sql', 'mysql', 'mariadb', 'postgresql', 'sqlite', 'plsql', 'transactsql', 'clickhouse', 'bigquery', 'snowflake',
  'redshift', 'trino', 'duckdb', 'spark', 'hive', 'db2', 'tidb', 'singlestoredb', 'n1ql',
]);
const DIALECT_ALIAS: Record<string, string> = { postgres: 'postgresql', pg: 'postgresql', tsql: 'transactsql', mssql: 'transactsql', oracle: 'plsql' };

function endsWithNewline(s: string): boolean {
  return s.endsWith('\n');
}

/** Keep the original's "ends with a newline" policy. */
function withEol(out: string, original: string): string {
  const body = out.replace(/\n+$/, '');
  return endsWithNewline(original) ? `${body}\n` : body;
}

function lineColAt(text: string, pos: number): { line: number; col: number } {
  const before = text.slice(0, Math.max(0, Math.min(pos, text.length)));
  const lines = before.split('\n');
  return { line: lines.length, col: lines[lines.length - 1].length + 1 };
}

/** Pull a position out of a JSON.parse error across engine message formats:
 *  V8 `… at position 12 (line 2 column 3)` / `… in JSON at position 12`,
 *  Firefox `… at line 2 column 3 of the JSON data`, Safari: none. */
export function jsonErrorPosition(message: string, text: string): { line?: number; col?: number } {
  const lc = /line (\d+) column (\d+)/i.exec(message);
  if (lc) return { line: Number(lc[1]), col: Number(lc[2]) };
  const pos = /position (\d+)/i.exec(message);
  if (pos) return lineColAt(text, Number(pos[1]));
  return {};
}

/** Offset of the first JSON syntax error (a tiny recursive-descent scanner),
 *  or -1 when valid. Used when the engine's message carries no position
 *  (current V8 says only `Unexpected token '}', …"…" is not valid JSON`). */
export function jsonErrorOffset(text: string): number {
  let i = 0;
  const n = text.length;
  const ws = () => {
    while (i < n && (text[i] === ' ' || text[i] === '\t' || text[i] === '\n' || text[i] === '\r')) i++;
  };
  const fail = (): never => {
    throw i;
  };
  const lit = (w: string) => {
    if (text.startsWith(w, i)) i += w.length;
    else fail();
  };
  const str = () => {
    i++;
    while (i < n && text[i] !== '"') {
      if (text[i] === '\\') {
        i++;
        if (text[i] === 'u') {
          if (!/^[0-9a-fA-F]{4}$/.test(text.slice(i + 1, i + 5))) fail();
          i += 4;
        } else if (!'"\\/bfnrt'.includes(text[i] ?? '')) fail();
      } else if (text.charCodeAt(i) < 0x20) fail();
      i++;
    }
    if (i >= n) fail();
    i++;
  };
  const num = () => {
    const m = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?/.exec(text.slice(i, i + 400));
    if (!m) fail();
    else i += m[0].length;
  };
  const value = (): void => {
    ws();
    const c = text[i];
    if (c === '{') {
      i++;
      ws();
      if (text[i] === '}') {
        i++;
        return;
      }
      for (;;) {
        ws();
        if (text[i] !== '"') fail();
        str();
        ws();
        if (text[i] !== ':') fail();
        i++;
        value();
        ws();
        if (text[i] === ',') {
          i++;
          continue;
        }
        if (text[i] === '}') {
          i++;
          return;
        }
        fail();
      }
    } else if (c === '[') {
      i++;
      ws();
      if (text[i] === ']') {
        i++;
        return;
      }
      for (;;) {
        value();
        ws();
        if (text[i] === ',') {
          i++;
          continue;
        }
        if (text[i] === ']') {
          i++;
          return;
        }
        fail();
      }
    } else if (c === '"') str();
    else if (c === 't') lit('true');
    else if (c === 'f') lit('false');
    else if (c === 'n') lit('null');
    else num();
  };
  try {
    value();
    ws();
    if (i < n) fail();
    return -1;
  } catch (pos) {
    return typeof pos === 'number' ? Math.min(pos, n) : 0;
  }
}

function jsonIssue(text: string): ValidationIssue | null {
  try {
    JSON.parse(text);
    return null;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const pos = jsonErrorPosition(msg, text);
    if (pos.line !== undefined) return { error: msg, ...pos };
    const off = jsonErrorOffset(text);
    return { error: msg, ...(off >= 0 ? lineColAt(text, off) : {}) };
  }
}

async function yamlIssue(text: string): Promise<ValidationIssue | null> {
  const YAML = await import('yaml');
  const docs = YAML.parseAllDocuments(text);
  for (const d of Array.isArray(docs) ? docs : [docs]) {
    const err = d.errors[0];
    if (err) {
      const lp = err.linePos?.[0];
      return { error: err.message.split('\n')[0], line: lp?.line, col: lp?.col };
    }
  }
  return null;
}

/** Light TOML sanity: every non-blank, non-comment line is a table header,
 *  a `key = value`, or a continuation of a multi-line array / string. */
function tomlIssue(text: string): ValidationIssue | null {
  const lines = text.split('\n');
  let depth = 0;
  let inMulti = false;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i].trim();
    const triples = (l.match(/"""|'''/g) ?? []).length;
    if (inMulti) {
      if (triples % 2 === 1) inMulti = false;
      continue;
    }
    if (!l || l.startsWith('#')) continue;
    if (depth > 0) {
      depth += (l.match(/\[/g) ?? []).length - (l.match(/\]/g) ?? []).length;
      continue;
    }
    if (/^\[\[?[^\]]+\]\]?\s*(#.*)?$/.test(l)) continue;
    const kv = /^[\w.\-"' ]+=\s*(.*)$/.exec(l);
    if (!kv) return { error: `expected "key = value" or [table]`, line: i + 1, col: 1 };
    if (triples % 2 === 1) inMulti = true;
    const v = kv[1];
    if (v.startsWith('[')) depth = Math.max(0, (v.match(/\[/g) ?? []).length - (v.match(/\]/g) ?? []).length);
  }
  return null;
}

function csvIssue(text: string, delimiter: string): ValidationIssue | null {
  // Unterminated quote: parse and check the quote balance per RFC 4180.
  let quoted = false;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '"') {
      if (quoted && text[i + 1] === '"') {
        i++;
        continue;
      }
      if (!quoted) start = i;
      quoted = !quoted;
    }
  }
  if (quoted) return { error: 'unterminated quoted field', ...lineColAt(text, start) };
  const rows = parseCsv(text, delimiter);
  const width = rows[0]?.length ?? 0;
  for (let r = 1; r < rows.length; r++) {
    if (rows[r].length !== width) {
      return { error: `row ${r + 1} has ${rows[r].length} fields, expected ${width}`, line: r + 1 };
    }
  }
  return null;
}

function markupIssue(text: string, mode: 'xml' | 'html'): ValidationIssue | null {
  const e = validateMarkup(text, mode);
  return e ? { error: e.message, line: e.line, col: e.col } : null;
}

/** Inline validation; null = valid (or not a validatable language). */
export async function validateContent(lang: string, text: string): Promise<ValidationIssue | null> {
  if (!text.trim()) return null;
  switch (lang) {
    case 'json':
      return jsonIssue(text);
    case 'yaml':
      return yamlIssue(text);
    case 'toml':
      return tomlIssue(text);
    case 'csv':
      return csvIssue(text, ',');
    case 'tsv':
      return csvIssue(text, '\t');
    case 'xml':
    case 'svg':
      return markupIssue(text, 'xml');
    default:
      return null;
  }
}

/** Whitespace-only normalisation: trailing spaces (Markdown hard breaks kept),
 *  leading tabs → spaces, ≤ 1 blank line between blocks (≤ 2 for code),
 *  CRLF → LF, one final newline when there was one. */
export function normalizeWhitespace(text: string, opts: { indent?: number; keepHardBreaks?: boolean; maxBlank?: number } = {}): string {
  const unit = ' '.repeat(opts.indent ?? 2);
  const maxBlank = opts.maxBlank ?? 2;
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  const out: string[] = [];
  let blank = 0;
  let fence = false;
  for (const raw of lines) {
    if (/^\s*(```|~~~)/.test(raw)) fence = !fence;
    let l = raw.replace(/^\t+/, (m) => unit.repeat(m.length));
    if (opts.keepHardBreaks && !fence && /\S {2}$/.test(l)) {
      l = l.replace(/ {3,}$/, '  ');
    } else {
      l = l.replace(/[ \t]+$/, '');
    }
    if (!l) {
      blank++;
      if (blank > maxBlank && !fence) continue;
    } else {
      blank = 0;
    }
    out.push(l);
  }
  while (out.length && out[0] === '') out.shift();
  return withEol(out.join('\n'), text);
}

function done(original: string, text: string): FormatResult {
  return { ok: true, text, changed: text !== original };
}

/** Format `text` as `lang`. Never throws: a parse error comes back as
 *  `{ ok: false, error, line?, col? }` and the caller leaves the doc as-is. */
export async function formatContent(
  lang: string,
  text: string,
  opts: { indent?: number; sqlDialect?: string } = {},
): Promise<FormatResult> {
  const indent = opts.indent ?? 2;
  try {
    switch (lang) {
      case 'json': {
        const issue = jsonIssue(text);
        if (issue) return { ok: false, ...issue };
        return done(text, withEol(JSON.stringify(JSON.parse(text), null, indent), text));
      }
      case 'yaml': {
        const issue = await yamlIssue(text);
        if (issue) return { ok: false, ...issue };
        const YAML = await import('yaml');
        const docs = YAML.parseAllDocuments(text);
        const list = Array.isArray(docs) ? docs : [docs];
        const out = list
          .map((d) => d.toString({ indent, lineWidth: 0 }).replace(/\n+$/, ''))
          .join('\n---\n');
        return done(text, withEol(out, text));
      }
      case 'sql': {
        const { format } = await import('sql-formatter');
        const d = (opts.sqlDialect ?? 'sql').toLowerCase();
        const language = (SQL_DIALECTS.has(d) ? d : (DIALECT_ALIAS[d] ?? 'sql')) as 'sql';
        const { masked, tokens } = maskQueryPlaceholders(text);
        const out = format(masked, { language, keywordCase: 'upper', tabWidth: indent });
        return done(text, withEol(unmaskQueryPlaceholders(out, tokens), text));
      }
      case 'html':
        return done(text, withEol(formatMarkup(text, 'html', indent), text));
      case 'xml':
      case 'svg':
        return done(text, withEol(formatMarkup(text, 'xml', indent), text));
      case 'csv':
      case 'tsv': {
        const d = lang === 'tsv' ? '\t' : ',';
        const issue = csvIssue(text, d);
        if (issue && /unterminated/.test(issue.error)) return { ok: false, ...issue };
        return done(text, withEol(stringifyCsv(parseCsv(text, d), d), text));
      }
      case 'md':
        return done(text, normalizeWhitespace(text, { indent, keepHardBreaks: true, maxBlank: 1 }));
      case 'toml': {
        const issue = tomlIssue(text);
        if (issue) return { ok: false, ...issue };
        const norm = normalizeWhitespace(text, { indent, maxBlank: 1 });
        // One blank line before every [table] header (except at the top).
        const out = norm.replace(/\n+(?=\[)/g, '\n\n');
        return done(text, out);
      }
      default:
        if (!canFormat(lang)) return { ok: false, error: `No formatter for ${lang}` };
        return done(text, normalizeWhitespace(text, { indent, maxBlank: 2 }));
    }
  } catch (e) {
    if (e instanceof MarkupError) return { ok: false, error: e.message, line: e.line, col: e.col };
    const msg = e instanceof Error ? e.message : String(e);
    const lc = /line (\d+)(?:,? col(?:umn)? (\d+))?/i.exec(msg);
    return { ok: false, error: msg.split('\n')[0], line: lc ? Number(lc[1]) : undefined, col: lc?.[2] ? Number(lc[2]) : undefined };
  }
}
