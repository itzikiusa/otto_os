// Tokenizer & recursive-descent JS subset parser for CSP-safe API Client scripts.

type TokKind = 'num' | 'str' | 'tmpl' | 'regex' | 'id' | 'op' | 'punc' | 'eof';
interface Tok { kind: TokKind; val: string; pos: number; }

const TWO_CHAR = new Set(['===', '!==', '>>>', '??=', '||=', '&&=', '==', '!=', '<=', '>=', '&&', '||', '??', '=>', '++', '--', '+=', '-=', '*=', '/=', '%=', '**', '?.', '<<', '>>', '...']);
const KEYWORDS = new Set(['let', 'const', 'var', 'function', 'return', 'if', 'else', 'for', 'while', 'do', 'break', 'continue', 'try', 'catch', 'finally', 'throw', 'new', 'typeof', 'void', 'delete', 'in', 'of', 'instanceof', 'switch', 'case', 'default', 'true', 'false', 'null', 'undefined']);

function canStartRegex(prev: Tok | null): boolean {
  if (!prev) return true;
  if (prev.kind === 'op') return prev.val !== '++' && prev.val !== '--';
  if (prev.kind === 'punc') return prev.val !== ')' && prev.val !== ']' && prev.val !== '}';
  if (prev.kind === 'id') return prev.val === 'return' || prev.val === 'throw' || prev.val === 'case' || prev.val === 'in' || prev.val === 'of' || prev.val === 'typeof' || prev.val === 'void' || prev.val === 'delete' || prev.val === 'new';
  return false;
}

function unescapeStr(src: string, start: number, quote: string): [string, number] {
  let out = '', i = start;
  while (i < src.length) {
    const ch = src[i++];
    if (ch === quote) return [out, i];
    if (ch === '\\') {
      if (i >= src.length) throw new SyntaxError('Unterminated string escape');
      const e = src[i++];
      if (e === 'n') out += '\n';
      else if (e === 'r') out += '\r';
      else if (e === 't') out += '\t';
      else if (e === '0') out += '\0';
      else if (e === 'u') {
        if (src[i] === '{') {
          const end = src.indexOf('}', i + 1);
          if (end < 0) throw new SyntaxError('Invalid unicode escape');
          out += String.fromCodePoint(parseInt(src.slice(i + 1, end), 16));
          i = end + 1;
        } else {
          out += String.fromCharCode(parseInt(src.slice(i, i + 4), 16));
          i += 4;
        }
      } else if (e === 'x') {
        out += String.fromCharCode(parseInt(src.slice(i, i + 2), 16));
        i += 2;
      } else out += e;
    } else out += ch;
  }
  throw new SyntaxError('Unterminated string literal');
}

