// Canonical `'otto.ide.preferences.v1'` schema, defaults, legacy alias normalization,
// and storage read/write helpers shared by CodeMirror, IDE Settings, Debugger Dock,
// Editor Bridge, and Shell Commands.
// Pure TypeScript (zero Svelte runes) so `node --test` unit tests import it directly.

export const IDE_PREFS_STORAGE_KEY = 'otto.ide.preferences.v1';
export const IDE_PREFS_EVENT = 'otto:ide-prefs-changed';

export type ConcreteIdeColorSchemeId =
  | 'jetbrains-islands-dark'
  | 'jetbrains-darcula'
  | 'jetbrains-islands-light'
  | 'vscode-dark-modern'
  | 'vscode-light-modern';

export type IdeColorSchemeId = 'auto' | ConcreteIdeColorSchemeId;
export type IdeKeymapPreset = 'jetbrains' | 'standard';

export interface IdeColorSchemeOption {
  id: IdeColorSchemeId;
  label: string;
  group: 'auto' | 'dark' | 'light';
  desc: string;
}

export const IDE_COLOR_SCHEME_OPTIONS: readonly IdeColorSchemeOption[] = [
  {
    id: 'auto',
    label: 'Follow Otto Dark/Light',
    group: 'auto',
    desc: 'Automatically switches between your preferred Dark and Light IDE schemes.',
  },
  {
    id: 'jetbrains-islands-dark',
    label: 'JetBrains Islands Dark',
    group: 'dark',
    desc: '2026 GoLand / IntelliJ Islands Dark (#191A1C editor, warm orange keywords, purple fields).',
  },
  {
    id: 'jetbrains-darcula',
    label: 'JetBrains Darcula',
    group: 'dark',
    desc: 'Classic IntelliJ IDEA / GoLand Darcula (#2B2B2B surface, warm amber & lavender palette).',
  },
  {
    id: 'vscode-dark-modern',
    label: 'VS Code Dark Modern',
    group: 'dark',
    desc: 'Visual Studio Code Dark+ / Modern (#1E1E1E canvas, sky-blue fields, yellow calls).',
  },
  {
    id: 'jetbrains-islands-light',
    label: 'JetBrains Islands Light',
    group: 'light',
    desc: '2026 GoLand / IntelliJ Islands Light (#FFFFFF canvas, navy keywords, teal calls).',
  },
  {
    id: 'vscode-light-modern',
    label: 'VS Code Light Modern',
    group: 'light',
    desc: 'Visual Studio Code Light+ / Modern (#FFFFFF canvas, royal blue & warm brown accents).',
  },
];

const VALID_SCHEMES = new Set<string>(IDE_COLOR_SCHEME_OPTIONS.map((o) => o.id));
const DARK_SCHEMES = new Set<ConcreteIdeColorSchemeId>([
  'jetbrains-islands-dark',
  'jetbrains-darcula',
  'vscode-dark-modern',
]);

export interface IdePreferences {
  colorScheme: IdeColorSchemeId;
  preferredDarkScheme: ConcreteIdeColorSchemeId;
  preferredLightScheme: ConcreteIdeColorSchemeId;
  /** Alias mirrors kept in lockstep for CodeEditor & cm-ide-theme callers. */
  darkColorScheme: ConcreteIdeColorSchemeId;
  lightColorScheme: ConcreteIdeColorSchemeId;
  rainbowBrackets: boolean;
  uiDensity: 'comfortable' | 'compact';
  debuggerLayout: 'tabbed' | 'split';
  keymapPreset: IdeKeymapPreset;
  fontSize: number;
  lineHeight: number;
  tabSize: number;
  wordWrap: boolean;
  fontLigatures: boolean;
  autoSave: boolean;
  maxOpenTabs: number;
  formatOnSave: boolean;
  autoFoldImports: boolean;
  breadcrumbs: boolean;
  /** Alias mirror kept in lockstep with `breadcrumbs`. */
  showBreadcrumbs: boolean;
  inlayHints: boolean;
  codeVision: boolean;
  compactHover: boolean;
  javaUnresolvedImports: boolean;
  lombokSupport: boolean;
  unusedImports: boolean;
  gitTreeBadges: boolean;
  localHistoryEnabled: boolean;
  localHistoryRetentionDays: number;
  breakOnExceptions: boolean;
  prettyJsonLogs: boolean;
  consoleLineWrap: boolean;
  dbgShowFrames: boolean;
  dbgShowVariables: boolean;
  dbgShowWatches: boolean;
  dbgShowConsole: boolean;
  hiddenDockTabs: string[];
  moduleSdkMap: Record<string, string>;
  csvDelimiter: 'auto' | ',' | '\t' | ';';
  csvFirstRowHeader: boolean;
  csvRainbowColumns: boolean;
}

