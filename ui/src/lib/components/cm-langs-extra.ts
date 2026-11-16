// On-demand StreamLanguage grammars for Shell (`sh`/`bash`/`zsh`), `go.mod`/`go.work`, and TOML (`Cargo.toml`/`pyproject.toml`).
// Kept out of static `cm-langs.ts` so non-IDE CodeEditor bundles (API, Workflows, K8s) stay minimal.
import { StreamLanguage } from '@codemirror/language';
import type { Extension } from '@codemirror/state';

const SHELL_KEYWORDS = new Set([
  'if', 'then', 'else', 'elif', 'fi', 'for', 'in', 'do', 'done', 'while', 'until',
  'case', 'esac', 'select', 'function', 'time', 'coproc', 'return', 'exit', 'break',
  'continue', 'local', 'export', 'readonly', 'declare', 'typeset', 'unset', 'shift',
  'source', 'trap', 'set', 'shopt', 'eval', 'exec',
]);

const SHELL_BUILTINS = new Set([
  'echo', 'printf', 'cd', 'pwd', 'test', 'true', 'false', 'read', 'wait', 'kill',
  'umask', 'ulimit', 'command', 'builtin', 'type', 'hash', 'alias', 'unalias',
]);

interface ShellScanState {
  inDoubleQuote: boolean;
  expectCmd: boolean;
}

export function shellLang(): Extension {
  return StreamLanguage.define<ShellScanState>({
    startState: () => ({ inDoubleQuote: false, expectCmd: true }),
    token(stream, state) {
      if (state.inDoubleQuote) {
        if (stream.match(/^\\./)) return 'escape';
        if (stream.match(/^\$(?:\{[A-Za-z_][\w]*[^}]*\}|[A-Za-z_]\w*|[0-9@#?$!*-])/)) return 'variableName.special';
        if (stream.eat('"')) { state.inDoubleQuote = false; return 'string'; }
        stream.next();
        while (!stream.eol() && !['"', '\\', '$'].includes(stream.peek() ?? '')) stream.next();
        return 'string';
      }
      if (stream.sol()) state.expectCmd = true;
      if (stream.eatSpace()) return null;
      if (stream.sol() && stream.match(/^#!.*/)) return 'meta';
      if (stream.match(/^#.*/)) return 'comment';
      if (stream.eat('"')) { state.inDoubleQuote = true; state.expectCmd = false; return 'string'; }
      if (stream.match(/^'(?:[^'])*'?/)) { state.expectCmd = false; return 'string'; }
      if (stream.match(/^`(?:[^`\\]|\\.)*`?/)) { state.expectCmd = false; return 'string.special'; }
      if (stream.match(/^\$(?:\{[A-Za-z_][\w]*[^}]*\}|\(\([^)]*\)\)|\([^)]*\)|[A-Za-z_]\w*|[0-9@#?$!*-])/)) {
        state.expectCmd = false;
        return 'variableName.special';
      }
      if (stream.match(/^(?:\|\||&&|;;|<<-?|>>|[|&;<>()])/)) {
        const tok = stream.current();
        if (tok === '|' || tok === '||' || tok === '&&' || tok === ';' || tok === '(') state.expectCmd = true;
        return 'operator';
      }
      if (stream.match(/^[A-Za-z_]\w*(?==)/)) return 'definition';
      if (stream.eat('=')) { state.expectCmd = false; return 'operator'; }
      if (stream.match(/^--?[A-Za-z0-9][\w-]*/)) { state.expectCmd = false; return 'attributeName'; }
      if (stream.match(/^[+-]?\d+(?:\.\d+)?\b/)) { state.expectCmd = false; return 'number'; }
      if (stream.match(/^[A-Za-z_][\w.-]*/)) {
        const word = stream.current();
        if (stream.match(/^\s*\(\s*\)/, false)) { state.expectCmd = false; return 'def'; }
        if (SHELL_KEYWORDS.has(word)) { state.expectCmd = word === 'then' || word === 'do' || word === 'else' || word === 'elif'; return 'keyword'; }
        if (SHELL_BUILTINS.has(word)) { state.expectCmd = false; return 'builtin'; }
        if (state.expectCmd) { state.expectCmd = false; return 'variableName.function'; }
        return 'variableName';
      }
      stream.next();
      state.expectCmd = false;
      return null;
    },
  });
}

export function goModLang(): Extension {
  const kw = new Set(['module', 'go', 'toolchain', 'require', 'replace', 'exclude', 'retract', 'use']);
  return StreamLanguage.define<{ afterModKw: boolean }>({
    startState: () => ({ afterModKw: false }),
    token(stream, st) {
      if (stream.sol()) st.afterModKw = false;
      if (stream.eatSpace()) return null;
      if (stream.match(/^\/\/.*/)) return 'comment';
      if (stream.match(/^=>/)) return 'operator';
      if (stream.match(/^[()]/)) return 'bracket';
      if (stream.match(/^v?\d+\.\d+(?:\.\d+)?(?:-[\w.+]+)?\b/)) return 'number';
      if (stream.match(/^[A-Za-z_]\w*/) && !stream.match(/^[./-]/, false)) {
        const w = stream.current();
        if (kw.has(w)) { st.afterModKw = w === 'module'; return 'keyword'; }
        if (st.afterModKw) { st.afterModKw = false; return 'string'; }
      }
      if (stream.match(/^[^\s()]+/)) return 'string';
      stream.next();
      return null;
    },
  });
}

export function tomlLang(): Extension {
  return StreamLanguage.define<Record<string, never>>({
    startState: () => ({}),
    token(stream) {
      if (stream.eatSpace()) return null;
      if (stream.match(/^#.*/)) return 'comment';
      if (stream.sol() && stream.match(/^\[\[?[^\]]+\]\]?/)) return 'typeName';
      if (stream.match(/^"(?:[^"\\]|\\.)*"?/) || stream.match(/^'(?:[^'])*'?/)) return 'string';
      if (stream.match(/^(?:true|false)\b/)) return 'atom';
      if (stream.match(/^[+-]?\d+(?:\.\d+)?\b/)) return 'number';
      if (stream.match(/^[A-Za-z0-9_-]+(?=\s*=)/)) return 'propertyName';
      stream.next();
      return null;
    },
  });
}
