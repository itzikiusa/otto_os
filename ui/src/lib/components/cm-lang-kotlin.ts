// Kotlin (.kt / .kts) language pack for CodeMirror 6.
// Covers Kotlin 2.x keywords, coroutine modifiers (`suspend`, `inline`, `crossinline`, `noinline`,
// `reified`), class modifiers (`data`, `sealed`, `value`, `inner`, `enum`, `annotation`, `object`,
// `companion`, `expect`, `actual`), annotations (`@Test`, `@file:JvmName`), string templates
// (`"$x ${expr}"`), raw triple-quoted strings (`"""..."""`), nested block comments (`/* /* */ */`),
// backtick identifiers (`` `adds numbers` ``), and brace folding.
import { foldService, StreamLanguage, type StringStream } from '@codemirror/language';
import type { Extension } from '@codemirror/state';

const KEYWORDS = new Set([
  'abstract', 'actual', 'annotation', 'as', 'break', 'by', 'catch', 'class', 'companion',
  'const', 'constructor', 'continue', 'crossinline', 'data', 'delegate', 'do', 'dynamic',
  'else', 'enum', 'expect', 'external', 'field', 'file', 'final', 'finally', 'for', 'fun',
  'get', 'if', 'import', 'in', 'infix', 'init', 'inline', 'inner', 'interface', 'internal',
  'is', 'lateinit', 'noinline', 'object', 'open', 'operator', 'out', 'override', 'package',
  'param', 'private', 'property', 'protected', 'public', 'receiver', 'reified', 'return',
  'sealed', 'set', 'setparam', 'super', 'suspend', 'tailrec', 'this', 'throw', 'try',
  'typealias', 'typeof', 'val', 'value', 'var', 'vararg', 'when', 'where', 'while',
]);

const ATOMS = new Set(['true', 'false', 'null']);

const STDLIB_TYPES = new Set([
  'Any', 'Array', 'Boolean', 'BooleanArray', 'Byte', 'ByteArray', 'Char', 'CharArray',
  'CharSequence', 'Collection', 'Comparable', 'Double', 'DoubleArray', 'Float', 'FloatArray',
  'Int', 'IntArray', 'Iterable', 'List', 'Long', 'LongArray', 'Map', 'MutableCollection',
  'MutableList', 'MutableMap', 'MutableSet', 'Nothing', 'Number', 'Pair', 'Result', 'Sequence',
  'Set', 'Short', 'ShortArray', 'String', 'Throwable', 'Triple', 'UByte', 'UInt', 'ULong',
  'UShort', 'Unit',
]);

interface KotlinState {
  blockCommentDepth: number;
  inRawString: boolean;
  expectDeclName: boolean;
}

