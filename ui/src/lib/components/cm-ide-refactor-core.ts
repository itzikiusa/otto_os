// Pure zero-Svelte refactoring & CodeVision analysis helpers for the Otto IDE
// (`R✎ Rename usages`, `R✎ Implement interface`, String Manipulation, Introduce
// Variable/Constant/Type, Inline Variable, Extract Method/Interface, Generate).

import { EditorState } from '@codemirror/state';

export type IdeInlineRefactorHint =
  | { kind: 'rename-usages'; line: number; column: number; oldName: string; newName: string }
  | { kind: 'implement-interface'; line: number; column: number; symbol: string };

export type StringManipKind =
  | 'upper' | 'lower' | 'camel' | 'pascal' | 'snake' | 'kebab'
  | 'screaming_snake' | 'sort_lines' | 'unique_lines';

export const MAX_CODEVISION_LINES = 4_000;

const CONTROL_WORDS = new Set([
  'if', 'else', 'for', 'while', 'switch', 'case', 'default', 'select', 'return',
  'break', 'continue', 'goto', 'defer', 'go', 'package', 'import', 'range',
  'throw', 'try', 'catch', 'finally', 'yield', 'await', 'do',
]);

interface ParsedDeclLine {
  kind: 'kw' | 'field' | 'member';
  indent: string;
  prefixKey: string;
  ident: string;
  identCol: number;
  suffixKey: string;
}

