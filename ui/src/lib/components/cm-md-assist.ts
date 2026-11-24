// Markdown writing help for the IDE editor (.md / .markdown), loaded with the Markdown pack
// (cm-langs.ts) and active only in IDE editors (`.is-ide`): the formatting row's commands
// (IdeMarkdownBar), ⌘B, Tab / Shift-Tab in lists and tables, a closing fence on
// Enter, ` * _ ~ around a selection, a "/" menu at the start of an empty line, and Enter
// continuing a list / checklist / quote (lang-markdown's command). A URL pasted over a
// selection becomes a link (lang-markdown's pasteURLAsLink).
import { completionStatus, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { indentLess, indentMore } from '@codemirror/commands';
import { insertNewlineContinueMarkupCommand, pasteURLAsLink } from '@codemirror/lang-markdown';
import { EditorSelection, EditorState, Prec, type ChangeSpec, type Extension } from '@codemirror/state';
import { EditorView, keymap, type Command } from '@codemirror/view';

const inIde = (view: EditorView): boolean => !!view.dom.closest('.is-ide');
const ide = (cmd: Command): Command => (view) => inIde(view) && cmd(view);

// ── Inline formatting ─────────────────────────────────────────────────────

/** Wrap each selection in `mark` (or unwrap it when already wrapped); an empty
 *  selection gets the pair with the cursor between. */
export function toggleWrap(mark: string): Command {
  return (view) => {
    const { state } = view;
    const tr = state.changeByRange((r) => {
      const n = mark.length;
      const before = state.sliceDoc(r.from - n, r.from);
      const after = state.sliceDoc(r.to, r.to + n);
      if (before === mark && after === mark) {
        return {
          changes: [{ from: r.from - n, to: r.from }, { from: r.to, to: r.to + n }],
          range: EditorSelection.range(r.from - n, r.to - n),
        };
      }
      return {
        changes: [{ from: r.from, insert: mark }, { from: r.to, insert: mark }],
        range: EditorSelection.range(r.from + n, r.to + n),
      };
    });
    view.dispatch(state.update(tr, { scrollIntoView: true, userEvent: 'input' }));
    return true;
  };
}

/** `[text](url)`: the selection becomes the text and "url" is selected; with no
 *  selection "text" is selected. */
export const insertLink: Command = (view) => {
  const { state } = view;
  const tr = state.changeByRange((r) => {
    const text = state.sliceDoc(r.from, r.to);
    const insert = `[${text || 'text'}](url)`;
    const sel = text ? EditorSelection.range(r.from + text.length + 3, r.from + text.length + 6) : EditorSelection.range(r.from + 1, r.from + 5);
    return { changes: { from: r.from, to: r.to, insert }, range: sel };
  });
  view.dispatch(state.update(tr, { scrollIntoView: true, userEvent: 'input' }));
  return true;
};

// ── Line prefixes ─────────────────────────────────────────────────────────

export type LineKind = 'bullet' | 'number' | 'check' | 'quote';
const ANY_PREFIX = /^(\s*)(?:[-*+] \[[ xX]\] |[-*+] |\d+[.)] |> ?)?/;
const HAS: Record<LineKind, RegExp> = {
  bullet: /^\s*[-*+] (?!\[[ xX]\] )/,
  number: /^\s*\d+[.)] /,
  check: /^\s*[-*+] \[[ xX]\] /,
  quote: /^\s*> ?/,
};

function selectedLines(state: EditorState): number[] {
  const out = new Set<number>();
  for (const r of state.selection.ranges) {
    const a = state.doc.lineAt(r.from).number;
    const b = state.doc.lineAt(r.to).number;
    for (let n = a; n <= b; n++) out.add(n);
  }
  return [...out].sort((x, y) => x - y);
}

/** Turn the selected lines into a list / checklist / quote, or back to text when
 *  they all already are one. */
export function toggleLinePrefix(kind: LineKind): Command {
  return (view) => {
    const { state } = view;
    const lines = selectedLines(state).map((n) => state.doc.line(n));
    const off = lines.every((l) => HAS[kind].test(l.text));
    let i = 0;
    const changes: ChangeSpec[] = lines.map((l) => {
      const m = ANY_PREFIX.exec(l.text)!;
      const indent = m[1];
      const prefix = off ? '' : kind === 'bullet' ? '- ' : kind === 'number' ? `${++i}. ` : kind === 'check' ? '- [ ] ' : '> ';
      return { from: l.from + indent.length, to: l.from + m[0].length, insert: prefix };
    });
    view.dispatch({ changes, scrollIntoView: true, userEvent: 'input' });
    return true;
  };
}

