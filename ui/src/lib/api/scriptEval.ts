// CSP-safe JavaScript subset evaluator for API Client pre/post scripts.
// Runs user Postman-style (`pm.*`) scripts with ZERO `eval` or `new Function`,
// allowing Tauri's CSP to drop `'unsafe-eval'`.
//
// Security properties:
// - Strict built-in allowlist (no `Function`, `eval`, `fetch`, `globalThis`, etc.).
// - Blocks `__proto__`, `constructor`, and `prototype` property access.
// - Enforces a bounded step budget so infinite loops fail cleanly.

const MAX_STEPS = 250_000;
const BLOCKED_PROPS = new Set(['__proto__', 'constructor', 'prototype']);

import { parseScript, type AstNode } from './scriptParser.ts';

class Env {
  readonly vars = new Map<string, { val: unknown; isConst: boolean }>();
  readonly parent: Env | null;
  readonly isFn: boolean;
  constructor(parent: Env | null, isFn = false) { this.parent = parent; this.isFn = isFn; }
  declare(name: string, val: unknown, kind: string): void {
    const target: Env = kind === 'var' ? this.fnEnv() : this;
    target.vars.set(name, { val, isConst: kind === 'const' });
  }
  fnEnv(): Env { let e: Env = this; while (!e.isFn && e.parent) e = e.parent; return e; }
  has(name: string): boolean { return this.vars.has(name) || (this.parent?.has(name) ?? false); }
  get(name: string): unknown {
    let e: Env | null = this;
    while (e) { if (e.vars.has(name)) return e.vars.get(name)!.val; e = e.parent; }
    throw new ReferenceError(`${name} is not defined`);
  }
  set(name: string, val: unknown): unknown {
    let e: Env | null = this;
    while (e) {
      const slot = e.vars.get(name);
      if (slot) {
        if (slot.isConst) throw new TypeError(`Assignment to constant variable '${name}'.`);
        slot.val = val; return val;
      }
      e = e.parent;
    }
    this.fnEnv().vars.set(name, { val, isConst: false });
    return val;
  }
}

const BREAK = { sig: 'break' } as const;
const CONTINUE = { sig: 'continue' } as const;
interface ReturnSig { sig: 'return'; val: unknown; }
type Signal = typeof BREAK | typeof CONTINUE | ReturnSig | null;

function isSafeProp(key: unknown): boolean {
  return !BLOCKED_PROPS.has(String(key));
}

function safeGet(obj: any, key: unknown): unknown {
  if (obj == null) throw new TypeError(`Cannot read properties of ${obj} (reading '${String(key)}')`);
  if (!isSafeProp(key)) return undefined;
  return obj[key as PropertyKey];
}

function safeSet(obj: any, key: unknown, val: unknown): unknown {
  if (obj == null) throw new TypeError(`Cannot set properties of ${obj} (setting '${String(key)}')`);
  if (!isSafeProp(key)) throw new TypeError(`Cannot set property '${String(key)}' in script runtime`);
  obj[key as PropertyKey] = val;
  return val;
}

const BUILTINS: Record<string, unknown> = {
  JSON, Math, Date, Object, Array, String, Number, Boolean, RegExp, Map, Set,
  Error, TypeError, RangeError, SyntaxError, URL, URLSearchParams,
  parseInt, parseFloat, isNaN, isFinite, NaN, Infinity, undefined,
  encodeURIComponent, decodeURIComponent, encodeURI, decodeURI,
  ...(typeof globalThis.btoa === 'function' ? { btoa: globalThis.btoa.bind(globalThis) } : {}),
  ...(typeof globalThis.atob === 'function' ? { atob: globalThis.atob.bind(globalThis) } : {}),
};

