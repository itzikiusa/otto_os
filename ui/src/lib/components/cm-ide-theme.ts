// Curated IDE Editor Color Schemes (JetBrains Islands Dark/Light, Darcula,
// VS Code Dark/Light Modern), CodeMirror 6 theme + Lezer HighlightStyle, and
// shared `'otto.ide.preferences.v1'` reader/writer.

import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import type { Extension } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { tags } from '@lezer/highlight';

import {
  DEFAULT_IDE_PREFERENCES,
  IDE_COLOR_SCHEME_OPTIONS,
  IDE_PREFS_EVENT,
  IDE_PREFS_STORAGE_KEY,
  isDarkIdeScheme,
  readIdePreferences,
  resolveEffectiveIdeScheme,
  writeIdePreferences,
  type ConcreteIdeColorSchemeId,
  type IdeColorSchemeId,
  type IdeColorSchemeOption,
  type IdeEditorPreferences,
  type IdeKeymapPreset,
  type IdePreferences,
} from './cm-ide-prefs.ts';

export {
  DEFAULT_IDE_PREFERENCES,
  IDE_COLOR_SCHEME_OPTIONS,
  IDE_PREFS_EVENT,
  IDE_PREFS_STORAGE_KEY,
  isDarkIdeScheme,
  resolveEffectiveIdeScheme,
  type ConcreteIdeColorSchemeId,
  type IdeColorSchemeId,
  type IdeColorSchemeOption,
  type IdeEditorPreferences,
  type IdeKeymapPreset,
  type IdePreferences,
};

export const readIdeEditorPreferences = readIdePreferences;
export const writeIdeEditorPreferences = writeIdePreferences;

export type IdeSyntaxPalette = Readonly<Record<string, string>>;

/** Authentic JetBrains GoLand / IntelliJ Islands Dark, Darcula, and Light syntax token palettes. */
export const JETBRAINS_THEME_PALETTES: Readonly<Record<
  'jetbrains-islands-dark' | 'jetbrains-darcula' | 'jetbrains-islands-light',
  IdeSyntaxPalette
>> = {
  'jetbrains-islands-dark': {
    '--syn-bg': '#191A1C',
    '--syn-gutter-bg': '#191A1C',
    '--syn-fg': '#BCBEC4',
    '--syn-keyword': '#CF8E6D',
    '--syn-control-keyword': '#CF8E6D',
    '--syn-func-decl': '#56A8F5',
    '--syn-func-call': '#57AAF7',
    '--syn-field': '#C77DBB',
    '--syn-type': '#6FAFBD',
    '--syn-annotation': '#BBB529',
    '--syn-string': '#6AAB73',
    '--syn-escape': '#CF8E6D',
    '--syn-number': '#2AACB8',
    '--syn-param': '#E5C07B',
    '--syn-unused': '#7A7E85',
    '--syn-comment': '#7A7E85',
    '--syn-doc-comment': '#5F826B',
  },
  'jetbrains-darcula': {
    '--syn-bg': '#2B2B2B',
    '--syn-gutter-bg': '#313335',
    '--syn-fg': '#BCBEC4',
    '--syn-keyword': '#CF8E6D',
    '--syn-control-keyword': '#CF8E6D',
    '--syn-func-decl': '#56A8F5',
    '--syn-func-call': '#57AAF7',
    '--syn-field': '#C77DBB',
    '--syn-type': '#6FAFBD',
    '--syn-annotation': '#BBB529',
    '--syn-string': '#6AAB73',
    '--syn-escape': '#CF8E6D',
    '--syn-number': '#2AACB8',
    '--syn-param': '#E5C07B',
    '--syn-unused': '#7A7E85',
    '--syn-comment': '#7A7E85',
    '--syn-doc-comment': '#5F826B',
  },
  'jetbrains-islands-light': {
    '--syn-bg': '#FFFFFF',
    '--syn-gutter-bg': '#F7F8FA',
    '--syn-fg': '#080808',
    '--syn-keyword': '#0033B3',
    '--syn-control-keyword': '#0033B3',
    '--syn-func-decl': '#00627A',
    '--syn-func-call': '#00627A',
    '--syn-field': '#871094',
    '--syn-type': '#007E8A',
    '--syn-annotation': '#9E880D',
    '--syn-string': '#067D17',
    '--syn-escape': '#0037A6',
    '--syn-number': '#1750EB',
    '--syn-param': '#8A5A00',
    '--syn-unused': '#8C8C8C',
    '--syn-comment': '#8C8C8C',
    '--syn-doc-comment': '#497C54',
  },
};

function resolvePaletteOverrides(
  dark: boolean,
  ideMode: boolean,
  scheme?: ConcreteIdeColorSchemeId | 'jetbrains-dark' | 'jetbrains-light',
): IdeSyntaxPalette {
  if (!ideMode) {
    return { '--syn-unused': dark ? '#7A7E85' : '#8C8C8C' };
  }
  if (scheme === 'vscode-dark-modern') return { '--syn-unused': '#7A7E85' };
  if (scheme === 'vscode-light-modern') return { '--syn-unused': '#8C8C8C' };
  if (scheme === 'jetbrains-darcula') return JETBRAINS_THEME_PALETTES['jetbrains-darcula'];
  if (scheme === 'jetbrains-islands-light' || scheme === 'jetbrains-light' || !dark) {
    return JETBRAINS_THEME_PALETTES['jetbrains-islands-light'];
  }
  return JETBRAINS_THEME_PALETTES['jetbrains-islands-dark'];
}

