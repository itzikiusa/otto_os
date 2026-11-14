// Compact JetBrains-style Quick-Doc Hover (`[data-testid="ide-quick-doc-hover"]`):
// Queries `POST /ide/lsp/hover` -> pooled LSP fallback -> clean local AST fallback,
// strips fenced backticks / `---` / domainless `pkg.go.dev` links / sibling block comments,
// syntax-colors signatures, caps summary <= 240 chars, and wires Definition (⌘B) & Usages (⌥F7).

import type { Extension } from '@codemirror/state';
import { EditorView, hoverTooltip } from '@codemirror/view';
import { applyConcreteEditsToView, computeDeterministicQuickFixes } from './cm-ide-quickfix.ts';
import { applyClientQuickFix, classifySemanticTokens, detectSemanticLang, getIdeQuickFixesAtLine, type SemanticLangKind } from './cm-ide-semantic.ts';
import { readIdeEditorPreferences, type IdeEditorPreferences } from './cm-ide-theme.ts';

export interface ParsedQuickDoc {
  symbol: string; kind: string; packageName: string | null; signature: string; summary: string;
  moreParagraphs: string[]; externalLinks: Array<{ label: string; url: string }>; definedIn: { path: string; line: number } | null;
}

const FENCED_BLOCK_RE = /```([\w+-]*)\n([\s\S]*?)```/g;
const PKG_GO_DEV_LINK_RE = /\[([^\]]+)\]\((https?:\/\/pkg\.go\.dev\/([^)#\s]+)(?:#[^)\s]*)?)\)/g;

function isValidExternalPkgGoDevModule(modPath: string): boolean {
  const firstSeg = modPath.replace(/^\/+/, '').split('/')[0] ?? '';
  return firstSeg.includes('.');
}

function isolateSymbolFromGoGroupBlock(code: string, targetSymbol: string): { sig: string; attachedDoc: string } | null {
  if (!targetSymbol || !/^(?:var|const)\s*\(/m.test(code)) return null;
  const kwMatch = /^(var|const)\s*\(/m.exec(code);
  const kw = kwMatch ? kwMatch[1] : 'var';
  const lines = code.split(/\r?\n/);
  const symRe = new RegExp(`^\\s*${targetSymbol.replace(/\$/g, '\\$')}\\b`);
  const idx = lines.findIndex((l) => !l.trim().startsWith('//') && symRe.test(l));
  if (idx < 0) return null;
  const docLines: string[] = [];
  for (let j = idx - 1; j >= 0; j--) {
    const t = lines[j].trim();
    if (t.startsWith('//')) docLines.unshift(t.replace(/^\/\/\s?/, ''));
    else break;
  }
  return { sig: `${kw} ${lines[idx].trim()}`, attachedDoc: docLines.join(' ').trim() };
}

export function sanitizeAndSplitHoverDoc(input: {
  symbol: string;
  kind?: string | null;
  packageName?: string | null;
  signature?: string | null;
  rawDoc?: string | null;
  fullDoc?: string | null;
  definedIn?: { path: string; line: number } | null;
  compact?: boolean;
}): ParsedQuickDoc | null {
  let sig = (input.signature ?? '').trim();
  const combinedRaw = [input.fullDoc ?? '', input.rawDoc ?? ''].find((s) => s.trim().length > 0) ?? '';
  let prose = combinedRaw;
  const externalLinks: Array<{ label: string; url: string }> = [];

  // 1. Extract fenced ```lang ... ``` code blocks so raw triple backticks NEVER render as text.
  const fencedBlocks: string[] = [];
  prose = prose.replace(FENCED_BLOCK_RE, (_all, _lang: string, body: string) => {
    const cleanBody = body.trim();
    if (cleanBody) fencedBlocks.push(cleanBody);
    return '\n\n';
  });
  if (!sig && fencedBlocks.length > 0) sig = fencedBlocks[0];
  sig = sig.replace(/^```[\w+-]*\s*|\s*```$/g, '').trim();

  // 2. If signature dumped an entire Go `var ( ... )` group, isolate ONLY the hovered symbol!
  let groupDoc = '';
  const isolated = isolateSymbolFromGoGroupBlock(sig, input.symbol);
  if (isolated) {
    sig = isolated.sig;
    groupDoc = isolated.attachedDoc;
  }

  // 3. Strip domainless local `pkg.go.dev/<local-mod>` links; keep valid external domain links.
  prose = prose.replace(PKG_GO_DEV_LINK_RE, (_all, label: string, url: string, modPath: string) => {
    if (isValidExternalPkgGoDevModule(modPath)) {
      const cleanLabel = label.replace(/`/g, '').trim();
      externalLinks.push({ label: cleanLabel, url });
    }
    return '';
  });

  // 4. Clean horizontal rules (`---`, `***`), HTML tags, and excessive blank lines.
  const cleanedLines = prose
    .split(/\r?\n/)
    .map((ln) => (/^\s*(?:---+|\*\*\*+|___+)\s*$/.test(ln) ? '' : ln.replace(/^\s*\*\s?/, (m) => m)))
    .join('\n');

  const rawParagraphs = (groupDoc && !cleanedLines.trim() ? [groupDoc] : cleanedLines.split(/\n\s*\n/))
    .map((p) => p.replace(/\s+/g, ' ').trim())
    .filter((p) => p.length > 0 && p !== sig);

  let summary = rawParagraphs[0] ?? '';
  const moreParagraphs = rawParagraphs.slice(1);

  // Cap summary to 2-3 concise sentences (<= 240 chars) in compact mode; push overflow to `moreParagraphs`.
  if (input.compact !== false && summary.length > 240) {
    const sentences = summary.match(/[^.!?]+[.!?]+(?:\s+|$)|[^.!?]+$/g) ?? [summary];
    let acc = '';
    let restStart = 0;
    for (let i = 0; i < sentences.length; i++) {
      if ((acc + sentences[i]).trim().length <= 240 && i < 3) {
        acc += sentences[i];
        restStart = i + 1;
      } else break;
    }
    if (acc.trim()) {
      const rest = sentences.slice(restStart).join('').trim();
      summary = acc.trim();
      if (rest) moreParagraphs.unshift(rest);
    } else {
      moreParagraphs.unshift(summary);
      summary = `${summary.slice(0, 237).trimEnd()}…`;
    }
  }

  if (!sig && !summary) return null;
  return {
    symbol: input.symbol,
    kind: input.kind || inferKindFromSignature(sig),
    packageName: input.packageName ?? null,
    signature: sig || input.symbol,
    summary,
    moreParagraphs,
    externalLinks,
    definedIn: input.definedIn ?? null,
  };
}

function inferKindFromSignature(sig: string): string {
  if (/(?:^|\b)(?:func|fn|def|function)\b/.test(sig)) return 'function';
  if (/(?:^|\b)(?:type|struct|class|interface|enum|record|trait)\b/.test(sig)) return 'type';
  if (/^const\b|\bstatic\s+final\b/.test(sig)) return 'constant';
  if (/^(?:export\s+)?(?:var|let)\b/.test(sig)) return 'variable';
  return 'symbol';
}

export function extractCleanSymbolHoverFromSource(
  docText: string,
  symbol: string,
  lang: SemanticLangKind,
  compact = true,
): ParsedQuickDoc | null {
  if (!docText || !symbol) return null;
  const lines = docText.split(/\r?\n/);
  const esc = symbol.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  let pkgName: string | null = null;
  let inGoGroup: 'var' | 'const' | null = null;
  let declIdx = -1;
  let sig = '';
  let kind = 'symbol';

  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const trimmed = raw.trim();
    if (!pkgName) {
      const mPkg = /^(?:package\s+([\w.]+)\s*;?)/.exec(trimmed);
      if (mPkg) pkgName = mPkg[1];
    }
    if (trimmed.startsWith('//') || trimmed.startsWith('#') || trimmed.startsWith('/*') || trimmed.startsWith('*')) continue;

    if (lang === 'go') {
      const mGrp = /^(var|const)\s*\(\s*$/.exec(trimmed);
      if (mGrp) { inGoGroup = mGrp[1] as 'var' | 'const'; continue; }
      if (inGoGroup && trimmed.startsWith(')')) { inGoGroup = null; continue; }
      if (inGoGroup) {
        if (new RegExp(`^${esc}\\b`).test(trimmed)) {
          declIdx = i;
          sig = `${inGoGroup} ${trimmed.replace(/\s*\/\/.*$/, '')}`;
          kind = inGoGroup === 'const' ? 'constant' : 'variable';
          break;
        }
        continue;
      }
      const mFn = new RegExp(`^func\\s+(?:\\([^)]+\\)\\s*)?${esc}\\s*\\(`).exec(trimmed);
      if (mFn) { declIdx = i; sig = trimmed.replace(/\s*\{\s*$/, ''); kind = 'function'; break; }
      const mTop = new RegExp(`^(var|const|type)\\s+${esc}\\b`).exec(trimmed);
      if (mTop) {
        declIdx = i;
        sig = trimmed.replace(/\s*\{\s*$/, '').replace(/\s*\/\/.*$/, '');
        kind = mTop[1] === 'type' ? 'type' : mTop[1] === 'const' ? 'constant' : 'variable';
        break;
      }
    } else if (lang === 'java') {
      if (new RegExp(`\\b(?:class|interface|enum|record)\\s+${esc}\\b`).test(trimmed)) {
        declIdx = i; sig = trimmed.replace(/\s*\{\s*$/, ''); kind = 'type'; break;
      }
      if (new RegExp(`^(?:public|protected|private|static|final|synchronized|default|abstract|\\s)+(?:<[^>{};]+>\\s+)?(?:[\\w.$?[\\]]+(?:<[^>{};]+>)?(?:\\[\\])*)\\s+${esc}\\s*\\(`).test(trimmed)) {
        declIdx = i; sig = trimmed.replace(/\s*\{\s*$/, ''); kind = 'method'; break;
      }
      if (new RegExp(`^(?:private|protected|public)\\s+(?:static\\s+|final\\s+)*(?:[\\w.$?[\\]]+(?:<[^>{};]+>)?(?:\\[\\])*)\\s+${esc}\\s*[;=]`).test(trimmed)) {
        declIdx = i; sig = trimmed.replace(/;\s*$/, ''); kind = 'field'; break;
      }
    } else {
      const mGen = new RegExp(
        `^(?:export\\s+)?(?:pub\\s+)?(?:async\\s+)?(?:function|fn|def|class|interface|type|struct|enum|trait|const|let)\\s+${esc}\\b`,
      ).exec(trimmed);
      if (mGen) { declIdx = i; sig = trimmed.replace(/\s*[:{]\s*$/, ''); kind = inferKindFromSignature(sig); break; }
    }
  }

  if (declIdx < 0) return null;

  // Strictly collect ONLY contiguous doc comments IMMEDIATELY above `declIdx`; stop at any blank or code line!
  const docLines: string[] = [];
  let j = declIdx - 1;
  // Skip annotations directly attached to Java/Python/TS/Rust declarations (@Override, #[derive]).
  while (j >= 0 && /^(?:@\w+|#\[)/.test(lines[j].trim())) j--;

  if (j >= 0 && lines[j].trim().endsWith('*/')) {
    while (j >= 0) {
      const t = lines[j].trim();
      docLines.unshift(t.replace(/^\/\*\*?/, '').replace(/\*\/$/, '').replace(/^\*\s?/, '').trim());
      if (t.startsWith('/*')) break;
      j--;
    }
  } else {
    while (j >= 0) {
      const t = lines[j].trim();
      if (!t) break; // Blank line boundary — NEVER scoop unrelated comments above!
      if (t.startsWith('//')) docLines.unshift(t.replace(/^\/\/\/?\s?/, ''));
      else if ((lang === 'python' || lang === 'shell') && t.startsWith('#') && !t.startsWith('#!')) docLines.unshift(t.replace(/^#\s?/, ''));
      else break;
      j--;
    }
  }

  return sanitizeAndSplitHoverDoc({
    symbol,
    kind,
    packageName: pkgName,
    signature: sig,
    rawDoc: docLines.filter(Boolean).join('\n'),
    compact,
  });
}

function appendInlineMarkdown(parent: HTMLElement, text: string): void {
  const tokenRe = /`([^`]+)`|\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = tokenRe.exec(text)) !== null) {
    if (m.index > last) parent.appendChild(document.createTextNode(text.slice(last, m.index)));
    if (m[1] != null) {
      const code = document.createElement('code');
      code.className = 'cm-ide-qdoc-inline-code';
      code.textContent = m[1];
      parent.appendChild(code);
    } else if (m[2] && m[3]) {
      const a = document.createElement('a');
      a.className = 'cm-ide-qdoc-link';
      a.href = m[3];
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
      a.textContent = m[2].replace(/`/g, '');
      parent.appendChild(a);
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) parent.appendChild(document.createTextNode(text.slice(last)));
}

function renderColoredSignature(sig: string, lang: SemanticLangKind): HTMLElement {
  const pre = document.createElement('pre');
  pre.className = 'cm-ide-qdoc-sig';
  pre.dataset.testid = 'ide-quick-doc-signature';
  const spans = classifySemanticTokens(sig, lang === 'other' ? 'go' : lang);
  let cursor = 0;
  for (const sp of spans) {
    if (sp.from < cursor) continue;
    if (sp.from > cursor) pre.appendChild(document.createTextNode(sig.slice(cursor, sp.from)));
    const el = document.createElement('span');
    el.className = `cm-syn-${sp.role}`;
    el.textContent = sig.slice(sp.from, sp.to);
    pre.appendChild(el);
    cursor = sp.to;
  }
  if (cursor < sig.length) pre.appendChild(document.createTextNode(sig.slice(cursor)));
  return pre;
}

const quickDocTheme = EditorView.baseTheme({
  '.cm-tooltip.cm-ide-qdoc-wrap, .cm-ide-qdoc-card, .cm-ide-quick-doc': {
    maxWidth: 'min(520px, calc(100vw - 24px))', backgroundColor: 'var(--surface)', color: 'var(--text)',
    border: '1px solid var(--border-strong)', borderRadius: 'var(--radius-m)', boxShadow: 'var(--shadow-card)',
    padding: '0', fontFamily: 'var(--font-ui)', fontSize: 'var(--fs-xs)', lineHeight: '1.45', overflow: 'hidden',
  },
  '.cm-ide-qdoc-head': { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 'var(--sp-3)', padding: 'var(--sp-2) var(--sp-3)', backgroundColor: 'var(--surface-2)', borderBlockEnd: '1px solid var(--separator)' },
  '.cm-ide-qdoc-pkg': { fontFamily: 'var(--font-mono)', color: 'var(--text-dim)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  '.cm-ide-qdoc-kind': { padding: '0 6px', borderRadius: 'var(--radius-s)', backgroundColor: 'var(--accent-soft)', color: 'var(--accent-text)', fontWeight: '600', flexShrink: '0' },
  '.cm-ide-qdoc-sig, .cm-ide-qd-sig': { margin: '0', padding: 'var(--sp-2) var(--sp-3)', backgroundColor: 'var(--syn-bg)', color: 'var(--syn-fg)', fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-xs)', whiteSpace: 'pre-wrap', wordBreak: 'break-word', borderBlockEnd: '1px solid var(--separator)' },
  '.cm-ide-qdoc-body, .cm-ide-qd-body': { padding: 'var(--sp-2) var(--sp-3)', maxHeight: '160px', overflowY: 'auto', whiteSpace: 'pre-line' },
  '.cm-ide-qdoc-body.is-expanded': { maxHeight: '280px' },
  '.cm-ide-qdoc-p': { margin: '0 0 var(--sp-2) 0', color: 'var(--text)' },
  '.cm-ide-qdoc-p:last-child': { marginBlockEnd: '0' },
  '.cm-ide-qdoc-inline-code': { fontFamily: 'var(--font-mono)', padding: '0 4px', borderRadius: 'var(--radius-s)', backgroundColor: 'var(--surface-2)' },
  '.cm-ide-qdoc-link, .cm-ide-qdoc-act, .cm-ide-qd-btn': { appearance: 'none', background: 'transparent', border: 'none', padding: '0', color: 'var(--accent-text)', fontSize: 'var(--fs-xs)', fontFamily: 'var(--font-ui)', cursor: 'pointer', textDecoration: 'none' },
  '.cm-ide-qdoc-link:hover, .cm-ide-qdoc-act:hover, .cm-ide-qd-btn:hover': { textDecoration: 'underline' },
  '.cm-ide-qdoc-qf': { fontWeight: '600', color: 'var(--accent-text)' },
  '.cm-ide-qdoc-foot, .cm-ide-qd-footer': { display: 'flex', alignItems: 'center', gap: 'var(--sp-4)', flexWrap: 'wrap', padding: 'var(--sp-2) var(--sp-3)', backgroundColor: 'var(--surface-2)', borderBlockStart: '1px solid var(--separator)' },
});

export interface CreateIdeHoverOptions {
  filePath: string; language?: string; root?: string; connectionId?: string | null; projectFolders?: string[];
  getPreferences?: () => Pick<IdeEditorPreferences, 'compactHover'>;
  onDefinitionJump?: ((target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number }) => void) | null;
  onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onQuickFix?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
}

function isDepManifestPath(filePath: string): boolean {
  const b = filePath.split('?')[0].split('/').pop()?.toLowerCase() ?? '';
  return b === 'go.mod' || b === 'pom.xml' || b.startsWith('build.gradle') || b === 'package.json' || b === 'cargo.toml' || b === 'requirements.txt' || b === 'pyproject.toml';
}

export function createIdeHoverExtension(opts: CreateIdeHoverOptions): Extension {
  const lang = detectSemanticLang(opts.filePath, opts.language);
  const tip = hoverTooltip(async (view, pos) => {
    if (isDepManifestPath(opts.filePath)) return null;
    const isCompact = (opts.getPreferences ? opts.getPreferences() : readIdeEditorPreferences()).compactHover !== false;
    const { queryPooledLspHoverMarkdown, tokenSpanAtPos } = await import('./cm-ide-lsp.ts');
    const ln = view.state.doc.lineAt(pos), lineNo = ln.number, char0 = pos - ln.from, docText = view.state.doc.toString();
    const span = tokenSpanAtPos(view.state, pos);
    const word = span && !span.isStringPath && span.word && !/^\d+$/.test(span.word) ? span.word : '';
    const lineFixes = getIdeQuickFixesAtLine(view.state, lineNo);
    const detFixes = computeDeterministicQuickFixes(docText, opts.filePath, opts.language, lineNo, word || undefined);
    const primaryLineFix = lineFixes[0] ?? null;
    const primaryDetFix = detFixes[0] ?? null;

    let parsed: ParsedQuickDoc | null = null;
    if (word && opts.root) {
      try {
        const { api } = await import('../api/client.ts');
        const resp = await api.post<{ found?: boolean; symbol?: string; kind?: string; package_name?: string | null; signature?: string; summary?: string; full_doc?: string | null; defined_in?: { path: string; line: number } | null }>('/ide/lsp/hover', {
          target: { kind: opts.connectionId ? 'ssh' : 'local', root: opts.root, connection_id: opts.connectionId ?? null, additional_roots: opts.projectFolders },
          path: opts.filePath, content: docText, line: lineNo, character: char0, column: char0 + 1, symbol: word, project_folders: opts.projectFolders,
        });
        if (resp?.found && (resp.signature || resp.summary || resp.full_doc)) {
          parsed = sanitizeAndSplitHoverDoc({ symbol: resp.symbol || word, kind: resp.kind, packageName: resp.package_name, signature: resp.signature, rawDoc: resp.summary, fullDoc: resp.full_doc, definedIn: resp.defined_in, compact: isCompact });
        }
      } catch { /* fall through */ }
    }
    if (!parsed && word) {
      const pooledMd = await queryPooledLspHoverMarkdown(view, pos);
      if (pooledMd) parsed = sanitizeAndSplitHoverDoc({ symbol: word, rawDoc: pooledMd, compact: isCompact });
    }
    if (!parsed && word) parsed = extractCleanSymbolHoverFromSource(docText, word, lang, isCompact);
    if (!parsed && (primaryLineFix || primaryDetFix)) {
      parsed = { symbol: word || `Line ${lineNo}`, kind: 'inspection', packageName: null, signature: ln.text.trim(), summary: `Quick Fix available: ${(primaryLineFix ?? primaryDetFix)!.title}`, moreParagraphs: [], externalLinks: [], definedIn: null };
    }
    if (!parsed) return null;

    const doc = parsed;
    return {
      pos: span ? span.from : ln.from, end: span ? span.to : ln.to, above: true,
      create() {
        const dom = document.createElement('div'); dom.className = 'cm-ide-qdoc-card'; dom.dataset.testid = 'ide-quick-doc-hover';
        const head = document.createElement('div'); head.className = 'cm-ide-qdoc-head';
        const pkgEl = document.createElement('span'); pkgEl.className = 'cm-ide-qdoc-pkg'; pkgEl.textContent = doc.packageName ? `package ${doc.packageName}` : doc.symbol;
        const kindEl = document.createElement('span'); kindEl.className = 'cm-ide-qdoc-kind'; kindEl.textContent = doc.kind;
        head.append(pkgEl, kindEl); dom.appendChild(head);
        dom.appendChild(renderColoredSignature(doc.signature, lang));

        if (doc.summary || doc.moreParagraphs.length > 0) {
          const body = document.createElement('div'); body.className = `cm-ide-qdoc-body${!isCompact ? ' is-expanded' : ''}`;
          if (doc.summary) {
            const p = document.createElement('p'); p.className = 'cm-ide-qdoc-p'; p.dataset.testid = 'ide-quick-doc-summary';
            appendInlineMarkdown(p, doc.summary); body.appendChild(p);
          }
          if (doc.moreParagraphs.length > 0) {
            const extraWrap = document.createElement('div'); extraWrap.hidden = isCompact;
            for (const mp of doc.moreParagraphs) { const p = document.createElement('p'); p.className = 'cm-ide-qdoc-p'; appendInlineMarkdown(p, mp); extraWrap.appendChild(p); }
            const moreBtn = document.createElement('button'); moreBtn.type = 'button'; moreBtn.className = 'cm-ide-qdoc-act'; moreBtn.dataset.testid = 'ide-hover-expand-btn'; moreBtn.textContent = isCompact ? 'Show full documentation' : 'Show less';
            moreBtn.addEventListener('click', (e) => { e.preventDefault(); e.stopPropagation(); extraWrap.hidden = !extraWrap.hidden; body.classList.toggle('is-expanded', !extraWrap.hidden); moreBtn.textContent = extraWrap.hidden ? 'Show full documentation' : 'Show less'; });
            body.append(extraWrap, moreBtn);
          }
          dom.appendChild(body);
        }

        const foot = document.createElement('div'); foot.className = 'cm-ide-qdoc-foot';
        if (primaryLineFix || primaryDetFix) {
          const qfBtn = document.createElement('button'); qfBtn.type = 'button'; qfBtn.className = 'cm-ide-qdoc-act cm-ide-qdoc-qf'; qfBtn.dataset.testid = 'ide-hover-quickfix-btn';
          const title = (primaryLineFix ?? primaryDetFix)!.title;
          qfBtn.textContent = `💡 Quick Fix: ${title} (⌥⏎)`;
          qfBtn.addEventListener('mousedown', (e) => {
            e.preventDefault(); e.stopPropagation();
            if (primaryLineFix) applyClientQuickFix(view, primaryLineFix);
            else if (primaryDetFix) applyConcreteEditsToView(view, primaryDetFix.edits, opts.filePath);
          });
          foot.appendChild(qfBtn);
          if (opts.onQuickFix) {
            const moreQf = document.createElement('button'); moreQf.type = 'button'; moreQf.className = 'cm-ide-qdoc-act'; moreQf.textContent = 'More actions... (⌥⏎)';
            moreQf.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); opts.onQuickFix?.({ line: lineNo, column: char0 + 1, symbol: word || undefined }); });
            foot.appendChild(moreQf);
          }
        }
        if (opts.onDefinitionJump && doc.kind !== 'inspection') {
          const defBtn = document.createElement('button'); defBtn.type = 'button'; defBtn.className = 'cm-ide-qdoc-act'; defBtn.dataset.testid = 'ide-hover-def-btn'; defBtn.textContent = 'Definition (⌘B)';
          defBtn.addEventListener('click', (e) => { e.preventDefault(); e.stopPropagation(); opts.onDefinitionJump?.(doc.definedIn ? { path: doc.definedIn.path, line: doc.definedIn.line, column: 1, symbol: doc.symbol, originLine: lineNo, originColumn: char0 + 1 } : { line: lineNo, column: char0 + 1, symbol: doc.symbol, originLine: lineNo, originColumn: char0 + 1 }); });
          foot.appendChild(defBtn);
        }
        if (opts.onFindUsages && doc.kind !== 'inspection') {
          const usgBtn = document.createElement('button'); usgBtn.type = 'button'; usgBtn.className = 'cm-ide-qdoc-act'; usgBtn.dataset.testid = 'ide-hover-usages-btn'; usgBtn.textContent = 'Find Usages (⌥F7)';
          usgBtn.addEventListener('click', (e) => { e.preventDefault(); e.stopPropagation(); opts.onFindUsages?.({ line: lineNo, column: char0 + 1, symbol: doc.symbol }); });
          foot.appendChild(usgBtn);
        }
        for (const ext of doc.externalLinks.slice(0, 1)) {
          const a = document.createElement('a'); a.className = 'cm-ide-qdoc-link'; a.href = ext.url; a.target = '_blank'; a.rel = 'noopener noreferrer'; a.textContent = 'pkg.go.dev ↗';
          foot.appendChild(a);
        }
        if (foot.childElementCount > 0) dom.appendChild(foot);
        return { dom };
      },
    };
  }, { hoverTime: 240 });

  return [quickDocTheme, tip];
}