function tokenize(src: string): Tok[] {
  const toks: Tok[] = [];
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i];
    if (ch <= ' ') { i++; continue; }
    if (ch === '/' && src[i + 1] === '/') { i += 2; while (i < n && src[i] !== '\n') i++; continue; }
    if (ch === '/' && src[i + 1] === '*') {
      const end = src.indexOf('*/', i + 2);
      if (end < 0) throw new SyntaxError('Unterminated block comment');
      i = end + 2; continue;
    }
    const pos = i;
    if (ch === '\'' || ch === '"') {
      const [val, next] = unescapeStr(src, i + 1, ch);
      toks.push({ kind: 'str', val, pos }); i = next; continue;
    }
    if (ch === '`') {
      // Encode template literal raw content so parser handles `${...}` cleanly.
      let j = i + 1, depth = 0;
      while (j < n) {
        if (src[j] === '\\') { j += 2; continue; }
        if (depth === 0 && src[j] === '`') break;
        if (src[j] === '$' && src[j + 1] === '{') { depth++; j += 2; continue; }
        if (depth > 0 && src[j] === '}') { depth--; j++; continue; }
        j++;
      }
      if (j >= n) throw new SyntaxError('Unterminated template literal');
      toks.push({ kind: 'tmpl', val: src.slice(i + 1, j), pos });
      i = j + 1; continue;
    }
    if (ch === '/' && canStartRegex(toks[toks.length - 1] ?? null)) {
      let j = i + 1, inClass = false;
      while (j < n) {
        if (src[j] === '\\') { j += 2; continue; }
        if (src[j] === '[') inClass = true;
        else if (src[j] === ']') inClass = false;
        else if (src[j] === '/' && !inClass) break;
        j++;
      }
      if (j >= n) throw new SyntaxError('Unterminated regex literal');
      let f = j + 1;
      while (f < n && /[a-z]/i.test(src[f])) f++;
      toks.push({ kind: 'regex', val: src.slice(i + 1, j) + '\0' + src.slice(j + 1, f), pos });
      i = f; continue;
    }
    if ((ch >= '0' && ch <= '9') || (ch === '.' && src[i + 1] >= '0' && src[i + 1] <= '9')) {
      let j = i;
      if (ch === '0' && (src[j + 1] === 'x' || src[j + 1] === 'X')) {
        j += 2; while (j < n && /[0-9a-fA-F_]/.test(src[j])) j++;
      } else {
        while (j < n && /[0-9_]/.test(src[j])) j++;
        if (src[j] === '.' && src[j + 1] !== '.') { j++; while (j < n && /[0-9_]/.test(src[j])) j++; }
        if (src[j] === 'e' || src[j] === 'E') {
          j++; if (src[j] === '+' || src[j] === '-') j++;
          while (j < n && /[0-9_]/.test(src[j])) j++;
        }
      }
      toks.push({ kind: 'num', val: src.slice(i, j).replace(/_/g, ''), pos });
      i = j; continue;
    }
    if (/[a-zA-Z_$]/.test(ch)) {
      let j = i + 1;
      while (j < n && /[a-zA-Z0-9_$]/.test(src[j])) j++;
      toks.push({ kind: 'id', val: src.slice(i, j), pos });
      i = j; continue;
    }
    const s3 = src.slice(i, i + 3);
    if (TWO_CHAR.has(s3)) { toks.push({ kind: s3 === '...' ? 'punc' : 'op', val: s3, pos }); i += 3; continue; }
    const s2 = src.slice(i, i + 2);
    if (TWO_CHAR.has(s2)) { toks.push({ kind: s2 === '?.' ? 'punc' : 'op', val: s2, pos }); i += 2; continue; }
    if ('(){}[],;:.'.includes(ch)) { toks.push({ kind: 'punc', val: ch, pos }); i++; continue; }
    if ('+-*/%<>=!&|^~?'.includes(ch)) { toks.push({ kind: 'op', val: ch, pos }); i++; continue; }
    throw new SyntaxError(`Unexpected character ${JSON.stringify(ch)} at ${pos}`);
  }
  toks.push({ kind: 'eof', val: '', pos: n });
  return toks;
}

export type AstNode = Record<string, any>;

class Parser {
  private toks: Tok[];
  private i = 0;
  constructor(toks: Tok[]) { this.toks = toks; }
  private peek(off = 0): Tok { return this.toks[Math.min(this.i + off, this.toks.length - 1)]; }
  private next(): Tok { return this.toks[this.i++]; }
  private is(kind: TokKind, val?: string): boolean {
    const t = this.peek();
    return t.kind === kind && (val === undefined || t.val === val);
  }
  private eat(kind: TokKind, val?: string): boolean {
    if (this.is(kind, val)) { this.i++; return true; }
    return false;
  }
  private expect(kind: TokKind, val?: string): Tok {
    const t = this.peek();
    if (t.kind !== kind || (val !== undefined && t.val !== val)) {
      throw new SyntaxError(`Expected ${val ?? kind}, got ${t.val || t.kind} at ${t.pos}`);
    }
    return this.next();
  }

  parseProgram(): AstNode[] {
    const body: AstNode[] = [];
    while (!this.is('eof')) body.push(this.parseStmt());
    return body;
  }