const kotlinStream = StreamLanguage.define<KotlinState>({
  name: 'kotlin',
  startState: () => ({
    blockCommentDepth: 0,
    inRawString: false,
    expectDeclName: false,
  }),
  token(stream: StringStream, state: KotlinState): string | null {
    // Nested block comments (`/* /* ... */ */`)
    if (state.blockCommentDepth > 0) {
      while (!stream.eol()) {
        if (stream.match('/*')) {
          state.blockCommentDepth++;
        } else if (stream.match('*/')) {
          state.blockCommentDepth--;
          if (state.blockCommentDepth === 0) break;
        } else {
          stream.next();
        }
      }
      return 'comment';
    }

    // Multiline raw string (`"""..."""`)
    if (state.inRawString) {
      while (!stream.eol()) {
        if (stream.match('"""')) {
          state.inRawString = false;
          break;
        }
        stream.next();
      }
      return 'string';
    }

    if (stream.eatSpace()) return null;

    // Line comments (`//`)
    if (stream.match('//')) {
      stream.skipToEnd();
      state.expectDeclName = false;
      return 'comment';
    }

    // Block / KDoc comments (`/*`, `/**`)
    if (stream.match('/*')) {
      state.blockCommentDepth = 1;
      while (!stream.eol()) {
        if (stream.match('/*')) {
          state.blockCommentDepth++;
        } else if (stream.match('*/')) {
          state.blockCommentDepth--;
          if (state.blockCommentDepth === 0) break;
        } else {
          stream.next();
        }
      }
      state.expectDeclName = false;
      return 'comment';
    }

    // Raw triple-quoted strings (`"""..."""`)
    if (stream.match('"""')) {
      state.inRawString = true;
      while (!stream.eol()) {
        if (stream.match('"""')) {
          state.inRawString = false;
          break;
        }
        stream.next();
      }
      state.expectDeclName = false;
      return 'string';
    }

    // Double-quoted strings (`"..."` with `\` escapes)
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

    // Character literals (`'a'`, `'\n'`)
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

    // Backtick-quoted identifiers (e.g. ``fun `adds two numbers`()``)
    if (stream.peek() === '`') {
      stream.next();
      while (!stream.eol()) {
        if (stream.next() === '`') break;
      }
      const wasDecl = state.expectDeclName;
      state.expectDeclName = false;
      return wasDecl ? 'definition(variableName)' : 'variableName';
    }

    // Annotations (`@Test`, `@file:JvmName`, `@get:Rule`, label `@loop`)
    if (stream.peek() === '@') {
      if (stream.match(/^@(?:(?:file|field|property|get|set|receiver|param|setparam|delegate):)?[A-Za-z_]\w*/)) {
        state.expectDeclName = false;
        return 'meta';
      }
    }

    // Numeric literals (hex, binary, decimal, float, long/unsigned suffixes)
    if (
      stream.match(/^0[xX][0-9a-fA-F_]+[uUlL]*/) ||
      stream.match(/^0[bB][01_]+[uUlL]*/) ||
      stream.match(/^\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d[\d_]*)?[fFuUlL]*/)
    ) {
      state.expectDeclName = false;
      return 'number';
    }

    // Identifiers, keywords, types
    if (stream.match(/^[A-Za-z_]\w*/)) {
      const word = stream.current();
      if (KEYWORDS.has(word)) {
        state.expectDeclName = word === 'fun' || word === 'class' || word === 'interface' || word === 'object' || word === 'typealias';
        return 'keyword';
      }
      if (ATOMS.has(word)) {
        state.expectDeclName = false;
        return 'atom';
      }
      if (STDLIB_TYPES.has(word)) {
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

    // Operators (`?.`, `?:`, `::`, `->`, `..`, `===`, `!==`, ...)
    if (stream.match(/^(?:\?\.|\?:|::|->|\.\.<|\.\.|===|!==|==|!=|<=|>=|&&|\|\||\+\+|--|\+=|-=|\*=|\/=|%=|[+\-*/%&|^!~<>?=:])/)) {
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
    commentTokens: { line: '//', block: { open: '/*', close: '*/' } },
    closeBrackets: { brackets: ['(', '[', '{', '"', "'", '`'] },
  },
});

const kotlinBraceFold = foldService.of((state, lineStart, lineEnd) => {
  const lineText = state.doc.sliceString(lineStart, lineEnd);
  const openIdx = lineText.lastIndexOf('{');
  if (openIdx < 0) return null;
  const from = lineStart + openIdx + 1;
  let depth = 1;
  let pos = from;
  const len = state.doc.length;
  let inStr: string | null = null;
  while (pos < len && depth > 0) {
    const ch = state.doc.sliceString(pos, pos + 1);
    if (inStr) {
      if (ch === '\\' && inStr !== '"""') {
        pos += 2;
        continue;
      }
      if (inStr === '"""' && state.doc.sliceString(pos, pos + 3) === '"""') {
        inStr = null;
        pos += 3;
        continue;
      }
      if (ch === inStr) inStr = null;
      pos++;
      continue;
    }
    if (state.doc.sliceString(pos, pos + 3) === '"""') {
      inStr = '"""';
      pos += 3;
      continue;
    }
    if (ch === '"' || ch === "'") {
      inStr = ch;
      pos++;
      continue;
    }
    if (ch === '/' && state.doc.sliceString(pos, pos + 2) === '//') {
      const nl = state.doc.lineAt(pos).to;
      pos = nl + 1;
      continue;
    }
    if (ch === '{') depth++;
    else if (ch === '}') {
      depth--;
      if (depth === 0) {
        const endLine = state.doc.lineAt(pos);
        if (endLine.from <= lineStart) return null;
        return { from, to: pos };
      }
    }
    pos++;
  }
  return null;
});

export function kotlinLang(): Extension {
  return [kotlinStream, kotlinBraceFold];
}
