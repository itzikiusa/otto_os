// `use:listNav` — the ONE keyboard walk of every list, tree and nav
// (Navigator, Rail, list panes): ↑ / ↓ move, Home / End jump, Enter or Space
// opens (rows that are not buttons are clicked), → / ← open and close a tree
// node or section (any row with aria-expanded), ← on a nested row goes to its
// parent (aria-level). The ContextMenu key / ⇧F10 stays with `use:rowMenu`
// on the row (SideRow and InboxRow carry it).
//
//   <div class="side-list-body" use:listNav>…rows with [data-side-row]…</div>
//   <nav use:listNav={{ rows: '.rail-btn' }}>…</nav>
//
// Only rows that are drawn count (a folded section's rows are skipped), and
// the walk never wraps. Modified keys and keys typed into a field inside the
// list pass through.

export const SIDE_ROW = '[data-side-row]';

/** The row index a key moves to, or null when the key isn't a move. */
export function moveIndex(key: string, from: number, count: number): number | null {
  if (count <= 0) return null;
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  if (key === 'ArrowDown') return from < 0 ? 0 : Math.min(count - 1, from + 1);
  if (key === 'ArrowUp') return from < 0 ? 0 : Math.max(0, from - 1);
  return null;
}

export type TreeStep = 'expand' | 'collapse' | 'child' | 'parent';
/** What → / ← do on a row: `expanded` is its aria-expanded (null = a leaf),
 *  `level` its aria-level (1 = top). In RTL the arrows swap. */
export function treeStep(key: string, expanded: boolean | null, level: number, rtl = false): TreeStep | null {
  const k = rtl ? (key === 'ArrowLeft' ? 'ArrowRight' : key === 'ArrowRight' ? 'ArrowLeft' : key) : key;
  if (k === 'ArrowRight') return expanded === false ? 'expand' : expanded === true ? 'child' : null;
  if (k === 'ArrowLeft') return expanded === true ? 'collapse' : level > 1 ? 'parent' : null;
  return null;
}

const FIELD = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])';

export interface ListNavOptions {
  /** The rows' selector (default `[data-side-row]`). */
  rows?: string;
}

function level(el: HTMLElement): number {
  return Number(el.getAttribute('aria-level')) || 1;
}

export function listNav(node: HTMLElement, opts: ListNavOptions = {}) {
  let sel = opts.rows ?? SIDE_ROW;
  const rows = (): HTMLElement[] =>
    [...node.querySelectorAll<HTMLElement>(sel)].filter((r) => r.getClientRects().length > 0 && !r.closest('[inert]'));

  function onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || e.altKey || e.metaKey || e.ctrlKey || e.shiftKey) return;
    const target = e.target as HTMLElement | null;
    if (!target || target.closest(FIELD)) return;
    const row = target.closest<HTMLElement>(sel);
    if (!row || !node.contains(row)) return;
    const list = rows();
    const at = list.indexOf(row);
    const next = moveIndex(e.key, at, list.length);
    if (next !== null) {
      e.preventDefault();
      list[next]?.focus();
      list[next]?.scrollIntoView?.({ block: 'nearest' });
      return;
    }
    if ((e.key === 'Enter' || e.key === ' ') && row === target && !row.matches('button, a[href], summary')) {
      e.preventDefault();
      row.click();
      return;
    }
    const exp = row.getAttribute('aria-expanded');
    const rtl = getComputedStyle(row).direction === 'rtl';
    const step = treeStep(e.key, exp === null ? null : exp === 'true', level(row), rtl);
    if (!step) return;
    e.preventDefault();
    if (step === 'expand' || step === 'collapse') {
      // The row itself toggles, or a twisty beside it does.
      const toggle = row.matches('[data-side-toggle]') ? row : row.parentElement?.querySelector<HTMLElement>('[data-side-toggle]') ?? row;
      toggle.click();
    } else if (step === 'child') {
      list[at + 1]?.focus();
    } else {
      const mine = level(row);
      for (let i = at - 1; i >= 0; i--) {
        if (level(list[i]) < mine) {
          list[i].focus();
          break;
        }
      }
    }
  }

  node.addEventListener('keydown', onKey);
  return {
    update(next: ListNavOptions = {}) {
      sel = next.rows ?? SIDE_ROW;
    },
    destroy() {
      node.removeEventListener('keydown', onKey);
    },
  };
}
