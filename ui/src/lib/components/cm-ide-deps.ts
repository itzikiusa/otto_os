// JetBrains GoLand / IntelliJ Manifest Dependency Intelligence (`go.mod`, `pom.xml`,
// `build.gradle*`, `package.json`, `Cargo.toml`, `requirements.txt`, `pyproject.toml`):
// Top-of-block CodeVision links, inline deprecated/outdated/CVE decorations, managed-dep
// `🎯↑` parent-POM gutter jump, rich hover card with 1-click actions, and manifest context menu.

import { EditorState, Prec, RangeSet, StateEffect, StateField, type Extension, type Range } from '@codemirror/state';
import {
  Decoration,
  EditorView,
  GutterMarker,
  ViewPlugin,
  WidgetType,
  gutter,
  hoverTooltip,
  type DecorationSet,
  type Tooltip,
  type ViewUpdate,
} from '@codemirror/view';
import type { MenuItem } from '../contextmenu.svelte.ts';
import { buildIdeVcsContextRows } from './cm-ide-history.ts';
import {
  applyBatchFallbackManifestUpdates,
  applyFallbackManifestVersionUpdate,
  detectManifestEcosystem,
  executeIdeDepAction,
  inspectIdeDependencies,
  isIdeManifestPath,
  isVersionOutdated,
  mergeDepItems,
  normalizeIdeDepAction,
  normalizeRemoteDepItem,
  parseManifestDependencies,
  type IdeDepItem,
  type IdeDepManagedLoc,
  type IdeDepsInspectResponse,
  type IdeDepVuln,
  type ManifestEcosystem,
  type RawRemoteDepItem,
} from './cm-ide-deps-model.ts';

export {
  applyBatchFallbackManifestUpdates,
  applyFallbackManifestVersionUpdate,
  detectManifestEcosystem,
  executeIdeDepAction,
  inspectIdeDependencies,
  isIdeManifestPath,
  isVersionOutdated,
  mergeDepItems,
  normalizeIdeDepAction,
  normalizeRemoteDepItem,
  parseManifestDependencies,
  type IdeDepItem,
  type IdeDepManagedLoc,
  type IdeDepsInspectResponse,
  type IdeDepVuln,
  type ManifestEcosystem,
  type RawRemoteDepItem,
};

const setManifestDepsEffect = StateEffect.define<IdeDepItem[]>();

const manifestDepsField = StateField.define<IdeDepItem[]>({
  create: () => [],
  update(value, tr) {
    for (const e of tr.effects) {
      if (e.is(setManifestDepsEffect)) {
        return e.value;
      }
    }
    return value;
  },
});

export function getManifestAllDeps(state: EditorState): IdeDepItem[] {
  return state.field(manifestDepsField, false) ?? [];
}

export function getManifestDepAtLine(state: EditorState, line: number): IdeDepItem | null {
  return (
    getManifestAllDeps(state).find(
      (d) => line >= (d.block_start_line ?? d.line) && line <= (d.end_line ?? d.block_end_line ?? d.line),
    ) ?? null
  );
}

class ManifestCodeVisionWidget extends WidgetType {
  actions: Array<{ label: string; action: string }>;
  onRun: (action: string) => void;

  constructor(actions: Array<{ label: string; action: string }>, onRun: (action: string) => void) {
    super();
    this.actions = actions;
    this.onRun = onRun;
  }

  eq(other: ManifestCodeVisionWidget): boolean {
    return this.actions.map((a) => a.action).join(',') === other.actions.map((a) => a.action).join(',');
  }

  toDOM(): HTMLElement {
    const bar = document.createElement('span');
    bar.className = 'cm-ide-dep-cv-bar';
    bar.dataset.testid = 'ide-dep-codevision-bar';
    for (const act of this.actions) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'cm-ide-dep-cv-link';
      btn.textContent = act.label;
      btn.dataset.depAction = act.action;
      btn.addEventListener('mousedown', (e) => {
        e.preventDefault();
        e.stopPropagation();
        this.onRun(act.action);
      });
      bar.appendChild(btn);
    }
    return bar;
  }
}

class ManagedDepGutterMarker extends GutterMarker {
  target: IdeDepManagedLoc;
  onJump: (loc: IdeDepManagedLoc) => void;

  constructor(target: IdeDepManagedLoc, onJump: (loc: IdeDepManagedLoc) => void) {
    super();
    this.target = target;
    this.onJump = onJump;
  }

