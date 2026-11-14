// Curated IDE Editor Color Schemes (JetBrains Islands Dark/Light, Darcula,
// VS Code Dark/Light Modern), CodeMirror 6 theme + Lezer HighlightStyle, and
// shared `'otto.ide.preferences.v1'` reader/writer.
// All color hexes live in `ui/src/lib/tokens.css`; this file uses 100% CSS vars.

import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import type { Extension } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { tags } from '@lezer/highlight';

export const IDE_PREFS_STORAGE_KEY = 'otto.ide.preferences.v1';
export const IDE_PREFS_EVENT = 'otto:ide-prefs-changed';

export type ConcreteIdeColorSchemeId =
  | 'jetbrains-islands-dark'
  | 'jetbrains-darcula'
  | 'jetbrains-islands-light'
  | 'vscode-dark-modern'
  | 'vscode-light-modern';

export type IdeColorSchemeId = 'auto' | ConcreteIdeColorSchemeId;

export interface IdeColorSchemeOption {
  id: IdeColorSchemeId;
  label: string;
  group: 'auto' | 'dark' | 'light';
}

export const IDE_COLOR_SCHEME_OPTIONS: readonly IdeColorSchemeOption[] = [
  { id: 'auto', label: 'Follow Otto Dark/Light', group: 'auto' },
  { id: 'jetbrains-islands-dark', label: 'JetBrains Islands Dark', group: 'dark' },
  { id: 'jetbrains-darcula', label: 'JetBrains Darcula', group: 'dark' },
  { id: 'vscode-dark-modern', label: 'VS Code Dark Modern', group: 'dark' },
  { id: 'jetbrains-islands-light', label: 'JetBrains Islands Light', group: 'light' },
  { id: 'vscode-light-modern', label: 'VS Code Light Modern', group: 'light' },
];

const VALID_SCHEMES = new Set<string>(IDE_COLOR_SCHEME_OPTIONS.map((o) => o.id));
const DARK_SCHEMES = new Set<ConcreteIdeColorSchemeId>([
  'jetbrains-islands-dark',
  'jetbrains-darcula',
  'vscode-dark-modern',
]);

export interface IdeEditorPreferences {
  colorScheme: IdeColorSchemeId;
  darkColorScheme: ConcreteIdeColorSchemeId;
  lightColorScheme: ConcreteIdeColorSchemeId;
  fontSize: number;
  lineHeight: number;
  tabSize: number;
  wordWrap: boolean;
  inlayHints: boolean;
  codeVision: boolean;
  rainbowBrackets: boolean;
  showBreadcrumbs: boolean;
  autoFoldImports: boolean;
  compactHover: boolean;
  javaUnresolvedImports: boolean;
  lombokSupport: boolean;
  unusedImports: boolean;
  formatOnSave: boolean;
  keymapPreset: 'jetbrains' | 'standard';
}

export const DEFAULT_IDE_PREFERENCES: IdeEditorPreferences = {
  colorScheme: 'auto',
  darkColorScheme: 'jetbrains-islands-dark',
  lightColorScheme: 'jetbrains-islands-light',
  fontSize: 13,
  lineHeight: 1.55,
  tabSize: 4,
  wordWrap: false,
  inlayHints: true,
  codeVision: true,
  rainbowBrackets: true,
  showBreadcrumbs: true,
  autoFoldImports: true,
  compactHover: true,
  javaUnresolvedImports: true,
  lombokSupport: true,
  unusedImports: true,
  formatOnSave: false,
  keymapPreset: 'jetbrains',
};

function asScheme(val: unknown, fallback: IdeColorSchemeId): IdeColorSchemeId {
  return typeof val === 'string' && VALID_SCHEMES.has(val) ? (val as IdeColorSchemeId) : fallback;
}

