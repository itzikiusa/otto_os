// A small, safe XML/HTML/SVG pretty-printer for the Workbench "Format" action.
//
// Tokenises tags / text / comments / CDATA / doctype / processing
// instructions, rebuilds the element tree and re-indents it. Guarantees:
// - attribute text is kept byte-for-byte (the open tag is copied verbatim);
// - <pre>/<script>/<style>/<textarea> (HTML) and CDATA bodies are verbatim;
// - an element whose content is short inline text stays on one line;
// - formatting the output again yields the same output (idempotent).
// XML mode is strict (mismatched / unclosed tags → an error with its line);
// HTML mode is tolerant (void elements, implicit closes, stray end tags).

export type MarkupMode = 'xml' | 'html';

export class MarkupError extends Error {
  line: number;
  col: number;
  constructor(message: string, line: number, col: number) {
    super(message);
    this.line = line;
    this.col = col;
  }
}

const VOID = new Set([
  'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr',
]);
const RAW = new Set(['pre', 'script', 'style', 'textarea']);
/** Block-level children force a multi-line layout even when short. */
const BLOCK = new Set([
  'html', 'head', 'body', 'div', 'section', 'article', 'header', 'footer', 'nav', 'main', 'aside', 'ul', 'ol',
  'li', 'table', 'thead', 'tbody', 'tfoot', 'tr', 'td', 'th', 'p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'form',
  'fieldset', 'dl', 'dt', 'dd', 'figure', 'blockquote', 'pre', 'script', 'style', 'title', 'meta', 'link', 'g',
  'svg', 'defs', 'select', 'option',
]);
const INLINE_MAX = 100;

type Node =
  | { kind: 'el'; name: string; open: string; close: string; children: Node[]; selfClose: boolean; raw?: string }
  | { kind: 'text'; text: string }
  | { kind: 'other'; text: string };

function lineCol(src: string, pos: number): { line: number; col: number } {
  let line = 1;
  let last = -1;
  for (let i = 0; i < pos && i < src.length; i++) {
    if (src[i] === '\n') {
      line++;
      last = i;
    }
  }
  return { line, col: pos - last };
}

/** Index just past the `>` closing a tag that starts at `i`, honouring quotes. */
function tagEnd(src: string, i: number): number {
  let q = '';
  for (let j = i + 1; j < src.length; j++) {
    const c = src[j];
    if (q) {
      if (c === q) q = '';
    } else if (c === '"' || c === "'") {
      q = c;
    } else if (c === '>') {
      return j + 1;
    }
  }
  return -1;
}

function parse(src: string, mode: MarkupMode): Node[] {
  const root: Node[] = [];
  const stack: { node: Extract<Node, { kind: 'el' }>; pos: number }[] = [];
  const cur = (): Node[] => (stack.length ? stack[stack.length - 1].node.children : root);
  const fail = (msg: string, pos: number): never => {
    const { line, col } = lineCol(src, pos);
    throw new MarkupError(`${msg} (line ${line}, col ${col})`, line, col);
  };
  const html = mode === 'html';
  let i = 0;
  const n = src.length;
  while (i < n) {
    if (src[i] !== '<') {
      const j = src.indexOf('<', i);
      const end = j < 0 ? n : j;
      cur().push({ kind: 'text', text: src.slice(i, end) });
      i = end;
      continue;
    }
    const pairs: [string, string][] = [
      ['<!--', '-->'],
      ['<![CDATA[', ']]>'],
      ['<?', '?>'],
    ];
    let handled = false;
    for (const [a, b] of pairs) {
      if (src.startsWith(a, i)) {
        const j = src.indexOf(b, i + a.length);
        if (j < 0) fail(`unterminated ${a}`, i);
        cur().push({ kind: 'other', text: src.slice(i, j + b.length) });
        i = j + b.length;
        handled = true;
        break;
      }
    }
    if (handled) continue;
    if (src.startsWith('<!', i)) {
      const e = tagEnd(src, i);
      if (e < 0) fail('unterminated declaration', i);
      cur().push({ kind: 'other', text: src.slice(i, e) });
      i = e;
      continue;
    }
    if (src.startsWith('</', i)) {
      const e = tagEnd(src, i);
      if (e < 0) fail('unterminated end tag', i);
      const name = src.slice(i + 2, e - 1).trim();
      const key = html ? name.toLowerCase() : name;
      const idx = stack.map((s) => (html ? s.node.name.toLowerCase() : s.node.name)).lastIndexOf(key);
      if (idx < 0) {
        if (!html) fail(`unexpected </${name}>`, i);
        // Stray end tag in HTML: keep it, verbatim, where it was.
        cur().push({ kind: 'other', text: src.slice(i, e) });
      } else {
        if (!html && idx !== stack.length - 1) {
          fail(`expected </${stack[stack.length - 1].node.name}> but found </${name}>`, i);
        }
        while (stack.length > idx + 1) stack.pop();
        const top = stack.pop();
        if (top) top.node.close = src.slice(i, e);
      }
      i = e;
      continue;
    }
    const m = /^<([A-Za-z_][\w:.-]*)/.exec(src.slice(i, i + 256));
    if (!m) {
      // A lone `<` in text (tolerated in HTML, an error in XML).
      if (!html) fail("stray '<'", i);
      cur().push({ kind: 'text', text: '<' });
      i++;
      continue;
    }
    const e = tagEnd(src, i);
    if (e < 0) fail(`unterminated <${m[1]}>`, i);
    const open = src.slice(i, e);
    const name = m[1];
    const lname = name.toLowerCase();
    const selfClose = /\/\s*>$/.test(open) || (html && VOID.has(lname));
    const node: Extract<Node, { kind: 'el' }> = { kind: 'el', name, open, close: '', children: [], selfClose };
    cur().push(node);
    i = e;
    if (selfClose) continue;
    if (html && RAW.has(lname)) {
      const re = new RegExp(`</${lname}\\s*>`, 'i');
      const rest = src.slice(i);
      const mm = re.exec(rest);
      if (!mm) {
        node.raw = rest;
        node.close = `</${name}>`;
        i = n;
      } else {
        node.raw = rest.slice(0, mm.index);
        node.close = mm[0];
        i += mm.index + mm[0].length;
      }
      continue;
    }
    stack.push({ node, pos: i });
  }
  if (stack.length) {
    if (!html) fail(`unclosed <${stack[stack.length - 1].node.name}>`, n);
    for (const s of stack) s.node.close = `</${s.node.name}>`;
  }
  return root;
}