const KW_DECL_RE = /^(\s*)((?:export\s+)?(?:default\s+)?(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:func(?:\s*\([^)]*\))?|fn|function|def|type|struct|class|interface|enum|trait|const|var|let)\s+)([A-Za-z_]\w*)(\b.*)$/;
export const GO_FIELD_RE = /^(\s+)([A-Za-z_]\w*)(\s+)((?:\*|\[\]|map\[|chan\b|func\(|interface\{|[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)?)(?:\s+`[^`]*`)?\s*(?:\/\/.*)?)$/;
const TYPED_MEMBER_RE = /^(\s*)((?:(?:public|private|protected|internal|static|final|readonly|abstract|override)\s+)+(?:[A-Za-z_][\w<>,.?[\]]*\s+)?)([A-Za-z_]\w*)(\s*[=(;:].*)$/;
const IMPL_STRUCT_RE = /^(\s*)(?:type\s+([A-Za-z_]\w*)\s+struct\b|(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_]\w*)\b|(?:export\s+)?(?:(?:public|abstract|final)\s+)*class\s+([A-Za-z_]\w*)\b)/;

function parseDeclLine(lineText: string): ParsedDeclLine | null {
  if (!lineText || lineText.length > 260) return null;
  const trimmed = lineText.trimStart();
  if (!trimmed || trimmed.startsWith('//') || trimmed.startsWith('#') || trimmed.startsWith('/*') || trimmed.startsWith('*')) {
    return null;
  }
  const kw = KW_DECL_RE.exec(lineText);
  if (kw && !CONTROL_WORDS.has(kw[3])) {
    return {
      kind: 'kw',
      indent: kw[1],
      prefixKey: kw[2].replace(/\s+/g, ' ').trim(),
      ident: kw[3],
      identCol: kw[1].length + kw[2].length + 1,
      suffixKey: kw[4].replace(/\s+/g, ' ').trim(),
    };
  }
  const tm = TYPED_MEMBER_RE.exec(lineText);
  if (tm && !CONTROL_WORDS.has(tm[3])) {
    return {
      kind: 'member',
      indent: tm[1],
      prefixKey: tm[2].replace(/\s+/g, ' ').trim(),
      ident: tm[3],
      identCol: tm[1].length + tm[2].length + 1,
      suffixKey: tm[4].replace(/\s+/g, ' ').trim(),
    };
  }
  const gf = GO_FIELD_RE.exec(lineText);
  if (gf && !CONTROL_WORDS.has(gf[2]) && !CONTROL_WORDS.has(gf[4].split(/\W/)[0] ?? '')) {
    return {
      kind: 'field',
      indent: gf[1],
      prefixKey: `field:${gf[1].length}`,
      ident: gf[2],
      identCol: gf[1].length + 1,
      suffixKey: gf[4].replace(/\s+/g, ' ').trim(),
    };
  }
  return null;
}

export function detectInlineRefactorHints(
  docText: string,
  baselineText: string | null | undefined,
  language?: string,
): IdeInlineRefactorHint[] {
  const docLines = docText.split(/\r\n?|\n/);
  if (docLines.length === 0 || docLines.length > MAX_CODEVISION_LINES) return [];
  const baseLines = baselineText != null && baselineText !== docText
    ? baselineText.split(/\r\n?|\n/)
    : null;
  const hints: IdeInlineRefactorHint[] = [];
  let implCount = 0;
  const langLower = (language ?? '').toLowerCase();
  const supportsImplHint = !langLower || ['go', 'rust', 'typescript', 'javascript', 'java', 'kotlin', 'python'].includes(langLower);

  for (let i = 0; i < docLines.length; i++) {
    const lineText = docLines[i];
    const lineNo = i + 1;
    if (baseLines && i < baseLines.length + 4) {
      const curDecl = parseDeclLine(lineText);
      if (curDecl) {
        let matchedBase: ParsedDeclLine | null = null;
        for (const delta of [0, -1, 1, -2, 2, -3, 3]) {
          const bi = i + delta;
          if (bi < 0 || bi >= baseLines.length) continue;
          const cand = parseDeclLine(baseLines[bi]);
          if (
            cand &&
            cand.kind === curDecl.kind &&
            cand.prefixKey === curDecl.prefixKey &&
            cand.suffixKey === curDecl.suffixKey
          ) {
            matchedBase = cand;
            break;
          }
        }
        if (matchedBase && matchedBase.ident !== curDecl.ident) {
          hints.push({
            kind: 'rename-usages',
            line: lineNo,
            column: curDecl.identCol,
            oldName: matchedBase.ident,
            newName: curDecl.ident,
          });
        }
      }
    }

    if (supportsImplHint && implCount < 24) {
      const m = IMPL_STRUCT_RE.exec(lineText);
      const sym = m?.[2] || m?.[3] || m?.[4];
      if (sym && !/\bimplements\b/.test(lineText)) {
        implCount += 1;
        hints.push({
          kind: 'implement-interface',
          line: lineNo,
          column: (m?.[1]?.length ?? 0) + 1,
          symbol: sym,
        });
      }
    }
  }
  return hints;
}

function splitWords(input: string): string[] {
  return input
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean);
}

export function transformStringManipulation(text: string, kind: StringManipKind): string {
  if (!text) return text;
  if (kind === 'upper') return text.toUpperCase();
  if (kind === 'lower') return text.toLowerCase();
  if (kind === 'sort_lines') return text.split('\n').sort((a, b) => a.localeCompare(b)).join('\n');
  if (kind === 'unique_lines') return [...new Set(text.split('\n'))].join('\n');
  return text.split('\n').map((line) => {
    const words = splitWords(line);
    if (!words.length) return line;
    const lower = words.map((w) => w.toLowerCase());
    if (kind === 'snake') return lower.join('_');
    if (kind === 'kebab') return lower.join('-');
    if (kind === 'screaming_snake') return lower.join('_').toUpperCase();
    if (kind === 'pascal') return lower.map((w) => w[0].toUpperCase() + w.slice(1)).join('');
    return lower.map((w, idx) => (idx === 0 ? w : w[0].toUpperCase() + w.slice(1))).join('');
  }).join('\n');
}

export function computeIntroduceVariableEdit(
  docText: string,
  selFrom: number,
  selTo: number,
  language?: string,
  asConstant = false,
): { changes: { from: number; to: number; insert: string }[]; selectFrom: number; selectTo: number } | null {
  const st = EditorState.create({ doc: docText });
  let from = Math.max(0, Math.min(st.doc.length, selFrom));
  let to = Math.max(from, Math.min(st.doc.length, selTo));
  if (from === to) {
    const ln = st.doc.lineAt(from);
    const rhs = /^(?:\s*(?:return|[A-Za-z_]\w*\s*:?=)\s*)(.+?);?\s*$/.exec(ln.text);
    if (!rhs?.[1]) return null;
    const idx = ln.text.lastIndexOf(rhs[1]);
    from = ln.from + idx;
    to = from + rhs[1].length;
  }
  const expr = st.sliceDoc(from, to).trim();
  if (!expr || expr.includes('\n')) return null;
  const line = st.doc.lineAt(from);
  const indent = /^(\s*)/.exec(line.text)?.[1] ?? '';
  const lang = (language ?? '').toLowerCase();
  const varName = asConstant ? (lang === 'go' ? 'extractedConst' : 'EXTRACTED_VALUE') : 'extracted';
  let declPrefix = `const ${varName} = `;
  let declSuffix = ';';
  if (lang === 'go') {
    declPrefix = asConstant ? `const ${varName} = ` : `${varName} := `;
    declSuffix = '';
  } else if (lang === 'rust') {
    declPrefix = asConstant ? `const ${varName}: _ = ` : `let ${varName} = `;
  } else if (lang === 'python') {
    declPrefix = `${varName} = `;
    declSuffix = '';
  } else if (lang === 'java') {
    declPrefix = asConstant ? `final var ${varName} = ` : `var ${varName} = `;
  }
  const declLine = `${indent}${declPrefix}${expr}${declSuffix}\n`;
  const nameOffsetInDecl = indent.length + declPrefix.indexOf(varName);
  return {
    changes: [
      { from: line.from, to: line.from, insert: declLine },
      { from, to, insert: varName },
    ],
    selectFrom: line.from + nameOffsetInDecl,
    selectTo: line.from + nameOffsetInDecl + varName.length,
  };
}

export function computeInlineVariableEdit(
  docText: string,
  cursorPos: number,
): { changes: { from: number; to: number; insert: string }[] } | null {
  const st = EditorState.create({ doc: docText });
  const ln = st.doc.lineAt(Math.max(0, Math.min(st.doc.length, cursorPos)));
  const m = /^\s*(?:(?:const|let|var|final\s+var)\s+)?([A-Za-z_]\w*)\s*(?::=|=)\s*(.+?);?\s*$/.exec(ln.text);
  if (!m) return null;
  const [, name, rawRhs] = m;
  const rhs = rawRhs.trim();
  if (!rhs || CONTROL_WORDS.has(name)) return null;
  const wrapped = /^[A-Za-z0-9_."'`()[\]]+$/.test(rhs) ? rhs : `(${rhs})`;
  const wordRe = new RegExp(`\\b${name}\\b`, 'g');
  const changes: { from: number; to: number; insert: string }[] = [];
  const delTo = ln.number < st.doc.lines ? st.doc.line(ln.number + 1).from : ln.to;
  changes.push({ from: ln.from, to: delTo, insert: '' });
  const maxLine = Math.min(st.doc.lines, ln.number + 80);
  let replaced = 0;
  for (let l = ln.number + 1; l <= maxLine; l++) {
    const cur = st.doc.line(l);
    if (/^}\s*$/.test(cur.text)) break;
    let hit: RegExpExecArray | null;
    wordRe.lastIndex = 0;
    while ((hit = wordRe.exec(cur.text)) !== null) {
      changes.push({ from: cur.from + hit.index, to: cur.from + hit.index + name.length, insert: wrapped });
      replaced += 1;
    }
  }
  return replaced > 0 ? { changes } : null;
}

export function computeExtractMethodEdit(
  docText: string,
  selFrom: number,
  selTo: number,
  language?: string,
): { changes: { from: number; to: number; insert: string }[] } | null {
  if (selTo <= selFrom) return null;
  const st = EditorState.create({ doc: docText });
  const startLine = st.doc.lineAt(selFrom);
  const endLine = st.doc.lineAt(Math.max(selFrom, selTo - 1));
  const bodyBlock = st.sliceDoc(startLine.from, endLine.to);
  if (!bodyBlock.trim()) return null;
  const indent = /^(\s*)/.exec(startLine.text)?.[1] ?? '\t';
  const lang = (language ?? '').toLowerCase();
  let insertLine = endLine.number;
  for (let l = endLine.number + 1; l <= Math.min(st.doc.lines, endLine.number + 150); l++) {
    const t = st.doc.line(l).text;
    if (/^}\s*$/.test(t)) { insertLine = l; break; }
  }
  const anchorPos = st.doc.line(insertLine).to;
  let callStmt = `${indent}extractedHelper()`;
  let fnBlock = `\n\nfunc extractedHelper() {\n${bodyBlock}\n}`;
  if (lang === 'rust') {
    callStmt = `${indent}extracted_helper();`;
    fnBlock = `\n\nfn extracted_helper() {\n${bodyBlock}\n}`;
  } else if (lang === 'python') {
    callStmt = `${indent}extracted_helper()`;
    fnBlock = `\n\ndef extracted_helper():\n${bodyBlock}`;
  } else if (lang === 'java') {
    callStmt = `${indent}extractedHelper();`;
    fnBlock = `\n\n    private void extractedHelper() {\n${bodyBlock}\n    }`;
  } else if (lang !== 'go') {
    callStmt = `${indent}extractedHelper();`;
    fnBlock = `\n\nfunction extractedHelper() {\n${bodyBlock}\n}`;
  }
  return {
    changes: [
      { from: startLine.from, to: endLine.to, insert: callStmt },
      { from: anchorPos, to: anchorPos, insert: fnBlock },
    ],
  };
}