/** `#`…`###` on the cursor's lines (the same level again removes it). */
export function setHeading(level: number): Command {
  return (view) => {
    const { state } = view;
    const changes: ChangeSpec[] = selectedLines(state).map((n) => {
      const l = state.doc.line(n);
      const m = /^(#{1,6}) +/.exec(l.text);
      const same = m && m[1].length === level;
      return { from: l.from, to: l.from + (m ? m[0].length : 0), insert: same ? '' : `${'#'.repeat(level)} ` };
    });
    view.dispatch({ changes, scrollIntoView: true, userEvent: 'input' });
    return true;
  };
}

// ── Blocks: the "/" menu and the formatting row share these ───────────────

export interface BlockSnippet {
  label: string;
  detail: string;
  text: string;
  /** Where the cursor (or selection [from, to]) goes, as offsets into `text`. */
  select: [number, number];
}

const TABLE = '| Column 1 | Column 2 | Column 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |';

export const BLOCKS: BlockSnippet[] = [
  { label: 'Heading 1', detail: '#', text: '# ', select: [2, 2] },
  { label: 'Heading 2', detail: '##', text: '## ', select: [3, 3] },
  { label: 'Heading 3', detail: '###', text: '### ', select: [4, 4] },
  { label: 'Table', detail: '3 × 3', text: TABLE, select: [2, 10] },
  { label: 'Code block', detail: '```', text: '```\n\n```', select: [4, 4] },
  { label: 'Checklist', detail: '- [ ]', text: '- [ ] ', select: [6, 6] },
  { label: 'Quote', detail: '>', text: '> ', select: [2, 2] },
  { label: 'Divider', detail: '---', text: '---\n', select: [4, 4] },
  { label: 'Callout', detail: '> [!NOTE]', text: '> [!NOTE]\n> ', select: [12, 12] },
];

/** Put a block on the cursor's line: in place of an empty line (or of `/`+query), else
 *  on a new line below with a blank line between. */
export function insertBlock(view: EditorView, b: BlockSnippet, from?: number, to?: number): void {
  const { state } = view;
  const line = state.doc.lineAt(state.selection.main.head);
  let at = from ?? line.from;
  let end = to ?? line.to;
  let lead = '';
  if (from === undefined && line.text.trim()) {
    at = end = line.to;
    lead = '\n\n';
  }
  const base = at + lead.length;
  view.dispatch({
    changes: { from: at, to: end, insert: lead + b.text },
    selection: EditorSelection.range(base + b.select[0], base + b.select[1]),
    scrollIntoView: true,
    userEvent: 'input',
  });
  view.focus();
}

/** "/" at the start of an empty line (a query may follow): the block menu. */
export function slashCompletions(ctx: CompletionContext): CompletionResult | null {
  if (ctx.view && !inIde(ctx.view)) return null;
  const line = ctx.state.doc.lineAt(ctx.pos);
  const before = ctx.state.sliceDoc(line.from, ctx.pos);
  if (!/^\/[\w ]*$/.test(before) || ctx.state.sliceDoc(ctx.pos, line.to).trim()) return null;
  return {
    from: line.from + 1,
    validFor: /^[\w ]*$/,
    options: BLOCKS.map(
      (b): Completion => ({
        label: b.label,
        detail: b.detail,
        type: 'text',
        apply: (view, _c, from, to) => insertBlock(view, b, from - 1, to),
      }),
    ),
  };
}

// ── Tables ────────────────────────────────────────────────────────────────

const isRow = (t: string): boolean => /^\s*\|.*\|\s*$/.test(t);
const cellsOf = (t: string): string[] =>
  t.trim().replace(/^\|/, '').replace(/\|$/, '').split(/(?<!\\)\|/).map((c) => c.trim());

/** Align a pipe table's columns (the separator row keeps its `:` alignment). */
export function formatTable(rows: string[]): string[] {
  const cells = rows.map(cellsOf);
  const cols = Math.max(...cells.map((r) => r.length));
  const isSep = (r: string[]): boolean => r.every((c) => /^:?-+:?$/.test(c));
  const width = Array.from({ length: cols }, (_, i) =>
    Math.max(3, ...cells.filter((r) => !isSep(r)).map((r) => (r[i] ?? '').length)),
  );
  return cells.map((r) => {
    const sep = isSep(r);
    const out = width.map((w, i) => {
      const c = r[i] ?? '';
      if (!sep) return c.padEnd(w);
      const left = c.startsWith(':'), right = c.endsWith(':');
      return `${left ? ':' : '-'}${'-'.repeat(w - 2)}${right ? ':' : '-'}`;
    });
    return `| ${out.join(' | ')} |`;
  });
}

/** The table block (first and last line numbers) around `lineNo`, or null. */
function tableAround(state: EditorState, lineNo: number): [number, number] | null {
  if (!isRow(state.doc.line(lineNo).text)) return null;
  let a = lineNo, b = lineNo;
  while (a > 1 && isRow(state.doc.line(a - 1).text)) a--;
  while (b < state.doc.lines && isRow(state.doc.line(b + 1).text)) b++;
  return b > a ? [a, b] : null;
}

/** "Format table": align the table under the cursor. */
export const formatTableAtCursor: Command = (view) => {
  const { state } = view;
  const block = tableAround(state, state.doc.lineAt(state.selection.main.head).number);
  if (!block) return false;
  const from = state.doc.line(block[0]).from, to = state.doc.line(block[1]).to;
  const rows = state.sliceDoc(from, to).split('\n');
  const next = formatTable(rows).join('\n');
  if (next !== rows.join('\n')) view.dispatch({ changes: { from, to, insert: next }, userEvent: 'input' });
  return true;
};

/** Tab in a table: the next cell (Shift-Tab the previous); past the last cell a new row. */
export function tableTab(dir: 1 | -1): Command {
  return (view) => {
    const { state } = view;
    const head = state.selection.main.head;
    const line = state.doc.lineAt(head);
    const block = tableAround(state, line.number);
    if (!block) return false;
    const pipes: number[] = [];
    for (let n = block[0]; n <= block[1]; n++) {
      const l = state.doc.line(n);
      if (/^\s*\|[\s:|-]+\|\s*$/.test(l.text) && n !== block[0]) continue; // the separator row
      for (let i = 0; i < l.text.length; i++) if (l.text[i] === '|' && l.text[i - 1] !== '\\') pipes.push(l.from + i);
    }
    // Cell starts: after each pipe that is not a row's last.
    const starts = pipes.filter((p) => p !== state.doc.lineAt(p).from + state.doc.lineAt(p).text.trimEnd().length - 1).map((p) => p + 1);
    const cur = dir === 1 ? starts.findIndex((s) => s > head) : starts.findLastIndex((s) => s < head - 1);
    if (cur < 0) {
      if (dir === -1) return true;
      const last = state.doc.line(block[1]);
      const cols = cellsOf(last.text).length;
      const row = `\n|${'  |'.repeat(cols)}`;
      view.dispatch({ changes: { from: last.to, insert: row }, selection: { anchor: last.to + 3 }, scrollIntoView: true, userEvent: 'input' });
      return true;
    }
    const s = starts[cur];
    const text = state.doc.lineAt(s).text;
    const rel = s - state.doc.lineAt(s).from;
    const end = text.indexOf('|', rel);
    const cell = text.slice(rel, end < 0 ? text.length : end);
    const lead = cell.length - cell.trimStart().length;
    const body = cell.trim().length;
    view.dispatch({ selection: EditorSelection.range(s + Math.min(lead, 1), s + (body ? lead + body : Math.min(lead, 1))), scrollIntoView: true });
    return true;
  };
}

// ── Lists: Tab / Shift-Tab indent the whole item ──────────────────────────

const onListItem = (state: EditorState): boolean =>
  selectedLines(state).every((n) => /^\s*(?:[-*+]|\d+[.)]) /.test(state.doc.line(n).text));
