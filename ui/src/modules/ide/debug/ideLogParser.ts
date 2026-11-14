// Structured JSON log line parser & stacktrace file:line linkifier for
// `IdeConsoleView.svelte` (used across both Debug Console and Run & Tests).

export type ConsoleLogTone = 'error' | 'warn' | 'info' | 'debug';

export interface ConsoleFileLink {
  display: string;
  file: string;
  line: number;
  col: number;
}

export interface ParsedJsonLog {
  prefix: string;
  timestamp: string | null;
  levelLabel: string;
  tone: ConsoleLogTone;
  caller: ConsoleFileLink | null;
  message: string;
  extraEntries: Array<[string, string]>;
  rawPretty: string;
}

export type PlainSegment =
  | { kind: 'text'; text: string }
  | { kind: 'link'; link: ConsoleFileLink };

// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07/g;

export function stripAnsiCodes(raw: string): string {
  return raw.replace(ANSI_RE, '');
}

const KNOWN_LOG_KEYS = new Set([
  'level',
  'lvl',
  'severity',
  'L',
  'msg',
  'message',
  'M',
  'ts',
  'time',
  'timestamp',
  '@timestamp',
  'T',
  'caller',
  'C',
  'logger',
  'err',
  'error',
]);

const CONSUMED_KEYS = new Set([
  'level',
  'lvl',
  'severity',
  'L',
  'msg',
  'message',
  'M',
  'ts',
  'time',
  'timestamp',
  '@timestamp',
  'T',
  'caller',
  'C',
]);

function normalizeLevel(raw: unknown): { label: string; tone: ConsoleLogTone } {
  if (typeof raw === 'number') {
    if (raw >= 50) return { label: 'ERROR', tone: 'error' };
    if (raw >= 40) return { label: 'WARN', tone: 'warn' };
    if (raw >= 30) return { label: 'INFO', tone: 'info' };
    return { label: 'DEBUG', tone: 'debug' };
  }
  const s = typeof raw === 'string' ? raw.trim().toLowerCase() : '';
  if (s === 'fatal' || s === 'panic' || s === 'dpanic' || s === 'critical') {
    return { label: 'FATAL', tone: 'error' };
  }
  if (s === 'error' || s === 'err' || s === 'severe') {
    return { label: 'ERROR', tone: 'error' };
  }
  if (s === 'warn' || s === 'warning') {
    return { label: 'WARN', tone: 'warn' };
  }
  if (s === 'debug' || s === 'dbg' || s === 'trace' || s === 'fine') {
    return { label: s === 'trace' ? 'TRACE' : 'DEBUG', tone: 'debug' };
  }
  return { label: 'INFO', tone: 'info' };
}

function formatShortTimestamp(raw: unknown): string | null {
  if (raw == null) return null;
  if (typeof raw === 'number' && Number.isFinite(raw)) {
    const ms = raw > 1e12 ? raw : raw * 1000;
    const d = new Date(ms);
    if (Number.isNaN(d.getTime())) return null;
    const hh = String(d.getHours()).padStart(2, '0');
    const mm = String(d.getMinutes()).padStart(2, '0');
    const ss = String(d.getSeconds()).padStart(2, '0');
    const mmm = String(d.getMilliseconds()).padStart(3, '0');
    return `${hh}:${mm}:${ss}.${mmm}`;
  }
  if (typeof raw !== 'string') return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  // Match `YYYY-MM-DD HH:MM:SS.mmm` or ISO `YYYY-MM-DDTHH:MM:SS.mmmZ`
  const m = /(\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?)/.exec(trimmed);
  if (m) {
    const t = m[1];
    const dot = t.indexOf('.');
    return dot >= 0 ? `${t.slice(0, dot)}.${t.slice(dot + 1, dot + 4)}` : t;
  }
  return trimmed.length <= 16 ? trimmed : trimmed.slice(0, 16);
}

export function parseCallerLink(raw: unknown, extraObj?: Record<string, unknown>): ConsoleFileLink | null {
  if (typeof raw === 'string' && raw.trim()) {
    const m = /^(.+?):(\d+)(?::(\d+))?$/.exec(raw.trim());
    if (m && m[1]) {
      const line = Math.max(1, Number(m[2]) || 1);
      const col = Math.max(1, Number(m[3]) || 1);
      return {
        display: `${m[1]}:${line}`,
        file: m[1],
        line,
        col,
      };
    }
  }
  if (raw && typeof raw === 'object') {
    const src = raw as Record<string, unknown>;
    if (typeof src.file === 'string' && src.file.trim()) {
      const line = Math.max(1, Number(src.line) || 1);
      return {
        display: `${src.file}:${line}`,
        file: src.file,
        line,
        col: 1,
      };
    }
  }
  if (extraObj && typeof extraObj.file === 'string' && extraObj.file.trim()) {
    const line = Math.max(1, Number(extraObj.line) || 1);
    return {
      display: `${extraObj.file}:${line}`,
      file: extraObj.file,
      line,
      col: 1,
    };
  }
  return null;
}

