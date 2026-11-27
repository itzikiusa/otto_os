// C# (.cs, .csx, .razor) and F# (.fs, .fsx, .fsi) language packs for CodeMirror 6.
// Covers keywords, contextual keywords (async/await/record/init/required/var/yield/nameof/when),
// primitive types, attributes (`[Fact]`, `[<EntryPoint>]`), XML doc comments (`///`),
// verbatim/interpolated/raw strings (`@"..."`, `$"..."`, `$@"..."`, `"""..."""`),
// numeric literals with `_` separators & type suffixes (`m`, `f`, `d`, `ul`), and brace folding.
import { foldService, StreamLanguage, type StringStream } from '@codemirror/language';
import type { Extension } from '@codemirror/state';

const CS_KEYWORDS = new Set([
  'abstract', 'add', 'alias', 'and', 'as', 'ascending', 'async', 'await', 'base', 'break',
  'by', 'case', 'catch', 'checked', 'class', 'const', 'continue', 'default', 'delegate',
  'descending', 'do', 'else', 'enum', 'equals', 'event', 'explicit', 'extern', 'file',
  'finally', 'fixed', 'for', 'foreach', 'from', 'get', 'global', 'goto', 'group', 'if',
  'implicit', 'in', 'init', 'interface', 'internal', 'into', 'is', 'join', 'let', 'lock',
  'managed', 'nameof', 'namespace', 'new', 'not', 'notnull', 'on', 'operator', 'or',
  'orderby', 'out', 'override', 'params', 'partial', 'private', 'protected', 'public',
  'readonly', 'record', 'ref', 'remove', 'required', 'return', 'scoped', 'sealed', 'select',
  'set', 'sizeof', 'stackalloc', 'static', 'struct', 'switch', 'this', 'throw', 'try',
  'typeof', 'unchecked', 'unmanaged', 'unsafe', 'using', 'value', 'var', 'virtual', 'void',
  'volatile', 'when', 'where', 'while', 'with', 'yield',
]);

const CS_ATOMS = new Set(['true', 'false', 'null']);

const CS_TYPES = new Set([
  'bool', 'byte', 'char', 'decimal', 'double', 'dynamic', 'float', 'int', 'long', 'nint',
  'nuint', 'object', 'sbyte', 'short', 'string', 'uint', 'ulong', 'ushort', 'Task',
  'ValueTask', 'Action', 'Func', 'List', 'Dictionary', 'HashSet', 'IEnumerable', 'IList',
  'IDictionary', 'Span', 'ReadOnlySpan', 'Memory', 'CancellationToken', 'Exception',
  'Guid', 'DateTime', 'DateTimeOffset', 'TimeSpan', 'String', 'Int32', 'Int64', 'Boolean',
]);

interface CsState {
  inBlockComment: boolean;
  inVerbatim: boolean;
  expectDeclName: boolean;
}