  eq(other: ManagedDepGutterMarker): boolean {
    return this.target.path === other.target.path && this.target.line === other.target.line;
  }

  toDOM(): HTMLElement {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'cm-ide-dep-gutter-btn';
    btn.dataset.testid = 'ide-dep-managed-jump';
    const baseName = this.target.path.split('/').pop() ?? this.target.path;
    const tip = `Navigate to managed dependency in ${baseName}:${this.target.line}`;
    btn.title = tip;
    btn.setAttribute('aria-label', tip);
    btn.textContent = '🎯↑';
    btn.addEventListener('mousedown', (e) => {
      e.preventDefault();
      e.stopPropagation();
      this.onJump(this.target);
    });
    return btn;
  }
}

class VulnShieldGutterMarker extends GutterMarker {
  tip: string;

  constructor(tip: string) {
    super();
    this.tip = tip;
  }

  eq(other: VulnShieldGutterMarker): boolean {
    return this.tip === other.tip;
  }

  toDOM(): HTMLElement {
    const s = document.createElement('span');
    s.className = 'cm-ide-dep-shield-marker';
    s.dataset.testid = 'ide-dep-vuln-gutter';
    s.title = this.tip;
    s.setAttribute('role', 'img');
    s.setAttribute('aria-label', this.tip);
    s.textContent = '🛡';
    return s;
  }
}

export interface CreateIdeDepsExtensionOptions {
  filePath: string;
  root: string;
  connectionId?: string | null;
  projectFolders?: string[];
  onApplyContent: (nextContent: string) => void;
  onOpenIntentionMenu?: (line: number, dep: IdeDepItem | null) => void;
  onJumpLocation?: (target: { path: string; line: number; column: number }) => void;
  onAddAgentContext?: () => void;
  onOpenLocalHistory?: () => void;
  onOpenDepAnalyzer?: () => void;
  onToggleBlame?: (() => void) | null;
  onDepsUpdated?: (items: IdeDepItem[]) => void;
}

const depTheme = EditorView.baseTheme({
  '.cm-ide-dep-deprecated': { textDecoration: 'line-through', color: 'var(--text-dim)' },
  '.cm-ide-dep-outdated': { textDecoration: 'underline wavy var(--warning)', textUnderlineOffset: '3px' },
  '.cm-ide-dep-vulnerable': {
    textDecoration: 'underline wavy var(--danger)',
    backgroundColor: 'color-mix(in srgb, var(--danger) 8%, transparent)',
    borderRadius: '2px',
  },
  '.cm-ide-dep-cv-bar': {
    display: 'inline-flex',
    alignItems: 'center',
    gap: '10px',
    marginInlineStart: '12px',
    userSelect: 'none',
  },
  '.cm-ide-dep-cv-link': {
    appearance: 'none',
    border: 'none',
    background: 'transparent',
    color: 'var(--accent-text)',
    fontFamily: 'var(--font-ui)',
    fontSize: 'var(--fs-xs)',
    cursor: 'pointer',
    padding: '0 2px',
  },
  '.cm-ide-dep-cv-link:hover': { textDecoration: 'underline' },
  '.cm-ide-dep-gutter': { width: '18px', userSelect: 'none' },
  '.cm-ide-dep-gutter-btn': {
    appearance: 'none',
    border: 'none',
    background: 'transparent',
    fontSize: 'var(--fs-xs)',
    padding: '0',
    cursor: 'pointer',
    lineHeight: '1',
  },
  '.cm-ide-dep-shield-marker': { fontSize: 'var(--fs-xs)', lineHeight: '1', cursor: 'help' },
});

