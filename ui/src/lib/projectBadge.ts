// A project's badge: two letters and a stable categorical tone (--cat-1…6),
// the same for a name everywhere it shows (the Threads top bar, inbox rows on
// every page — lib/components/InboxRow.svelte).
// Pure, import-free (unit/threadsProjectBadge.test.ts).

export interface ProjectBadgeInfo {
  /** Two upper-case letters: the first two words' initials, else a single
   *  word's first and last letter ("harbor-api" → HA, "harbor" → HR). */
  initials: string;
  /** 1–6: which `--cat-N` token tints it. */
  tone: number;
}

export function projectBadge(name: string): ProjectBadgeInfo {
  const words = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  const [a = '', b = ''] = words;
  const pair = b ? [...a][0] + [...b][0] : [...a].length > 1 ? [...a][0] + [...a].at(-1) : [...a][0] ?? '?';
  let h = 0;
  for (const c of name) h = (h * 31 + (c.codePointAt(0) ?? 0)) >>> 0;
  return { initials: pair.toUpperCase(), tone: (h % 6) + 1 };
}