export type IdeEditorPreferences = IdePreferences;

export const DEFAULT_IDE_PREFERENCES: IdePreferences = {
  colorScheme: 'auto',
  preferredDarkScheme: 'jetbrains-islands-dark',
  preferredLightScheme: 'jetbrains-islands-light',
  darkColorScheme: 'jetbrains-islands-dark',
  lightColorScheme: 'jetbrains-islands-light',
  rainbowBrackets: true,
  uiDensity: 'comfortable',
  debuggerLayout: 'tabbed',
  keymapPreset: 'jetbrains',
  fontSize: 13,
  lineHeight: 1.55,
  tabSize: 4,
  wordWrap: false,
  fontLigatures: true,
  autoSave: true,
  maxOpenTabs: 20,
  formatOnSave: false,
  autoFoldImports: true,
  breadcrumbs: true,
  showBreadcrumbs: true,
  inlayHints: true,
  codeVision: true,
  compactHover: true,
  javaUnresolvedImports: true,
  lombokSupport: true,
  unusedImports: true,
  gitTreeBadges: true,
  localHistoryEnabled: true,
  localHistoryRetentionDays: 14,
  breakOnExceptions: false,
  prettyJsonLogs: true,
  consoleLineWrap: true,
  dbgShowFrames: true,
  dbgShowVariables: true,
  dbgShowWatches: true,
  dbgShowConsole: true,
  hiddenDockTabs: [],
  moduleSdkMap: {},
  csvDelimiter: 'auto',
  csvFirstRowHeader: true,
  csvRainbowColumns: true,
};

const STALE_ALIAS_KEYS = [
  'scheme',
  'editorScheme',
  'theme',
  'darkColorScheme',
  'darkScheme',
  'lightColorScheme',
  'lightScheme',
  'showBreadcrumbs',
  'keymap',
  'gitStatusInTree',
  'showGitStatusInTree',
  'enableLocalHistory',
  'consoleWrapLines',
  'wrapConsoleLines',
  'consolePrettyJson',
  'debuggerPrettyJson',
  'defaultBuildToolJava',
  'goModulesEnabled',
  'goProxy',
  'cargoCheckOnSave',
  'npmPackageManager',
  'goEnabledModules',
  'javaLanguageLevel',
  'javaCompilerOutput',
  'shellDialect',
  'jsonValidateSchemas',
] as const;

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

function asEnum<T extends string>(val: unknown, allowed: readonly T[], fallback: T): T {
  return typeof val === 'string' && (allowed as readonly string[]).includes(val)
    ? (val as T)
    : fallback;
}

function asRecord<V>(val: unknown, guard: (v: unknown) => v is V): Record<string, V> {
  if (!val || typeof val !== 'object' || Array.isArray(val)) return {};
  const out: Record<string, V> = {};
  for (const [k, v] of Object.entries(val as Record<string, unknown>)) {
    if (guard(v)) out[k] = v;
  }
  return out;
}

export function cloneIdePreferences(p: IdePreferences): IdePreferences {
  return {
    ...p,
    darkColorScheme: p.preferredDarkScheme,
    lightColorScheme: p.preferredLightScheme,
    showBreadcrumbs: p.breadcrumbs,
    hiddenDockTabs: [...p.hiddenDockTabs],
    moduleSdkMap: { ...p.moduleSdkMap },
  };
}

