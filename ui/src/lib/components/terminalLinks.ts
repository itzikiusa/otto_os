/** Terminal text recognition is independent of session state and side effects. */
export interface TerminalLink {
  start: number;
  end: number;
  kind: 'url' | 'file';
  text: string;
  path?: string;
  line?: number;
  col?: number;
}
const SOURCE_EXTS = new Set('rs ts tsx js jsx mjs cjs go py rb java kt c h cc cpp hpp cs php swift scala sh bash zsh sql html css scss svelte vue json toml yaml yml md mdx txt xml proto lock cfg ini env csv log'.split(' '));
const SOURCE_NAMES = new Set(['Dockerfile', 'Makefile', 'Justfile', 'LICENSE', 'README', '.gitignore', '.env']);

function fileTarget(raw: string): Pick<TerminalLink, 'path' | 'line' | 'col'> | null {
  if (!raw || /[\n\r\x00]/.test(raw) || /^[a-z][a-z\d+.-]*:\/\//i.test(raw)) return null;
  const location = /(?::([1-9]\d*)(?::([1-9]\d*))?|#L([1-9]\d*)(?:C([1-9]\d*))?)$/.exec(raw);
  const path = location ? raw.slice(0, location.index) : raw;
  if (/[<>|*?]/.test(path) || path.includes(':') || path.endsWith('/')) return null;
  const name = path.split('/').at(-1) ?? '';
  const ext = /\.([a-z\d]{1,12})$/i.exec(name)?.[1].toLowerCase();
  if (!SOURCE_NAMES.has(name) && !(ext && (path.includes('/') || SOURCE_EXTS.has(ext)))) return null;
  const line = location ? Number(location[1] ?? location[3]) : undefined;
  const col = location?.[2] || location?.[4] ? Number(location[2] ?? location[4]) : undefined;
  if ((line !== undefined && !Number.isSafeInteger(line)) || (col !== undefined && !Number.isSafeInteger(col))) return null;
  return { path, line, col };
}

// Printed rooted paths can contain directory spaces. Stop at the first file
// boundary rather than greedily consuming following prose or another reference.
function firstRootedFile(raw: string): string | null {
  const bounded = raw.slice(0, 2048).split(/[\n\r\t<>"'`(){}\[\]|]/, 1)[0]
    .split(/\s+(?:(?:and|or|then|but)\s+|(?=~?\/|\.{1,2}\/))/, 1)[0];
  const end = /\.[a-z\d]{1,12}(?::[1-9]\d*(?::[1-9]\d*)?|#L[1-9]\d*(?:C[1-9]\d*)?)?(?=$|[\s,;!?]|\.(?=\s|$))/i.exec(bounded);
  if (!end) return null;
  const candidate = bounded.slice(0, end.index + end[0].length);
  const target = fileTarget(candidate);
  if (!target?.path) return null;
  // A host:port after an incomplete path is prose, not a spaced filename.
  const name = target.path.split('/').at(-1) ?? '';
  if (/\s/.test(name) && /\.(?:com|org|net|io)(?::|$)/i.test(candidate)) return null;
  return candidate;
}

export function scanTerminalLinks(text: string, allowFiles = true): TerminalLink[] {
  const hits: TerminalLink[] = [];
  const occupied: { start: number; end: number }[] = [];
  const overlap = (start: number, end: number) => occupied.some(h => start < h.end && end > h.start);
  const add = (raw: string, start: number) => {
    if (overlap(start, start + raw.length)) return;
    if (/^https?:\/\//i.test(raw)) {
      hits.push({ start, end: start + raw.length, text: raw, kind: 'url' });
    } else {
      const target = allowFiles ? fileTarget(raw) : null;
      if (!target) return;
      hits.push({ start, end: start + raw.length, text: raw, kind: 'file', ...target });
    }
    occupied.push({ start, end: start + raw.length });
  };
  // Delimited destinations are parsed first, retaining spaces rather than
  // trying to guess where a path ends in ordinary prose.
  for (const match of text.matchAll(/\]\(([^)\n]+)\)|(["'`])([^\n]*?)\2/g)) {
    const raw = match[1] ?? match[3];
    const start = match.index + (match[1] !== undefined ? 2 : 1);
    add(raw, start);
  }
  for (const match of text.matchAll(/\bhttps?:\/\/[^\s<>"'`()\[\]]+/gi)) {
    add(match[0].replace(/[.,;:!?]+$/, ''), match.index);
  }
  if (allowFiles) {
    for (const match of text.matchAll(/(?<![\p{L}\p{N}_@.+/:~-])(?:~\/|\.{1,2}\/|\/)/gu)) {
      const raw = firstRootedFile(text.slice(match.index));
      if (raw) add(raw, match.index);
    }
    // Tokenize separately from classifying: no guessing routes, host:port, or
    // task identifiers merely because they contain slash characters.
    for (const match of text.matchAll(/(?<![\p{L}\p{N}_@.+/:~-])(?:~\/|\.{1,2}\/|\/)?[\p{L}\p{N}_@.+-]+(?:\/[\p{L}\p{N}_@.+-]+)*(?::\d+(?::\d+)?|#L\d+(?:C\d+)?)?/gu)) {
      add(match[0].replace(/[.,;]+$/, ''), match.index);
    }
  }
  return hits.sort((a, b) => a.start - b.start);
}

/** No file lookup or cwd mutation: resolve exactly the printed path. */
export function resolveTerminalFile(path: string, cwd: string): string | null {
  if (path.startsWith('/') || path.startsWith('~/')) return path;
  if (!cwd) return null;
  // The daemon canonicalizes through the real filesystem. Lexically removing
  // '..' here changes its meaning when an earlier component is a symlink.
  return `${cwd.replace(/\/$/, '')}/${path}`;
}

export function oscTerminalLink(target: string, cwd: string, allowFiles: boolean): TerminalLink | null {
  if (/^https?:\/\//i.test(target)) return { start: 0, end: target.length, text: target, kind: 'url' };
  if (!allowFiles) return null;
  let raw = target;
  if (/^file:\/\//i.test(raw)) {
    try {
      const url = new URL(raw);
      if (url.hostname && url.hostname !== 'localhost') return null;
      // URL.pathname normalizes dot segments, which is wrong for filesystem
      // symlinks. Validate authority with URL, decode the original path only.
      const original = /^file:\/\/[^/?#]*(\/[^?#]*)(#[^\s]*)?$/i.exec(raw);
      if (!original) return null;
      raw = decodeURIComponent(original[1]) + (original[2] ?? '');
    } catch { return null; }
  } else if (/^[a-z][a-z\d+.-]*:/i.test(raw)) return null;
  const parsed = fileTarget(raw);
  if (!parsed?.path) return null;
  const path = resolveTerminalFile(parsed.path, cwd);
  return path ? { start: 0, end: target.length, text: target, kind: 'file', ...parsed, path } : null;
}

interface Cell {
  getChars(): string;
  getWidth(): number;
  getFgColorMode?(): number;
  getFgColor?(): number;
  isBold?(): number;
}
interface BufferLine { isWrapped: boolean; length: number; getCell(x: number): Cell | undefined }
interface Buffer { length: number; getLine(y: number): BufferLine | undefined }
interface Point { x: number; y: number }
export interface TerminalBufferLink extends TerminalLink { range: { start: Point; end: Point } }

interface MappedRow {
  text: string;
  starts: Point[];
  ends: Point[];
  styles: (string | null)[];
}

function mappedRow(buffer: Buffer, y: number, keepPadding = false): MappedRow {
  const result: MappedRow = { text: '', starts: [], ends: [], styles: [] };
  const line = buffer.getLine(y);
  if (!line) return result;
  let last = line.length;
  if (!keepPadding) while (last > 0 && !line.getCell(last - 1)?.getChars()) last--;
  for (let x = 0; x < last; x++) {
    const cell = line.getCell(x), width = cell?.getWidth() ?? 1;
    if (!width) continue;
    const chars = cell?.getChars() || ' ';
    const colorMode = cell?.getFgColorMode?.() ?? 0, bold = cell?.isBold?.() ?? 0;
    const style = colorMode || bold ? `${colorMode}:${cell?.getFgColor?.() ?? 0}:${bold}` : null;
    result.text += chars;
    for (let i = 0; i < chars.length; i++) {
      result.starts.push({ x: x + 1, y: y + 1 });
      result.ends.push({ x: x + width, y: y + 1 });
      result.styles.push(style);
    }
  }
  return result;
}

interface LogicalRow extends MappedRow { endY: number }

function logicalStart(buffer: Buffer, y: number): number | null {
  let count = 0;
  while (y > 0 && buffer.getLine(y)?.isWrapped) {
    if (++count > 128) return null;
    y--;
  }
  return y;
}

function mappedLogicalRow(buffer: Buffer, y: number): LogicalRow | null {
  const result: LogicalRow = { text: '', starts: [], ends: [], styles: [], endY: y };
  for (let count = 0; count < 128; count++, y++) {
    const continues = buffer.getLine(y + 1)?.isWrapped ?? false;
    const part = mappedRow(buffer, y, continues);
    result.text += part.text;
    result.starts.push(...part.starts);
    result.ends.push(...part.ends);
    result.styles.push(...part.styles);
    result.endY = y;
    if (result.text.length > 16384) return null;
    if (!continues) return result;
  }
  return null;
}

/** TUIs can write physical CRLF/cursor-addressed rows instead of soft wraps.
 * Only pair an explicitly delimited path, or an incomplete styled path at the
 * right margin with a matching styled continuation. Never join ordinary prose. */
function hardWrappedLinks(buffer: Buffer, row: number): TerminalBufferLink[] {
  const links: TerminalBufferLink[] = [];
  const start = logicalStart(buffer, row - 1);
  if (start === null) return [];
  const candidates = [start];
  for (let count = 0; count < 3 && candidates[0] > 0; count++) {
    const previous = logicalStart(buffer, candidates[0] - 1);
    if (previous === null) break;
    candidates.unshift(previous);
  }
  for (const y of candidates) {
    const first = mappedLogicalRow(buffer, y);
    if (!first) continue;
    for (const match of first.text.matchAll(/(?<![\p{L}\p{N}_@.+/:~-])(?:~\/|\.{1,2}\/|\/|[\p{L}\p{N}_@.+-]+\/)/gu)) {
      const prefix = first.text.slice(match.index).trimEnd();
      if (!prefix || prefix.length > 2048 || fileTarget(prefix) || /[<>|?*:"'`(){}\[\]]/.test(prefix)) continue;
      const before = first.text.slice(0, match.index).trimEnd().at(-1);
      const closing = before === '(' ? ')' : before === '"' || before === "'" || before === '`' ? before : null;
      const style = first.styles[match.index];
      const uniform = (part: MappedRow, start: number, end: number) => !!style && part.styles.slice(start, end).every(value => value === style);
      const atEdge = (part: MappedRow, end: number) => {
        const point = part.ends[end - 1];
        return !!point && point.x >= (buffer.getLine(point.y - 1)?.length ?? 0) - 8;
      };
      const prefixEnd = match.index + prefix.length;
      const relativeBold = /^\.{1,2}\//.test(prefix) && !!buffer.getLine(first.starts[match.index].y - 1)?.getCell(first.starts[match.index].x - 1)?.isBold?.()
        && prefixEnd === first.text.length;
      if (!closing && !uniform(first, match.index, prefixEnd)) continue;
      const edge = atEdge(first, prefixEnd);
      if (!closing && !edge && !relativeBold) continue;
      let text = prefix;
      const segments = [{ part: first, start: match.index, end: match.index + prefix.length }];
      let nextY = first.endY + 1;
      for (let count = 0; count < 3 && nextY < buffer.length; count++) {
        const next = mappedLogicalRow(buffer, nextY);
        if (!next) break;
        nextY = next.endY + 1;
        const indent = /^[ \t]{1,8}(?=\S)/.exec(next.text)?.[0].length;
        if (!indent) break;
        let fragment = next.text.slice(indent), complete = false;
        if (closing) {
          const end = fragment.indexOf(closing);
          if (end >= 0) { fragment = fragment.slice(0, end); complete = true; }
          else fragment = fragment.trimEnd();
          if (/[<>|?*:"'`(){}\[\]]/.test(fragment)) break;
        } else {
          fragment = /^[\p{L}\p{N}_@.+~/#:-]+/u.exec(fragment)?.[0] ?? '';
          fragment = fragment.replace(/[.,;]+$/, '');
          if (!fragment || !uniform(next, indent, indent + fragment.length)) break;
          // Claude can replay its bold ../path after the viewport grew. A
          // leading slash continues that relative directory, with no need to
          // mistake unrelated absolute references for wrapped text.
          if (count === 0 && !edge && !(relativeBold && fragment.startsWith('/'))) break;
          complete = !!fileTarget(text + fragment);
          if (!complete && (!atEdge(next, indent + fragment.length) || indent + fragment.length !== next.text.length)) break;
        }
        if (!fragment || text.length + fragment.length > 2048) break;
        text += fragment;
        segments.push({ part: next, start: indent, end: indent + fragment.length });
        if (!complete) continue;
        const target = fileTarget(text);
        if (target) {
          for (const segment of segments) {
            const positions = segment.part.starts.slice(segment.start, segment.end);
            const firstIndex = positions.findIndex(point => point.y === row);
            const lastIndex = positions.findLastIndex(point => point.y === row);
            const start = segment.part.starts[segment.start + firstIndex], end = segment.part.ends[segment.start + lastIndex];
            if (firstIndex >= 0 && start && end) links.push({ start: segment.start, end: segment.end, kind: 'file', text, ...target, range: { start, end } });
          }
        }
        break;
      }
    }
  }
  // The first (longest rooted) reconstruction claims its cells before a suffix.
  return links.filter((link, index) => !links.slice(0, index).some(previous => previous.range.start.x <= link.range.end.x && previous.range.end.x >= link.range.start.x));
}

/** xterm rows are 1-based; link end cells are inclusive. Match a logical line
 * across soft wraps, preserving the position of wide and combining characters.
 * A pathological uninterrupted stream is bounded to 128 rows per hover. */
export function terminalLinksForRow(buffer: Buffer, row: number, allowFiles: boolean): TerminalBufferLink[] {
  const first = logicalStart(buffer, row - 1);
  if (first === null) return [];
  const mapped = mappedLogicalRow(buffer, first);
  if (!mapped) return [];
  const { text, starts, ends } = mapped;
  const hard = allowFiles ? hardWrappedLinks(buffer, row) : [];
  const ordinary = scanTerminalLinks(text, allowFiles).flatMap(hit => {
    const start = starts[hit.start], end = ends[hit.end - 1];
    return start && end && start.y <= row && end.y >= row ? [{ ...hit, range: { start, end } }] : [];
  });
  return [...hard, ...ordinary.filter(link => !hard.some(h =>
    (link.range.start.y < row || link.range.start.x <= h.range.end.x)
    && (link.range.end.y > row || link.range.end.x >= h.range.start.x)))];
}