function formatFieldValue(v: unknown): string {
  if (v === null) return 'null';
  if (v === undefined) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}

export function parseStructuredJsonLog(cleanLine: string): ParsedJsonLog | null {
  const trimmed = cleanLine.trim();
  if (!trimmed || !trimmed.endsWith('}')) return null;
  const openIdx = trimmed.indexOf('{');
  if (openIdx < 0 || openIdx > 48) return null;

  const prefix = openIdx > 0 ? trimmed.slice(0, openIdx).trim() : '';
  const jsonSlice = trimmed.slice(openIdx);
  let parsed: unknown;
  try {
    parsed = JSON.parse(jsonSlice);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null;
  const obj = parsed as Record<string, unknown>;
  const keys = Object.keys(obj);
  if (keys.length === 0 || !keys.some((k) => KNOWN_LOG_KEYS.has(k))) return null;

  const rawLevel = obj.level ?? obj.lvl ?? obj.severity ?? obj.L;
  const { label: levelLabel, tone } = normalizeLevel(rawLevel);
  const timestamp = formatShortTimestamp(
    obj.ts ?? obj.time ?? obj.timestamp ?? obj['@timestamp'] ?? obj.T,
  );
  const caller = parseCallerLink(obj.caller ?? obj.C ?? obj.source, obj);

  const rawMsg = obj.msg ?? obj.message ?? obj.M;
  let message = typeof rawMsg === 'string' ? rawMsg : rawMsg != null ? formatFieldValue(rawMsg) : '';
  if (!message && typeof obj.error === 'string') message = obj.error;
  if (!message && typeof obj.err === 'string') message = obj.err;

  const extraEntries: Array<[string, string]> = [];
  for (let i = 0; i < keys.length; i++) {
    const k = keys[i];
    if (CONSUMED_KEYS.has(k)) continue;
    if (caller && (k === 'file' || k === 'line' || k === 'source')) continue;
    extraEntries.push([k, formatFieldValue(obj[k])]);
  }

  return {
    prefix,
    timestamp,
    levelLabel,
    tone,
    caller,
    message: message || jsonSlice,
    extraEntries,
    rawPretty: JSON.stringify(obj, null, 2),
  };
}

const FILE_LOC_RE =
  /(?:File "([^"\n]+\.[a-zA-Z0-9]+)", line (\d+))|((?:\/|~\/|\.\.?\/)?(?:[a-zA-Z0-9_.-]+\/)*[a-zA-Z0-9_.-]+\.(?:go|rs|ts|tsx|js|jsx|mjs|cjs|py|java|kt|sh|bash|zsh|json|svelte|sql|c|cc|cpp|h)):(\d+)(?::(\d+))?/g;

export function parsePlainTextSegments(cleanLine: string): PlainSegment[] {
  if (!cleanLine) return [{ kind: 'text', text: '' }];
  const out: PlainSegment[] = [];
  let cursor = 0;
  FILE_LOC_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = FILE_LOC_RE.exec(cleanLine)) !== null) {
    if (m.index > cursor) {
      out.push({ kind: 'text', text: cleanLine.slice(cursor, m.index) });
    }
    if (m[1] && m[2]) {
      const line = Math.max(1, Number(m[2]) || 1);
      out.push({
        kind: 'link',
        link: { display: m[0], file: m[1], line, col: 1 },
      });
    } else if (m[3] && m[4]) {
      const line = Math.max(1, Number(m[4]) || 1);
      const col = Math.max(1, Number(m[5]) || 1);
      out.push({
        kind: 'link',
        link: { display: m[0], file: m[3], line, col },
      });
    } else {
      out.push({ kind: 'text', text: m[0] });
    }
    cursor = m.index + m[0].length;
  }
  if (cursor < cleanLine.length) {
    out.push({ kind: 'text', text: cleanLine.slice(cursor) });
  }
  return out.length > 0 ? out : [{ kind: 'text', text: cleanLine }];
}

export function isPlainErrorLine(cleanLine: string): boolean {
  return /^\s*(FAIL\b|---\s*FAIL:|Error:|panic:|fatal:|Traceback\b|Exception\b|E\s+|error\[)/i.test(
    cleanLine,
  );
}
