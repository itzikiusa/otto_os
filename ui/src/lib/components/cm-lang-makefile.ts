// Makefile language for the editor (Makefile, makefile, GNUmakefile, Makefile.*, *.mk,
// *.make, *.mak — isMakefilePath in cm-editor-helpers.ts). A StreamLanguage, loaded with
// the file type (cm-langs.ts). Each line is classified once at its start:
//   • a TAB-indented line after the first rule is a recipe: shell (shellToken), with the
//     Makefile references inside it still coloured and `@` / `-` / `+` prefixes marked;
//   • a comment, a directive (include, export, define…endef, ifeq…endif, override…),
//     a variable assignment (`:=` `=` `?=` `+=` `!=` `::=`: the name is a definition),
//     or a rule (targets — special ones like .PHONY apart — then prerequisites).
// A trailing backslash carries the line's kind to the next line. References — `$(NAME)`,
// `${NAME}`, `$@ $< $^ $* $?`, and functions (`$(shell …)`, `$(call …)`, `$(or …)`) with
// the function name apart from variable names — are coloured in every kind of line.
import { StreamLanguage, type StringStream } from '@codemirror/language';
import type { Extension } from '@codemirror/state';
import { shellToken, type ShellScanState } from './cm-langs-extra.ts';

const FUNCS = new Set([
  'subst', 'patsubst', 'strip', 'findstring', 'filter', 'filter-out', 'sort', 'word', 'wordlist', 'words',
  'firstword', 'lastword', 'dir', 'notdir', 'suffix', 'basename', 'addsuffix', 'addprefix', 'join',
  'wildcard', 'realpath', 'abspath', 'error', 'warning', 'info', 'shell', 'origin', 'flavor', 'foreach',
  'if', 'or', 'and', 'call', 'eval', 'file', 'value', 'let', 'intcmp', 'guile',
]);
const DIRECTIVE = /^(?:-include|sinclude|include|export|unexport|override|private|define|endef|undefine|ifeq|ifneq|ifdef|ifndef|else|endif|vpath)(?![\w.-])/;
const ASSIGN = /^\s*(?:(?:export|override|private)\s+)*[^\s:#=]+\s*(?:::?=|:::=|\?=|\+=|!=|=)/;
const ASSIGN_OP = /^(?::::=|::=|:=|\?=|\+=|!=|=)/;

type Mode = 'rule' | 'prereq' | 'assign' | 'value' | 'directive' | 'recipe' | 'define';

export interface MakeState {
  mode: Mode;
  /** The previous line ended with a backslash: this one keeps its kind. */
  cont: boolean;
  /** A rule was seen, so TAB-indented lines are recipes. */
  seenRule: boolean;
  /** Closers of the open references: `$)` / `$}` close a reference, `)` a plain paren in one. */
  refs: string[];
  /** Just after `$(`: the next word is a function or variable name. */
  refHead: boolean;
  /** The assignment's (or define's) name was read. */
  named: boolean;
  /** A fresh recipe line whose `@` / `-` / `+` prefix is still to come. */
  recipeHead: boolean;
  /** In a recipe's single-quoted string (make still expands `$(…)` in it). */
  sq: boolean;
  sh: ShellScanState;
}

function startLine(stream: StringStream, s: MakeState): void {
  const cont = s.cont;
  s.cont = /\\$/.test(stream.string);
  if (cont) return;
  s.refs = [];
  s.refHead = false;
  if (s.mode === 'define') return;
  const line = stream.string;
  if (line.startsWith('\t') && s.seenRule) {
    s.mode = 'recipe';
    s.recipeHead = true;
    s.sq = false;
    s.sh = { inDoubleQuote: false, expectCmd: true };
  } else if (DIRECTIVE.test(line.trimStart()) && !ASSIGN.test(line.replace(/^\s*(?:export|override|private)\s+/, '')) ) {
    s.mode = 'directive';
  } else if (ASSIGN.test(line)) {
    s.mode = 'assign';
    s.named = false;
  } else if (/^[^#]*:/.test(line) && !/^\s*#/.test(line)) {
    s.mode = 'rule';
    s.seenRule = true;
  } else {
    s.mode = 'value';
  }
}

/** A reference or a token inside one; null when the stream is not at/in a reference. */
function refToken(stream: StringStream, s: MakeState): string | null | undefined {
  if (stream.match(/^\$[({]/)) {
    s.refs.push(stream.current()[1] === '(' ? '$)' : '$}');
    s.refHead = true;
    return 'escape';
  }
  if (stream.match(/^\$\$/)) {
    // An escaped dollar: the shell's own `$$VAR` / `$$(cmd)` in a recipe.
    stream.match(/^(?:[A-Za-z_]\w*|\{[^}]*\})/);
    return 'variableName.special';
  }
  if (stream.match(/^\$[@<^*?+%|A-Za-z0-9]/)) return 'variableName.special';
  if (!s.refs.length) return undefined;
  if (s.refHead) {
    s.refHead = false;
    if (stream.match(/^[a-z][\w-]*(?=[\s,)}]|$)/)) {
      const w = stream.current();
      if (FUNCS.has(w) && !stream.match(/^[)}]/, false)) return 'variableName.function';
      return 'variableName.special';
    }
    if (stream.match(/^[^\s$(){}:,=#]+/)) return 'variableName.special';
  }
  const top = s.refs[s.refs.length - 1];
  const ch = stream.peek();
  if ((ch === ')' || ch === '}') && (top === '$' + ch || top === ch)) {
    stream.next();
    s.refs.pop();
    return top.length === 2 ? 'escape' : null;
  }
  if (ch === '(') {
    stream.next();
    s.refs.push(')');
    return null;
  }
  if (ch === ',') {
    stream.next();
    return 'punctuation';
  }
  return undefined;
}

function token(stream: StringStream, s: MakeState): string | null {
  if (stream.sol()) startLine(stream, s);
  const ref = refToken(stream, s);
  if (ref !== undefined) return ref;
  const inRef = s.refs.length > 0;
  switch (s.mode) {
    case 'recipe':
      if (inRef) break;
      if (stream.sol() && stream.match(/^\t+/)) return null;
      if (s.recipeHead) {
        s.recipeHead = false;
        if (stream.match(/^[@+-]+/)) return 'meta';
      }
      if (s.sq || (!s.sh.inDoubleQuote && stream.peek() === "'")) {
        // A single-quoted string, stopping at a reference so refToken colours it.
        if (!s.sq) {
          stream.next();
          s.sq = true;
        }
        while (!stream.eol() && !stream.match(/^\$[({]/, false)) {
          if (stream.next() === "'") {
            s.sq = false;
            break;
          }
        }
        return 'string';
      }
      if (stream.peek() === '$') {
        stream.next();
        return null;
      }
      // The shell's own string stops before a `$`, so the next token is still a reference.
      return shellToken(stream, s.sh);
    case 'define':
      if (stream.sol() && stream.match(/^\s*endef\b/)) {
        s.mode = 'value';
        return 'keyword';
      }
      if (!s.named && !stream.sol()) {
        if (stream.eatSpace()) return null;
        s.named = true;
        if (stream.match(/^[^\s=]+/)) return 'propertyName.definition';
      }
      break;
    case 'directive': {
      if (stream.eatSpace()) return null;
      if (stream.match(DIRECTIVE)) {
        const w = stream.current();
        if (w === 'define') {
          s.mode = 'define';
          s.named = false;
        }
        else if (w.endsWith('include')) s.mode = 'value';
        return 'keyword';
      }
      if (stream.match(/^[^\s$(),#]+/)) return 'variableName.special';
      break;
    }
    case 'assign': {
      if (stream.eatSpace()) return null;
      if (!s.named) {
        if (stream.match(/^(?:export|override|private)(?=\s)/)) return 'keyword';
        if (stream.match(/^[^\s:#=?+!$]+/)) {
          s.named = true;
          return /^\.[A-Z_]+$/.test(stream.current()) ? 'meta' : 'propertyName.definition';
        }
      }
      if (stream.match(ASSIGN_OP)) {
        s.mode = 'value';
        return 'operator';
      }
      break;
    }
    case 'rule':
    case 'prereq': {
      if (stream.eatSpace()) return null;
      if (stream.match(/^#.*/)) return 'comment';
      if (s.mode === 'rule' && stream.match(/^::?(?!=)/)) {
        s.mode = ASSIGN.test(stream.string.slice(stream.pos)) ? 'assign' : 'prereq';
        s.named = false;
        return 'operator';
      }
      if (stream.match(/^[|;]/)) {
        if (stream.current() === ';') {
          s.mode = 'recipe';
          s.sh = { inDoubleQuote: false, expectCmd: true };
        }
        return 'operator';
      }
      if (stream.match(/^\\$/)) return 'escape';
      if (stream.match(/^[^\s:;|#$\\]+/)) {
        if (s.mode === 'prereq') return 'variableName.function';
        return /^\.[A-Z_]+$/.test(stream.current()) ? 'meta' : 'variableName.function.definition';
      }
      break;
    }
  }
  // Values, directive arguments, define bodies and reference arguments.
  if (stream.eatSpace()) return null;
  if (!inRef && stream.match(/^#.*/)) return 'comment';
  if (stream.match(/^\\$/)) return 'escape';
  stream.next();
  while (!stream.eol() && !/[\s$(){},#\\]/.test(stream.peek() ?? '')) stream.next();
  return null;
}

export const makefileParser = {
  name: 'makefile',
  startState: (): MakeState => ({
    mode: 'value', cont: false, seenRule: false, refs: [], refHead: false, named: false, recipeHead: false, sq: false,
    sh: { inDoubleQuote: false, expectCmd: true },
  }),
  copyState: (s: MakeState): MakeState => ({ ...s, refs: [...s.refs], sh: { ...s.sh } }),
  blankLine: (s: MakeState): void => {
    s.cont = false;
  },
  token,
  languageData: { commentTokens: { line: '#' } },
};

export function makefileLang(): Extension {
  return StreamLanguage.define(makefileParser);
}