  private parseStmt(): AstNode {
    if (this.eat('punc', ';')) return { t: 'empty' };
    if (this.is('punc', '{')) return this.parseBlock();
    if (this.is('id', 'let') || this.is('id', 'const') || this.is('id', 'var')) return this.parseVarDecl(true);
    if (this.is('id', 'function')) return this.parseFuncDecl();
    if (this.eat('id', 'if')) {
      this.expect('punc', '('); const test = this.parseExpr(); this.expect('punc', ')');
      const cons = this.parseStmt();
      const alt = this.eat('id', 'else') ? this.parseStmt() : null;
      return { t: 'if', test, cons, alt };
    }
    if (this.eat('id', 'while')) {
      this.expect('punc', '('); const test = this.parseExpr(); this.expect('punc', ')');
      return { t: 'while', test, body: this.parseStmt() };
    }
    if (this.eat('id', 'do')) {
      const body = this.parseStmt();
      this.expect('id', 'while'); this.expect('punc', '('); const test = this.parseExpr(); this.expect('punc', ')');
      this.eat('punc', ';');
      return { t: 'do', test, body };
    }
    if (this.eat('id', 'for')) return this.parseFor();
    if (this.eat('id', 'switch')) return this.parseSwitch();
    if (this.eat('id', 'return')) {
      const arg = (this.is('punc', ';') || this.is('punc', '}') || this.is('eof')) ? null : this.parseExpr();
      this.eat('punc', ';');
      return { t: 'return', arg };
    }
    if (this.eat('id', 'break')) { this.eat('punc', ';'); return { t: 'break' }; }
    if (this.eat('id', 'continue')) { this.eat('punc', ';'); return { t: 'continue' }; }
    if (this.eat('id', 'throw')) { const arg = this.parseExpr(); this.eat('punc', ';'); return { t: 'throw', arg }; }
    if (this.eat('id', 'try')) {
      const block = this.parseBlock();
      let param: string | null = null, handler: AstNode | null = null, finalizer: AstNode | null = null;
      if (this.eat('id', 'catch')) {
        if (this.eat('punc', '(')) { param = this.expect('id').val; this.expect('punc', ')'); }
        handler = this.parseBlock();
      }
      if (this.eat('id', 'finally')) finalizer = this.parseBlock();
      return { t: 'try', block, param, handler, finalizer };
    }
    const expr = this.parseExpr();
    this.eat('punc', ';');
    return { t: 'expr', expr };
  }

  private parseBlock(): AstNode {
    this.expect('punc', '{');
    const body: AstNode[] = [];
    while (!this.is('punc', '}') && !this.is('eof')) body.push(this.parseStmt());
    this.expect('punc', '}');
    return { t: 'block', body };
  }

  private parsePattern(): AstNode {
    if (this.eat('punc', '{')) {
      const props: Array<{ key: string; target: AstNode; def: AstNode | null }> = [];
      while (!this.is('punc', '}')) {
        const key = this.peek().kind === 'str' ? this.next().val : this.expect('id').val;
        const target = this.eat('punc', ':') ? this.parsePattern() : { t: 'patId', name: key };
        const def = this.eat('op', '=') ? this.parseAssign() : null;
        props.push({ key, target, def });
        if (!this.eat('punc', ',')) break;
      }
      this.expect('punc', '}');
      return { t: 'patObj', props };
    }
    if (this.eat('punc', '[')) {
      const items: Array<{ target: AstNode | null; def: AstNode | null; rest?: boolean }> = [];
      while (!this.is('punc', ']')) {
        if (this.is('punc', ',')) { items.push({ target: null, def: null }); this.next(); continue; }
        const rest = this.eat('punc', '...');
        const target = this.parsePattern();
        const def = !rest && this.eat('op', '=') ? this.parseAssign() : null;
        items.push({ target, def, rest });
        if (!this.eat('punc', ',')) break;
      }
      this.expect('punc', ']');
      return { t: 'patArr', items };
    }
    return { t: 'patId', name: this.expect('id').val };
  }

  private parseVarDecl(semi: boolean): AstNode {
    const kind = this.next().val;
    const decls: Array<{ pat: AstNode; init: AstNode | null }> = [];
    do {
      const pat = this.parsePattern();
      const init = this.eat('op', '=') ? this.parseAssign() : null;
      decls.push({ pat, init });
    } while (this.eat('punc', ','));
    if (semi) this.eat('punc', ';');
    return { t: 'var', kind, decls };
  }

  private parseParams(): Array<{ pat: AstNode; def: AstNode | null; rest: boolean }> {
    this.expect('punc', '(');
    const params: Array<{ pat: AstNode; def: AstNode | null; rest: boolean }> = [];
    while (!this.is('punc', ')')) {
      const rest = this.eat('punc', '...');
      const pat = this.parsePattern();
      const def = !rest && this.eat('op', '=') ? this.parseAssign() : null;
      params.push({ pat, def, rest });
      if (!this.eat('punc', ',')) break;
    }
    this.expect('punc', ')');
    return params;
  }