export function normalizeIdePreferencesRecord(
  p: Record<string, unknown>,
  legacyFmt = false,
  legacyKeymap: string | null = null,
): IdePreferences {
  const d = DEFAULT_IDE_PREFERENCES;
  const preferredDarkScheme = asConcreteScheme(
    p.preferredDarkScheme ?? p.darkColorScheme ?? p.darkScheme,
    d.preferredDarkScheme,
  );
  const preferredLightScheme = asConcreteScheme(
    p.preferredLightScheme ?? p.lightColorScheme ?? p.lightScheme,
    d.preferredLightScheme,
  );
  const breadcrumbs = asBool(p.breadcrumbs ?? p.showBreadcrumbs, d.breadcrumbs);
  const keymapRaw = p.keymapPreset ?? p.keymap ?? legacyKeymap;
  const keymapPreset: IdeKeymapPreset =
    keymapRaw === 'standard' || keymapRaw === 'vscode' ? 'standard' : 'jetbrains';

  return {
    colorScheme: asScheme(p.colorScheme ?? p.editorScheme ?? p.scheme ?? p.theme, d.colorScheme),
    preferredDarkScheme,
    preferredLightScheme,
    darkColorScheme: preferredDarkScheme,
    lightColorScheme: preferredLightScheme,
    rainbowBrackets: asBool(p.rainbowBrackets, d.rainbowBrackets),
    uiDensity: asEnum(p.uiDensity, ['comfortable', 'compact'] as const, d.uiDensity),
    debuggerLayout: asEnum(p.debuggerLayout, ['tabbed', 'split'] as const, d.debuggerLayout),
    keymapPreset,
    fontSize: asNum(p.fontSize ?? p.editorFontSize, 10, 24, d.fontSize),
    lineHeight: asNum(p.lineHeight ?? p.editorLineHeight, 1.1, 2.2, d.lineHeight),
    tabSize: asNum(p.tabSize, 2, 8, d.tabSize),
    wordWrap: asBool(p.wordWrap ?? p.softWrap, d.wordWrap),
    fontLigatures: asBool(p.fontLigatures, d.fontLigatures),
    autoSave: asBool(p.autoSave, d.autoSave),
    maxOpenTabs: Math.round(asNum(p.maxOpenTabs, 15, 40, d.maxOpenTabs)),
    formatOnSave: asBool(p.formatOnSave, legacyFmt),
    autoFoldImports: asBool(p.autoFoldImports ?? p.foldImports, d.autoFoldImports),
    breadcrumbs,
    showBreadcrumbs: breadcrumbs,
    inlayHints: asBool(p.inlayHints ?? p.showInlayHints ?? p.parameterHints, d.inlayHints),
    codeVision: asBool(p.codeVision ?? p.showCodeVision, d.codeVision),
    compactHover: asBool(p.compactHover, d.compactHover),
    javaUnresolvedImports: asBool(p.javaUnresolvedImports, d.javaUnresolvedImports),
    lombokSupport: asBool(p.lombokSupport, d.lombokSupport),
    unusedImports: asBool(p.unusedImports ?? p.inspectUnusedImports, d.unusedImports),
    gitTreeBadges: asBool(
      p.gitTreeBadges ?? p.gitStatusInTree ?? p.showGitStatusInTree,
      d.gitTreeBadges,
    ),
    localHistoryEnabled: asBool(
      p.localHistoryEnabled ?? p.enableLocalHistory,
      d.localHistoryEnabled,
    ),
    localHistoryRetentionDays: asNum(
      p.localHistoryRetentionDays,
      1,
      90,
      d.localHistoryRetentionDays,
    ),
    breakOnExceptions: asBool(p.breakOnExceptions, d.breakOnExceptions),
    prettyJsonLogs: asBool(
      p.prettyJsonLogs ?? p.consolePrettyJson ?? p.debuggerPrettyJson,
      d.prettyJsonLogs,
    ),
    consoleLineWrap: asBool(
      p.consoleLineWrap ?? p.consoleWrapLines ?? p.wrapConsoleLines,
      d.consoleLineWrap,
    ),
    dbgShowFrames: asBool(p.dbgShowFrames, d.dbgShowFrames),
    dbgShowVariables: asBool(p.dbgShowVariables, d.dbgShowVariables),
    dbgShowWatches: asBool(p.dbgShowWatches, d.dbgShowWatches),
    dbgShowConsole: asBool(p.dbgShowConsole, d.dbgShowConsole),
    hiddenDockTabs: Array.isArray(p.hiddenDockTabs)
      ? p.hiddenDockTabs.filter((x): x is string => typeof x === 'string')
      : [...d.hiddenDockTabs],
    moduleSdkMap: asRecord(p.moduleSdkMap, (v): v is string => typeof v === 'string'),
    csvDelimiter: asEnum(p.csvDelimiter, ['auto', ',', '\t', ';'] as const, d.csvDelimiter),
    csvFirstRowHeader: asBool(p.csvFirstRowHeader, d.csvFirstRowHeader),
    csvRainbowColumns: asBool(p.csvRainbowColumns, d.csvRainbowColumns),
  };
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return Boolean(v) && typeof v === 'object' && !Array.isArray(v);
}