function asConcreteScheme(
  val: unknown,
  fallback: ConcreteIdeColorSchemeId,
): ConcreteIdeColorSchemeId {
  return typeof val === 'string' && val !== 'auto' && VALID_SCHEMES.has(val)
    ? (val as ConcreteIdeColorSchemeId)
    : fallback;
}

function asBool(val: unknown, fallback: boolean): boolean {
  return typeof val === 'boolean' ? val : fallback;
}

function asNum(val: unknown, min: number, max: number, fallback: number): number {
  const n = typeof val === 'number' ? val : Number(val);
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
}

export function readIdeEditorPreferences(): IdeEditorPreferences {
  try {
    if (typeof localStorage === 'undefined') return { ...DEFAULT_IDE_PREFERENCES };
    const raw = localStorage.getItem(IDE_PREFS_STORAGE_KEY);
    const legacyFmt = localStorage.getItem('otto.ide.formatOnSave') === '1';
    if (!raw) return { ...DEFAULT_IDE_PREFERENCES, formatOnSave: legacyFmt };
    const p = JSON.parse(raw) as Record<string, unknown>;
    const schemeRaw = p.colorScheme ?? p.editorScheme ?? p.scheme ?? p.theme;
    const keymapRaw = p.keymapPreset ?? p.keymap;
    return {
      colorScheme: asScheme(schemeRaw, DEFAULT_IDE_PREFERENCES.colorScheme),
      darkColorScheme: asConcreteScheme(
        p.preferredDarkScheme ?? p.darkColorScheme ?? p.darkScheme,
        DEFAULT_IDE_PREFERENCES.darkColorScheme,
      ),
      lightColorScheme: asConcreteScheme(
        p.preferredLightScheme ?? p.lightColorScheme ?? p.lightScheme,
        DEFAULT_IDE_PREFERENCES.lightColorScheme,
      ),
      fontSize: asNum(p.fontSize ?? p.editorFontSize, 11, 24, DEFAULT_IDE_PREFERENCES.fontSize),
      lineHeight: asNum(p.lineHeight ?? p.editorLineHeight, 1.2, 2.2, DEFAULT_IDE_PREFERENCES.lineHeight),
      tabSize: asNum(p.tabSize, 2, 8, DEFAULT_IDE_PREFERENCES.tabSize),
      wordWrap: asBool(p.wordWrap ?? p.softWrap, DEFAULT_IDE_PREFERENCES.wordWrap),
      inlayHints: asBool(p.inlayHints ?? p.showInlayHints ?? p.parameterHints, DEFAULT_IDE_PREFERENCES.inlayHints),
      codeVision: asBool(p.codeVision ?? p.showCodeVision, DEFAULT_IDE_PREFERENCES.codeVision),
      rainbowBrackets: asBool(p.rainbowBrackets, DEFAULT_IDE_PREFERENCES.rainbowBrackets),
      showBreadcrumbs: asBool(p.breadcrumbs ?? p.showBreadcrumbs, DEFAULT_IDE_PREFERENCES.showBreadcrumbs),
      autoFoldImports: asBool(p.autoFoldImports ?? p.foldImports, DEFAULT_IDE_PREFERENCES.autoFoldImports),
      compactHover: asBool(p.compactHover, DEFAULT_IDE_PREFERENCES.compactHover),
      javaUnresolvedImports: asBool(p.javaUnresolvedImports, DEFAULT_IDE_PREFERENCES.javaUnresolvedImports),
      lombokSupport: asBool(p.lombokSupport, DEFAULT_IDE_PREFERENCES.lombokSupport),
      unusedImports: asBool(p.unusedImports, DEFAULT_IDE_PREFERENCES.unusedImports),
      formatOnSave: asBool(p.formatOnSave, legacyFmt),
      keymapPreset: keymapRaw === 'standard' || keymapRaw === 'vscode' ? 'standard' : 'jetbrains',
    };
  } catch {
    return { ...DEFAULT_IDE_PREFERENCES };
  }
}

