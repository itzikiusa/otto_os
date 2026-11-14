// Reactive draft/apply Settings & Project Structure state + category tree for
// the IntelliJ IDEA / GoLand-style Unified IDE Settings modal (`⌘,`).

import { lsGet, lsSet } from '../../../lib/storage';
import {
  ide,
  ideBasename,
  type IdeEnvVar,
  type IdeKeymapPreset,
  type IdeProjectEnvironment,
} from '../ideStore.svelte';

export const IDE_PREFS_STORAGE_KEY = 'otto.ide.preferences.v1';

export type IdeEditorSchemeId =
  | 'auto'
  | 'jetbrains-islands-dark'
  | 'jetbrains-darcula'
  | 'jetbrains-islands-light'
  | 'vscode-dark-modern'
  | 'vscode-light-modern';

export type ConcreteIdeSchemeId = Exclude<IdeEditorSchemeId, 'auto'>;

export const EDITOR_SCHEMES: readonly {
  id: IdeEditorSchemeId;
  label: string;
  tone: 'auto' | 'dark' | 'light';
  desc: string;
}[] = [
  { id: 'auto', label: 'Follow Otto (Dark / Light)', tone: 'auto', desc: 'Switches automatically between your preferred dark and light editor schemes' },
  { id: 'jetbrains-islands-dark', label: 'JetBrains Islands Dark', tone: 'dark', desc: 'GoLand / IntelliJ 2025 Islands Dark (#CF8E6D keywords, #56A8F5 funcs, #6AAB73 strings)' },
  { id: 'jetbrains-darcula', label: 'JetBrains Darcula', tone: 'dark', desc: 'Classic IntelliJ Darcula (#CC7832 keywords, #FFC66D declarations, #9876AA fields)' },
  { id: 'jetbrains-islands-light', label: 'JetBrains Islands Light', tone: 'light', desc: 'Crisp JetBrains daylight palette (#0033B3 keywords, #067D17 strings, #871094 fields)' },
  { id: 'vscode-dark-modern', label: 'VS Code Dark Modern', tone: 'dark', desc: 'Familiar Dark+ / Dark Modern (#569CD6 / #C586C0 / #DCDCAA / #4EC9B0)' },
  { id: 'vscode-light-modern', label: 'VS Code Light Modern', tone: 'light', desc: 'Clean Light Modern (#0000FF / #AF00DB / #795E26 / #267F99)' },
];

export interface IdePreferences {
  colorScheme: IdeEditorSchemeId;
  preferredDarkScheme: ConcreteIdeSchemeId;
  preferredLightScheme: ConcreteIdeSchemeId;
  rainbowBrackets: boolean;
  uiDensity: 'comfortable' | 'compact';
  debuggerLayout: 'tabbed' | 'split';
  keymapPreset: IdeKeymapPreset;
  fontSize: number;
  lineHeight: number;
  fontLigatures: boolean;
  formatOnSave: boolean;
  autoFoldImports: boolean;
  breadcrumbs: boolean;
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
  defaultBuildToolJava: 'auto' | 'maven' | 'gradle';
  goModulesEnabled: boolean;
  goProxy: string;
  cargoCheckOnSave: boolean;
  npmPackageManager: 'npm' | 'pnpm' | 'yarn' | 'bun';
  goEnabledModules: Record<string, boolean>;
  moduleSdkMap: Record<string, string>;
  javaLanguageLevel: '8' | '11' | '17' | '21' | '23';
  javaCompilerOutput: string;
  shellDialect: 'zsh' | 'bash' | 'POSIX sh';
  jsonValidateSchemas: boolean;
  csvDelimiter: 'auto' | ',' | '\t' | ';';
  csvFirstRowHeader: boolean;
  csvRainbowColumns: boolean;
}