export function readIdePreferences(): IdePreferences {
  try {
    if (typeof localStorage === 'undefined') return cloneIdePreferences(DEFAULT_IDE_PREFERENCES);
    const raw = localStorage.getItem(IDE_PREFS_STORAGE_KEY);
    const legacyFmt = localStorage.getItem('otto.ide.formatOnSave') === '1';
    const legacyKeymap =
      localStorage.getItem('otto.ide.keymapPreset') ?? localStorage.getItem('otto.ide.keymap');
    if (!raw) {
      return normalizeIdePreferencesRecord({}, legacyFmt, legacyKeymap);
    }
    const parsed = JSON.parse(raw) as unknown;
    return normalizeIdePreferencesRecord(isPlainObject(parsed) ? parsed : {}, legacyFmt, legacyKeymap);
  } catch {
    return cloneIdePreferences(DEFAULT_IDE_PREFERENCES);
  }
}

export function writeIdePreferences(patch: Partial<IdePreferences>): IdePreferences {
  const cur = readIdePreferences();
  const nextRaw: Record<string, unknown> = { ...cur, ...patch };
  if (patch.preferredDarkScheme !== undefined) {
    nextRaw.darkColorScheme = patch.preferredDarkScheme;
  } else if (patch.darkColorScheme !== undefined) {
    nextRaw.preferredDarkScheme = patch.darkColorScheme;
  }
  if (patch.preferredLightScheme !== undefined) {
    nextRaw.lightColorScheme = patch.preferredLightScheme;
  } else if (patch.lightColorScheme !== undefined) {
    nextRaw.preferredLightScheme = patch.lightColorScheme;
  }
  if (patch.breadcrumbs !== undefined) {
    nextRaw.showBreadcrumbs = patch.breadcrumbs;
  } else if (patch.showBreadcrumbs !== undefined) {
    nextRaw.breadcrumbs = patch.showBreadcrumbs;
  }

  const merged = normalizeIdePreferencesRecord(nextRaw, Boolean(nextRaw.formatOnSave), null);
  try {
    if (typeof localStorage !== 'undefined') {
      const existingRaw = localStorage.getItem(IDE_PREFS_STORAGE_KEY);
      const parsedExisting = existingRaw ? (JSON.parse(existingRaw) as unknown) : null;
      const existing = isPlainObject(parsedExisting) ? parsedExisting : {};
      const persisted: Record<string, unknown> = { ...existing, ...merged };
      for (const staleKey of STALE_ALIAS_KEYS) {
        delete persisted[staleKey];
      }
      localStorage.setItem(IDE_PREFS_STORAGE_KEY, JSON.stringify(persisted));
      localStorage.setItem('otto.ide.formatOnSave', merged.formatOnSave ? '1' : '0');
      localStorage.setItem('otto.ide.keymapPreset', merged.keymapPreset);
      localStorage.removeItem('otto.ide.keymap');
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
  prefs: Pick<IdePreferences, 'colorScheme'> &
    Partial<
      Pick<
        IdePreferences,
        'preferredDarkScheme' | 'preferredLightScheme' | 'darkColorScheme' | 'lightColorScheme'
      >
    >,
  appScheme: 'light' | 'dark',
): ConcreteIdeColorSchemeId {
  if (prefs.colorScheme !== 'auto') return prefs.colorScheme;
  const light = prefs.preferredLightScheme ?? prefs.lightColorScheme ?? 'jetbrains-islands-light';
  const dark = prefs.preferredDarkScheme ?? prefs.darkColorScheme ?? 'jetbrains-islands-dark';
  return appScheme === 'light' ? light : dark;
}

export function isDarkIdeScheme(scheme: ConcreteIdeColorSchemeId): boolean {
  return DARK_SCHEMES.has(scheme);
}
