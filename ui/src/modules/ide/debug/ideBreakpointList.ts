// The one breakpoint list (I2-11): grouped by file, each row says where it is,
// what code is there (when the file is loaded), its condition / hit / log
// chip, and whether the debugger could bind it.
//
// Import-free (type-only imports), so Node runs it directly in unit tests.

export interface BpLike {
  path: string;
  line: number;
  enabled: boolean;
  condition?: string | null;
  hit_condition?: string | null;
  log_message?: string | null;
  verified?: boolean;
  message?: string | null;
}

export interface BpRow<B extends BpLike> {
  bp: B;
  /** Trimmed code on that line, or '' when the file isn't loaded. */
  preview: string;
  chip: { kind: 'condition' | 'hit' | 'log'; label: string } | null;
  /** The adapter said it could not bind this breakpoint. */
  unbound: boolean;
}

export interface BpGroup<B extends BpLike> {
  path: string;
  rows: BpRow<B>[];
}

const PREVIEW_MAX = 80;

function chipFor(bp: BpLike): BpRow<BpLike>['chip'] {
  if (bp.log_message) return { kind: 'log', label: `log ${bp.log_message}` };
  if (bp.condition) return { kind: 'condition', label: `if ${bp.condition}` };
  if (bp.hit_condition) return { kind: 'hit', label: `hit ${bp.hit_condition}` };
  return null;
}

/** Group by file (files sorted by path, rows by line). `contentOf(path)`
 *  returns the loaded text of a file, or null. */
export function groupBreakpoints<B extends BpLike>(
  list: readonly B[],
  contentOf: (path: string) => string | null | undefined,
): BpGroup<B>[] {
  const byPath = new Map<string, B[]>();
  for (const bp of list) {
    const arr = byPath.get(bp.path);
    if (arr) arr.push(bp);
    else byPath.set(bp.path, [bp]);
  }
  const groups: BpGroup<B>[] = [];
  for (const path of [...byPath.keys()].sort()) {
    const text = contentOf(path);
    const lines = typeof text === 'string' ? text.split('\n') : null;
    const rows = (byPath.get(path) ?? [])
      .slice()
      .sort((a, b) => a.line - b.line)
      .map((bp) => {
        const raw = lines?.[bp.line - 1]?.trim() ?? '';
        return {
          bp,
          preview: raw.length > PREVIEW_MAX ? `${raw.slice(0, PREVIEW_MAX - 1)}…` : raw,
          chip: chipFor(bp),
          unbound: bp.verified === false,
        };
      });
    groups.push({ path, rows });
  }
  return groups;
}