export function createIdeDepsExtension(opts: CreateIdeDepsExtensionOptions): Extension {
  const eco = detectManifestEcosystem(opts.filePath);
  if (!eco) {
    return [];
  }

  const triggerAction = (view: EditorView, action: string, dep?: IdeDepItem | null, version?: string | null) => {
    void executeIdeDepAction({
      root: opts.root,
      connectionId: opts.connectionId,
      projectFolders: opts.projectFolders,
      path: opts.filePath,
      content: view.state.doc.toString(),
      action,
      dep,
      version,
      deps: getManifestAllDeps(view.state),
    }).then((res) => {
      if (res.items && view.dom.isConnected) {
        view.dispatch({ effects: setManifestDepsEffect.of(res.items) });
        opts.onDepsUpdated?.(res.items);
      }
      if (res.updated_content && res.updated_content !== view.state.doc.toString()) {
        opts.onApplyContent(res.updated_content);
      }
    });
  };

  const depGutter = gutter({
    class: 'cm-ide-dep-gutter',
    markers(view) {
      const deps = getManifestAllDeps(view.state);
      if (!deps.length) {
        return RangeSet.empty;
      }
      const markers: Range<GutterMarker>[] = [];
      for (const d of deps) {
        if (d.line < 1 || d.line > view.state.doc.lines) {
          continue;
        }
        const pos = view.state.doc.line(d.line).from;
        if (d.managed_in && opts.onJumpLocation) {
          const marker = new ManagedDepGutterMarker(d.managed_in, (loc) => {
            opts.onJumpLocation?.({ path: loc.path, line: loc.line, column: 1 });
          });
          markers.push(marker.range(pos));
        } else if (d.vulnerabilities?.length) {
          const tip = `${d.vulnerabilities[0].id}: ${d.vulnerabilities[0].summary}`;
          markers.push(new VulnShieldGutterMarker(tip).range(pos));
        }
      }
      return RangeSet.of(markers.sort((a, b) => a.from - b.from), true);
    },
  });

  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet = Decoration.none;
      private timer: ReturnType<typeof setTimeout> | null = null;
      private abort: AbortController | null = null;

      constructor(view: EditorView) {
        const initial = parseManifestDependencies(opts.filePath, view.state.doc.toString());
        queueMicrotask(() => {
          if (!view.dom.isConnected) {
            return;
          }
          view.dispatch({ effects: setManifestDepsEffect.of(initial) });
          opts.onDepsUpdated?.(initial);
          this.scheduleInspect(view, 40);
        });
        this.decorations = this.buildDecos(view, initial);
      }

      update(u: ViewUpdate) {
        if (u.docChanged) {
          const parsed = parseManifestDependencies(opts.filePath, u.state.doc.toString());
          queueMicrotask(() => {
            if (u.view.dom.isConnected) {
              u.view.dispatch({ effects: setManifestDepsEffect.of(parsed) });
              opts.onDepsUpdated?.(parsed);
            }
          });
          this.scheduleInspect(u.view, 300);
        }
        const depsChanged = u.transactions.some((t) => t.effects.some((e) => e.is(setManifestDepsEffect)));
        if (u.docChanged || u.viewportChanged || depsChanged) {
          this.decorations = this.buildDecos(u.view, getManifestAllDeps(u.state));
        }
      }

      destroy() {
        if (this.timer) {
          clearTimeout(this.timer);
        }
        this.abort?.abort();
      }

      scheduleInspect(view: EditorView, ms: number) {
        if (this.timer) {
          clearTimeout(this.timer);
        }
        this.abort?.abort();
        this.timer = setTimeout(() => {
          const ctrl = new AbortController();
          this.abort = ctrl;
          const snap = view.state.doc.toString();
          void inspectIdeDependencies({
            root: opts.root,
            connectionId: opts.connectionId,
            projectFolders: opts.projectFolders,
            path: opts.filePath,
            content: snap,
            signal: ctrl.signal,
          }).then((items) => {
            if (view.dom.isConnected && view.state.doc.toString() === snap) {
              view.dispatch({ effects: setManifestDepsEffect.of(items) });
              opts.onDepsUpdated?.(items);
            }
          });
        }, ms);
      }

      buildDecos(view: EditorView, deps: IdeDepItem[]): DecorationSet {
        const doc = view.state.doc;
        const ranges: Range<Decoration>[] = [];
        const pushCv = (lineTo: number, actions: Array<{ label: string; action: string }>) => {
          const widget = new ManifestCodeVisionWidget(actions, (act) => triggerAction(view, act));
          ranges.push(Decoration.widget({ widget, side: 1 }).range(lineTo));
        };
        for (let ln = 1; ln <= Math.min(doc.lines, 80); ln++) {
          const lineObj = doc.line(ln);
          const t = lineObj.text.trim();
          if (eco === 'go' && (/^require\s*\(/.test(t) || /^require\s+\S+/.test(t))) {
            pushCv(lineObj.to, [
              { label: '↻ Update all dependencies', action: 'update_all_latest' },
              { label: '↻ Update direct dependencies', action: 'update_direct_latest' },
              { label: '🧹 Go Mod Tidy', action: 'tidy' },
            ]);
            break;
          }
          if (eco === 'maven' && t === '<dependencies>') {
            pushCv(lineObj.to, [
              { label: '↻ Sync Maven', action: 'maven_sync' },
              { label: '🛡 Vulnerability Scan', action: 'vuln_scan' },
            ]);
            break;
          }
          if (eco === 'gradle' && /^dependencies\s*\{/.test(t)) {
            pushCv(lineObj.to, [
              { label: '↻ Sync Gradle', action: 'gradle_sync' },
              { label: '🛡 Vulnerability Scan', action: 'vuln_scan' },
            ]);
            break;
          }
          const isGenericBlock =
            (eco === 'npm' && /^"dependencies"\s*:/.test(t)) ||
            (eco === 'cargo' && t === '[dependencies]') ||
            (eco === 'pypi' &&
              (ln === 1 || /^dependencies\s*=/.test(t) || /^\[tool\.poetry\.dependencies\]/.test(t)));
          if (isGenericBlock) {
            pushCv(lineObj.to, [
              { label: '↻ Update dependencies', action: 'update_all_latest' },
              { label: '🛡 Vulnerability Scan', action: 'vuln_scan' },
            ]);
            break;
          }
        }
        for (const d of deps) {
          const blkStart = Math.max(1, Math.min(doc.lines, d.block_start_line ?? d.line));
          const blkEnd = Math.min(doc.lines, Math.max(blkStart, d.end_line ?? d.block_end_line ?? d.line));
          if (d.line < 1 || d.line > doc.lines) {
            continue;
          }
          const startLn = doc.line(blkStart);
          const endLn = doc.line(blkEnd);
          if (d.deprecated) {
            const artifact = d.name.includes(':') ? d.name.split(':')[1] : d.name;
            let depFrom = -1;
            let depTo = -1;
            for (let l = blkStart; l <= blkEnd; l++) {
              const lo = doc.line(l);
              const idx = lo.text.indexOf(artifact);
              if (idx >= 0) {
                depFrom = lo.from + idx;
                depTo = depFrom + artifact.length;
                break;
              }
            }
            if (depFrom < 0) {
              const anch = doc.line(d.line);
              depFrom = anch.from + (anch.text.length - anch.text.trimStart().length);
              depTo = anch.to;
            }
            if (depTo > depFrom) {
              const mark = Decoration.mark({
                class: 'cm-ide-dep-deprecated',
                attributes: { 'data-testid': 'ide-dep-deprecated' },
              });
              ranges.push(mark.range(depFrom, depTo));
            }
          }
          if (isVersionOutdated(d.current_version, d.latest_version)) {
            for (let l = blkStart; l <= blkEnd; l++) {
              const lo = doc.line(l);
              const vIdx = lo.text.indexOf(d.current_version);
              if (vIdx >= 0) {
                const mark = Decoration.mark({
                  class: 'cm-ide-dep-outdated',
                  attributes: { 'data-testid': 'ide-dep-outdated' },
                });
                ranges.push(mark.range(lo.from + vIdx, lo.from + vIdx + d.current_version.length));
                break;
              }
            }
          }
          if (d.vulnerabilities?.length && endLn.to > startLn.from) {
            const vulnFrom = startLn.from + (startLn.text.length - startLn.text.trimStart().length);
            const mark = Decoration.mark({
              class: 'cm-ide-dep-vulnerable',
              attributes: { 'data-testid': 'ide-dep-vulnerable' },
            });
            ranges.push(mark.range(vulnFrom, endLn.to));
          }
        }
        return ranges.length ? Decoration.set(ranges.sort((a, b) => a.from - b.from), true) : Decoration.none;
      }
    },
    { decorations: (v) => v.decorations },
  );

  const depHover = hoverTooltip(
    (view, pos): Tooltip | null => {
      const ln = view.state.doc.lineAt(pos);
      const dep = getManifestDepAtLine(view.state, ln.number);
      if (!dep) {
        return null;
      }
      const targetVer = dep.vulnerabilities?.find((v) => v.fixed_version)?.fixed_version ?? dep.latest_version ?? null;
      return {
        pos: ln.from,
        end: ln.to,
        above: true,
        create() {
          const dom = document.createElement('div');
          dom.className = 'cm-ide-quick-doc cm-ide-dep-hover';
          dom.dataset.testid = 'ide-dep-hover-card';
          const head = document.createElement('div');
          head.className = 'cm-ide-qd-sig';
          head.textContent = `${dep.name}  ${dep.current_version}${dep.indirect ? '  // indirect' : ''}`;
          dom.appendChild(head);
          const body = document.createElement('div');
          body.className = 'cm-ide-qd-body';
          const notes: string[] = [];
          if (isVersionOutdated(dep.current_version, dep.latest_version)) {
            notes.push(`Newer version is available: ${dep.latest_version}`);
          }
          if (dep.deprecated) {
            notes.push(dep.deprecation_note || `${dep.name} is deprecated.`);
          }
          for (const v of dep.vulnerabilities ?? []) {
            const fixedSuffix = v.fixed_version ? ` — Fixed in ${v.fixed_version}` : '';
            notes.push(`🛡 ${v.id} (${v.severity}): ${v.summary}${fixedSuffix}`);
          }
          if (dep.managed_in) {
            const managedFile = dep.managed_in.path.split('/').pop() ?? dep.managed_in.path;
            notes.push(`🎯↑ Managed in ${managedFile}:${dep.managed_in.line}`);
          }
          body.textContent = notes.length
            ? notes.join('\n')
            : `Declared ${eco} dependency (${dep.current_version}).`;
          dom.appendChild(body);
          const foot = document.createElement('div');
          foot.className = 'cm-ide-qd-footer';
          const mkBtn = (label: string, fn: () => void) => {
            const b = document.createElement('button');
            b.type = 'button';
            b.className = 'cm-ide-qd-btn';
            b.textContent = label;
            b.addEventListener('mousedown', (e) => {
              e.preventDefault();
              e.stopPropagation();
              fn();
            });
            foot.appendChild(b);
          };
          if (targetVer && targetVer !== dep.current_version) {
            mkBtn(`Update to ${targetVer}`, () => triggerAction(view, 'update_one', dep, targetVer));
          }
          mkBtn('Update dependencies... ⌥⇧⏎', () => triggerAction(view, 'update_all_latest', dep));
          mkBtn('More actions... ⌥⏎', () => opts.onOpenIntentionMenu?.(dep.line, dep));
          dom.appendChild(foot);
          return { dom };
        },
      };
    },
    { hoverTime: 220 },
  );

  const ctxHandler = Prec.highest(
    EditorView.domEventHandlers({
      contextmenu(e, view) {
        if (!(e.target instanceof Element) || e.target.closest('.cm-gutters')) {
          return false;
        }
        const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
        const ln =
          pos != null
            ? view.state.doc.lineAt(pos).number
            : view.state.doc.lineAt(view.state.selection.main.head).number;
        const dep = getManifestDepAtLine(view.state, ln);
        const showMain = () => {
          const items: MenuItem[] = [
            {
              label: 'Show Context Actions',
              icon: 'bulb',
              hint: '⌥⏎',
              action: () => opts.onOpenIntentionMenu?.(ln, dep),
            },
            {
              label: 'Add to Agent Context',
              icon: 'assistant',
              action: () => opts.onAddAgentContext?.(),
            },
            ...buildIdeVcsContextRows(e, showMain, {
              filePath: opts.filePath,
              readOnly: view.state.readOnly,
              onToggleBlame: opts.onToggleBlame,
            }),
            { separator: true },
          ];
          if (eco === 'go') {
            items.push(
              { label: 'Go Mod Tidy', icon: 'refresh', action: () => triggerAction(view, 'tidy') },
              { label: 'Go Mod Vendor', icon: 'folder', action: () => triggerAction(view, 'vendor') },
              { label: 'Sync Go Module', icon: 'check', action: () => triggerAction(view, 'sync') },
            );
          } else if (eco === 'maven' || eco === 'gradle') {
            const syncAction = eco === 'maven' ? 'maven_sync' : 'gradle_sync';
            const syncLabel = eco === 'maven' ? 'Sync / Reload Maven' : 'Sync Gradle Project';
            items.push(
              { label: syncLabel, icon: 'refresh', action: () => triggerAction(view, syncAction) },
              { label: 'Dependency Tree', icon: 'layers', action: () => opts.onOpenDepAnalyzer?.() },
            );
          } else {
            items.push(
              { label: 'Update Dependencies', icon: 'refresh', action: () => triggerAction(view, 'update_all_latest') },
              { label: 'Dependency Analyzer', icon: 'layers', action: () => opts.onOpenDepAnalyzer?.() },
            );
          }
          const syn = new MouseEvent('contextmenu', {
            clientX: e.clientX,
            clientY: e.clientY,
            bubbles: true,
          });
          void import('../contextmenu.svelte.ts').then(({ ctxMenu }) => ctxMenu.show(syn, items));
        };
        e.preventDefault();
        showMain();
        return true;
      },
    }),
  );

  return [manifestDepsField, depTheme, Prec.high(depGutter), plugin, depHover, ctxHandler];
}
