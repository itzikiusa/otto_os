// Zig & ZON (Zig Object Notation) language pack for CodeMirror 6.
// Covers keywords, @builtins, primitive & arbitrary-width integer types (u1..u65535 / i1..i65535),
// doc comments (`///`, `//!`), multiline string literals (`\\...`), quoted strings & chars,
// enum/field literals (`.foo`), hex/bin/oct/float literals, and brace folding.
import { foldService, StreamLanguage, type StringStream } from '@codemirror/language';
import type { Extension } from '@codemirror/state';

const KEYWORDS = new Set([
  'addrspace', 'align', 'allowzero', 'and', 'anyframe', 'anytype', 'asm', 'async', 'await',
  'break', 'callconv', 'catch', 'comptime', 'const', 'continue', 'defer', 'else', 'enum',
  'errdefer', 'error', 'export', 'extern', 'fn', 'for', 'if', 'inline', 'linksection',
  'noalias', 'noinline', 'nosuspend', 'opaque', 'or', 'orelse', 'packed', 'pub', 'resume',
  'return', 'struct', 'suspend', 'switch', 'test', 'threadlocal', 'try', 'union',
  'unreachable', 'var', 'volatile', 'while',
]);

const ATOMS = new Set(['true', 'false', 'null', 'undefined']);

const PRIMITIVE_TYPES = new Set([
  'anyerror', 'anyopaque', 'bool', 'c_char', 'c_int', 'c_long', 'c_longdouble', 'c_longlong',
  'c_short', 'c_uint', 'c_ulong', 'c_ulonglong', 'c_ushort', 'comptime_float', 'comptime_int',
  'f16', 'f32', 'f64', 'f80', 'f128', 'isize', 'noreturn', 'type', 'usize', 'void',
]);

const ARB_INT_RE = /^[iu](?:[1-9]\d*)$/;

interface ZigState {
  expectDeclName: boolean;
}

const zigStream = StreamLanguage.define<ZigState>({
  name: 'zig',
  startState: () => ({ expectDeclName: false }),
  token(stream: StringStream, state: ZigState): string | null {
    if (stream.eatSpace()) return null;

    // Comments (`///` doc comment, `//!` top-level doc comment, `//` line comment)
    if (stream.match('//')) {
      const isDoc = stream.peek() === '/' || stream.peek() === '!';
      stream.skipToEnd();
      state.expectDeclName = false;
      return isDoc ? 'comment' : 'comment';
    }

    // Multiline string literal (`\\...` to end of line)
    if (stream.match('\\\\')) {
      stream.skipToEnd();
      state.expectDeclName = false;
      return 'string';
    }

    // Double-quoted string literal
    if (stream.peek() === '"') {
      stream.next();
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') {
          stream.next();
        } else if (ch === '"') {
          break;
        }
      }
      state.expectDeclName = false;
      return 'string';
    }

    // Character literal (`'a'`, `'\n'`, `'\x1b'`)
    if (stream.peek() === "'") {
      stream.next();
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') {
          stream.next();
        } else if (ch === "'") {
          break;
        }
      }
      state.expectDeclName = false;
      return 'string';
    }

    // Builtin call (`@import`, `@as`, `@intCast`, ...)
    if (stream.peek() === '@') {
      if (stream.match(/^@"(?:\\.|[^"\\])*"/)) {
        state.expectDeclName = false;
        return 'variableName';
      }
      if (stream.match(/^@[A-Za-z_]\w*/)) {
        state.expectDeclName = false;
        return 'meta';
      }
    }

    // Numeric literals (hex, binary, octal, decimal, float with optional `_` separators)
    if (
      stream.match(/^0[xX][0-9a-fA-F_]+(?:\.[0-9a-fA-F_]+)?(?:[pP][+-]?[0-9_]+)?/) ||
      stream.match(/^0[bB][01_]+/) ||
      stream.match(/^0[oO][0-7_]+/) ||
      stream.match(/^\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d[\d_]*)?/)
    ) {
      state.expectDeclName = false;
      return 'number';
    }

    // Enum literal / ZON field / struct field access (`.foo`)
    if (stream.peek() === '.') {
      if (stream.match(/^\.{2,3}/) || stream.match(/^\.\{/)) {
        state.expectDeclName = false;
        return 'operator';
      }
      if (stream.match(/^\.[A-Za-z_]\w*/)) {
        state.expectDeclName = false;
        return 'propertyName';
      }
      stream.next();
      return 'punctuation';
    }

    // Identifiers, keywords, primitives, types
    if (stream.match(/^[A-Za-z_]\w*/)) {
      const word = stream.current();
      if (KEYWORDS.has(word)) {
        state.expectDeclName = word === 'fn';
        return 'keyword';
      }
      if (ATOMS.has(word)) {
        state.expectDeclName = false;
        return 'atom';
      }
      if (PRIMITIVE_TYPES.has(word) || ARB_INT_RE.test(word)) {
        state.expectDeclName = false;
        return 'typeName';
      }
      if (state.expectDeclName) {
        state.expectDeclName = false;
        return 'definition(variableName)';
      }
      if (/^[A-Z][A-Za-z0-9_]*$/.test(word)) {
        return 'typeName';
      }
      return 'variableName';
    }

    // Operators & punctuation
    if (stream.match(/^(?:<<=|>>=|->|\+\+|\*\*|\+=|-=|\*=|\/=|%=|&=|\|=|\^=|==|!=|<=|>=|<<|>>|\+%|\-%|\*%|\+\||-\||\*\||[+\-*/%&|^!~<>?=:])/)) {
      state.expectDeclName = false;
      return 'operator';
    }

    const ch = stream.next();
    if (ch && '{}()[]'.includes(ch)) {
      state.expectDeclName = false;
      return 'bracket';
    }
    return 'punctuation';
  },
  languageData: {
    commentTokens: { line: '//' },
    closeBrackets: { brackets: ['(', '[', '{', '"', "'"] },
  },
});

const zigBraceFold = foldService.of((state, lineStart, lineEnd) => {
  const lineText = state.doc.sliceString(lineStart, lineEnd);
  const openIdx = lineText.lastIndexOf('{');
  if (openIdx < 0) return null;
  // Ignore `{` inside line comments.
  const commentIdx = lineText.indexOf('//');
  if (commentIdx >= 0 && commentIdx < openIdx) return null;
  const openPos = lineStart + openIdx;
  const maxPos = Math.min(state.doc.length, openPos + 65536);
  const chunk = state.doc.sliceString(openPos + 1, maxPos);
  let depth = 1;
  let inStr: string | null = null;
  let inLineComment = false;
  for (let i = 0; i < chunk.length; i++) {
    const ch = chunk[i];
    if (inLineComment) {
      if (ch === '\n') inLineComment = false;
      continue;
    }
    if (inStr) {
      if (ch === '\\') {
        i++;
      } else if (ch === inStr) {
        inStr = null;
      }
      continue;
    }
    if (ch === '/' && chunk[i + 1] === '/') {
      inLineComment = true;
      i++;
      continue;
    }
    if (ch === '"' || ch === "'") {
      inStr = ch;
      continue;
    }
    if (ch === '{') {
      depth++;
    } else if (ch === '}') {
      depth--;
      if (depth === 0) {
        const closePos = openPos + 1 + i;
        if (state.doc.lineAt(closePos).number > state.doc.lineAt(openPos).number) {
          return { from: openPos + 1, to: closePos };
        }
        return null;
      }
    }
  }
  return null;
});

export function zigLang(): Extension {
  return [zigStream, zigBraceFold];
}
