// Minimal CodeMirror highlighter for the Redis command console. There's no
// off-the-shelf Redis language for CodeMirror 6, so this StreamLanguage colors
// the essentials: the leading command keyword on each line, quoted bulk strings,
// numbers, and `#` comments. Token names are the classic CM5 ones that
// StreamLanguage maps to standard highlight tags, so they pick up the same light
// (defaultHighlightStyle) / dark (oneDark) colors as every other language.

import { StreamLanguage } from '@codemirror/language';
import type { Extension } from '@codemirror/state';

interface RedisState {
  cmd: boolean;
}

export function redisLang(): Extension {
  return StreamLanguage.define<RedisState>({
    startState: () => ({ cmd: true }),
    token(stream, state) {
      if (stream.sol()) state.cmd = true;
      if (stream.eatSpace()) return null;
      if (stream.match(/^#.*/)) return 'comment';
      if (stream.match(/^"(?:[^"\\]|\\.)*"?/)) return 'string';
      if (stream.match(/^'(?:[^'\\]|\\.)*'?/)) return 'string';
      if (stream.match(/^[+-]?\d+(?:\.\d+)?\b/)) return 'number';
      // First bareword on a line is the command; the rest are keys/args.
      if (state.cmd) {
        state.cmd = false;
        stream.match(/^\S+/);
        return 'keyword';
      }
      stream.match(/^\S+/);
      return null;
    },
  });
}