export function writeIdeEditorPreferences(patch: Partial<IdeEditorPreferences>): IdeEditorPreferences {
  const merged = { ...readIdeEditorPreferences(), ...patch };
  try {
    if (typeof localStorage !== 'undefined') {
      const existingRaw = localStorage.getItem(IDE_PREFS_STORAGE_KEY);
      const existing = existingRaw ? (JSON.parse(existingRaw) as Record<string, unknown>) : {};
      const next = {
        ...existing,
        ...merged,
        scheme: merged.colorScheme,
        editorScheme: merged.colorScheme,
        breadcrumbs: merged.showBreadcrumbs,
        showBreadcrumbs: merged.showBreadcrumbs,
        preferredDarkScheme: merged.darkColorScheme,
        darkColorScheme: merged.darkColorScheme,
        preferredLightScheme: merged.lightColorScheme,
        lightColorScheme: merged.lightColorScheme,
      };
      localStorage.setItem(IDE_PREFS_STORAGE_KEY, JSON.stringify(next));
      localStorage.setItem('otto.ide.formatOnSave', merged.formatOnSave ? '1' : '0');
    }
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(IDE_PREFS_EVENT, { detail: merged }));
    }
  } catch {
    /* ignore storage quota errors */
  }
  return merged;
}

export function resolveEffectiveIdeScheme(
  prefs: Pick<IdeEditorPreferences, 'colorScheme' | 'darkColorScheme' | 'lightColorScheme'>,
  appScheme: 'light' | 'dark',
): ConcreteIdeColorSchemeId {
  if (prefs.colorScheme !== 'auto') return prefs.colorScheme;
  return appScheme === 'light' ? prefs.lightColorScheme : prefs.darkColorScheme;
}

export function isDarkIdeScheme(scheme: ConcreteIdeColorSchemeId): boolean {
  return DARK_SCHEMES.has(scheme);
}

const ideHighlightStyle = HighlightStyle.define([
  {
    tag: [tags.keyword, tags.modifier, tags.self, tags.null, tags.bool, tags.atom, tags.standard(tags.typeName)],
    color: 'var(--syn-keyword)',
  },
  {
    tag: [tags.controlKeyword, tags.moduleKeyword, tags.operatorKeyword, tags.definitionKeyword],
    color: 'var(--syn-control-keyword)',
  },
  {
    tag: [
      tags.function(tags.definition(tags.variableName)),
      tags.function(tags.definition(tags.propertyName)),
      tags.definition(tags.function(tags.variableName)),
    ],
    color: 'var(--syn-func-decl)',
  },
  {
    tag: [tags.function(tags.variableName), tags.function(tags.propertyName)],
    color: 'var(--syn-func-call)',
  },
  {
    tag: [tags.propertyName, tags.definition(tags.propertyName), tags.special(tags.variableName)],
    color: 'var(--syn-field)',
  },
  {
    tag: [tags.typeName, tags.className, tags.namespace, tags.macroName, tags.labelName],
    color: 'var(--syn-type)',
  },
  {
    tag: [tags.meta, tags.annotation, tags.attributeName],
    color: 'var(--syn-annotation)',
  },
  {
    tag: [tags.string, tags.special(tags.string), tags.regexp, tags.character],
    color: 'var(--syn-string)',
  },
  { tag: tags.escape, color: 'var(--syn-escape)' },
  { tag: [tags.number, tags.integer, tags.float], color: 'var(--syn-number)' },
  {
    tag: [tags.comment, tags.lineComment, tags.blockComment],
    color: 'var(--syn-comment)',
    fontStyle: 'italic',
  },
  { tag: tags.docComment, color: 'var(--syn-doc-comment)', fontStyle: 'italic' },
  { tag: [tags.variableName, tags.definition(tags.variableName)], color: 'var(--syn-fg)' },
  {
    tag: [tags.punctuation, tags.paren, tags.brace, tags.squareBracket, tags.separator, tags.derefOperator],
    color: 'var(--syn-punct)',
  },
  {
    tag: [tags.operator, tags.compareOperator, tags.arithmeticOperator, tags.logicOperator, tags.bitwiseOperator],
    color: 'var(--syn-operator)',
  },
  { tag: tags.heading, color: 'var(--syn-func-decl)', fontWeight: '600' },
  { tag: tags.strong, fontWeight: '600' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.link, color: 'var(--accent-text)', textDecoration: 'underline' },
]);

