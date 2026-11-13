// Path links: turn a path an agent PRINTED (terminal output, chat prose) into
// the file or folder it really names. The printed text is relative to wherever
// the agent was standing, which the UI does not know for sure — a session's
// start folder, a repo root, another registered repo, the workspace folder —
// so a relative path is joined to each candidate root IN ORDER and the first
// one the daemon says exists wins. Nothing is guessed: a path that exists
// nowhere is reported as not found instead of opening a broken viewer.
//
// Pure (no stores, no fetch): the caller hands in the roots and a `stat`.
// lib/pathLinks/open.ts wires it to `/fs/stat`, the stores and the UI.

export interface PathRef {
  path: string;
  line?: number;
  col?: number;
}

/** One `stat` answer: a definite kind, definitely missing, or no answer
 *  (forbidden / busy / offline — the path may well exist). */
export type StatAnswer = { kind: 'file' | 'dir'; path: string } | 'missing' | 'unknown';
export type StatFn = (path: string) => Promise<StatAnswer>;

export type PathResolution =
  | { found: true; path: string; dir: boolean; line?: number; col?: number; verified: boolean }
  | { found: false; path: string; tried: string[] };

/** At most this many candidates are asked about per link (the daemon admits
 *  eight stats at once; a link is one click, never a scan). */
export const MAX_CANDIDATES = 8;