  private parseFuncDecl(): AstNode {
    this.expect('id', 'function');
    const name = this.expect('id').val;
    const params = this.parseParams();
    const body = this.parseBlock();
    return { t: 'fnDecl', name, params, body };
  }

  private parseFor(): AstNode {
    this.expect('punc', '(');
    let init: AstNode | null = null;
    if (!this.is('punc', ';')) {
      if (this.is('id', 'let') || this.is('id', 'const') || this.is('id', 'var')) {
        const kind = this.next().val;
        const pat = this.parsePattern();
        if (this.eat('id', 'of') || this.eat('id', 'in')) {
          const mode = this.toks[this.i - 1].val;
          const right = this.parseExpr();
          this.expect('punc', ')');
          return { t: mode === 'of' ? 'forOf' : 'forIn', kind, pat, right, body: this.parseStmt() };
        }
        const decls: Array<{ pat: AstNode; init: AstNode | null }> = [{ pat, init: this.eat('op', '=') ? this.parseAssign() : null }];
        while (this.eat('punc', ',')) {
          const p = this.parsePattern();
          decls.push({ pat: p, init: this.eat('op', '=') ? this.parseAssign() : null });
        }
        init = { t: 'var', kind, decls };
      } else {
        init = { t: 'expr', expr: this.parseExpr() };
      }
    }
    this.expect('punc', ';');
    const test = this.is('punc', ';') ? null : this.parseExpr();
    this.expect('punc', ';');
    const update = this.is('punc', ')') ? null : this.parseExpr();
    this.expect('punc', ')');
    return { t: 'for', init, test, update, body: this.parseStmt() };
  }

  private parseSwitch(): AstNode {
    this.expect('punc', '('); const disc = this.parseExpr(); this.expect('punc', ')');
    this.expect('punc', '{');
    const cases: Array<{ test: AstNode | null; body: AstNode[] }> = [];
    while (!this.is('punc', '}') && !this.is('eof')) {
      let test: AstNode | null = null;
      if (this.eat('id', 'case')) { test = this.parseExpr(); this.expect('punc', ':'); }
      else { this.expect('id', 'default'); this.expect('punc', ':'); }
      const body: AstNode[] = [];
      while (!this.is('id', 'case') && !this.is('id', 'default') && !this.is('punc', '}') && !this.is('eof')) body.push(this.parseStmt());
      cases.push({ test, body });
    }
    this.expect('punc', '}');
    return { t: 'switch', disc, cases };
  }

  parseExpr(): AstNode {
    let left = this.parseAssign();
    if (this.is('punc', ',')) {
      const exprs = [left];
      while (this.eat('punc', ',')) exprs.push(this.parseAssign());
      return { t: 'seq', exprs };
    }
    return left;
  }

  private tryArrow(): AstNode | null {
    const save = this.i;
    if (this.is('id') && !KEYWORDS.has(this.peek().val) && this.peek(1).kind === 'op' && this.peek(1).val === '=>') {
      const name = this.next().val;
      this.next();
      const body = this.is('punc', '{') ? this.parseBlock() : this.parseAssign();
      return { t: 'fn', params: [{ pat: { t: 'patId', name }, def: null, rest: false }], body, exprBody: body.t !== 'block' };
    }
    if (this.is('punc', '(')) {
      try {
        const params = this.parseParams();
        if (this.eat('op', '=>')) {
          const body = this.is('punc', '{') ? this.parseBlock() : this.parseAssign();
          return { t: 'fn', params, body, exprBody: body.t !== 'block' };
        }
      } catch {
        // Not an arrow parameter list — backtrack.
      }
      this.i = save;
    }
    return null;
  }

  private parseAssign(): AstNode {
    const arrow = this.tryArrow();
    if (arrow) return arrow;
    const left = this.parseCond();
    if (this.is('op') && ['=', '+=', '-=', '*=', '/=', '%=', '??=', '||=', '&&='].includes(this.peek().val)) {
      const op = this.next().val;
      return { t: 'assign', op, left, right: this.parseAssign() };
    }
    return left;
  }