function makeEditorSurfaceTheme(dark: boolean, ideMode: boolean): Extension {
  return EditorView.theme(
    {
      '&': {
        color: 'var(--syn-fg)',
        backgroundColor: ideMode || dark ? 'var(--syn-bg)' : 'transparent',
        height: '100%',
      },
      '.cm-scroller': {
        backgroundColor: ideMode ? 'var(--syn-bg)' : 'transparent',
        minHeight: '100%',
        fontFamily: 'var(--font-mono)',
      },
      '.cm-content': {
        caretColor: 'var(--syn-fg)',
        color: 'var(--syn-fg)',
        minHeight: '100%',
      },
      '.cm-cursor, .cm-dropCursor': {
        borderLeftColor: 'var(--syn-fg)',
        borderLeftWidth: '1.5px',
      },
      '.cm-selectionBackground, &.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, ::selection': {
        backgroundColor: 'var(--syn-selection)',
      },
      '.cm-activeLine': {
        backgroundColor: 'var(--syn-active-line)',
      },
      '.cm-gutters': {
        backgroundColor: ideMode ? 'var(--syn-gutter-bg)' : 'transparent',
        color: 'var(--text-dim)',
        border: 'none',
        borderInlineEnd: ideMode ? '1px solid var(--syn-gutter-border)' : 'none',
        minHeight: '100%',
      },
      '.cm-activeLineGutter': {
        backgroundColor: 'var(--syn-active-line)',
        color: 'var(--syn-fg)',
      },
      '.cm-matchingBracket, &.cm-focused .cm-matchingBracket': {
        backgroundColor: 'color-mix(in srgb, var(--accent-text) 22%, transparent)',
        outline: '1px solid var(--accent-line)',
        borderRadius: '2px',
      },
      '.cm-nonmatchingBracket, &.cm-focused .cm-nonmatchingBracket': {
        backgroundColor: 'var(--danger-soft)',
        color: 'var(--danger)',
      },
      '.cm-foldPlaceholder': {
        backgroundColor: 'var(--syn-inlay-bg)',
        color: 'var(--syn-inlay-fg)',
        border: '1px solid var(--syn-gutter-border)',
        borderRadius: 'var(--radius-s)',
        padding: '0 6px',
        margin: '0 3px',
      },
      // Semantic token overrides — match nested Lezer spans too so computed RGB is exact.
      '.cm-syn-fg, .cm-syn-fg span': { color: 'var(--syn-fg) !important' },
      '.cm-syn-keyword, .cm-syn-keyword span': { color: 'var(--syn-keyword) !important' },
      '.cm-syn-control, .cm-syn-control span': { color: 'var(--syn-control-keyword) !important' },
      '.cm-syn-func-decl, .cm-syn-func-decl span': { color: 'var(--syn-func-decl) !important' },
      '.cm-syn-func-call, .cm-syn-func-call span': { color: 'var(--syn-func-call) !important' },
      '.cm-syn-field, .cm-syn-field span': { color: 'var(--syn-field) !important' },
      '.cm-syn-type, .cm-syn-type span': { color: 'var(--syn-type) !important' },
      '.cm-syn-annotation, .cm-syn-annotation span': { color: 'var(--syn-annotation) !important' },
      '.cm-syn-param, .cm-syn-param span': { color: 'var(--syn-param) !important' },
      '.cm-syn-string, .cm-syn-string span': { color: 'var(--syn-string) !important' },
      '.cm-syn-number, .cm-syn-number span': { color: 'var(--syn-number) !important' },
      '.cm-syn-doc-comment, .cm-syn-doc-comment span': {
        color: 'var(--syn-doc-comment) !important',
        fontStyle: 'italic',
      },
      '.cm-syn-doc-tag, .cm-syn-doc-tag span': {
        color: 'var(--syn-doc-tag) !important',
        fontWeight: '600',
        textDecoration: 'underline',
        textUnderlineOffset: '2px',
      },
      '.cm-syn-doc-param, .cm-syn-doc-param span': {
        color: 'var(--syn-doc-param) !important',
        fontStyle: 'normal',
      },
      '.cm-syn-unresolved, .cm-syn-unresolved span': {
        color: 'var(--syn-unresolved) !important',
        textDecoration: 'underline wavy var(--syn-unresolved)',
        textUnderlineOffset: '2px',
      },
      // Rainbow Brackets (4-level IntelliJ nested bracket coloring).
      '.cm-syn-rb-1, .cm-syn-rb-1 span': { color: 'var(--syn-rb-1) !important' },
      '.cm-syn-rb-2, .cm-syn-rb-2 span': { color: 'var(--syn-rb-2) !important' },
      '.cm-syn-rb-3, .cm-syn-rb-3 span': { color: 'var(--syn-rb-3) !important' },
      '.cm-syn-rb-4, .cm-syn-rb-4 span': { color: 'var(--syn-rb-4) !important' },
      '.cm-syn-rb-unmatched, .cm-syn-rb-unmatched span': {
        color: 'var(--danger) !important',
        textDecoration: 'underline wavy var(--danger)',
      },
      '.cm-syn-rb-active': {
        backgroundColor: 'color-mix(in srgb, var(--accent-text) 20%, transparent)',
        outline: '1px solid var(--accent-line)',
        borderRadius: '2px',
      },
      // Inlay parameter hints, CodeVision, breakpoint line tint, and reveal pulse.
      '.cm-ide-inlay-hint': {
        display: 'inline-block',
        padding: '0 4px',
        marginInlineEnd: '3px',
        borderRadius: 'var(--radius-s)',
        backgroundColor: 'var(--syn-inlay-bg)',
        color: 'var(--syn-inlay-fg)',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-xs)',
        lineHeight: '1.25',
        verticalAlign: 'baseline',
        userSelect: 'none',
        pointerEvents: 'none',
      },
      '.cm-ide-codevision': {
        display: 'inline-block',
        marginInlineStart: 'var(--sp-4)',
        color: 'var(--syn-inlay-fg)',
        fontFamily: 'var(--font-ui)',
        fontSize: 'var(--fs-xs)',
        fontStyle: 'normal',
        userSelect: 'none',
        cursor: 'pointer',
      },
      '.cm-ide-bp-line': {
        backgroundColor: 'color-mix(in srgb, var(--danger) 9%, transparent)',
      },
      '.cm-ide-reveal-pulse': {
        backgroundColor: 'color-mix(in srgb, var(--accent-solid) 18%, transparent)',
      },
    },
    { dark },
  );
}

const highlightExt = syntaxHighlighting(ideHighlightStyle);
const themeExtCache = new Map<string, Extension>();

export function createIdeSyntaxTheme(opts: {
  dark: boolean;
  plain?: boolean;
  ideMode?: boolean;
}): Extension {
  const dark = Boolean(opts.dark);
  const ideMode = Boolean(opts.ideMode);
  const plain = Boolean(opts.plain);
  const key = `${dark ? 1 : 0}:${ideMode ? 1 : 0}:${plain ? 1 : 0}`;
  let cached = themeExtCache.get(key);
  if (!cached) {
    const surface = makeEditorSurfaceTheme(dark, ideMode);
    cached = plain ? surface : [surface, highlightExt];
    themeExtCache.set(key, cached);
  }
  return cached;
}