const LOCATION = /(?::([1-9]\d*)(?::([1-9]\d*))?|#L([1-9]\d*)(?:C([1-9]\d*))?)$/;
const WRAPPERS: Record<string, string> = { '"': '"', "'": "'", '`': '`', '(': ')', '[': ']', '<': '>', '{': '}', '“': '”', '‘': '’' };

/** Version tags, numbers and fractions read like paths but never are. */
export function looksLikeNonPath(path: string): boolean {
  const p = path.replace(/\/+$/, '');
  if (!p) return true;
  if (/^v?\d+(?:\.\d+)+(?:[-+][\w.-]+)?$/i.test(p)) return true; // v0.2.2, 1.2.3, 1.0.0-rc.1
  if (/^\d+(?:\/\d+)+$/.test(p)) return true; // 1/2, 3/4/5
  if (/^[a-z][a-z\d+.-]*:/i.test(p)) return true; // any scheme (mailto:, javascript:, http:)
  // A host without a scheme (example.com/a, www.x.io): not a local path.
  if (!/^[~./]/.test(p) && /^(?:www\.)?[a-z\d-]+(?:\.[a-z\d-]+)*\.(?:com|org|net|io|dev|app|ai|co|gov|edu)(?:\/|:\d|$)/i.test(p)) return true;
  return false;
}

/** The path a piece of printed text names: wrappers (quotes, backticks,
 *  brackets) and trailing sentence punctuation dropped, `:line:col` /
 *  `#L12C3` split off. A trailing `/` is kept (it says "a folder"). */
export function cleanPathRef(raw: string): PathRef | null {
  let text = raw.trim();
  for (let i = 0; i < 3 && text.length >= 2; i++) {
    const close = WRAPPERS[text[0]];
    if (close && text.endsWith(close)) text = text.slice(1, -1).trim();
    else break;
  }
  text = text.replace(/^[("'`[<“‘]+/, '');
  for (let prev = ''; prev !== text; ) {
    prev = text;
    text = text.replace(/[)"'`\]>”’]+$/, '').replace(/[.,;:!?]+$/, '');
  }
  if (!text || text.length > 4096 || /[\n\r\0]/.test(text)) return null;
  const loc = LOCATION.exec(text);
  const path = loc ? text.slice(0, loc.index) : text;
  if (!path || looksLikeNonPath(path)) return null;
  const line = loc ? Number(loc[1] ?? loc[3]) : undefined;
  const colText = loc ? (loc[2] ?? loc[4]) : undefined;
  return { path, line, col: colText ? Number(colText) : undefined };
}

/** Join a relative path onto a root WITHOUT folding `..` — the daemon
 *  canonicalizes through the real filesystem, and a lexical `a/../b` is wrong
 *  when `a` is a symlink. `./` is dropped (it means the root itself). */
export function joinRoot(root: string, path: string): string {
  const rel = path.replace(/^(?:\.\/)+/, '');
  const base = root.replace(/\/+$/, '');
  if (!rel || rel === '.') return base || '/';
  return `${base}/${rel}`;
}

/** The absolute paths to ask about, in order: an absolute or `~` path is
 *  itself (still verified); a relative one is tried under each root. */
export function candidatePaths(path: string, roots: readonly (string | null | undefined)[]): string[] {
  const trimmed = path.length > 1 ? path.replace(/\/+$/, '') : path;
  if (trimmed.startsWith('/') || trimmed === '~' || trimmed.startsWith('~/')) return [trimmed];
  const out: string[] = [];
  for (const root of roots) {
    if (!root || !(root.startsWith('/') || root.startsWith('~'))) continue;
    const candidate = joinRoot(root, trimmed);
    if (!out.includes(candidate)) out.push(candidate);
    if (out.length >= MAX_CANDIDATES) break;
  }
  return out;
}

/** Root folders, deduped in order, without the empty ones. */
export function orderRoots(...groups: readonly (readonly (string | null | undefined)[])[]): string[] {
  const out: string[] = [];
  for (const group of groups) for (const r of group) {
    const root = r ? (r.length > 1 ? r.replace(/\/+$/, '') : r) : '';
    if (root && !out.includes(root)) out.push(root);
  }
  return out;
}

/** The first candidate that exists. All candidates of a batch are asked at
 *  once (a click must not wait N round trips), but ORDER decides the winner.
 *  `more` (e.g. the workspace's repos, fetched on demand) is only asked when
 *  `roots` found nothing. A candidate the daemon could not answer for
 *  (forbidden / busy) is opened as printed only when nothing was verified —
 *  the viewer then reports a real failure, as before. */
export async function resolvePathRef(
  ref: PathRef,
  roots: readonly string[],
  stat: StatFn,
  more?: () => Promise<readonly string[]>,
): Promise<PathResolution> {
  const tried: string[] = [];
  let unknown: string | null = null;
  const dirHint = ref.path.length > 1 && ref.path.endsWith('/');
  const check = async (batch: string[]): Promise<PathResolution | null> => {
    const fresh = batch.filter((p) => !tried.includes(p)).slice(0, MAX_CANDIDATES - tried.length);
    tried.push(...fresh);
    const answers = await Promise.all(fresh.map((p) => stat(p).catch((): StatAnswer => 'unknown')));
    for (let i = 0; i < fresh.length; i++) {
      const a = answers[i];
      if (a === 'unknown') unknown ??= fresh[i];
      else if (a !== 'missing') {
        // A file asked for as a folder (`a.rs/`) is not that folder.
        if (dirHint && a.kind !== 'dir') continue;
        return { found: true, path: a.path || fresh[i], dir: a.kind === 'dir', line: ref.line, col: ref.col, verified: true };
      }
    }
    return null;
  };
  const first = await check(candidatePaths(ref.path, roots));
  if (first) return first;
  const absolute = candidatePaths(ref.path, []).length > 0;
  if (more && !absolute && tried.length < MAX_CANDIDATES) {
    const extra = await more().catch(() => [] as string[]);
    const hit = await check(candidatePaths(ref.path, extra));
    if (hit) return hit;
  }
  if (unknown) return { found: true, path: unknown, dir: dirHint, line: ref.line, col: ref.col, verified: false };
  return { found: false, path: ref.path, tried };
}

/** "Looked in 3 folders" — the not-found toast's detail (a count, not a wall). */
export function notFoundDetail(res: { path: string; tried: string[] }): string {
  const n = res.tried.length;
  if (n === 0) return 'No folder is known for this session to look in.';
  if (n === 1) return `Looked for it at ${res.tried[0]}.`;
  return `Looked in ${n} places (the session’s folders, its repositories, the workspace folder).`;
}