export const DEFAULT_IDE_PREFERENCES: IdePreferences = {
  colorScheme: 'auto',
  preferredDarkScheme: 'jetbrains-islands-dark',
  preferredLightScheme: 'jetbrains-islands-light',
  rainbowBrackets: true,
  uiDensity: 'comfortable',
  debuggerLayout: 'tabbed',
  keymapPreset: 'jetbrains',
  fontSize: 13,
  lineHeight: 1.55,
  fontLigatures: true,
  formatOnSave: false,
  autoFoldImports: true,
  breadcrumbs: true,
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
  consoleLineWrap: false,
  defaultBuildToolJava: 'auto',
  goModulesEnabled: true,
  goProxy: 'https://proxy.golang.org,direct',
  cargoCheckOnSave: true,
  npmPackageManager: 'npm',
  goEnabledModules: {},
  moduleSdkMap: {},
  javaLanguageLevel: '21',
  javaCompilerOutput: 'target/classes',
  shellDialect: 'zsh',
  jsonValidateSchemas: true,
  csvDelimiter: 'auto',
  csvFirstRowHeader: true,
  csvRainbowColumns: true,
};

export function loadIdePreferences(): IdePreferences {
  try {
    const raw = lsGet(IDE_PREFS_STORAGE_KEY);
    if (!raw) {
      return {
        ...DEFAULT_IDE_PREFERENCES,
        keymapPreset: ide.keymapPreset,
        formatOnSave: ide.formatOnSave,
      };
    }
    const parsed = JSON.parse(raw) as Partial<IdePreferences>;
    return {
      ...DEFAULT_IDE_PREFERENCES,
      ...parsed,
      goEnabledModules: { ...(parsed.goEnabledModules ?? {}) },
      moduleSdkMap: { ...(parsed.moduleSdkMap ?? {}) },
    };
  } catch {
    return { ...DEFAULT_IDE_PREFERENCES };
  }
}

function clonePrefs(p: IdePreferences): IdePreferences {
  return {
    ...p,
    goEnabledModules: { ...p.goEnabledModules },
    moduleSdkMap: { ...p.moduleSdkMap },
  };
}

export interface SettingsTreeNode {
  id: string;
  label: string;
  desc: string;
  keywords: string[];
  children?: SettingsTreeNode[];
}

