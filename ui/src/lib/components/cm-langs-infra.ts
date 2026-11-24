// On-demand StreamLanguages for infrastructure files (each loads with its file type, see
// cm-langs.ts; the file → language table is langForPath.ts): Dockerfile / Containerfile
// (RUN / CMD / ENTRYPOINT bodies as shell), nginx, Protocol Buffers, HCL / Terraform,
// unified diffs, PowerShell, Windows batch files and WebVTT subtitles.
import { StreamLanguage, type StringStream } from '@codemirror/language';
import type { Extension } from '@codemirror/state';
import { shellToken } from './cm-langs-extra.ts';
import { indentFold } from './cm-langs-config.ts';

// ── Dockerfile ──────────────────────────────────────────────────────────────────────────

const DOCKER_INSTR = /^(?:FROM|RUN|CMD|LABEL|MAINTAINER|EXPOSE|ENV|ADD|COPY|ENTRYPOINT|VOLUME|USER|WORKDIR|ARG|ONBUILD|STOPSIGNAL|HEALTHCHECK|SHELL)(?=\s|$)/i;

export interface DockerState {
  /** What the current instruction's arguments are. */
  kind: '' | 'shell' | 'kv' | 'from' | 'args';
  cont: boolean;
  /** FROM … AS: the next word names the stage. */
  stage: boolean;
  /** Shell scan state (cm-langs-extra.ts shellToken), kept flat so a line copy is cheap. */
  inDoubleQuote: boolean;
  expectCmd: boolean;
}