export function computeExtractInterfaceEdit(
  docText: string,
  cursorPos: number,
  language?: string,
): { insertPos: number; insertText: string } | null {
  const st = EditorState.create({ doc: docText });
  const lang = (language ?? '').toLowerCase();
  if (lang === 'go' || docText.includes('package ')) {
    const curLn = st.doc.lineAt(Math.max(0, Math.min(st.doc.length, cursorPos)));
    let structName = '';
    let structLineFrom = curLn.from;
    for (let l = curLn.number; l >= 1; l--) {
      const m = /^\s*type\s+([A-Za-z_]\w*)\s+struct\b/.exec(st.doc.line(l).text);
      if (m) { structName = m[1]; structLineFrom = st.doc.line(l).from; break; }
    }
    if (!structName) {
      const first = /^\s*type\s+([A-Za-z_]\w*)\s+struct\b/m.exec(docText);
      if (!first) return null;
      structName = first[1];
      structLineFrom = first.index;
    }
    const methodRe = new RegExp(`^\\s*func\\s*\\(\\s*(?:[A-Za-z_]\\w*\\s+)?\\*?${structName}\\s*\\)\\s*([A-Z]\\w*\\([^)]*\\)(?:\\s*(?:\\([^)]*\\)|[^{]+))?)\\s*\\{`, 'gm');
    const sigs: string[] = [];
    let m: RegExpExecArray | null;
    while ((m = methodRe.exec(docText)) !== null) sigs.push(`\t${m[1].trim()}`);
    const body = sigs.length ? `${sigs.join('\n')}\n` : '\t// Add interface methods\n';
    return {
      insertPos: structLineFrom,
      insertText: `type ${structName}Contract interface {\n${body}}\n\n`,
    };
  }
  const curLn = st.doc.lineAt(Math.max(0, Math.min(st.doc.length, cursorPos)));
  const cls = /\bclass\s+([A-Za-z_]\w*)/.exec(curLn.text) ?? /\bclass\s+([A-Za-z_]\w*)/m.exec(docText);
  if (!cls) return null;
  const name = cls[1];
  const pos = docText.indexOf(cls[0]);
  return {
    insertPos: Math.max(0, pos),
    insertText: `export interface ${name}Contract {\n}\n\n`,
  };
}