export const SETTINGS_TREE: readonly SettingsTreeNode[] = [
  {
    id: 'project',
    label: 'Project Structure',
    desc: 'Configure project name, primary SDK, language level, multi-repo modules, and installed/detected SDKs.',
    keywords: ['project', 'structure', 'module', 'sdk', 'workspace', 'folder'],
    children: [
      { id: 'project.project', label: 'Project', desc: 'Project name, primary SDK, and compiler language level.', keywords: ['project', 'name', 'sdk', 'compiler', 'language level'] },
      { id: 'project.modules', label: 'Modules', desc: 'Multi-folder repository roots attached to this project, bulk attach, and per-module SDKs.', keywords: ['modules', 'folders', 'multi-repo', 'roots', 'attach'] },
      { id: 'project.sdks', label: 'SDKs', desc: 'All 8 language ecosystems: installed toolchains, detected host SDKs, LSP servers, and DAP debuggers.', keywords: ['sdks', 'toolchains', 'download', 'detected', 'goroot', 'jdk', 'node', 'python', 'rust'] },
    ],
  },
  {
    id: 'appearance',
    label: 'Appearance & Behavior',
    desc: 'Customize editor & chrome color themes, UI density, and tool window defaults.',
    keywords: ['appearance', 'theme', 'dark', 'light', 'density'],
    children: [
      { id: 'appearance.ui', label: 'Appearance', desc: 'Editor color theme (Islands Dark/Light, Darcula, VS Code, Follow Otto), UI density, and debugger layout.', keywords: ['appearance', 'theme', 'islands', 'darcula', 'vscode', 'density', 'tabbed', 'split'] },
    ],
  },
  {
    id: 'keymap',
    label: 'Keymap',
    desc: 'Choose between JetBrains (IntelliJ / GoLand) and Standard / VS Code shortcuts, and search all bindings.',
    keywords: ['keymap', 'shortcuts', 'hotkeys', 'jetbrains', 'goland', 'intellij', 'vscode', 'f8', 'breakpoint'],
  },
  {
    id: 'editor',
    label: 'Editor',
    desc: 'Font metrics, syntax color schemes, rainbow brackets, inlay hints, and live code inspections.',
    keywords: ['editor', 'font', 'colors', 'inlay', 'inspections', 'rainbow'],
    children: [
      { id: 'editor.font', label: 'General & Font', desc: 'Font size, line height, ligatures, format on save, import folding, and breadcrumbs.', keywords: ['font', 'size', 'ligatures', 'fold', 'imports', 'breadcrumbs', 'format on save'] },
      { id: 'editor.colors', label: 'Color Scheme', desc: 'Live Go & Java syntax preview across all 6 curated dark and light schemes.', keywords: ['color', 'scheme', 'syntax', 'islands', 'darcula', 'vscode', 'preview'] },
      { id: 'editor.hints', label: 'Inlay Hints & Quick Doc', desc: 'Parameter name hints, CodeVision symbol usages, and compact hover documentation.', keywords: ['inlay', 'hints', 'parameter', 'codevision', 'hover', 'quick doc', 'javadoc'] },
      { id: 'editor.inspections', label: 'Inspections', desc: 'Java unresolved imports, Lombok (@Slf4j / @Data / @Builder) awareness, and unused imports.', keywords: ['inspections', 'lombok', 'slf4j', 'unresolved', 'imports', 'warnings', 'java'] },
      { id: 'editor.rainbow', label: 'Rainbow Brackets', desc: 'Cycle 4 distinct bracket colors by nesting depth for (), [], and {}.', keywords: ['rainbow', 'brackets', 'parentheses', 'braces', 'nesting'] },
    ],
  },
  {
    id: 'vcs',
    label: 'Version Control',
    desc: 'Git tree status decorations and automatic Local History file snapshots.',
    keywords: ['vcs', 'git', 'version control', 'history'],
    children: [
      { id: 'vcs.git', label: 'Git', desc: 'File status coloring in the Project tree and quick access to the IDE Git tool window.', keywords: ['git', 'branch', 'commit', 'diff', 'status', 'badges'] },
      { id: 'vcs.history', label: 'Local History', desc: 'Automatic local file snapshots on every save, agent edit, and refactor with retention controls.', keywords: ['history', 'local history', 'snapshots', 'retention', 'restore', 'diff'] },
    ],
  },
  {
    id: 'build',
    label: 'Build, Execution, Deployment',
    desc: 'Configure build tools, DAP debugger behavior, login shell environment variables, and SSH targets.',
    keywords: ['build', 'execution', 'deployment', 'debugger', 'shell', 'ssh'],
    children: [
      { id: 'build.tools', label: 'Build Tools', desc: 'Maven, Gradle, Cargo, Go Modules, and npm/pnpm/yarn/bun preferences.', keywords: ['build tools', 'maven', 'gradle', 'cargo', 'npm', 'pnpm', 'go modules'] },
      { id: 'build.debugger', label: 'Debugger', desc: 'Exception breakpoints, structured JSON log pretty-printing, console wrapping, and Tabbed vs Split layout.', keywords: ['debugger', 'delve', 'jdwp', 'exceptions', 'json', 'console', 'wrap', 'tabbed', 'split'] },
      { id: 'build.env', label: 'Shell Environment', desc: 'Interactive login shell (~/.zshenv + ~/.zshrc) loader for GO_DEBUGGING_*, GOPRIVATE, PATH, and overrides.', keywords: ['shell', 'environment', 'zshrc', 'zshenv', 'goprivate', 'go_debugging', 'path', 'dotenv'] },
      { id: 'build.ssh', label: 'Remote SSH Targets', desc: 'Connect the IDE to remote Linux/macOS hosts over saved SSH connections.', keywords: ['ssh', 'remote', 'server', 'host', 'target'] },
    ],
  },
  {
    id: 'lang',
    label: 'Languages & Frameworks',
    desc: 'Per-language SDKs, GOROOT & Go Modules checklists, Detected JDKs, Python .venv, Rust, TS, Shell, JSON, and CSV.',
    keywords: ['languages', 'frameworks', 'go', 'java', 'python', 'rust', 'node', 'typescript'],
    children: [
      { id: 'lang.go', label: 'Go', desc: 'GOROOT, GOPATH, GOPRIVATE, Go Modules proxy, and per-module Go support checklist.', keywords: ['go', 'golang', 'goroot', 'gopath', 'goprivate', 'gomod', 'gopls', 'dlv', 'delve'] },
      { id: 'lang.java', label: 'Java', desc: 'JDK & Classpath picker with auto-detected Homebrew / SDKMAN / GraalVM JDKs and broken-SDK warnings.', keywords: ['java', 'jdk', 'java_home', 'classpath', 'maven', 'gradle', 'jdtls', 'graalvm', 'openjdk', 'lombok'] },
      { id: 'lang.node', label: 'JavaScript & TypeScript', desc: 'Node.js runtime and typescript-language-server configuration.', keywords: ['javascript', 'typescript', 'node', 'tsserver', 'npm'] },
      { id: 'lang.python', label: 'Python', desc: 'Python interpreter, .venv / uv / pyenv environments, pyright, and debugpy.', keywords: ['python', 'venv', 'virtualenv', 'uv', 'pyenv', 'pyright', 'debugpy'] },
      { id: 'lang.rust', label: 'Rust', desc: 'rustup toolchain, rust-analyzer, and codelldb / lldb-dap debugger.', keywords: ['rust', 'cargo', 'rustup', 'rust-analyzer', 'codelldb', 'lldb'] },
      { id: 'lang.shell', label: 'Shell', desc: 'Shell script dialect and bash-language-server diagnostics.', keywords: ['shell', 'bash', 'zsh', 'sh'] },
      { id: 'lang.json', label: 'JSON & Schemas', desc: 'JSON / JSONL schema validation and formatting.', keywords: ['json', 'jsonl', 'schema'] },
      { id: 'lang.csv', label: 'CSV Formats', desc: 'CSV / TSV delimiter detection, header row, and rainbow column alignment.', keywords: ['csv', 'tsv', 'delimiter', 'table'] },
    ],
  },
  {
    id: 'plugins',
    label: 'Plugins & Language Servers',
    desc: 'Live health of LSP servers (gopls, jdtls, tsserver, pyright, rust-analyzer, bash-ls) and runtime IDE plugins.',
    keywords: ['plugins', 'extensions', 'lsp', 'language server', 'gopls', 'jdtls', 'pyright', 'rust-analyzer', 'catalog'],
  },
];

