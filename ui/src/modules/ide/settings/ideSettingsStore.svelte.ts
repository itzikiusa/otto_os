// Reactive draft/apply Settings & Project Structure state + category tree for
// the IntelliJ IDEA / GoLand-style Unified IDE Settings modal (`⌘,`).

import {
  cloneIdePreferences as clonePrefs,
  DEFAULT_IDE_PREFERENCES,
  IDE_COLOR_SCHEME_OPTIONS,
  IDE_PREFS_STORAGE_KEY,
  readIdePreferences,
  writeIdePreferences,
  type ConcreteIdeColorSchemeId as ConcreteIdeSchemeId,
  type IdeColorSchemeId as IdeEditorSchemeId,
  type IdePreferences,
} from '../../../lib/idePreferences.svelte';
import {
  ide,
  ideBasename,
  type IdeEnvVar,
  type IdeKeymapPreset,
  type IdeNetworkBridgeConfig,
  type IdeProjectEnvironment,
} from '../ideStore.svelte';

export {
  DEFAULT_IDE_PREFERENCES,
  IDE_PREFS_STORAGE_KEY,
  type ConcreteIdeSchemeId,
  type IdeEditorSchemeId,
  type IdePreferences,
};

export const EDITOR_SCHEMES: readonly {
  id: IdeEditorSchemeId;
  label: string;
  tone: 'auto' | 'dark' | 'light';
  desc: string;
}[] = IDE_COLOR_SCHEME_OPTIONS.map((o) => ({
  id: o.id,
  label: o.label,
  tone: o.group,
  desc: o.desc,
}));

