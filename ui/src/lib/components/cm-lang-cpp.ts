/**
 * Syntax + folding support for C, C++23 (with Unreal Engine reflection macros),
 * CMake (`CMakeLists.txt` / `*.cmake`), and Meson (`meson.build` / `meson_options.txt`).
 *
 * Uses `@codemirror/language`'s `StreamLanguage` + a brace `foldService` so no
 * extra npm package is required and every C/C++/CMake/Meson buffer gets full
 * keyword, preprocessor, raw-string, Unreal-macro, type, and comment highlighting.
 */
import { foldService, LanguageSupport, StreamLanguage, type StringStream } from '@codemirror/language';

const CPP_KEYWORDS = new Set([
  'alignas', 'alignof', 'and', 'and_eq', 'asm', 'auto', 'bitand', 'bitor',
  'break', 'case', 'catch', 'class', 'co_await', 'co_return', 'co_yield',
  'compl', 'concept', 'const', 'const_cast', 'consteval', 'constexpr',
  'constinit', 'continue', 'decltype', 'default', 'delete', 'do',
  'dynamic_cast', 'else', 'enum', 'explicit', 'export', 'extern', 'final',
  'for', 'friend', 'goto', 'if', 'import', 'inline', 'module', 'mutable',
  'namespace', 'new', 'noexcept', 'not', 'not_eq', 'operator', 'or', 'or_eq',
  'override', 'private', 'protected', 'public', 'register', 'reinterpret_cast',
  'requires', 'restrict', 'return', 'sizeof', 'static', 'static_assert',
  'static_cast', 'struct', 'switch', 'template', 'this', 'thread_local',
  'throw', 'try', 'typedef', 'typeid', 'typename', 'union', 'using',
  'virtual', 'volatile', 'while', 'xor', 'xor_eq',
]);

const CPP_ATOMS = new Set(['true', 'false', 'nullptr', 'NULL', 'INDEX_NONE']);

const CPP_TYPES = new Set([
  'void', 'bool', 'char', 'char8_t', 'char16_t', 'char32_t', 'wchar_t',
  'short', 'int', 'long', 'signed', 'unsigned', 'float', 'double',
  'size_t', 'ssize_t', 'ptrdiff_t', 'intptr_t', 'uintptr_t',
  'int8_t', 'int16_t', 'int32_t', 'int64_t',
  'uint8_t', 'uint16_t', 'uint32_t', 'uint64_t',
  // Unreal Engine core primitives & containers
  'int8', 'int16', 'int32', 'int64', 'uint8', 'uint16', 'uint32', 'uint64',
  'FString', 'FName', 'FText', 'FVector', 'FVector2D', 'FRotator', 'FTransform',
  'FQuat', 'FColor', 'FLinearColor', 'TArray', 'TMap', 'TSet', 'TSharedPtr',
  'TSharedRef', 'TWeakPtr', 'TUniquePtr', 'TObjectPtr', 'TSubclassOf',
  'TOptional', 'UObject', 'AActor', 'APawn', 'ACharacter', 'UActorComponent',
  'USceneComponent', 'UWorld', 'UGameInstance', 'APlayerController',
]);

const UNREAL_AND_TEST_MACROS = new Set([
  'UCLASS', 'USTRUCT', 'UENUM', 'UINTERFACE', 'UPROPERTY', 'UFUNCTION',
  'UMETA', 'UPARAM', 'GENERATED_BODY', 'GENERATED_UCLASS_BODY',
  'GENERATED_USTRUCT_BODY', 'DECLARE_LOG_CATEGORY_EXTERN', 'DEFINE_LOG_CATEGORY',
  'UE_LOG', 'CHECK', 'check', 'checkf', 'ensure', 'ensureMsgf',
  'IMPLEMENT_PRIMARY_GAME_MODULE', 'IMPLEMENT_MODULE', 'LOCTEXT', 'NSLOCTEXT',
  'TEXT', 'TEXTVIEW',
  // GoogleTest & Catch2 macros
  'TEST', 'TEST_F', 'TEST_P', 'TYPED_TEST', 'TYPED_TEST_P', 'INSTANTIATE_TEST_SUITE_P',
  'EXPECT_EQ', 'EXPECT_NE', 'EXPECT_TRUE', 'EXPECT_FALSE', 'EXPECT_LT', 'EXPECT_LE',
  'EXPECT_GT', 'EXPECT_GE', 'EXPECT_NEAR', 'EXPECT_THROW', 'EXPECT_NO_THROW',
  'ASSERT_EQ', 'ASSERT_NE', 'ASSERT_TRUE', 'ASSERT_FALSE', 'ASSERT_THROW',
  'TEST_CASE', 'SCENARIO', 'SECTION', 'GIVEN', 'WHEN', 'THEN', 'REQUIRE', 'CHECK_THROWS',
]);