const ideHighlightStyle = HighlightStyle.define([
  {
    tag: [tags.keyword, tags.modifier, tags.self, tags.null, tags.bool, tags.atom],
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
    tag: [tags.typeName, tags.standard(tags.typeName), tags.className, tags.namespace, tags.macroName, tags.labelName],
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

function makeEditorSurfaceTheme(
  dark: boolean,
  ideMode: boolean,
  scheme?: ConcreteIdeColorSchemeId | 'jetbrains-dark' | 'jetbrains-light',
): Extension {
  const paletteVars = resolvePaletteOverrides(dark, ideMode, scheme);
  return EditorView.theme(
    {
      '&': {
        ...paletteVars,
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
      // Find / Replace panel: app controls, not the stock CodeMirror look.
      '.cm-panels': {
        backgroundColor: 'var(--surface-2)',
        color: 'var(--text)',
      },
      '.cm-panels.cm-panels-top': { borderBottom: '1px solid var(--border)' },
      '.cm-panels.cm-panels-bottom': { borderTop: '1px solid var(--border)' },
      '.cm-search': {
        display: 'flex',
        flexWrap: 'wrap',
        alignItems: 'center',
        gap: '6px',
        padding: '6px 8px',
        fontSize: 'var(--fs-xs)',
        fontFamily: 'var(--font-ui)',
      },
      '.cm-search br': { flexBasis: '100%', height: '0' },
      '.cm-search .cm-textfield': {
        margin: '0',
        minWidth: '160px',
        padding: '3px 8px',
        fontSize: 'var(--fs-xs)',
        color: 'var(--text)',
        backgroundColor: 'var(--surface)',
        border: '1px solid var(--control-border)',
        borderRadius: 'var(--radius-s)',
      },
      '.cm-search .cm-textfield:focus': { outline: '2px solid var(--accent-line)', outlineOffset: '-1px' },
      '.cm-search .cm-button': {
        margin: '0',
        padding: '3px 8px',
        fontSize: 'var(--fs-xs)',
        textTransform: 'capitalize',
        color: 'var(--text)',
        backgroundImage: 'none',
        backgroundColor: 'var(--surface)',
        border: '1px solid var(--control-border)',
        borderRadius: 'var(--radius-s)',
        cursor: 'pointer',
      },
      '.cm-search .cm-button:hover': { backgroundColor: 'var(--accent-soft)' },
      // Lint tooltip fix buttons: CodeMirror's default is a dark block on every theme.
      '.cm-diagnosticAction': {
        font: 'inherit',
        fontSize: 'var(--fs-xs)',
        padding: '2px 8px',
        marginInlineStart: '0',
        marginInlineEnd: '6px',
        marginTop: '4px',
        color: 'var(--accent-text)',
        backgroundColor: 'var(--accent-soft)',
        border: '1px solid var(--control-border)',
        borderRadius: 'var(--radius-s)',
        cursor: 'pointer',
      },
      '.cm-diagnosticAction:hover': { backgroundColor: 'var(--surface)' },
      // The LSP plugin links `Source(code)` under the message; CodeMirror repeats it below.
      '.cm-diagnostic:has(.cm-diagnostic-code-link) > .cm-diagnosticSource': { display: 'none' },
      '.cm-search label': { display: 'inline-flex', alignItems: 'center', gap: '4px', color: 'var(--text-dim)' },
      '.cm-search input[type=checkbox]': { accentColor: 'var(--accent-solid)', margin: '0' },
      '.cm-search button[name=close]': {
        position: 'static',
        marginInlineStart: 'auto',
        padding: '0 6px',
        fontSize: 'var(--fs-m)',
        color: 'var(--text-dim)',
        background: 'transparent',
        border: 'none',
        cursor: 'pointer',
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
      '.cm-syn-unused, .cm-syn-unused span': {
        color: 'var(--syn-unused) !important',
        textDecoration: 'underline wavy var(--warning)',
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
        appearance: 'none',
        border: 'none',
        background: 'transparent',
        padding: '0',
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
      '&.cm-editor .cm-line.cm-ide-exec-line, &.cm-editor .cm-activeLine.cm-ide-exec-line, &.cm-editor .cm-line.cm-ide-paused-line, &.cm-editor .cm-activeLine.cm-ide-paused-line': {
        backgroundColor: dark
          ? 'color-mix(in srgb, var(--accent-solid) 34%, transparent)'
          : 'color-mix(in srgb, var(--accent-solid) 22%, transparent)',
        boxShadow: 'inset 3px 0 0 var(--accent-text)',
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
  scheme?: ConcreteIdeColorSchemeId | 'jetbrains-dark' | 'jetbrains-light';
}): Extension {
  const dark = Boolean(opts.dark);
  const ideMode = Boolean(opts.ideMode);
  const plain = Boolean(opts.plain);
  const scheme = opts.scheme ?? (dark ? 'jetbrains-islands-dark' : 'jetbrains-islands-light');
  const key = `${dark ? 1 : 0}:${ideMode ? 1 : 0}:${plain ? 1 : 0}:${scheme}`;
  let cached = themeExtCache.get(key);
  if (!cached) {
    const surface = makeEditorSurfaceTheme(dark, ideMode, scheme);
    cached = plain ? surface : [surface, highlightExt];
    themeExtCache.set(key, cached);
  }
  return cached;
}