const SECTION_ALIASES: Record<string, string> = {
  sdk: 'project.sdks',
  sdks: 'project.sdks',
  toolchains: 'project.sdks',
  modules: 'project.modules',
  goroot: 'lang.go',
  go: 'lang.go',
  java: 'lang.java',
  jdk: 'lang.java',
  python: 'lang.python',
  rust: 'lang.rust',
  node: 'lang.node',
  env: 'build.env',
  environment: 'build.env',
  ssh: 'build.ssh',
  debugger: 'build.debugger',
  history: 'vcs.history',
  'local-history': 'vcs.history',
  git: 'vcs.git',
  colors: 'editor.colors',
  rainbow: 'editor.rainbow',
  inlay: 'editor.hints',
  lombok: 'editor.inspections',
};

export function normalizeSectionId(raw?: string | null): string {
  if (!raw) return 'project.project';
  const clean = raw.trim();
  if (SECTION_ALIASES[clean]) return SECTION_ALIASES[clean];
  for (const top of SETTINGS_TREE) {
    if (top.id === clean) return top.id;
    if (top.children?.some((c) => c.id === clean)) return clean;
  }
  return 'project.project';
}

export function findSettingsNode(id: string): { node: SettingsTreeNode; parent: SettingsTreeNode | null } | null {
  for (const top of SETTINGS_TREE) {
    if (top.id === id) return { node: top, parent: null };
    for (const child of top.children ?? []) {
      if (child.id === id) return { node: child, parent: top };
    }
  }
  return null;
}

export function filterSettingsTree(query: string): SettingsTreeNode[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...SETTINGS_TREE];
  const matches = (n: SettingsTreeNode): boolean =>
    n.label.toLowerCase().includes(q) ||
    n.desc.toLowerCase().includes(q) ||
    n.keywords.some((k) => k.toLowerCase().includes(q));

  const out: SettingsTreeNode[] = [];
  for (const top of SETTINGS_TREE) {
    const matchingKids = (top.children ?? []).filter(matches);
    if (matches(top) || matchingKids.length > 0) {
      out.push({
        ...top,
        children: matchingKids.length > 0 ? matchingKids : top.children,
      });
    }
  }
  return out;
}