  private parseCond(): AstNode {
    const test = this.parseBin(0);
    if (this.eat('op', '?')) {
      const cons = this.parseAssign();
      this.expect('punc', ':');
      return { t: 'cond', test, cons, alt: this.parseAssign() };
    }
    return test;
  }

  private static readonly PREC: Record<string, number> = {
    '??': 1, '||': 2, '&&': 3, '|': 4, '^': 5, '&': 6,
    '==': 7, '!=': 7, '===': 7, '!==': 7,
    '<': 8, '<=': 8, '>': 8, '>=': 8, in: 8, instanceof: 8,
    '<<': 9, '>>': 9, '>>>': 9,
    '+': 10, '-': 10, '*': 11, '/': 11, '%': 11, '**': 12,
  };

  private parseBin(minPrec: number): AstNode {
    let left = this.parseUnary();
    while (true) {
      const t = this.peek();
      const op = (t.kind === 'op' || (t.kind === 'id' && (t.val === 'in' || t.val === 'instanceof'))) ? t.val : '';
      const p = Parser.PREC[op] ?? 0;
      if (p <= minPrec) break;
      this.next();
      const right = this.parseBin(op === '**' ? p - 1 : p);
      left = { t: 'bin', op, left, right };
    }
    return left;
  }

  private parseUnary(): AstNode {
    const t = this.peek();
    if ((t.kind === 'op' && ['!', '-', '+', '~', '++', '--'].includes(t.val)) || (t.kind === 'id' && ['typeof', 'void', 'delete'].includes(t.val))) {
      this.next();
      const arg = this.parseUnary();
      if (t.val === '++' || t.val === '--') return { t: 'update', op: t.val, prefix: true, arg };
      return { t: 'unary', op: t.val, arg };
    }
    return this.parsePostfix();
  }

  private parsePostfix(): AstNode {
    let expr = this.parseCall();
    if (this.is('op', '++') || this.is('op', '--')) {
      return { t: 'update', op: this.next().val, prefix: false, arg: expr };
    }
    return expr;
  }

  private parseCall(): AstNode {
    if (this.eat('id', 'new')) {
      const callee = this.parseMember(false);
      const args = this.is('punc', '(') ? this.parseArgs() : [];
      return this.parseCallTail({ t: 'new', callee, args });
    }
    return this.parseCallTail(this.parsePrimary());
  }

  private parseMember(allowCall: boolean): AstNode {
    let expr = this.parsePrimary();
    while (true) {
      if (this.eat('punc', '.')) {
        const prop = this.next();
        expr = { t: 'member', obj: expr, prop: prop.val, computed: false, opt: false };
      } else if (this.eat('punc', '[')) {
        const prop = this.parseExpr(); this.expect('punc', ']');
        expr = { t: 'member', obj: expr, prop, computed: true, opt: false };
      } else if (allowCall && this.is('punc', '(')) {
        expr = { t: 'call', callee: expr, args: this.parseArgs(), opt: false };
      } else break;
    }
    return expr;
  }

  private parseCallTail(expr: AstNode): AstNode {
    while (true) {
      if (this.eat('punc', '?.')) {
        if (this.eat('punc', '[')) {
          const prop = this.parseExpr(); this.expect('punc', ']');
          expr = { t: 'member', obj: expr, prop, computed: true, opt: true };
        } else if (this.is('punc', '(')) {
          expr = { t: 'call', callee: expr, args: this.parseArgs(), opt: true };
        } else {
          expr = { t: 'member', obj: expr, prop: this.next().val, computed: false, opt: true };
        }
      } else if (this.eat('punc', '.')) {
        expr = { t: 'member', obj: expr, prop: this.next().val, computed: false, opt: false };
      } else if (this.eat('punc', '[')) {
        const prop = this.parseExpr(); this.expect('punc', ']');
        expr = { t: 'member', obj: expr, prop, computed: true, opt: false };
      } else if (this.is('punc', '(')) {
        expr = { t: 'call', callee: expr, args: this.parseArgs(), opt: false };
      } else break;
    }
    return expr;
  }

