// On-demand StreamLanguages for configuration files (each loads with its file type, see
// cm-langs.ts; the file → language table is langForPath.ts): Java-style .properties (a
// value that is JSON is highlighted as JSON), YAML, TOML, INI / conf / .env and the generic
// "key = value" fallback, ignore files (.gitignore, .gitattributes, CODEOWNERS) and pip
// requirements. States are a few flat fields (StreamLanguage copies them once per line) and
// no token scans past its own text, so a 4 KB line costs what its characters cost.
import { foldService, StreamLanguage, type StringStream } from '@codemirror/language';
import type { EditorState, Extension } from '@codemirror/state';

const NUMBER = /^[+-]?(?:0x[\da-fA-F_]+|0o[0-7_]+|0b[01_]+|(?:\d[\d_]*)?\.?\d[\d_]*(?:[eE][+-]?\d+)?)$/;
const PLACEHOLDER = /^\$\{[^}\s]*\}?|^\$[A-Za-z_]\w*|^%\([\w.-]+\)[sd]|^@[\w.-]+@/;

/** A plain scalar's style: numbers and the well-known words apart from text. */
function scalarStyle(word: string, atoms: RegExp): string {
  if (NUMBER.test(word)) return 'number';
  return atoms.test(word) ? 'atom' : 'string';
}

// ── JSON inside a value ─────────────────────────────────────────────────────────────────

/** Open JSON containers as a string ('{' / '['; immutable, so copies are free) and whether
 *  the next string in the innermost object is a key. */
export interface JsonScan { stack: string; expectKey: boolean; inStr: boolean; strIsKey: boolean }

/** One JSON token; the value is closed when `stack` is empty again. */
export function jsonToken(stream: StringStream, st: JsonScan): string | null {
  if (st.inStr) {
    while (!stream.eol()) {
      const ch = stream.next();
      if (ch === '\\') stream.next();
      else if (ch === '"') { st.inStr = false; break; }
    }
    return st.strIsKey ? 'propertyName' : 'string';
  }
  if (stream.eatSpace()) return null;
  const ch = stream.next() ?? '';
  if (ch === '{' || ch === '[') { st.stack += ch; st.expectKey = ch === '{'; return 'punctuation'; }
  if (ch === '}' || ch === ']') { st.stack = st.stack.slice(0, -1); st.expectKey = false; return 'punctuation'; }
  if (ch === ',') { st.expectKey = st.stack.endsWith('{'); return 'punctuation'; }
  if (ch === ':') { st.expectKey = false; return 'punctuation'; }
  if (ch === '"') {
    st.strIsKey = st.expectKey && st.stack.endsWith('{');
    st.inStr = true;
    return jsonToken(stream, st);
  }
  stream.backUp(1);
  if (stream.match(/^(?:true|false|null)\b/)) return 'atom';
  if (stream.match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/)) return 'number';
  if (stream.match(PLACEHOLDER)) return 'labelName';
  stream.next();
  return null;
}

// ── .properties ─────────────────────────────────────────────────────────────────────────

export interface PropsState extends JsonScan {
  phase: 'key' | 'sep' | 'value' | 'json';
  /** The previous line ended in an odd number of backslashes: this one continues it. */
  cont: boolean;
  /** The value's first character is still to come (a { or [ there starts JSON). */
  valueStart: boolean;
}

const oddTrailingBackslashes = (line: string): boolean => {
  let n = 0;
  for (let i = line.length - 1; i >= 0 && line[i] === '\\'; i--) n++;
  return n % 2 === 1;
};

/** Past this column a value is one token: a 4 KB line of JSON is ~400 spans, and a screen
 *  of them took 110-150 ms a frame to lay out on every scroll (800 columns: 70-93 ms). The
 *  first 300 characters (two screen widths) keep their colours. */
const DETAIL_COLUMNS = 300;

