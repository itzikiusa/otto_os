// Pure `@` mention token parser, replacement splicer, file/folder ranker,
// unified file + agent list builder, and user-bubble context parser.
// Zero Svelte/DOM runtime imports so `node --test` runs directly against it.

import type { AskOption } from './askAgents';

export type MentionFilterMode = 'all' | 'files' | 'agents';

export interface AtMentionToken { query: string; start: number; end: number; isPathLike: boolean }

export interface RawFsCompletionItem {
  path: string; display?: string; insert_token?: string; name?: string;
  kind?: 'file' | 'dir' | string; is_dir?: boolean; openTab?: boolean; activeTab?: boolean;
}

export interface HighlightPart { text: string; hit: boolean }

export interface RankedFileMention {
  key: string; path: string; name: string; display: string; dirHint: string;
  insertToken: string; isDir: boolean; openTab: boolean; score: number; nameParts: HighlightPart[];
}

export type UnifiedMentionItem =
  | { kind: 'file'; file: RankedFileMention }
  | { kind: 'agent'; agent: AskOption };

export interface UnifiedMentionList {
  files: RankedFileMention[]; agents: AskOption[]; agentsFirst: boolean; flat: UnifiedMentionItem[];
}

export interface ParsedContextBadge {
  kind: 'selection' | 'file' | 'folder' | 'symbol' | 'diagnostic' | 'test_failure' | 'debug_frame' | 'run_output';
  label: string; path?: string; line?: number;
}

const trimSlash = (p: string): string => (p === '/' ? '/' : p.replace(/\/+$/, ''));

function baseName(p: string): string {
  const c = trimSlash(p);
  const idx = c.lastIndexOf('/');
  return idx >= 0 ? c.slice(idx + 1) : c;
}