  private parseArgs(): Array<{ expr: AstNode; spread: boolean }> {
    this.expect('punc', '(');
    const args: Array<{ expr: AstNode; spread: boolean }> = [];
    while (!this.is('punc', ')')) {
      const spread = this.eat('punc', '...');
      args.push({ expr: this.parseAssign(), spread });
      if (!this.eat('punc', ',')) break;
    }
    this.expect('punc', ')');
    return args;
  }

  private parseTemplate(raw: string): AstNode {
    const parts: Array<string | AstNode> = [];
    let i = 0, lit = '';
    while (i < raw.length) {
      if (raw[i] === '\\' && i + 1 < raw.length) {
        const e = raw[i + 1];
        lit += e === 'n' ? '\n' : e === 'r' ? '\r' : e === 't' ? '\t' : e;
        i += 2; continue;
      }
      if (raw[i] === '$' && raw[i + 1] === '{') {
        if (lit) { parts.push(lit); lit = ''; }
        let j = i + 2, depth = 1;
        while (j < raw.length && depth > 0) {
          if (raw[j] === '{') depth++;
          else if (raw[j] === '}') depth--;
          if (depth > 0) j++;
        }
        parts.push(new Parser(tokenize(raw.slice(i + 2, j))).parseExpr());
        i = j + 1; continue;
      }
      lit += raw[i++];
    }
    if (lit) parts.push(lit);
    return { t: 'tmpl', parts };
  }

  private parsePrimary(): AstNode {
    const t = this.peek();
    if (t.kind === 'num') { this.next(); return { t: 'lit', val: Number(t.val) }; }
    if (t.kind === 'str') { this.next(); return { t: 'lit', val: t.val }; }
    if (t.kind === 'tmpl') { this.next(); return this.parseTemplate(t.val); }
    if (t.kind === 'regex') {
      this.next();
      const nul = t.val.indexOf('\0');
      return { t: 'regex', pat: t.val.slice(0, nul), flags: t.val.slice(nul + 1) };
    }
    if (t.kind === 'id') {
      if (t.val === 'true') { this.next(); return { t: 'lit', val: true }; }
      if (t.val === 'false') { this.next(); return { t: 'lit', val: false }; }
      if (t.val === 'null') { this.next(); return { t: 'lit', val: null }; }
      if (t.val === 'undefined') { this.next(); return { t: 'lit', val: undefined }; }
      if (t.val === 'function') {
        this.next();
        const name = this.is('id') ? this.next().val : null;
        const params = this.parseParams();
        const body = this.parseBlock();
        return { t: 'fn', name, params, body, exprBody: false };
      }
      this.next();
      return { t: 'id', name: t.val };
    }
    if (this.eat('punc', '(')) {
      const expr = this.parseExpr();
      this.expect('punc', ')');
      return expr;
    }
    if (this.eat('punc', '[')) {
      const items: Array<{ expr: AstNode; spread: boolean }> = [];
      while (!this.is('punc', ']')) {
        const spread = this.eat('punc', '...');
        items.push({ expr: this.parseAssign(), spread });
        if (!this.eat('punc', ',')) break;
      }
      this.expect('punc', ']');
      return { t: 'arr', items };
    }
    if (this.eat('punc', '{')) {
      const props: Array<{ key?: AstNode | string; val: AstNode; computed?: boolean; spread?: boolean }> = [];
      while (!this.is('punc', '}')) {
        if (this.eat('punc', '...')) {
          props.push({ val: this.parseAssign(), spread: true });
        } else if (this.eat('punc', '[')) {
          const key = this.parseExpr(); this.expect('punc', ']'); this.expect('punc', ':');
          props.push({ key, val: this.parseAssign(), computed: true });
        } else {
          const k = this.next();
          const key = k.val;
          if (this.eat('punc', ':')) props.push({ key, val: this.parseAssign() });
          else if (this.is('punc', '(')) {
            const params = this.parseParams(); const body = this.parseBlock();
            props.push({ key, val: { t: 'fn', params, body, exprBody: false } });
          } else props.push({ key, val: { t: 'id', name: key } });
        }
        if (!this.eat('punc', ',')) break;
      }
      this.expect('punc', '}');
      return { t: 'obj', props };
    }
    throw new SyntaxError(`Unexpected token ${t.val || t.kind} at ${t.pos}`);
  }
}

export function parseScript(src: string): AstNode[] {
  return new Parser(tokenize(src)).parseProgram();
}