/** Evaluate a Postman-style JS script without `eval` or `new Function`. */
export function evalScript(code: string, globals: Record<string, unknown>, maxSteps = MAX_STEPS): void {
  const ast = parseScript(code);
  const root = new Env(null, true);
  for (const [k, v] of Object.entries(BUILTINS)) root.declare(k, v, 'const');
  for (const [k, v] of Object.entries(globals)) root.declare(k, v, 'let');
  let steps = 0;
  const tick = () => { if (++steps > maxSteps) throw new RangeError('Script step budget exceeded.'); };

  const bindPat = (pat: AstNode, val: unknown, env: Env, kind: string) => {
    if (pat.t === 'patId') { env.declare(pat.name, val, kind); return; }
    if (pat.t === 'patObj') {
      const o = val as Record<string, unknown>;
      for (const p of pat.props) {
        let v = safeGet(o, p.key);
        if (v === undefined && p.def) v = evalExpr(p.def, env);
        bindPat(p.target, v, env, kind);
      }
      return;
    }
    if (pat.t === 'patArr') {
      const arr = Array.from(val as Iterable<unknown>);
      for (let idx = 0; idx < pat.items.length; idx++) {
        const it = pat.items[idx];
        if (!it.target) continue;
        if (it.rest) { bindPat(it.target, arr.slice(idx), env, kind); break; }
        let v = arr[idx];
        if (v === undefined && it.def) v = evalExpr(it.def, env);
        bindPat(it.target, v, env, kind);
      }
    }
  };

  const makeFn = (node: AstNode, closure: Env) => {
    const fn = function (this: unknown, ...callArgs: unknown[]) {
      const fenv = new Env(closure, true);
      if (node.name) fenv.declare(node.name, fn, 'const');
      fenv.declare('this', this, 'const');
      for (let i = 0; i < node.params.length; i++) {
        const p = node.params[i];
        if (p.rest) { bindPat(p.pat, callArgs.slice(i), fenv, 'let'); break; }
        let v = callArgs[i];
        if (v === undefined && p.def) v = evalExpr(p.def, fenv);
        bindPat(p.pat, v, fenv, 'let');
      }
      if (node.exprBody) return evalExpr(node.body, fenv);
      const sig = execList(node.body.body, fenv);
      return sig && sig.sig === 'return' ? sig.val : undefined;
    };
    return fn;
  };

  const execList = (stmts: AstNode[], env: Env): Signal => {
    // Hoist function declarations inside this block/function.
    for (const s of stmts) if (s.t === 'fnDecl') env.declare(s.name, makeFn(s, env), 'let');
    for (const s of stmts) {
      if (s.t === 'fnDecl') continue;
      const sig = execStmt(s, env);
      if (sig) return sig;
    }
    return null;
  };

  const execStmt = (s: AstNode, env: Env): Signal => {
    tick();
    switch (s.t) {
      case 'empty': return null;
      case 'block': return execList(s.body, new Env(env));
      case 'expr': evalExpr(s.expr, env); return null;
      case 'var':
        for (const d of s.decls) bindPat(d.pat, d.init ? evalExpr(d.init, env) : undefined, env, s.kind);
        return null;
      case 'fnDecl': env.declare(s.name, makeFn(s, env), 'let'); return null;
      case 'if':
        return evalExpr(s.test, env) ? execStmt(s.cons, env) : (s.alt ? execStmt(s.alt, env) : null);
      case 'while':
        while (evalExpr(s.test, env)) {
          tick();
          const sig = execStmt(s.body, env);
          if (sig === BREAK) break;
          if (sig && sig !== CONTINUE) return sig;
        }
        return null;
      case 'do':
        do {
          tick();
          const sig = execStmt(s.body, env);
          if (sig === BREAK) break;
          if (sig && sig !== CONTINUE) return sig;
        } while (evalExpr(s.test, env));
        return null;
      case 'for': {
        const lenv = new Env(env);
        if (s.init) execStmt(s.init, lenv);
        while (!s.test || evalExpr(s.test, lenv)) {
          tick();
          const sig = execStmt(s.body, new Env(lenv));
          if (sig === BREAK) break;
          if (sig && sig !== CONTINUE) return sig;
          if (s.update) evalExpr(s.update, lenv);
        }
        return null;
      }
      case 'forOf':
        for (const v of evalExpr(s.right, env) as Iterable<unknown>) {
          tick();
          const lenv = new Env(env);
          bindPat(s.pat, v, lenv, s.kind);
          const sig = execStmt(s.body, lenv);
          if (sig === BREAK) break;
          if (sig && sig !== CONTINUE) return sig;
        }
        return null;
      case 'forIn':
        for (const k in evalExpr(s.right, env) as Record<string, unknown>) {
          tick();
          const lenv = new Env(env);
          bindPat(s.pat, k, lenv, s.kind);
          const sig = execStmt(s.body, lenv);
          if (sig === BREAK) break;
          if (sig && sig !== CONTINUE) return sig;
        }
        return null;
      case 'switch': {
        const v = evalExpr(s.disc, env);
        let matched = false;
        const senv = new Env(env);
        for (const c of s.cases) {
          if (!matched && (c.test === null || evalExpr(c.test, senv) === v)) matched = true;
          if (matched) {
            const sig = execList(c.body, senv);
            if (sig === BREAK) break;
            if (sig) return sig;
          }
        }
        return null;
      }
      case 'return': return { sig: 'return', val: s.arg ? evalExpr(s.arg, env) : undefined };
      case 'break': return BREAK;
      case 'continue': return CONTINUE;
      case 'throw': throw evalExpr(s.arg, env);
      case 'try': {
        let sig: Signal = null;
        try { sig = execStmt(s.block, env); }
        catch (err) {
          if (!s.handler) throw err;
          const cenv = new Env(env);
          if (s.param) cenv.declare(s.param, err, 'let');
          sig = execStmt(s.handler, cenv);
        } finally {
          if (s.finalizer) execStmt(s.finalizer, env);
        }
        return sig;
      }
      default: throw new SyntaxError(`Unsupported statement: ${s.t}`);
    }
  };

  const applyAssignOp = (op: string, cur: any, rhs: () => unknown): unknown => {
    if (op === '=') return rhs();
    if (op === '??=') return cur ?? rhs();
    if (op === '||=') return cur || rhs();
    if (op === '&&=') return cur && rhs();
    const r: any = rhs();
    if (op === '+=') return cur + r;
    if (op === '-=') return cur - r;
    if (op === '*=') return cur * r;
    if (op === '/=') return cur / r;
    if (op === '%=') return cur % r;
    return r;
  };

  const evalArgs = (args: Array<{ expr: AstNode; spread: boolean }>, env: Env): unknown[] => {
    const out: unknown[] = [];
    for (const a of args) {
      const v = evalExpr(a.expr, env);
      if (a.spread) out.push(...(v as Iterable<unknown>));
      else out.push(v);
    }
    return out;
  };

  const evalExpr = (e: AstNode, env: Env): any => {
    tick();
    switch (e.t) {
      case 'lit': return e.val;
      case 'regex': return new RegExp(e.pat, e.flags);
      case 'tmpl': return e.parts.map((p: string | AstNode) => typeof p === 'string' ? p : String(evalExpr(p, env))).join('');
      case 'id': return env.get(e.name);
      case 'fn': return makeFn(e, env);
      case 'seq': { let r; for (const x of e.exprs) r = evalExpr(x, env); return r; }
      case 'arr': return evalArgs(e.items, env);
      case 'obj': {
        const o: Record<string, unknown> = {};
        for (const p of e.props) {
          if (p.spread) Object.assign(o, evalExpr(p.val, env));
          else {
            const k = p.computed ? String(evalExpr(p.key, env)) : String(p.key);
            safeSet(o, k, evalExpr(p.val, env));
          }
        }
        return o;
      }
      case 'member': {
        const obj = evalExpr(e.obj, env);
        if (e.opt && obj == null) return undefined;
        const key = e.computed ? evalExpr(e.prop, env) : e.prop;
        return safeGet(obj, key);
      }
      case 'call': {
        if (e.callee.t === 'member') {
          const obj = evalExpr(e.callee.obj, env);
          if (e.callee.opt && obj == null) return undefined;
          const key = e.callee.computed ? evalExpr(e.callee.prop, env) : e.callee.prop;
          const fn = safeGet(obj, key);
          if (e.opt && fn == null) return undefined;
          if (typeof fn !== 'function') throw new TypeError(`${String(key)} is not a function`);
          return fn.apply(obj, evalArgs(e.args, env));
        }
        const fn = evalExpr(e.callee, env);
        if (e.opt && fn == null) return undefined;
        if (typeof fn !== 'function') throw new TypeError('Callee is not a function');
        return fn(...evalArgs(e.args, env));
      }
      case 'new': {
        const Ctor = evalExpr(e.callee, env);
        if (typeof Ctor !== 'function') throw new TypeError('Constructor is not a function');
        return new Ctor(...evalArgs(e.args, env));
      }
      case 'unary': {
        if (e.op === 'typeof') {
          if (e.arg.t === 'id' && !env.has(e.arg.name)) return 'undefined';
          return typeof evalExpr(e.arg, env);
        }
        if (e.op === 'delete') {
          if (e.arg.t === 'member') {
            const o = evalExpr(e.arg.obj, env);
            const k = e.arg.computed ? evalExpr(e.arg.prop, env) : e.arg.prop;
            if (!isSafeProp(k)) return false;
            return delete o[k];
          }
          return true;
        }
        const v = evalExpr(e.arg, env);
        if (e.op === '!') return !v;
        if (e.op === '-') return -v;
        if (e.op === '+') return +v;
        if (e.op === '~') return ~v;
        if (e.op === 'void') return undefined;
        return v;
      }
      case 'update': {
        const delta = e.op === '++' ? 1 : -1;
        if (e.arg.t === 'id') {
          const prev = Number(env.get(e.arg.name));
          const next = prev + delta;
          env.set(e.arg.name, next);
          return e.prefix ? next : prev;
        }
        if (e.arg.t === 'member') {
          const o = evalExpr(e.arg.obj, env);
          const k = e.arg.computed ? evalExpr(e.arg.prop, env) : e.arg.prop;
          const prev = Number(safeGet(o, k));
          const next = prev + delta;
          safeSet(o, k, next);
          return e.prefix ? next : prev;
        }
        throw new SyntaxError('Invalid update target');
      }
      case 'assign': {
        if (e.left.t === 'id') {
          const cur = e.op === '=' ? undefined : env.get(e.left.name);
          const next = applyAssignOp(e.op, cur, () => evalExpr(e.right, env));
          return env.set(e.left.name, next);
        }
        if (e.left.t === 'member') {
          const o = evalExpr(e.left.obj, env);
          const k = e.left.computed ? evalExpr(e.left.prop, env) : e.left.prop;
          const cur = e.op === '=' ? undefined : safeGet(o, k);
          const next = applyAssignOp(e.op, cur, () => evalExpr(e.right, env));
          return safeSet(o, k, next);
        }
        throw new SyntaxError('Invalid assignment target');
      }
      case 'cond': return evalExpr(e.test, env) ? evalExpr(e.cons, env) : evalExpr(e.alt, env);
      case 'bin': {
        if (e.op === '&&') { const l = evalExpr(e.left, env); return l ? evalExpr(e.right, env) : l; }
        if (e.op === '||') { const l = evalExpr(e.left, env); return l ? l : evalExpr(e.right, env); }
        if (e.op === '??') { const l = evalExpr(e.left, env); return l ?? evalExpr(e.right, env); }
        const l = evalExpr(e.left, env), r = evalExpr(e.right, env);
        switch (e.op) {
          case '+': return l + r;
          case '-': return l - r;
          case '*': return l * r;
          case '/': return l / r;
          case '%': return l % r;
          case '**': return l ** r;
          case '===': return l === r;
          case '!==': return l !== r;
          case '==': return l == r;
          case '!=': return l != r;
          case '<': return l < r;
          case '<=': return l <= r;
          case '>': return l > r;
          case '>=': return l >= r;
          case '<<': return l << r;
          case '>>': return l >> r;
          case '>>>': return l >>> r;
          case '&': return l & r;
          case '|': return l | r;
          case '^': return l ^ r;
          case 'in': return l in r;
          case 'instanceof': return l instanceof r;
        }
      }
    }
    throw new SyntaxError(`Unsupported expression: ${e.t}`);
  };

  execList(ast, root);
}