/** Detect an active `@<query>` token ending at `caret` (defaults to `text.length`). */
export function atMentionTokenAt(text: string, caret = text.length): AtMentionToken | null {
  const clampedCaret = Math.max(0, Math.min(text.length, caret));
  const head = text.slice(0, clampedCaret);

  // Scan backward for the nearest `@` preceded by start-of-string or whitespace/delimiter.
  let atIdx = -1;
  for (let i = head.length - 1; i >= 0; i--) {
    if (head[i] === '@') {
      const prev = i === 0 ? ' ' : head[i - 1];
      if (/[\s([{"']/.test(prev)) {
        // Ignore `foo"@` where quote closed an earlier token; allow ` (@` or start `@`.
        if ((prev === '"' || prev === "'") && i >= 2 && !/\s/.test(head[i - 2])) continue;
        atIdx = i;
        break;
      }
    }
  }
  if (atIdx < 0) return null;

  const afterAt = head.slice(atIdx + 1);
  if (afterAt.startsWith('"')) {
    const inner = afterAt.slice(1);
    // Once the closing quote is typed, the mention token is complete.
    if (inner.includes('"') || /[\r\n]/.test(inner)) return null;
    let end = clampedCaret;
    let probe = clampedCaret;
    while (probe < text.length && text[probe] !== '"' && !/[\r\n]/.test(text[probe])) probe++;
    if (probe < text.length && text[probe] === '"') end = probe + 1;
    return {
      query: inner,
      start: atIdx,
      end,
      isPathLike: /[/.~]/.test(inner),
    };
  }

  // Unquoted token: cannot contain whitespace between `@` and caret.
  if (/\s/.test(afterAt)) return null;
  // Reject punctuation-only trailing tokens like `@,` or `@)`.
  if (afterAt.length > 0 && /^[),.;:!?\]]+$/.test(afterAt)) return null;

  let end = clampedCaret;
  while (end < text.length && !/\s/.test(text[end])) end++;

  return {
    query: afterAt,
    start: atIdx,
    end,
    isPathLike: /[/.~]/.test(afterAt),
  };
}

function unwrapInsertPath(token: string): string {
  return token.replace(/^@+/, '').replace(/^"(.*)"$/, '$1');
}

function formatInsertToken(rawPath: string, isDir = false): string {
  const unwrapped = unwrapInsertPath(rawPath);
  if (!unwrapped) return '';
  const body = isDir
    ? unwrapped.endsWith('/') ? unwrapped : `${unwrapped}/`
    : trimSlash(unwrapped);
  return /\s/.test(body) ? `@"${body}"` : `@${body}`;
}

/** Splice a chosen file, folder drill-down, or empty removal into `text`. */
export function replaceAtMentionToken(
  text: string,
  token: Pick<AtMentionToken, 'start' | 'end'>,
  insertToken: string,
  opts?: { keepOpen?: boolean },
): { text: string; caret: number } {
  const before = text.slice(0, token.start);
  const after = text.slice(token.end);
  const cleanInsert = unwrapInsertPath(insertToken);

  if (!cleanInsert) {
    const trimmedBefore = before.endsWith(' ') && after.startsWith(' ') ? before.slice(0, -1) : before;
    return { text: `${trimmedBefore}${after}`, caret: trimmedBefore.length };
  }

  if (opts?.keepOpen) {
    const withSlash = cleanInsert.endsWith('/') ? cleanInsert : `${cleanInsert}/`;
    const piece = /\s/.test(withSlash) ? `@"${withSlash}` : `@${withSlash}`;
    const nextText = `${before}${piece}${after}`;
    return { text: nextText, caret: before.length + piece.length };
  }

  const wrapped = /\s/.test(cleanInsert) ? `"${cleanInsert}"` : cleanInsert;
  const piece = `@${wrapped}`;
  if (after.startsWith(' ')) {
    return { text: `${before}${piece}${after}`, caret: before.length + piece.length + 1 };
  }
  return { text: `${before}${piece} ${after}`, caret: before.length + piece.length + 1 };
}

export function computeRelativeDisplay(
  fullPath: string,
  cwd?: string | null,
  roots?: readonly string[],
): { display: string; insertToken: string } {
  const cleanPath = trimSlash(fullPath);
  const cleanCwd = trimSlash(cwd ?? '');
  if (cleanCwd && cleanCwd !== '~' && cleanPath.startsWith(`${cleanCwd}/`)) {
    const rel = cleanPath.slice(cleanCwd.length + 1);
    return { display: rel, insertToken: formatInsertToken(rel) };
  }
  const rootList = (roots ?? []).map(trimSlash).filter((r) => r && r !== '~');
  const multiRoot = rootList.length > 1;
  for (let i = 0; i < rootList.length; i++) {
    const r = rootList[i];
    if (cleanPath === r) {
      const b = baseName(r);
      return { display: b, insertToken: formatInsertToken(b) };
    }
    if (cleanPath.startsWith(`${r}/`)) {
      const sub = cleanPath.slice(r.length + 1);
      const prefixed = multiRoot || r !== cleanCwd ? `${baseName(r)}/${sub}` : sub;
      return { display: prefixed, insertToken: formatInsertToken(prefixed) };
    }
  }
  return { display: cleanPath, insertToken: formatInsertToken(cleanPath) };
}

export function highlightSegments(text: string, needleRaw: string): HighlightPart[] {
  const needle = needleRaw.trim().toLowerCase();
  if (!needle || !text) return [{ text, hit: false }];
  const lower = text.toLowerCase();
  const directIdx = lower.indexOf(needle);
  if (directIdx >= 0) {
    const out: HighlightPart[] = [];
    if (directIdx > 0) out.push({ text: text.slice(0, directIdx), hit: false });
    out.push({ text: text.slice(directIdx, directIdx + needle.length), hit: true });
    if (directIdx + needle.length < text.length) {
      out.push({ text: text.slice(directIdx + needle.length), hit: false });
    }
    return out;
  }

  // Subsequence highlight fallback.
  const hitIndices = new Set<number>();
  let ni = 0;
  for (let ti = 0; ti < lower.length && ni < needle.length; ti++) {
    if (lower[ti] === needle[ni]) {
      hitIndices.add(ti);
      ni++;
    }
  }
  if (ni < needle.length) return [{ text, hit: false }];

  const parts: HighlightPart[] = [];
  let cur = '';
  let curHit = hitIndices.has(0);
  for (let i = 0; i < text.length; i++) {
    const h = hitIndices.has(i);
    if (h !== curHit && cur) {
      parts.push({ text: cur, hit: curHit });
      cur = '';
    }
    curHit = h;
    cur += text[i];
  }
  if (cur) parts.push({ text: cur, hit: curHit });
  return parts;
}

function fuzzySubsequenceScore(haystack: string, needle: string): number {
  if (!needle) return 1;
  let hi = 0;
  let consecutive = 0;
  let bonus = 0;
  for (let ni = 0; ni < needle.length; ni++) {
    const found = haystack.indexOf(needle[ni], hi);
    if (found < 0) return -1;
    if (found === hi) {
      consecutive++;
      bonus += consecutive * 4;
    } else {
      consecutive = 0;
    }
    if (found === 0 || '/._-'.includes(haystack[found - 1])) bonus += 8;
    hi = found + 1;
  }
  return 260 + Math.min(180, bonus) - Math.min(80, haystack.length * 0.4);
}

export function rankFileMentionItems(
  items: readonly RawFsCompletionItem[],
  query: string,
  opts?: { cwd?: string | null; roots?: readonly string[]; limit?: number },
): RankedFileMention[] {
  const limit = opts?.limit ?? 35;
  const qClean = query.trim();
  const qLower = qClean.toLowerCase();
  const slashIdx = qLower.lastIndexOf('/');
  const leafQuery = slashIdx >= 0 ? qLower.slice(slashIdx + 1) : qLower;
  const dirQuery = slashIdx >= 0 ? qLower.slice(0, slashIdx) : '';
  const isNavQuery = /^(?:~\/|\.\.?\/|\/)/.test(qClean) || slashIdx >= 0;

  const byKey = new Map<string, RankedFileMention>();

  for (let i = 0; i < items.length; i++) {
    const raw = items[i];
    if (!raw.path) continue;
    const isDir = Boolean(raw.is_dir || raw.kind === 'dir');
    const cleanPath = trimSlash(raw.path);
    const name = (raw.name ? trimSlash(raw.name) : '') || baseName(cleanPath);
    if (!name) continue;

    const fallbackRel = computeRelativeDisplay(cleanPath, opts?.cwd, opts?.roots);
    const displayBase = trimSlash(raw.display || fallbackRel.display);
    const display = isDir && !displayBase.endsWith('/') ? `${displayBase}/` : displayBase;
    const rawInsert = raw.insert_token || fallbackRel.insertToken;
    const insertToken = formatInsertToken(rawInsert, isDir);

    const nameLower = name.toLowerCase();
    const dispLower = displayBase.toLowerCase();
    const tokenLower = trimSlash(unwrapInsertPath(insertToken)).toLowerCase();

    let score = -1;
    if (!qLower) {
      score = raw.activeTab ? 980 : raw.openTab ? 920 - i : isDir ? 520 - i : 560 - i;
    } else if (isNavQuery && leafQuery === '') {
      // Drilled into a directory (`@~/go_admission/` or `@../`): keep server entries that match prefix, or all returned entries from `/ide/fs/complete`.
      const prefixProbe = trimSlash(qLower);
      if (
        !raw.openTab ||
        tokenLower.startsWith(prefixProbe) ||
        dispLower.startsWith(prefixProbe) ||
        cleanPath.toLowerCase().includes(prefixProbe.replace(/^[.~]+/, ''))
      ) {
        score = (isDir ? 760 : 740) - Math.min(100, i);
      }
    } else {
      const targetLeaf = leafQuery || qLower;
      if (nameLower === targetLeaf) score = 1000;
      else if (nameLower.startsWith(targetLeaf)) score = 860 - Math.min(60, name.length * 0.5);
      else if (new RegExp(`[_.-]${targetLeaf.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`).test(nameLower)) {
        score = 750;
      } else if (nameLower.includes(targetLeaf)) {
        score = 660 - Math.min(40, nameLower.indexOf(targetLeaf) * 2);
      } else if (!isNavQuery && dispLower.includes(qLower)) {
        score = 560 - Math.min(50, dispLower.indexOf(qLower));
      } else if (isNavQuery && (tokenLower.includes(qLower) || dispLower.includes(targetLeaf))) {
        score = 590;
      } else {
        const fz = fuzzySubsequenceScore(isNavQuery ? nameLower : dispLower, isNavQuery ? targetLeaf : qLower);
        if (fz > 0) score = fz;
      }

      if (score > 0 && dirQuery && !isNavQuery) {
        if (dispLower.includes(dirQuery)) score += 60;
      }
      // If user typed a path prefix like `promo/main`, ensure directory prefix also matches when not already server-filtered.
      if (score > 0 && dirQuery && raw.openTab) {
        const hay = `${tokenLower} ${cleanPath.toLowerCase()}`;
        const cleanDirQ = dirQuery.replace(/^(?:~\/|\.\.?\/)+/, '');
        if (cleanDirQ && !hay.includes(cleanDirQ)) score = -1;
      }
    }

    if (score < 0) continue;
    if (raw.activeTab) score += 90;
    else if (raw.openTab) score += 55;
    if (isDir && qClean.endsWith('/')) score += 15;

    const dirSlash = displayBase.lastIndexOf('/');
    const dirHint = dirSlash > 0 ? displayBase.slice(0, dirSlash) : isDir ? 'Folder' : '';
    const key = `${isDir ? 'd' : 'f'}:${cleanPath}`;
    const candidate: RankedFileMention = {
      key,
      path: cleanPath,
      name: isDir ? `${name}/` : name,
      display,
      dirHint,
      insertToken,
      isDir,
      openTab: Boolean(raw.openTab),
      score,
      nameParts: highlightSegments(isDir ? `${name}/` : name, leafQuery || qClean),
    };

    const prev = byKey.get(key);
    if (!prev || candidate.score > prev.score) byKey.set(key, candidate);
  }

  return [...byKey.values()]
    .sort((a, b) => b.score - a.score || a.display.localeCompare(b.display))
    .slice(0, limit);
}

export function buildUnifiedMentionResults(opts: {
  query: string;
  filter: MentionFilterMode;
  files: readonly RankedFileMention[];
  agents: readonly AskOption[];
  allowAgents?: boolean;
}): UnifiedMentionList {
  const q = opts.query.trim().toLowerCase();
  const isPath = /[/.~]/.test(q);
  const files = opts.filter === 'agents' ? [] : [...opts.files];
  const agents =
    opts.filter === 'files' || opts.allowAgents === false || isPath
      ? []
      : [...opts.agents];

  const strongAgentMatch =
    q.length >= 2 &&
    !isPath &&
    agents.some((a) => {
      const p = a.provider.toLowerCase();
      const lbl = a.label.toLowerCase();
      return p.startsWith(q) || lbl.startsWith(q);
    }) &&
    !files.some((f) => f.score >= 980);

  const agentsFirst = opts.filter === 'agents' || (opts.filter === 'all' && strongAgentMatch);
  const flat: UnifiedMentionItem[] = agentsFirst
    ? [
        ...agents.map((agent): UnifiedMentionItem => ({ kind: 'agent', agent })),
        ...files.map((file): UnifiedMentionItem => ({ kind: 'file', file })),
      ]
    : [
        ...files.map((file): UnifiedMentionItem => ({ kind: 'file', file })),
        ...agents.map((agent): UnifiedMentionItem => ({ kind: 'agent', agent })),
      ];

  return { files, agents, agentsFirst, flat };
}

const IDE_CTX_BLOCK_RE = /<ide_context\b[^>]*>[\s\S]*?<\/ide_context>\s*/gi;
const IDE_ITEM_TAG_RE = /<(item|selection|file|symbol|diagnostic|test_failure|debug_frame|run_output)\b([^>]*)>/gi;
const ATTR_KV_RE = /(\w+)="([^"]*)"/g;
const FILE_CTX_SECTION_RE = /\n*---\s*(File|Folder)\s+context:\s*(.+?)\s*---\n(?:File|Folder)\s+path:\s*([^\n]+)(?:\n\n(?!---\s*(?:File|Folder|Diff|Review)\s+(?:context|conversation):)[\s\S]*?(?=\n+---\s*(?:File|Folder|Diff|Review)\s+(?:context|conversation):|$))?/g;

