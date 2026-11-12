// "Explain these lines": read a text selection in the PR's Code tab back into
// diff rows, without a hook in DiffViewer. Every mounted row carries its key
// in `data-rk` (`<path>␟l<hunk>:<i>` unified, `…␟s…` split — diff-model.ts
// buildFileRows) and its numbers in the `.gut.old` / `.gut.new` cells, so the
// selection's rows, file and line range come straight off the DOM. Rows are
// windowed, so a selection only ever spans mounted rows.
import { ROW_KEY_SEP } from './diff-model';
import { linePickFrom, type LinePick, type PickedRow } from './pr-agent-context';

/** A row key's file path and row kind (`l` line, `s` split, `f` file, …). */
export function parseRowKey(key: string): { path: string; kind: string } | null {
  const i = key.lastIndexOf(ROW_KEY_SEP);
  if (i <= 0 || i === key.length - 1) return null;
  return { path: key.slice(0, i), kind: key[i + 1] };
}

/** The viewer's sign cell ('+', '−', '') as a diff sign. */
export const diffSign = (s: string | null | undefined): string => {
  const t = (s ?? '').trim();
  return t === '+' ? '+' : t === '−' || t === '-' ? '-' : ' ';
};

const lineNo = (el: Element | null): number | null => {
  const t = el?.textContent?.trim() ?? '';
  return /^\d+$/.test(t) ? Number(t) : null;
};

function splitRows(el: Element): PickedRow[] {
  const [left, right] = Array.from(el.querySelectorAll('.code.half'));
  const oldLine = lineNo(el.querySelector('.gut.old'));
  const newLine = lineNo(el.querySelector('.gut.new'));
  const text = (h: Element | undefined): string | null => (h && !h.classList.contains('void') ? (h.textContent ?? '') : null);
  const l = text(left);
  const r = text(right);
  if (l !== null && r !== null && !left.classList.contains('del') && !right.classList.contains('add')) {
    return [{ sign: ' ', oldLine, newLine, code: r }];
  }
  const out: PickedRow[] = [];
  if (l !== null) out.push({ sign: '-', oldLine, newLine: null, code: l });
  if (r !== null) out.push({ sign: '+', oldLine: null, newLine, code: r });
  return out;
}

/** The PR diff rows `sel` covers: one file's code rows, the PR number (from
 *  PrDetail's `data-pr`) and where to show the action. Null when the
 *  selection is not code rows of ONE file in a PR's Code tab. */
export function pickFromSelection(sel: Selection | null): { pick: LinePick; pr: number; rect: DOMRect } | null {
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
  const range = sel.getRangeAt(0);
  const at = (n: Node): Element | null => (n instanceof Element ? n : n.parentElement);
  const code = at(range.startContainer)?.closest('.prd-code');
  const end = at(range.endContainer);
  if (!code || !end || !code.contains(end)) return null;
  const pr = Number(code.closest<HTMLElement>('.prd')?.dataset.pr);
  if (!Number.isInteger(pr) || pr <= 0) return null;
  let path: string | null = null;
  const rows: PickedRow[] = [];
  for (const el of Array.from(code.querySelectorAll<HTMLElement>('.vrow[data-rk]'))) {
    if (!range.intersectsNode(el)) continue;
    const k = parseRowKey(el.dataset.rk ?? '');
    if (!k || (k.kind !== 'l' && k.kind !== 's')) continue;
    if (path === null) path = k.path;
    else if (path !== k.path) return null; // one file at a time
    if (k.kind === 's') rows.push(...splitRows(el));
    else rows.push({
      sign: diffSign(el.querySelector('.sign')?.textContent),
      oldLine: lineNo(el.querySelector('.gut.old')),
      newLine: lineNo(el.querySelector('.gut.new')),
      code: el.querySelector('.code')?.textContent ?? '',
    });
  }
  const pick = path === null ? null : linePickFrom(path, rows);
  return pick ? { pick, pr, rect: range.getBoundingClientRect() } : null;
}