export function loadIdePreferences(): IdePreferences {
  return readIdePreferences();
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
    desc: 'Configure project name, primary SDK, multi-repo modules, and installed/detected SDKs.',
    keywords: ['project', 'structure', 'module', 'sdk', 'workspace', 'folder'],
    children: [
      { id: 'project.project', label: 'Project', desc: 'Project name and primary SDK.', keywords: ['project', 'name', 'sdk'] },
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
      { id: 'appearance.ui', label: 'Appearance', desc: 'Editor color theme (Islands Dark/Light, Darcula, VS Code, Follow Otto), UI density, debugger layout, and project tabs.', keywords: ['appearance', 'theme', 'islands', 'darcula', 'vscode', 'density', 'tabbed', 'split', 'scratches', 'projects', 'tabs'] },
    ],
  },
  {
    id: 'keymap',
    label: 'Keymap',
    desc: 'Choose between JetBrains (IntelliJ / GoLand) and Standard / VS Code shortcuts, and search all bindings.',
    keywords: ['keymap', 'shortcuts', 'hotkeys', 'jetbrains', 'goland', 'intellij', 'vscode', 'f8', 'breakpoint', 'optimize imports'],
  },
  {
    id: 'editor',
    label: 'Editor',
    desc: 'Font metrics, auto-save, open tab limits, syntax color schemes, rainbow brackets, inlay hints, and live code inspections.',
    keywords: ['editor', 'font', 'colors', 'inlay', 'inspections', 'rainbow', 'autosave', 'tabs'],
    children: [
      { id: 'editor.font', label: 'General & Font', desc: 'Font size, line height, ligatures, auto-save, maximum open tabs, format on save, import folding, and breadcrumbs.', keywords: ['font', 'size', 'ligatures', 'autosave', 'auto-save', 'max tabs', 'fold', 'imports', 'breadcrumbs', 'format on save'] },
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
    desc: 'Configure build flags, Testcontainers lifecycle, DAP debugger behavior, login shell environment variables, and SSH targets.',
    keywords: ['build', 'execution', 'deployment', 'debugger', 'shell', 'ssh', 'testcontainers'],
    children: [
      { id: 'build.tools', label: 'Build Tools', desc: 'Offline build mode, test-skipping flags, Testcontainers lifecycle, and run configurations.', keywords: ['build tools', 'maven', 'gradle', 'offline', 'skip tests', 'testcontainers'] },
      { id: 'build.debugger', label: 'Debugger', desc: 'Exception breakpoints, structured JSON log pretty-printing, console wrapping, and Tabbed vs Split layout.', keywords: ['debugger', 'delve', 'jdwp', 'exceptions', 'json', 'console', 'wrap', 'tabbed', 'split'] },
      { id: 'build.env', label: 'Shell Environment', desc: 'Interactive login shell (~/.zshenv + ~/.zshrc) loader for GO_DEBUGGING_*, GOPRIVATE, PATH, and overrides.', keywords: ['shell', 'environment', 'zshrc', 'zshenv', 'goprivate', 'go_debugging', 'path', 'dotenv'] },
      { id: 'build.ssh', label: 'Remote SSH Targets', desc: 'Connect the IDE to remote Linux/macOS hosts over saved SSH connections.', keywords: ['ssh', 'remote', 'server', 'host', 'target'] },
    ],
  },
  {
    id: 'lang',
    label: 'Languages & Frameworks',
    desc: 'Per-language SDKs, GOROOT, Detected JDKs, Python .venv, Rust, Node/TypeScript, and CSV.',
    keywords: ['languages', 'frameworks', 'go', 'java', 'python', 'rust', 'node', 'typescript', 'csv'],
    children: [
      { id: 'lang.go', label: 'Go', desc: 'GOROOT, GOPATH, GOPRIVATE, gopls, and Delve debugger configuration.', keywords: ['go', 'golang', 'goroot', 'gopath', 'goprivate', 'gomod', 'gopls', 'dlv', 'delve'] },
      { id: 'lang.java', label: 'Java', desc: 'JDK & Classpath picker with auto-detected Homebrew / SDKMAN / GraalVM JDKs and broken-SDK warnings.', keywords: ['java', 'jdk', 'java_home', 'classpath', 'maven', 'gradle', 'jdtls', 'graalvm', 'openjdk', 'lombok'] },
      { id: 'lang.node', label: 'JavaScript & TypeScript', desc: 'Node.js runtime and typescript-language-server configuration.', keywords: ['javascript', 'typescript', 'node', 'tsserver', 'npm'] },
      { id: 'lang.python', label: 'Python', desc: 'Python interpreter, .venv / uv / pyenv environments, pyright, and debugpy.', keywords: ['python', 'venv', 'virtualenv', 'uv', 'pyenv', 'pyright', 'debugpy'] },
      { id: 'lang.rust', label: 'Rust', desc: 'rustup toolchain, rust-analyzer, and codelldb / lldb-dap debugger.', keywords: ['rust', 'cargo', 'rustup', 'rust-analyzer', 'codelldb', 'lldb'] },
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
  'lang.shell': 'plugins',
  'lang.json': 'editor.font',
  env: 'build.env',
  environment: 'build.env',
  'build.shell': 'build.env',
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
  network_bridge?: IdeNetworkBridgeConfig | null;
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
    network_bridge: env?.network_bridge ?? undefined,
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
  /** Plugin on/off switches staged until Apply (Cancel drops them), by slug. */
  pluginDraft = $state<Record<string, boolean>>({});

  readonly isDirty = $derived(
    this.envDirty ||
      Object.keys(this.pluginDraft).length > 0 ||
      this.projectNameDraft.trim() !== this.initialProjectName.trim() ||
      JSON.stringify(this.draft) !== JSON.stringify(this.committed),
  );

  syncOnOpen(): void {
    const loaded = loadIdePreferences();
    this.committed = clonePrefs(loaded);
    this.draft = clonePrefs(loaded);
    this.envDraft = emptyEnvDraft(ide.projectEnv);
    this.envDirty = false;
    this.pluginDraft = {};
    const pname = ide.activeProject?.name || (ide.target.root ? ideBasename(ide.target.root) : '');
    this.projectNameDraft = pname;
    this.initialProjectName = pname;
  }

  syncEnvFromStore(): void {
    if (!this.envDirty && ide.projectEnv) {
      this.envDraft = emptyEnvDraft(ide.projectEnv);
    }
  }

  /** Stage a plugin switch; switching it back to the installed state unstages it. */
  stagePlugin(slug: string, enabled: boolean): void {
    const { [slug]: _, ...rest } = this.pluginDraft;
    const installed = ide.plugins.find((p) => p.slug === slug)?.enabled;
    this.pluginDraft = installed === enabled ? rest : { ...rest, [slug]: enabled };
  }

  patchEnv(patch: Partial<EnvDraftState>): void {
    this.envDraft = { ...this.envDraft, ...patch };
    this.envDirty = true;
  }

  async apply(): Promise<boolean> {
    const snap = writeIdePreferences(this.draft);
    this.committed = clonePrefs(snap);
    this.draft = clonePrefs(snap);
    ide.setKeymapPreset(snap.keymapPreset);
    ide.setFormatOnSave(snap.formatOnSave);
    ide.breakOnExceptions = snap.breakOnExceptions;

    let ok = true;
    if (this.projectNameDraft.trim() && this.projectNameDraft.trim() !== this.initialProjectName.trim() && ide.target.root) {
      const nextName = this.projectNameDraft.trim();
      const saved = await ide.saveNamedProject({
        id: ide.activeProject?.id,
        name: nextName,
        primary_root: ide.target.root,
        folders: ide.projectRoots.map((r) => r.path),
        pinned: ide.activeProject?.pinned ?? true,
      });
      if (saved) this.initialProjectName = nextName;
      else ok = false;
    }

    if ((this.envDirty || Object.keys(snap.moduleSdkMap).length > 0) && ide.hasTarget) {
      const envOk = await ide.saveEnvironment({
        java_home: this.envDraft.java_home || null,
        go_root: this.envDraft.go_root || null,
        rust_toolchain: this.envDraft.rust_toolchain || null,
        node_home: this.envDraft.node_home || null,
        python_interpreter: this.envDraft.python_interpreter || null,
        inherit_shell_env: this.envDraft.inherit_shell_env,
        dotenv_path: this.envDraft.dotenv_path || null,
        module_sdks: snap.moduleSdkMap,
        network_bridge: this.envDraft.network_bridge ?? ide.projectEnv?.network_bridge ?? undefined,
        env_vars: this.envDraft.env_vars.filter((v) => v.key.trim().length > 0),
      });
      if (envOk) this.envDirty = false;
      else ok = false;
    }
    for (const [slug, enabled] of Object.entries(this.pluginDraft)) {
      if (await ide.togglePlugin(slug, enabled)) this.stagePlugin(slug, enabled);
      else ok = false;
    }
    return ok;
  }

  cancel(): void {
    this.draft = clonePrefs(this.committed);
    this.envDraft = emptyEnvDraft(ide.projectEnv);
    this.envDirty = false;
    this.pluginDraft = {};
    this.projectNameDraft = this.initialProjectName;
    ide.settingsOpen = false;
  }
}

export const ideSettings = new IdeSettingsController();