const decodeXmlAttr = (s: string): string =>
  s.replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');

/** Extract clean clickable badges from `<ide_context>` or `--- File context ---` blocks so user bubbles stay crisp in both Threads and IDE. */
export function parseUserMessageWithBadges(raw: string): { badges: ParsedContextBadge[]; prose: string } {
  if (!raw) return { badges: [], prose: '' };
  const badges: ParsedContextBadge[] = [];

  for (const blockMatch of raw.matchAll(IDE_CTX_BLOCK_RE)) {
    const block = blockMatch[0];
    let matchedAny = false;
    for (const itemMatch of block.matchAll(IDE_ITEM_TAG_RE)) {
      matchedAny = true;
      const rawTag = itemMatch[1].toLowerCase();
      const attrs: Record<string, string> = {};
      for (const kv of itemMatch[2].matchAll(ATTR_KV_RE)) attrs[kv[1]] = decodeXmlAttr(kv[2]);
      const kind = (rawTag === 'item' ? attrs.kind || 'file' : rawTag) as ParsedContextBadge['kind'];
      const line = attrs.lines ? Number(attrs.lines.split('-')[0]) || undefined : Number(attrs.line) || undefined;
      const label =
        attrs.label ||
        (attrs.path ? `@${baseName(attrs.path)}${attrs.lines ? `:L${attrs.lines}` : ''}` : kind);
      badges.push({ kind, label, ...(attrs.path ? { path: attrs.path } : {}), ...(line ? { line } : {}) });
    }
    if (!matchedAny) badges.push({ kind: 'file', label: 'IDE context' });
  }

  let cleaned = raw
    .replace(/^echo:\s*(?=<ide_context\b)/i, '')
    .replace(IDE_CTX_BLOCK_RE, '')
    .replace(/^\[PLAN MODE:[^\]]*\]\s*/i, '')
    .replace(/^User request:\s*/i, '');

  cleaned = cleaned.replace(FILE_CTX_SECTION_RE, (_m, kindWord: string, label: string, path: string) => {
    const cleanPath = path.trim();
    const cleanLabel = label.trim();
    if (!badges.some((b) => b.path === cleanPath && b.label === cleanLabel)) {
      badges.push({
        kind: kindWord.toLowerCase() === 'folder' ? 'folder' : 'file',
        label: cleanLabel,
        path: cleanPath,
      });
    }
    return '';
  });

  return { badges, prose: cleaned.trim() };
}