/** One-line rendering of a node, or null when it can't/shouldn't be inline. */
function inline(node: Node, mode: MarkupMode): string | null {
  // XML text is data: keep its inner spacing (a newline forces block layout).
  if (node.kind === 'text') return mode === 'xml' ? (node.text.includes('\n') && node.text.trim() ? null : node.text) : node.text.replace(/\s+/g, ' ');
  // XML: only text / CDATA stays inline (any child element or comment → block).
  if (mode === 'xml') return node.kind === 'other' && node.text.startsWith('<![CDATA[') && !node.text.includes('\n') ? node.text : null;
  if (node.kind === 'other') return node.text.includes('\n') ? null : node.text;
  if (BLOCK.has(node.name.toLowerCase()) || node.raw !== undefined || node.open.includes('\n')) return null;
  if (node.selfClose) return node.open;
  let s = node.open;
  for (const c of node.children) {
    const r = inline(c, mode);
    if (r === null) return null;
    s += r;
  }
  return s + node.close;
}

function print(nodes: Node[], depth: number, unit: string, mode: MarkupMode, out: string[]): void {
  const pad = unit.repeat(depth);
  for (const node of nodes) {
    if (node.kind === 'text') {
      for (const line of node.text.split('\n')) {
        const t = line.trim();
        if (t) out.push(pad + (mode === 'xml' ? t : t.replace(/[ \t]+/g, ' ')));
      }
      continue;
    }
    if (node.kind === 'other') {
      out.push(pad + node.text);
      continue;
    }
    if (node.selfClose) {
      out.push(pad + node.open);
      continue;
    }
    if (node.raw !== undefined) {
      out.push(pad + node.open + node.raw + node.close);
      continue;
    }
    // Short content with no block children → one line.
    let body = '';
    let ok = true;
    for (const c of node.children) {
      const r = inline(c, mode);
      if (r === null) {
        ok = false;
        break;
      }
      body += r;
    }
    body = body.trim();
    if (ok && pad.length + node.open.length + body.length + node.close.length <= INLINE_MAX + pad.length) {
      out.push(pad + node.open + body + node.close);
      continue;
    }
    out.push(pad + node.open);
    print(node.children, depth + 1, unit, mode, out);
    out.push(pad + node.close);
  }
}

/** Pretty-print markup. Throws `MarkupError` on invalid XML. */
export function formatMarkup(src: string, mode: MarkupMode, indent = 2): string {
  const tree = parse(src, mode);
  const out: string[] = [];
  print(tree, 0, ' '.repeat(indent), mode, out);
  return out.join('\n');
}

/** Validate markup; null when well-formed (XML) / parseable (HTML). */
export function validateMarkup(src: string, mode: MarkupMode): MarkupError | null {
  try {
    parse(src, mode);
    return null;
  } catch (e) {
    if (e instanceof MarkupError) return e;
    throw e;
  }
}