function dockerToken(stream: StringStream, s: DockerState): string | null {
  if (stream.sol()) {
    const cont = s.cont;
    s.cont = /\\$/.test(stream.string);
    if (!cont) s.kind = '';
    if (stream.eatSpace()) return null;
    if (!cont && stream.match(/^#\s*(?:syntax|escape|check)\s*=.*/i)) return 'meta';
    if (stream.match(/^#.*/)) { s.cont = cont; return 'comment'; }
  }
  if (stream.eatSpace()) return null;
  if (!s.kind) {
    if (stream.match(DOCKER_INSTR)) {
      const w = stream.current().toUpperCase();
      s.kind = w === 'RUN' || w === 'CMD' || w === 'ENTRYPOINT' || w === 'SHELL' ? 'shell'
        : w === 'ENV' || w === 'ARG' || w === 'LABEL' ? 'kv' : w === 'FROM' ? 'from' : 'args';
      s.inDoubleQuote = false;
      s.expectCmd = true;
      s.stage = false;
      return 'keyword';
    }
    s.kind = 'args';
  }
  if (stream.match(/^--[\w-]+(?:=\S*)?/)) return 'attributeName';
  if (stream.match(/^\\$/)) return 'escape';
  if (s.kind === 'shell') {
    if (stream.match(/^\[(?=\s*")/)) return 'punctuation'; // the exec form ["executable", "arg"]
    return shellToken(stream, s);
  }
  if (stream.match(/^\$\{[^}]*\}?|^\$\w+/)) return 'variableName.special';
  if (stream.match(/^"(?:[^"\\]|\\.)*"?|^'[^']*'?/)) return 'string';
  if (s.kind === 'kv' && stream.match(/^[\w.-]+(?==)/)) return 'propertyName';
  if (stream.eat('=')) return 'operator';
  if (s.kind === 'from' && stream.match(/^as(?=\s)/i)) { s.stage = true; return 'keyword'; }
  if (s.stage && stream.match(/^[\w.-]+/)) { s.stage = false; return 'labelName'; }
  if (stream.match(/^\d+(?:\/(?:tcp|udp))?(?=\s|$)/)) return 'number';
  if (stream.match(/^[^\s$"'=\\]+/)) return 'string';
  stream.next();
  return null;
}

export const dockerfileParser = {
  name: 'dockerfile',
  startState: (): DockerState => ({ kind: '', cont: false, stage: false, inDoubleQuote: false, expectCmd: true }),
  token: dockerToken,
  languageData: { commentTokens: { line: '#' } },
};

// ── nginx ───────────────────────────────────────────────────────────────────────────────

function nginxToken(stream: StringStream, s: { directive: boolean }): string | null {
  if (stream.eatSpace()) return null;
  if (stream.match(/^#.*/)) return 'comment';
  if (stream.match(/^"(?:[^"\\]|\\.)*"?|^'(?:[^'\\]|\\.)*'?/)) { s.directive = false; return 'string'; }
  if (stream.match(/^\$\{?\w+\}?/)) { s.directive = false; return 'variableName.special'; }
  const ch = stream.peek() ?? '';
  if (ch === '{' || ch === '}' || ch === ';') { stream.next(); s.directive = true; return 'punctuation'; }
  if (s.directive && stream.match(/^[A-Za-z_][\w.-]*/)) { s.directive = false; return 'keyword'; }
  if (stream.match(/^(?:~\*?|\^~|=)(?=\s)/)) return 'operator';
  if (stream.match(/^\d+(?:\.\d+)?[kKmMgGsShHdDwWyY]?(?=[\s;]|$)/)) return 'number';
  if (stream.match(/^(?:on|off)(?=[\s;])/)) return 'atom';
  if (stream.match(/^[^\s;{}"'$#]+/)) return 'string';
  stream.next();
  return null;
}

export const nginxParser = { name: 'nginx', startState: () => ({ directive: true }), token: nginxToken, languageData: { commentTokens: { line: '#' } } };

// ── Protocol Buffers ────────────────────────────────────────────────────────────────────

const PROTO_KW = new Set(['syntax', 'edition', 'package', 'import', 'option', 'message', 'enum', 'service', 'rpc', 'returns', 'stream',
  'repeated', 'optional', 'required', 'oneof', 'map', 'reserved', 'extend', 'extensions', 'to', 'max', 'weak', 'public', 'group']);
const PROTO_TYPES = new Set(['double', 'float', 'int32', 'int64', 'uint32', 'uint64', 'sint32', 'sint64', 'fixed32', 'fixed64',
  'sfixed32', 'sfixed64', 'bool', 'string', 'bytes']);

function protoToken(stream: StringStream, s: { comment: boolean; next: string }): string | null {
  if (s.comment) {
    if (stream.skipTo('*/')) { stream.match('*/'); s.comment = false; } else stream.skipToEnd();
    return 'comment';
  }
  if (stream.eatSpace()) return null;
  if (stream.match(/^\/\/.*/)) return 'comment';
  if (stream.match('/*')) { s.comment = true; return protoToken(stream, s) ?? 'comment'; }
  if (stream.match(/^"(?:[^"\\]|\\.)*"?|^'(?:[^'\\]|\\.)*'?/)) return 'string';
  if (stream.match(/^-?(?:0x[\da-fA-F]+|\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b/)) return 'number';
  if (stream.match(/^[A-Za-z_][\w.]*/)) {
    const w = stream.current();
    if (s.next) {
      const kind = s.next;
      s.next = '';
      return kind === 'rpc' ? 'variableName.function.definition' : 'typeName';
    }
    if (PROTO_KW.has(w)) {
      if (w === 'message' || w === 'enum' || w === 'service' || w === 'rpc' || w === 'extend') s.next = w;
      return 'keyword';
    }
    if (PROTO_TYPES.has(w)) return 'typeName';
    if (w === 'true' || w === 'false') return 'atom';
    if (stream.match(/^\s*=/, false)) return 'propertyName';
    return /^[A-Z]/.test(w.slice(w.lastIndexOf('.') + 1)) ? 'typeName' : 'variableName';
  }
  if (stream.match(/^[{}()[\];,<>]/)) return 'punctuation';
  if (stream.eat('=')) return 'operator';
  stream.next();
  return null;
}

export const protoParser = {
  name: 'proto',
  startState: () => ({ comment: false, next: '' }),
  token: protoToken,
  languageData: { commentTokens: { line: '//', block: { open: '/*', close: '*/' } } },
};

// ── HCL / Terraform ─────────────────────────────────────────────────────────────────────

export interface HclState {
  comment: boolean;
  /** In a "…" string; interp counts the open interpolations (dollar-brace … brace) in it. */
  str: boolean;
  interp: number;
  /** In a heredoc: its closing word. */
  heredoc: string;
}

const HCL_REFS = /^(?:var|local|module|data|each|count|path|terraform|self)(?=\.)/;

function hclToken(stream: StringStream, s: HclState): string | null {
  if (s.heredoc) {
    if (stream.sol() && stream.string.trim() === s.heredoc) { stream.skipToEnd(); s.heredoc = ''; return 'operator'; }
    stream.skipToEnd();
    return 'string';
  }
  if (s.comment) {
    if (stream.skipTo('*/')) { stream.match('*/'); s.comment = false; } else stream.skipToEnd();
    return 'comment';
  }
  if (s.str && !s.interp) {
    if (stream.match(/^\$\{/)) { s.interp = 1; return 'escape'; }
    while (!stream.eol()) {
      if (stream.match(/^\$\{/, false)) break;
      const ch = stream.next();
      if (ch === '\\') stream.next();
      else if (ch === '"') { s.str = false; break; }
    }
    return 'string';
  }
  if (stream.eatSpace()) return null;
  if (s.interp) {
    if (stream.eat('}')) { s.interp--; return s.interp ? 'punctuation' : 'escape'; }
    if (stream.eat('{')) { s.interp++; return 'punctuation'; }
  }
  if (stream.match(/^(?:#|\/\/).*/)) return 'comment';
  if (stream.match('/*')) { s.comment = true; return hclToken(stream, s) ?? 'comment'; }
  const heredoc = stream.match(/^<<-?([A-Za-z_]\w*)/);
  if (heredoc && typeof heredoc === 'object') { s.heredoc = heredoc[1]; return 'operator'; }
  if (!s.interp && stream.eat('"')) { s.str = true; return hclToken(stream, s) ?? 'string'; }
  if (stream.match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/)) return 'number';
  if (stream.match(/^(?:true|false|null)\b/)) return 'atom';
  if (stream.match(HCL_REFS)) return 'keyword';
  if (stream.match(/^[A-Za-z_][\w-]*/)) {
    if (stream.match(/^\s*\(/, false)) return 'variableName.function';
    if (stream.match(/^\s*=(?!=)/, false)) return 'propertyName';
    // A block: its type, then optional "labels", then {.
    if (stream.start === stream.indentation() && stream.match(/^(?:\s+(?:"[^"]*"|[A-Za-z_][\w-]*))*\s*\{/, false)) return 'keyword';
    return 'variableName';
  }
  if (stream.match(/^(?:==|!=|<=|>=|&&|\|\||=>|[=!<>?:+\-*/%])/)) return 'operator';
  if (stream.match(/^[{}()[\],.]/)) return 'punctuation';
  stream.next();
  return null;
}

export const hclParser = {
  name: 'hcl',
  startState: (): HclState => ({ comment: false, str: false, interp: 0, heredoc: '' }),
  token: hclToken,
  languageData: { commentTokens: { line: '#', block: { open: '/*', close: '*/' } } },
};

// ── Unified diff / patch ────────────────────────────────────────────────────────────────

function diffToken(stream: StringStream): string | null {
  const line = stream.string;
  if (stream.pos === 0) {
    if (/^(?:diff |index |similarity |rename |new file|deleted file|old mode|new mode|Binary files)/.test(line)) { stream.skipToEnd(); return 'meta'; }
    if (/^(?:\+\+\+|---)(?: |$)/.test(line)) { stream.skipToEnd(); return 'heading'; }
    if (stream.match(/^@@[^@]*@@/)) return 'typeName';
    if (line[0] === '+') { stream.skipToEnd(); return 'inserted'; }
    if (line[0] === '-') { stream.skipToEnd(); return 'deleted'; }
    if (line[0] === '\\') { stream.skipToEnd(); return 'comment'; }
  }
  const after = stream.pos > 0; // the text after a hunk header names the function
  stream.skipToEnd();
  return after ? 'comment' : null;
}

export const diffParser = { name: 'diff', startState: () => ({}), token: diffToken };

// ── PowerShell ──────────────────────────────────────────────────────────────────────────

const PS_KW = new Set(['if', 'else', 'elseif', 'switch', 'foreach', 'for', 'while', 'do', 'until', 'function', 'filter', 'param',
  'begin', 'process', 'end', 'return', 'try', 'catch', 'finally', 'throw', 'trap', 'break', 'continue', 'exit', 'class', 'enum',
  'using', 'in', 'dynamicparam', 'workflow', 'parallel', 'sequence', 'inlinescript', 'data']);
const PS_OPS = /^-(?:eq|ne|gt|ge|lt|le|like|notlike|match|notmatch|contains|notcontains|in|notin|replace|split|join|and|or|not|xor|is|isnot|as|band|bor|f)\b/i;
/** "…" (the backtick, U+0060, escapes in it) or '…' ('' escapes). */
const PS_STRING = /^"(?:[^"\u0060]|\u0060.)*"?|^'(?:[^']|'')*'?/;

function psToken(stream: StringStream, s: { comment: boolean }): string | null {
  if (s.comment) {
    if (stream.skipTo('#>')) { stream.match('#>'); s.comment = false; } else stream.skipToEnd();
    return 'comment';
  }
  if (stream.eatSpace()) return null;
  if (stream.match('<#')) { s.comment = true; return psToken(stream, s) ?? 'comment'; }
  if (stream.match(/^#.*/)) return 'comment';
  if (stream.match(/^\$(?:true|false|null)\b/i)) return 'atom';
  if (stream.match(/^\$(?:\{[^}]*\}|[\w:?]+)/)) return 'variableName.special';
  if (stream.match(PS_STRING)) return 'string';
  if (stream.match(/^\[[A-Za-z_][\w.[\]]*\]/)) return 'typeName';
  if (stream.match(PS_OPS)) return 'operator';
  if (stream.match(/^-[A-Za-z][\w-]*/)) return 'attributeName';
  if (stream.match(/^\d+(?:\.\d+)?(?:[kmgtp]b)?\b/i)) return 'number';
  if (stream.match(/^[A-Za-z]+-[A-Za-z]\w*/)) return 'variableName.function';
  if (stream.match(/^[A-Za-z_]\w*/)) return PS_KW.has(stream.current().toLowerCase()) ? 'keyword' : 'variableName';
  if (stream.match(/^[{}()[\];,|.]/)) return 'punctuation';
  if (stream.match(/^[=+\-*/%!<>]+/)) return 'operator';
  stream.next();
  return null;
}

export const powershellParser = {
  name: 'powershell',
  startState: () => ({ comment: false }),
  token: psToken,
  languageData: { commentTokens: { line: '#', block: { open: '<#', close: '#>' } } },
};

// ── Windows batch ───────────────────────────────────────────────────────────────────────

const BAT_KW = /^(?:echo|set|if|else|goto|call|exit|for|in|do|not|exist|defined|errorlevel|setlocal|endlocal|pushd|popd|shift|start|cd|chdir|del|copy|move|mkdir|md|rmdir|rd|type|pause|cls|title|choice|where|equ|neq|lss|leq|gtr|geq)(?![\w.])/i;

function batToken(stream: StringStream): string | null {
  if (stream.sol()) {
    if (stream.match(/^\s*(?:rem(?:\s.*|$)|::.*)/i)) return 'comment';
    if (stream.match(/^\s*:[\w.-]+/)) return 'labelName';
  }
  if (stream.eatSpace()) return null;
  if (stream.eat('@')) return 'meta';
  if (stream.match(/^%%~?\w|^%~[\w$:]*\d|^%[\w.~:=*,-]+%|^![\w.-]+!|^%\d/)) return 'variableName.special';
  if (stream.match(/^"[^"]*"?/)) return 'string';
  if (stream.match(BAT_KW)) return 'keyword';
  if (stream.match(/^(?:==|>>|&&|\|\||[|&<>])/)) return 'operator';
  if (stream.match(/^\/[A-Za-z?](?![\w])/)) return 'attributeName';
  if (stream.match(/^\d+\b/)) return 'number';
  if (stream.match(/^[^\s%!"@|&<>=]+/)) return null;
  stream.next();
  return null;
}

export const batchParser = { name: 'batch', startState: () => ({}), token: batToken, languageData: { commentTokens: { line: 'REM' } } };

// ── WebVTT subtitles ────────────────────────────────────────────────────────────────────

function vttToken(stream: StringStream, s: { note: boolean }): string | null {
  if (stream.sol()) {
    if (stream.match(/^WEBVTT\b.*/)) return 'meta';
    if (stream.match(/^(?:NOTE|STYLE|REGION)\b.*/)) { s.note = true; return 'comment'; }
    if (s.note) { stream.skipToEnd(); return 'comment'; }
  }
  if (stream.eatSpace()) return null;
  if (stream.match(/^(?:\d{2,}:)?\d{2}:\d{2}\.\d{3}/)) return 'number';
  if (stream.match('-->')) return 'operator';
  if (stream.match(/^(?:vertical|line|position|size|align|region):\S+/)) return 'attributeName';
  if (stream.match(/^<\/?[\w.]+[^>]*>/)) return 'labelName';
  stream.match(/^[^<\s]+/) || stream.next();
  return null;
}

export const vttParser = { name: 'vtt', startState: () => ({ note: false }), token: vttToken, blankLine: (s: { note: boolean }) => { s.note = false; } };

// Defined once: a new Language per call would re-parse the file on every reconfigure.
const languages = {
  dockerfile: StreamLanguage.define(dockerfileParser),
  nginx: StreamLanguage.define(nginxParser),
  proto: StreamLanguage.define(protoParser),
  hcl: StreamLanguage.define(hclParser),
  diff: StreamLanguage.define(diffParser),
  powershell: StreamLanguage.define(powershellParser),
  batch: StreamLanguage.define(batchParser),
  vtt: StreamLanguage.define(vttParser),
};
export const dockerfileLang = (): Extension => languages.dockerfile;
export const nginxLang = (): Extension => [languages.nginx, indentFold];
export const protoLang = (): Extension => [languages.proto, indentFold];
export const hclLang = (): Extension => [languages.hcl, indentFold];
export const diffLang = (): Extension => languages.diff;
export const powershellLang = (): Extension => languages.powershell;
export const batchLang = (): Extension => languages.batch;
export const vttLang = (): Extension => languages.vtt;