interface CppState {
  blockComment: boolean;
  rawDelim: string | null;
}

const cppStream = StreamLanguage.define<CppState>({
  name: 'cpp',
  startState: () => ({ blockComment: false, rawDelim: null }),
  copyState: (s) => ({ blockComment: s.blockComment, rawDelim: s.rawDelim }),
  token(stream: StringStream, state: CppState): string | null {
    if (state.blockComment) {
      while (!stream.eol()) {
        if (stream.match('*/')) {
          state.blockComment = false;
          break;
        }
        stream.next();
      }
      return 'comment';
    }
    if (state.rawDelim !== null) {
      const endToken = `)${state.rawDelim}"`;
      while (!stream.eol()) {
        if (stream.match(endToken)) {
          state.rawDelim = null;
          break;
        }
        stream.next();
      }
      return 'string';
    }
    if (stream.eatSpace()) return null;

    // Preprocessor directive at start of line
    if (stream.sol() || stream.string.slice(0, stream.pos).trim() === '') {
      if (stream.match(/^#\s*[A-Za-z_]\w*/)) return 'meta';
    }
    if (stream.match('//')) {
      stream.skipToEnd();
      return 'comment';
    }
    if (stream.match('/*')) {
      state.blockComment = true;
      return 'comment';
    }
    // C++ raw string literal: R"delim(...)delim"
    const rawMatch = stream.match(/^(?:u8|u|U|L)?R"([^\s()\\]{0,16})\(/);
    if (rawMatch && Array.isArray(rawMatch)) {
      state.rawDelim = rawMatch[1] ?? '';
      return 'string';
    }
    // String & char literals (including u8/u/U/L prefixes)
    if (stream.match(/^(?:u8|u|U|L)?"/)) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') { stream.next(); continue; }
        if (ch === '"') break;
      }
      return 'string';
    }
    if (stream.match(/^(?:u8|u|U|L)?'/)) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') { stream.next(); continue; }
        if (ch === "'") break;
      }
      return 'string';
    }
    // Numbers (hex, binary, octal, floats with digit separators)
    if (stream.match(/^0[xX][0-9a-fA-F']+(?:\.[0-9a-fA-F']*)?(?:[pP][+-]?[0-9']+)?[uUlLfF]*/)) return 'number';
    if (stream.match(/^0[bB][01']+[uUlL]*/)) return 'number';
    if (stream.match(/^\d[\d']*(?:\.[\d']*)?(?:[eE][+-]?[\d']+)?[uUlLfF]*/)) return 'number';

    // Identifiers, keywords, types, Unreal/GTest macros, std:: types
    if (stream.match(/^[A-Za-z_]\w*/)) {
      const w = stream.current();
      if (CPP_KEYWORDS.has(w)) return 'keyword';
      if (CPP_ATOMS.has(w)) return 'atom';
      if (CPP_TYPES.has(w)) return 'typeName';
      if (UNREAL_AND_TEST_MACROS.has(w) || w.endsWith('_API')) return 'macroName';
      if (/^[A-Z][A-Za-z0-9_]*$/.test(w) && /[a-z]/.test(w)) return 'typeName';
      if (stream.peek() === '(') return 'function';
      return 'variableName';
    }
    if (stream.match(/^(?:::|->|\.\.\.|<=|>=|==|!=|&&|\|\||<<=|>>=|<<|>>|[+\-*/%&|^~!=<>?:])/)) {
      return 'operator';
    }
    stream.next();
    return null;
  },
  languageData: {
    commentTokens: { line: '//', block: { open: '/*', close: '*/' } },
    closeBrackets: { brackets: ['(', '[', '{', '"', "'"] },
  },
});

const braceFold = foldService.of((state, lineStart, lineEnd) => {
  const lineText = state.doc.sliceString(lineStart, lineEnd);
  const openIdx = lineText.indexOf('{');
  if (openIdx < 0) return null;
  let depth = 0;
  let pos = lineStart + openIdx;
  const maxPos = state.doc.length;
  while (pos < maxPos) {
    const ch = state.doc.sliceString(pos, pos + 1);
    if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) {
        if (pos <= lineEnd) return null;
        return { from: lineStart + openIdx + 1, to: pos };
      }
    }
    pos += 1;
  }
  return null;
});

export function cppLang(): LanguageSupport {
  return new LanguageSupport(cppStream, [braceFold]);
}

const CMAKE_KEYWORDS = new Set([
  'cmake_minimum_required', 'project', 'add_executable', 'add_library',
  'target_link_libraries', 'target_include_directories', 'target_compile_definitions',
  'target_compile_options', 'target_compile_features', 'target_sources',
  'find_package', 'find_library', 'find_path', 'find_program',
  'include', 'include_directories', 'add_subdirectory', 'enable_testing',
  'add_test', 'set', 'unset', 'option', 'list', 'string', 'file', 'message',
  'install', 'configure_file', 'execute_process', 'add_custom_command',
  'add_custom_target', 'fetchcontent_declare', 'fetchcontent_makeavailable',
  'if', 'elseif', 'else', 'endif', 'foreach', 'endforeach', 'while', 'endwhile',
  'function', 'endfunction', 'macro', 'endmacro', 'return', 'break', 'continue',
  'gtest_discover_tests', 'catch_discover_tests',
]);

const CMAKE_MODIFIERS = new Set([
  'PUBLIC', 'PRIVATE', 'INTERFACE', 'REQUIRED', 'CONFIG', 'MODULE', 'STATIC',
  'SHARED', 'OBJECT', 'IMPORTED', 'ALIAS', 'GLOBAL', 'CACHE', 'FORCE',
  'PARENT_SCOPE', 'ON', 'OFF', 'TRUE', 'FALSE', 'AND', 'OR', 'NOT',
  'VERSION', 'LANGUAGES', 'COMMAND', 'NAME', 'TARGETS', 'DESTINATION',
]);

const cmakeStream = StreamLanguage.define<Record<string, never>>({
  name: 'cmake',
  startState: () => ({}),
  token(stream: StringStream): string | null {
    if (stream.eatSpace()) return null;
    if (stream.match('#')) { stream.skipToEnd(); return 'comment'; }
    if (stream.match('"')) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') { stream.next(); continue; }
        if (ch === '"') break;
      }
      return 'string';
    }
    if (stream.match(/^\$\{[^}]+\}/) || stream.match(/^\$<[^>]+>/)) return 'variableName.special';
    if (stream.match(/^\d+(?:\.\d+)*/)) return 'number';
    if (stream.match(/^[A-Za-z_][A-Za-z0-9_-]*/)) {
      const w = stream.current();
      if (CMAKE_KEYWORDS.has(w.toLowerCase())) return 'keyword';
      if (CMAKE_MODIFIERS.has(w)) return 'atom';
      if (stream.peek() === '(') return 'function';
      return 'variableName';
    }
    stream.next();
    return null;
  },
  languageData: {
    commentTokens: { line: '#' },
    closeBrackets: { brackets: ['(', '"'] },
  },
});

export function cmakeLang(): LanguageSupport {
  return new LanguageSupport(cmakeStream);
}

const MESON_KEYWORDS = new Set([
  'if', 'elif', 'else', 'endif', 'foreach', 'endforeach', 'break', 'continue',
  'and', 'or', 'not', 'in', 'true', 'false',
]);

const MESON_BUILTINS = new Set([
  'project', 'executable', 'library', 'static_library', 'shared_library',
  'both_libraries', 'dependency', 'subproject', 'subdir', 'test', 'benchmark',
  'declare_dependency', 'include_directories', 'configure_file', 'custom_target',
  'find_program', 'get_option', 'option', 'files', 'message', 'warning', 'error',
  'summary', 'install_headers', 'install_data', 'add_project_arguments',
  'meson', 'host_machine', 'build_machine', 'target_machine', 'import',
]);

const mesonStream = StreamLanguage.define<Record<string, never>>({
  name: 'meson',
  startState: () => ({}),
  token(stream: StringStream): string | null {
    if (stream.eatSpace()) return null;
    if (stream.match('#')) { stream.skipToEnd(); return 'comment'; }
    if (stream.match("'''")) {
      while (!stream.eol()) {
        if (stream.match("'''")) break;
        stream.next();
      }
      return 'string';
    }
    if (stream.match("'")) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === '\\') { stream.next(); continue; }
        if (ch === "'") break;
      }
      return 'string';
    }
    if (stream.match(/^\d+(?:\.\d+)?/)) return 'number';
    if (stream.match(/^[A-Za-z_]\w*/)) {
      const w = stream.current();
      if (MESON_KEYWORDS.has(w)) return 'keyword';
      if (MESON_BUILTINS.has(w)) return 'function';
      return 'variableName';
    }
    stream.next();
    return null;
  },
  languageData: {
    commentTokens: { line: '#' },
    closeBrackets: { brackets: ['(', '[', '{', "'"] },
  },
});

export function mesonLang(): LanguageSupport {
  return new LanguageSupport(mesonStream);
}