export interface EnvDraftState {
  java_home: string;
  go_root: string;
  rust_toolchain: string;
  node_home: string;
  python_interpreter: string;
  inherit_shell_env: boolean;
  dotenv_path: string;
  env_vars: IdeEnvVar[];
}

function emptyEnvDraft(env: IdeProjectEnvironment | null): EnvDraftState {
  return {
    java_home: env?.java_home ?? '',
    go_root: env?.go_root ?? '',
    rust_toolchain: env?.rust_toolchain ?? '',
    node_home: env?.node_home ?? '',
    python_interpreter: env?.python_interpreter ?? '',
    inherit_shell_env: env?.inherit_shell_env ?? true,
    dotenv_path: env?.dotenv_path ?? '',
    env_vars: (env?.env_vars ?? []).map((v) => ({ ...v })),
  };
}

class IdeSettingsController {
  committed = $state<IdePreferences>(loadIdePreferences());
  draft = $state<IdePreferences>(loadIdePreferences());
  envDraft = $state<EnvDraftState>(emptyEnvDraft(null));
  envDirty = $state(false);
  projectNameDraft = $state('');
  initialProjectName = $state('');

  readonly isDirty = $derived(
    this.envDirty ||
      this.projectNameDraft.trim() !== this.initialProjectName.trim() ||
      JSON.stringify(this.draft) !== JSON.stringify(this.committed),
  );

  syncOnOpen(): void {
    const loaded = loadIdePreferences();
    this.committed = clonePrefs(loaded);
    this.draft = clonePrefs(loaded);
    this.envDraft = emptyEnvDraft(ide.projectEnv);
    this.envDirty = false;
    const pname = ide.activeProject?.name || (ide.target.root ? ideBasename(ide.target.root) : '');
    this.projectNameDraft = pname;
    this.initialProjectName = pname;
  }

  syncEnvFromStore(): void {
    if (!this.envDirty && ide.projectEnv) {
      this.envDraft = emptyEnvDraft(ide.projectEnv);
    }
  }

  patchEnv(patch: Partial<EnvDraftState>): void {
    this.envDraft = { ...this.envDraft, ...patch };
    this.envDirty = true;
  }

  async apply(): Promise<void> {
    const snap = clonePrefs(this.draft);
    lsSet(IDE_PREFS_STORAGE_KEY, JSON.stringify(snap));
    this.committed = clonePrefs(snap);
    ide.setKeymapPreset(snap.keymapPreset);
    ide.setFormatOnSave(snap.formatOnSave);
    ide.breakOnExceptions = snap.breakOnExceptions;

    if (this.projectNameDraft.trim() && this.projectNameDraft.trim() !== this.initialProjectName.trim() && ide.target.root) {
      const nextName = this.projectNameDraft.trim();
      await ide.saveNamedProject({
        id: ide.activeProject?.id,
        name: nextName,
        primary_root: ide.target.root,
        folders: ide.projectRoots.map((r) => r.path),
        pinned: ide.activeProject?.pinned ?? true,
      });
      this.initialProjectName = nextName;
    }

    if ((this.envDirty || Object.keys(snap.moduleSdkMap).length > 0) && ide.hasTarget) {
      await ide.saveEnvironment({
        java_home: this.envDraft.java_home || null,
        go_root: this.envDraft.go_root || null,
        rust_toolchain: this.envDraft.rust_toolchain || null,
        node_home: this.envDraft.node_home || null,
        python_interpreter: this.envDraft.python_interpreter || null,
        inherit_shell_env: this.envDraft.inherit_shell_env,
        dotenv_path: this.envDraft.dotenv_path || null,
        module_sdks: snap.moduleSdkMap,
        env_vars: this.envDraft.env_vars.filter((v) => v.key.trim().length > 0),
      });
      this.envDirty = false;
    }

    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('otto:ide-prefs-changed', { detail: snap }));
    }
  }

  cancel(): void {
    this.draft = clonePrefs(this.committed);
    this.envDraft = emptyEnvDraft(ide.projectEnv);
    this.envDirty = false;
    this.projectNameDraft = this.initialProjectName;
    ide.settingsOpen = false;
  }
}

export const ideSettings = new IdeSettingsController();