function propsToken(stream: StringStream, s: PropsState): string | null {
  if (stream.sol()) {
    const cont = s.cont;
    s.cont = oddTrailingBackslashes(stream.string);
    if (!cont) {
      s.phase = 'key';
      s.stack = '';
      s.inStr = false;
    }
    if (stream.eatSpace()) return null;
    if (!cont && stream.match(/^[#!].*/)) {
      s.cont = false; // a comment never continues
      return 'comment';
    }
  }
  if (s.phase === 'key') {
    if (stream.match(/^(?:\\.|[^\s=:\\])+/)) { s.phase = 'sep'; return 'propertyName'; }
    if (stream.match(/^\\$/)) return 'escape';
    s.phase = 'sep';
  }
  if (s.phase === 'sep') {
    if (stream.eatSpace()) return null;
    s.phase = 'value';
    s.valueStart = true;
    if (stream.match(/^[=:]/)) return 'operator';
  }
  if (s.valueStart) {
    if (stream.eatSpace()) return null;
    s.valueStart = false;
    if (stream.peek() === '{' || stream.peek() === '[') { s.phase = 'json'; s.stack = ''; s.inStr = false; }
  }
  if (stream.match(/^\\$/)) return 'escape';
  if (stream.pos >= DETAIL_COLUMNS) {
    stream.skipToEnd();
    if (s.cont && stream.string.endsWith('\\')) stream.backUp(1);
    s.phase = 'value';
    s.stack = '';
    s.inStr = false;
    return 'string';
  }
  if (s.phase === 'json') {
    const style = jsonToken(stream, s);
    if (!s.stack && !s.inStr) s.phase = 'value';
    return style;
  }
  if (stream.match(/^\\(?:u[\da-fA-F]{4}|.)/)) return 'escape';
  if (stream.match(PLACEHOLDER)) return 'labelName';
  stream.next();
  while (!stream.eol() && !/[\\$%@]/.test(stream.peek() ?? '')) stream.next();
  return 'string';
}

export const propertiesParser = {
  name: 'properties',
  startState: (): PropsState => ({ phase: 'key', cont: false, valueStart: false, stack: '', expectKey: false, inStr: false, strIsKey: false }),
  token: propsToken,
  blankLine: (s: PropsState): void => { s.cont = false; },
  languageData: { commentTokens: { line: '#' } },
};

// ── YAML ────────────────────────────────────────────────────────────────────────────────

export interface YamlState {
  /** In a block scalar (| or >): the indentation of the line that opened it, else -1. */
  block: number;
  /** This line's indentation (block scalars end at a line not indented past it). */
  indent: number;
  /** Depth of open [ / { flow collections. */
  flow: number;
  /** In a quoted scalar that runs on: its quote. */
  quote: string;
  /** A mapping key was read on this line (a later ": " is text). */
  keyed: boolean;
}

const YAML_ATOMS = /^(?:true|false|yes|no|on|off|null|~|True|False|TRUE|FALSE|Null|NULL|\.inf|\.nan)$/;

function yamlQuoted(stream: StringStream, s: YamlState): string {
  while (!stream.eol()) {
    const ch = stream.next();
    if (s.quote === '"' && ch === '\\') stream.next();
    else if (ch === s.quote) {
      if (s.quote === "'" && stream.eat("'")) continue;
      s.quote = '';
      break;
    }
  }
  return 'string';
}

function yamlToken(stream: StringStream, s: YamlState): string | null {
  if (stream.sol()) {
    s.keyed = false;
    s.indent = stream.indentation();
    if (s.block >= 0) {
      if (s.indent > s.block || !stream.string.trim()) { stream.skipToEnd(); return 'string'; }
      s.block = -1;
    }
  }
  if (s.quote) return yamlQuoted(stream, s);
  if (stream.eatSpace()) return null;
  if (stream.sol() || stream.column() === 0) {
    if (stream.match(/^(?:---|\.\.\.)(?=\s|$)/)) return 'meta';
    if (stream.match(/^%(?:YAML|TAG)\b.*/)) return 'meta';
  }
  const ch = stream.peek() ?? '';
  const prev = stream.pos > 0 ? stream.string[stream.pos - 1] : ' ';
  if (ch === '#' && /\s/.test(prev)) { stream.skipToEnd(); return 'comment'; }
  if (ch === '"' || ch === "'") {
    stream.next();
    s.quote = ch;
    const style = yamlQuoted(stream, s);
    if (!s.keyed && !s.quote && stream.match(/^\s*:(?=\s|$)/, false)) { s.keyed = true; return 'propertyName'; }
    return style;
  }
  if (stream.match(/^[-?](?=\s|$)/)) return 'punctuation';
  if (stream.match(/^[[{]/)) { s.flow++; return 'punctuation'; }
  if (stream.match(/^[\]}]/)) { s.flow = Math.max(0, s.flow - 1); return 'punctuation'; }
  if (stream.match(/^,/)) { s.keyed = false; return 'punctuation'; }
  if (stream.match(/^:(?=\s|$)/) || (s.flow && stream.match(/^:/))) return 'punctuation';
  if (stream.match(/^&[^\s,[\]{}]+/) || stream.match(/^\*[^\s,[\]{}]+/)) return 'labelName';
  if (stream.match(/^!!?[\w./:-]*(?:<[^>]*>)?/)) return 'meta';
  if (stream.match(/^[|>][-+]?\d*(?=\s*(?:#|$))/)) { s.block = s.indent; return 'operator'; }
  // A plain scalar: up to " #", ": " (a key), the line end, or a flow indicator in [ ] / { }.
  const start = stream.pos;
  while (!stream.eol()) {
    const c = stream.peek() ?? '';
    if (c === '#' && stream.pos > start && /\s/.test(stream.string[stream.pos - 1])) break;
    if (c === ':' && (s.flow ? true : /\s/.test(stream.string[stream.pos + 1] ?? ' '))) break;
    if (s.flow && /[,[\]{}]/.test(c)) break;
    stream.next();
  }
  if (stream.pos === start) { stream.next(); return null; }
  const text = stream.current();
  const trimmed = text.replace(/\s+$/, '');
  stream.backUp(text.length - trimmed.length);
  if (!s.keyed && (stream.match(/^\s*:(?=\s|$)/, false) || (s.flow > 0 && stream.match(/^\s*:/, false)))) {
    s.keyed = true;
    return 'propertyName';
  }
  return scalarStyle(trimmed, YAML_ATOMS);
}

export const yamlParser = {
  name: 'yaml',
  startState: (): YamlState => ({ block: -1, indent: 0, flow: 0, quote: '', keyed: false }),
  token: yamlToken,
  languageData: { commentTokens: { line: '#' } },
};

// ── TOML ────────────────────────────────────────────────────────────────────────────────

export interface TomlState {
  /** In a multi-line string: its delimiter (""" or '''). */
  ml: string;
  /** Open [ / { as a string (arrays and inline tables run on across lines). */
  stack: string;
  /** The next bare word is a key (line start, or after { or , in an inline table). */
  expectKey: boolean;
}

const TOML_KEY = /^(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*"|'[^']*')(?:\s*\.\s*(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*"|'[^']*'))*(?=\s*=)/;
const TOML_DATE = /^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:\d{2})?)?|^\d{2}:\d{2}:\d{2}(?:\.\d+)?/;

function tomlToken(stream: StringStream, s: TomlState): string | null {
  if (s.ml) {
    while (!stream.eol()) {
      if (stream.match(s.ml)) { s.ml = ''; break; }
      if (stream.next() === '\\' && s.ml === '"""') stream.next();
    }
    return 'string';
  }
  if (stream.sol() && !s.stack) s.expectKey = true;
  if (stream.eatSpace()) return null;
  if (stream.match(/^#.*/)) return 'comment';
  if (!s.stack && s.expectKey && stream.match(/^\[\[?\s*[^\]#]+\]\]?/)) return 'typeName';
  if (s.expectKey && stream.match(TOML_KEY)) { s.expectKey = false; return 'propertyName'; }
  if (stream.match(/^=/)) { s.expectKey = false; return 'operator'; }
  if (stream.match(/^(?:"""|''')/)) { s.ml = stream.current(); return tomlToken(stream, s) ?? 'string'; }
  if (stream.match(/^"(?:[^"\\]|\\.)*"?/) || stream.match(/^'[^']*'?/)) return 'string';
  if (stream.match(TOML_DATE)) return 'number';
  if (stream.match(/^[+-]?(?:inf|nan)\b/) || stream.match(/^[+-]?(?:0x[\da-fA-F_]+|0o[0-7_]+|0b[01_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)\b/)) return 'number';
  if (stream.match(/^(?:true|false)\b/)) return 'atom';
  const ch = stream.next() ?? '';
  if (ch === '[' || ch === '{') { s.stack += ch; s.expectKey = ch === '{'; return 'punctuation'; }
  if (ch === ']' || ch === '}') { s.stack = s.stack.slice(0, -1); return 'punctuation'; }
  if (ch === ',') { s.expectKey = s.stack.endsWith('{'); return 'punctuation'; }
  return null;
}

export const tomlParser = {
  name: 'toml',
  startState: (): TomlState => ({ ml: '', stack: '', expectKey: true }),
  token: tomlToken,
  languageData: { commentTokens: { line: '#' } },
};

// ── INI / conf / .env and the generic key = value fallback ─────────────────────────────

export interface ConfState {
  /** 'key' at a line start, then 'value' after the separator (or the first word). */
  phase: 'key' | 'value';
  /** A key was read: its = or : comes next. */
  sep: boolean;
  cont: boolean;
  /** .env: "export KEY=…" */
  dotenv: boolean;
}

const CONF_ATOMS = /^(?:true|false|yes|no|on|off|null|none|enabled|disabled)$/i;

function confToken(stream: StringStream, s: ConfState): string | null {
  if (stream.sol()) {
    const cont = s.cont;
    s.cont = /\\$/.test(stream.string);
    if (!cont) { s.phase = 'key'; s.sep = false; }
    if (stream.eatSpace()) return null;
    if (!cont && stream.match(/^[#;].*/)) { s.cont = false; return 'comment'; }
    if (!cont && stream.match(/^\[[^\]]*\]?/)) { s.phase = 'value'; return 'typeName'; }
  }
  if (stream.eatSpace()) return null;
  if (/[#;]/.test(stream.peek() ?? '') && /\s/.test(stream.string[stream.pos - 1] ?? '')) { stream.skipToEnd(); return 'comment'; }
  if (s.phase === 'key') {
    if (s.dotenv && stream.match(/^export(?=\s)/)) return 'keyword';
    s.phase = 'value';
    // key = value, key: value (a key may hold dots, slashes, @, and :), else "Directive value".
    if (stream.match(/^[^\s=#;"']+?(?=\s*=)/) || stream.match(/^[^\s:=#;"']+(?=\s*:(?:\s|$))/)) { s.sep = true; return 'propertyName'; }
    if (stream.match(/^\d{1,3}(?:\.\d{1,3}){3}(?=\s)|^[0-9a-fA-F:]*::[0-9a-fA-F:]*(?=\s)/)) return 'number';
    if (stream.match(/^[A-Za-z_][\w.-]*(?=\s+\S)/)) return 'propertyName';
  }
  if (s.sep && stream.match(/^[=:]/)) { s.sep = false; return 'operator'; }
  if (stream.match(/^"(?:[^"\\]|\\.)*"?/) || stream.match(/^'[^']*'?/)) return 'string';
  if (stream.match(PLACEHOLDER)) return 'labelName';
  if (stream.match(/^\\$/)) return 'escape';
  if (stream.match(/^[^\s"'$%@\\]+/)) return scalarStyle(stream.current(), CONF_ATOMS);
  stream.next();
  return null;
}

const confParser = (dotenv: boolean) => ({
  name: dotenv ? 'dotenv' : 'conf',
  startState: (): ConfState => ({ phase: 'key', sep: false, cont: false, dotenv }),
  token: confToken,
  languageData: { commentTokens: { line: '#' } },
});
export const confParserConf = confParser(false);
export const confParserDotenv = confParser(true);

// ── Ignore files, .gitattributes, CODEOWNERS ────────────────────────────────────────────

function ignoreToken(stream: StringStream, st: { rest: boolean }): string | null {
  if (stream.sol()) {
    st.rest = false;
    if (stream.match(/^\s*#.*/)) return 'comment';
    if (stream.eat('!')) return 'keyword';
  }
  if (stream.eatSpace()) { st.rest = true; return null; }
  if (st.rest) {
    // After the pattern: gitattributes' attributes, CODEOWNERS' owners.
    if (stream.match(/^@[\w./-]+/) || stream.match(/^[\w.+-]+@[\w.-]+/)) return 'labelName';
    if (stream.match(/^[-!]?[\w.-]+/)) return 'attributeName';
    if (stream.eat('=')) return 'operator';
    if (stream.match(/^[^\s]+/)) return 'string';
  }
  if (stream.match(/^\*\*|^[*?]/)) return 'operator';
  if (stream.match(/^\[[^\]]*\]?/)) return 'string.special';
  if (stream.eat('/')) return 'punctuation';
  if (stream.match(/^\\./)) return 'escape';
  if (stream.match(/^[^\s*?[/\\]+/)) return 'string';
  stream.next();
  return null;
}

export const ignoreParser = { name: 'ignore', startState: () => ({ rest: false }), token: ignoreToken, languageData: { commentTokens: { line: '#' } } };

// ── pip requirements ────────────────────────────────────────────────────────────────────

function requirementsToken(stream: StringStream, st: { phase: 'name' | 'spec' | 'marker' | 'option' }): string | null {
  if (stream.sol()) st.phase = 'name';
  if (stream.eatSpace()) return null;
  if (stream.match(/^#.*/)) return 'comment';
  if (stream.sol() || stream.start === stream.string.search(/\S/)) {
    if (stream.match(/^--?[\w-]+/)) { st.phase = 'option'; return 'attributeName'; }
  }
  if (st.phase === 'option') { stream.match(/^\S+/) || stream.next(); return 'string'; }
  if (st.phase === 'name' && stream.match(/^[A-Za-z0-9][\w.-]*/)) { st.phase = 'spec'; return 'propertyName'; }
  if (stream.match(/^\[[^\]]*\]/)) return 'typeName';
  if (stream.match(/^(?:===|==|!=|~=|>=|<=|>|<)/)) return 'operator';
  if (stream.eat(';')) { st.phase = 'marker'; return 'operator'; }
  if (stream.eat('@')) { stream.match(/^\s*\S+/); return 'string'; }
  if (st.phase === 'marker' && stream.match(/^(?:and|or|in|not)\b/)) return 'keyword';
  if (stream.match(/^"[^"]*"?|^'[^']*'?/)) return 'string';
  if (stream.match(/^[\w.*+!-]+/)) return st.phase === 'marker' ? 'variableName' : 'number';
  if (stream.eat(',')) return 'punctuation';
  stream.next();
  return null;
}

export const requirementsParser = {
  name: 'requirements',
  startState: () => ({ phase: 'name' as 'name' | 'spec' | 'marker' | 'option' }),
  token: requirementsToken,
  languageData: { commentTokens: { line: '#' } },
};

// ── Folding ─────────────────────────────────────────────────────────────────────────────

/** Each line's fold end (1-based line numbers; 0 = not foldable), computed once per
 *  document version in one pass: the fold gutter asks about every visible line on each
 *  update, and a per-line scan to the block's end would make that quadratic. */
function foldEnds(state: EditorState, cache: WeakMap<object, Int32Array>, fill: (lines: string[], ends: Int32Array) => void): Int32Array {
  let ends = cache.get(state.doc);
  if (!ends) {
    const lines: string[] = [];
    for (const it = state.doc.iterLines(); !it.next().done; ) lines.push(it.value);
    ends = new Int32Array(lines.length + 2);
    fill(lines, ends);
    cache.set(state.doc, ends);
  }
  return ends;
}

function foldAt(ends: Int32Array, state: EditorState, from: number): { from: number; to: number } | null {
  const line = state.doc.lineAt(from), end = ends[line.number];
  return end > line.number ? { from: line.to, to: state.doc.line(end).to } : null;
}

const indentCache = new WeakMap<object, Int32Array>();
/** A line whose next non-blank line is indented deeper folds to the end of that block
 *  (YAML mappings and lists; HCL, proto and nginx blocks in cm-langs-infra.ts). */
export const indentFold = foldService.of((state, from) => foldAt(foldEnds(state, indentCache, (lines, ends) => {
  const open: { line: number; indent: number }[] = [];
  let lastText = 0;
  lines.forEach((t, i) => {
    if (!t.trim()) return;
    const n = i + 1, indent = t.length - t.trimStart().length;
    while (open.length && open[open.length - 1].indent >= indent) ends[open.pop()!.line] = lastText;
    open.push({ line: n, indent });
    lastText = n;
  });
  for (const o of open) ends[o.line] = lastText;
}), state, from));

const HEADER = /^\s*\[[^\]]+\]\]?\s*(?:[#;].*)?$/;
const sectionCache = new WeakMap<object, Int32Array>();
/** A [section] / [table] header folds to the last line before the next one (TOML, INI). */
export const sectionFold = foldService.of((state, from) => foldAt(foldEnds(state, sectionCache, (lines, ends) => {
  let header = 0, lastText = 0;
  lines.forEach((t, i) => {
    if (HEADER.test(t)) { if (header) ends[header] = lastText; header = i + 1; lastText = i + 1; }
    else if (t.trim()) lastText = i + 1;
  });
  if (header) ends[header] = lastText;
}), state, from));

// Defined once: a new Language per call would re-parse the file on every reconfigure.
const languages = {
  properties: StreamLanguage.define(propertiesParser),
  yaml: StreamLanguage.define(yamlParser),
  toml: StreamLanguage.define(tomlParser),
  conf: StreamLanguage.define(confParserConf),
  dotenv: StreamLanguage.define(confParserDotenv),
  ignore: StreamLanguage.define(ignoreParser),
  requirements: StreamLanguage.define(requirementsParser),
};
export const propertiesLang = (): Extension => languages.properties;
export const yamlLang = (): Extension => [languages.yaml, indentFold];
export const tomlLang = (): Extension => [languages.toml, sectionFold];
export const confLang = (): Extension => [languages.conf, sectionFold];
export const dotenvLang = (): Extension => languages.dotenv;
export const ignoreLang = (): Extension => languages.ignore;
export const requirementsLang = (): Extension => languages.requirements;
