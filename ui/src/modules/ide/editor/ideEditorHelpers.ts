// Pure helpers for IDE editor safe-delete prompting and split-pane keyboard resizing.
// Kept rune-free and dependency-free so `node --test` unit tests can import it directly.

const IDENT_RE = /^[A-Za-z_]\w*$/;

/** Extract a valid identifier symbol from a file's basename for safe-delete reference lookup.
 *  Returns `null` for non-identifier names (e.g. `my-comp.svelte`, `foo.test.ts`) so callers
 *  skip symbol lookup instead of reading stale usages. */
export function extractSafeDeleteSymbol(filePath: string): string | null {
  const base = (filePath.split('/').pop() ?? '').replace(/\.[^.]+$/, '');
  return base && IDENT_RE.test(base) ? base : null;
}

export function formatSafeDeleteMessage(opts: {
  relPath: string;
  symbol: string | null;
  checked: boolean;
  externalRefs: number;
}): string {
  if (!opts.checked || !opts.symbol) {
    return `Delete “${opts.relPath}”? Symbol usage search was skipped because the file name isn’t a code identifier.`;
  }
  if (opts.externalRefs > 0) {
    const noun = opts.externalRefs === 1 ? 'workspace usage' : 'workspace usages';
    return `Found ${opts.externalRefs} ${noun} referencing “${opts.symbol}”. Delete “${opts.relPath}” anyway?`;
  }
  return `No external symbol usages found for “${opts.symbol}”. Delete “${opts.relPath}”?`;
}

const SPLIT_MIN = 0.2;
const SPLIT_MAX = 0.8;
const SPLIT_STEP = 0.05;
const SPLIT_BIG_STEP = 0.1;

function clampRatio(v: number): number {
  return Math.max(SPLIT_MIN, Math.min(SPLIT_MAX, Math.round(v * 1000) / 1000));
}

/** Compute next split ratio for keyboard resize on the split editor separator.
 *  Flips Left/Right in RTL side-by-side layouts; returns `null` if key is unhandled. */
export function computeSplitRatioFromKey(
  ratio: number,
  key: string,
  opts: { down: boolean; rtl: boolean; shiftKey?: boolean },
): number | null {
  if (key === 'Home') return SPLIT_MIN;
  if (key === 'End') return SPLIT_MAX;
  if (key === 'Enter') return 0.5;
  const step = opts.shiftKey ? SPLIT_BIG_STEP : SPLIT_STEP;
  if (opts.down) {
    if (key === 'ArrowUp') return clampRatio(ratio - step);
    if (key === 'ArrowDown') return clampRatio(ratio + step);
    return null;
  }
  if (key === 'ArrowLeft') return clampRatio(ratio + (opts.rtl ? step : -step));
  if (key === 'ArrowRight') return clampRatio(ratio + (opts.rtl ? -step : step));
  return null;
}