export const listIndent = (more: boolean): Command => (view) => onListItem(view.state) && (more ? indentMore(view) : indentLess(view));

// ── Typing help ───────────────────────────────────────────────────────────

/** Enter after an opening fence that nothing closes: the closing fence too, cursor inside. */
const closeFence: Command = (view) => {
  const { state } = view;
  const { head, empty } = state.selection.main;
  const line = state.doc.lineAt(head);
  const m = /^(\s*)(`{3,}|~{3,})[\w+-]*\s*$/.exec(line.text);
  if (!empty || !m || head !== line.to) return false;
  let fences = 0;
  for (let n = 1; n <= state.doc.lines; n++) if (/^\s*(`{3,}|~{3,})/.test(state.doc.line(n).text)) fences++;
  if (fences % 2 === 0) return false;
  const insert = `\n${m[1]}\n${m[1]}${m[2]}`;
  view.dispatch({ changes: { from: head, insert }, selection: { anchor: head + 1 + m[1].length }, scrollIntoView: true, userEvent: 'input' });
  return true;
};

const WRAP_CHARS = new Set(['`', '*', '_', '~']);

const typing = EditorView.inputHandler.of((view, from, to, text) => {
  if (!WRAP_CHARS.has(text) || from === to || view.state.selection.ranges.length !== 1 || !inIde(view)) return false;
  return toggleWrap(text)(view);
});

/** Enter: a closing fence, else lang-markdown's list / checklist / quote continuation, an
 *  empty item ending the list (the IDE's own Enter keeps indentation only). */
const continueMarkup = insertNewlineContinueMarkupCommand({ nonTightLists: false });
export const markdownEnter: Command = (view) =>
  completionStatus(view.state) !== 'active' && (closeFence(view) || continueMarkup(view));

export function markdownAssist(): Extension {
  return [
    // Above the IDE keymap, whose JetBrains ⌘B is Go to Definition (nothing to go to in
    // prose). ⌘I, ⌘E and ⌘K stay the IDE's (Edit with AI, Recent files, the command bar):
    // the app claims those chords before the editor sees them.
    Prec.highest(
      keymap.of([
        { key: 'Mod-b', run: ide(toggleWrap('**')) },
        { key: 'Tab', run: ide((v) => tableTab(1)(v) || listIndent(true)(v)) },
        { key: 'Shift-Tab', run: ide((v) => tableTab(-1)(v) || listIndent(false)(v)) },
        { key: 'Enter', run: ide(markdownEnter) },
      ]),
    ),
    EditorState.languageData.of(() => [{ autocomplete: slashCompletions }]),
    typing,
    pasteURLAsLink,
  ];
}