export function computeIntroduceTypeEdit(
  docText: string,
  selFrom: number,
  selTo: number,
  language?: string,
): { changes: { from: number; to: number; insert: string }[] } | null {
  if (selTo <= selFrom) return null;
  const st = EditorState.create({ doc: docText });
  const typeExpr = st.sliceDoc(selFrom, selTo).trim();
  if (!typeExpr || typeExpr.includes('\n')) return null;
  const lang = (language ?? '').toLowerCase();
  const aliasName = 'ExtractedType';
  let insertLine = st.doc.lineAt(selFrom).number;
  for (let l = insertLine; l >= 1; l--) {
    if (/^\S/.test(st.doc.line(l).text)) { insertLine = l; break; }
  }
  const declFrom = st.doc.line(insertLine).from;
  const aliasDecl = lang === 'go'
    ? `type ${aliasName} ${typeExpr}\n\n`
    : `type ${aliasName} = ${typeExpr};\n\n`;
  return {
    changes: [
      { from: declFrom, to: declFrom, insert: aliasDecl },
      { from: selFrom, to: selTo, insert: aliasName },
    ],
  };
}

export function computeGenerateCodeSnippet(
  docText: string,
  cursorPos: number,
  kind: 'constructor' | 'getters' | 'test_stub',
  wordHint = 'Feature',
  language?: string,
): { insertPos: number; insertText: string } | null {
  const st = EditorState.create({ doc: docText });
  const lang = (language ?? '').toLowerCase();
  if (kind === 'test_stub') {
    const w = wordHint || 'Feature';
    const cap = w[0].toUpperCase() + w.slice(1);
    const snippet = lang === 'go'
      ? `\n\nfunc Test${cap}(t *testing.T) {\n\t// Arrange & Act\n}\n`
      : lang === 'rust'
        ? `\n\n#[test]\nfn test_${w.toLowerCase()}() {\n    // Arrange & Act\n}\n`
        : `\n\ntest('${w}', () => {\n  // Arrange & Act\n});\n`;
    return { insertPos: st.doc.length, insertText: snippet };
  }
  if (lang === 'go' || docText.includes('package ')) {
    const curLn = st.doc.lineAt(Math.max(0, Math.min(st.doc.length, cursorPos)));
    let structLine = -1;
    let structName = '';
    for (let l = curLn.number; l >= 1; l--) {
      const m = /^\s*type\s+([A-Za-z_]\w*)\s+struct\s*\{/.exec(st.doc.line(l).text);
      if (m) { structLine = l; structName = m[1]; break; }
    }
    if (structLine < 0) return null;
    const fields: { name: string; typ: string }[] = [];
    let endStructLine = structLine;
    for (let l = structLine + 1; l <= st.doc.lines; l++) {
      const txt = st.doc.line(l).text;
      if (/^\s*\}/.test(txt)) { endStructLine = l; break; }
      const fm = GO_FIELD_RE.exec(txt);
      if (fm) fields.push({ name: fm[2], typ: fm[4].split('`')[0].split('//')[0].trim() });
    }
    const insertPos = st.doc.line(endStructLine).to;
    const recv = structName[0].toLowerCase();
    if (kind === 'constructor') {
      const params = fields.map((f) => `${f.name[0].toLowerCase() + f.name.slice(1)} ${f.typ}`).join(', ');
      const inits = fields.map((f) => `\t\t${f.name}: ${f.name[0].toLowerCase() + f.name.slice(1)},`).join('\n');
      const body = inits ? `\n${inits}\n\t` : '';
      return {
        insertPos,
        insertText: `\n\nfunc New${structName}(${params}) *${structName} {\n\treturn &${structName}{${body}}\n}`,
      };
    }
    if (!fields.length) return null;
    const getters = fields.map((f) => {
      const pub = f.name[0].toUpperCase() + f.name.slice(1);
      return `func (${recv} *${structName}) Get${pub}() ${f.typ} {\n\treturn ${recv}.${f.name}\n}`;
    }).join('\n\n');
    return { insertPos, insertText: `\n\n${getters}` };
  }
  return null;
}