const csharpStream = StreamLanguage.define<CsState>({
  name: 'csharp',
  startState: () => ({ inBlockComment: false, inVerbatim: false, expectDeclName: false }),
  token(stream: StringStream, state: CsState): string | null {
    if (state.inBlockComment) {
      while (!stream.eol()) {
        if (stream.match('*/')) {
          state.inBlockComment = false;
          return 'comment';
        }
        stream.next();
      }
      return 'comment';
    }

    if (state.inVerbatim) {
      while (!stream.eol()) {
        if (stream.match('""')) continue;
        if (stream.next() === '"') {
          state.inVerbatim = false;
          return 'string';
        }
      }
      return 'string';
    }

    if (stream.eatSpace()) return null;

    // Preprocessor directives (`#region`, `#if UNITY_EDITOR`, `#nullable enable`, ...)
    if (stream.sol() && stream.peek() === '#') {
      stream.skipToEnd();
      state.expectDeclName = false;
      return 'meta';
    }

    // Comments (`///` XML doc comment, `//` line comment, `/* ... */` block comment)
    if (stream.match('//')) {
      stream.skipToEnd();
      state.expectDeclName = false;
      return 'comment';
    }
    if (stream.match('/*')) {
      state.inBlockComment = true;
      while (!stream.eol()) {
        if (stream.match('*/')) {
          state.inBlockComment = false;
          break;
        }
        stream.next();
      }
      state.expectDeclName = false;
      return 'comment';
    }

    // Verbatim / interpolated verbatim strings (`@"..."`, `$@"..."`, `@$"..."`)
    if (stream.match(/^(?:\$@|@\$|@)"/)) {
      state.inVerbatim = true;
      while (!stream.eol()) {
        if (stream.match('""')) continue;
        if (stream.next() === '"') {
          state.inVerbatim = false;
          break;
        }
      }
      state.expectDeclName = false;
      return 'string';
    }

    // Standard / interpolated strings (`"..."`, `$"..."`)
    if (stream.match(/^\$?"/)) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') stream.next();
        else if (ch === '"') break;
      }
      state.expectDeclName = false;
      return 'string';
    }

    // Character literal
    if (stream.peek() === "'") {
      stream.next();
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') stream.next();
        else if (ch === "'") break;
      }
      state.expectDeclName = false;
      return 'string';
    }

    // Numeric literals (hex, binary, decimal, float with `_` and C# suffixes)
    if (
      stream.match(/^0[xX][0-9a-fA-F_]+[uUlL]*/) ||
      stream.match(/^0[bB][01_]+[uUlL]*/) ||
      stream.match(/^\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d[\d_]*)?[fFdDmMuUlL]*/)
    ) {
      state.expectDeclName = false;
      return 'number';
    }

    // Identifiers, verbatim identifiers (`@class`), keywords, types
    if (stream.match(/^@?[A-Za-z_]\w*/)) {
      const word = stream.current();
      if (word.startsWith('@')) {
        state.expectDeclName = false;
        return 'variableName';
      }
      if (state.expectDeclName) {
        state.expectDeclName = false;
        return 'definition';
      }
      if (CS_ATOMS.has(word)) return 'atom';
      if (CS_TYPES.has(word)) return 'typeName';
      if (CS_KEYWORDS.has(word)) {
        if (word === 'class' || word === 'struct' || word === 'interface' || word === 'enum' || word === 'record' || word === 'namespace') {
          state.expectDeclName = true;
        }
        return 'keyword';
      }
      if (/^[A-Z][A-Za-z0-9_]*$/.test(word)) {
        return stream.peek() === '(' ? 'function' : 'typeName';
      }
      if (stream.peek() === '(') return 'function';
      return 'variableName';
    }

    if (stream.match(/^(?:=>|\?\?=|\?\?|\?\.|==|!=|<=|>=|&&|\|\||<<|>>|\+\+|--|[+\-*/%&|^~!<>?=:])/)) {
      state.expectDeclName = false;
      return 'operator';
    }

    const ch = stream.next();
    if (ch && '{}()[];,.'.includes(ch)) {
      if (ch === '{' || ch === ';' || ch === ')') state.expectDeclName = false;
      return 'punctuation';
    }
    return null;
  },
  languageData: {
    commentTokens: { line: '//', block: { open: '/*', close: '*/' } },
    closeBrackets: { brackets: ['(', '[', '{', '"', "'"] },
  },
});

const braceFold = foldService.of((state, lineStart, lineEnd) => {
  const text = state.sliceDoc(lineStart, lineEnd);
  const openIdx = text.lastIndexOf('{');
  if (openIdx < 0) return null;
  const closeOnSame = text.indexOf('}', openIdx + 1);
  if (closeOnSame >= 0) return null;
  let depth = 0;
  const docLen = state.doc.length;
  for (let pos = lineStart + openIdx; pos < docLen; pos++) {
    const c = state.sliceDoc(pos, pos + 1);
    if (c === '{') depth++;
    else if (c === '}') {
      depth--;
      if (depth === 0) {
        const endLine = state.doc.lineAt(pos);
        if (endLine.from > lineStart) return { from: lineEnd, to: pos };
        return null;
      }
    }
  }
  return null;
});

const FS_KEYWORDS = new Set([
  'abstract', 'and', 'as', 'assert', 'base', 'begin', 'class', 'default', 'delegate', 'do',
  'done', 'downcast', 'downto', 'elif', 'else', 'end', 'exception', 'extern', 'finally',
  'fixed', 'for', 'fun', 'function', 'global', 'if', 'in', 'inherit', 'inline', 'interface',
  'internal', 'lazy', 'let', 'match', 'member', 'module', 'mutable', 'namespace', 'new',
  'not', 'of', 'open', 'or', 'override', 'private', 'public', 'rec', 'return', 'select',
  'static', 'struct', 'then', 'to', 'try', 'type', 'upcast', 'use', 'val', 'void', 'when',
  'while', 'with', 'yield', 'async', 'task', 'seq',
]);

const FS_ATOMS = new Set(['true', 'false', 'null', 'None', 'Some', 'Ok', 'Error']);

const FS_TYPES = new Set([
  'unit', 'bool', 'byte', 'sbyte', 'int16', 'uint16', 'int', 'int32', 'uint32', 'int64',
  'uint64', 'nativeint', 'unativeint', 'float', 'float32', 'single', 'double', 'decimal',
  'char', 'string', 'obj', 'exn', 'list', 'array', 'option', 'voption', 'Result', 'Async',
  'Task', 'Map', 'Set',
]);

interface FsState {
  blockDepth: number;
}

const fsharpStream = StreamLanguage.define<FsState>({
  name: 'fsharp',
  startState: () => ({ blockDepth: 0 }),
  token(stream: StringStream, state: FsState): string | null {
    if (state.blockDepth > 0) {
      while (!stream.eol()) {
        if (stream.match('(*')) state.blockDepth++;
        else if (stream.match('*)')) {
          state.blockDepth--;
          if (state.blockDepth === 0) return 'comment';
        } else {
          stream.next();
        }
      }
      return 'comment';
    }
    if (stream.eatSpace()) return null;
    if (stream.match('//')) {
      stream.skipToEnd();
      return 'comment';
    }
    if (stream.match('(*')) {
      state.blockDepth = 1;
      while (!stream.eol()) {
        if (stream.match('(*')) state.blockDepth++;
        else if (stream.match('*)')) {
          state.blockDepth--;
          if (state.blockDepth === 0) break;
        } else {
          stream.next();
        }
      }
      return 'comment';
    }
    if (stream.match(/^\[<[^>]*>\]/)) return 'meta';
    if (stream.match(/^"""[\s\S]*?"""/) || stream.match(/^@"(?:""|[^"])*"/)) return 'string';
    if (stream.match(/^\$?"/)) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') stream.next();
        else if (ch === '"') break;
      }
      return 'string';
    }
    if (stream.match(/^0[xX][0-9a-fA-F_]+[uUlLyYsS]*/) || stream.match(/^\d[\d_]*(?:\.\d[\d_]*)?[fFmMuUlLyYsS]*/)) {
      return 'number';
    }
    if (stream.match(/^``[^`]+``/)) return 'variableName';
    if (stream.match(/^[A-Za-z_][\w']*/)) {
      const w = stream.current();
      if (FS_ATOMS.has(w)) return 'atom';
      if (FS_TYPES.has(w)) return 'typeName';
      if (FS_KEYWORDS.has(w)) return 'keyword';
      if (/^[A-Z]/.test(w)) return 'typeName';
      return 'variableName';
    }
    if (stream.match(/^(?:\|>|<\||->|<-|::|<>|<=|>=|&&|\|\||[+\-*/%=<>!:|])/)) return 'operator';
    stream.next();
    return null;
  },
  languageData: {
    commentTokens: { line: '//', block: { open: '(*', close: '*)' } },
    closeBrackets: { brackets: ['(', '[', '{', '"'] },
  },
});

export function csharpLang(): Extension {
  return [csharpStream, braceFold];
}

export function fsharpLang(): Extension {
  return [fsharpStream, braceFold];
}
