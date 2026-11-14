// JetBrains GoLand / IntelliJ Manifest Dependency Intelligence (`go.mod`, `pom.xml`,
// `build.gradle*`, `package.json`, `Cargo.toml`, `requirements.txt`, `pyproject.toml`):
// Top-of-block CodeVision links, inline deprecated/outdated/CVE decorations, managed-dep
// `🎯↑` parent-POM gutter jump, rich hover card with 1-click actions, and manifest context menu.

import { EditorState, Prec, RangeSet, StateEffect, StateField, type Extension, type Range } from '@codemirror/state';
import {
  Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType, gutter, hoverTooltip,
  type DecorationSet, type Tooltip, type ViewUpdate,
} from '@codemirror/view';
import type { MenuItem } from '../contextmenu.svelte.ts';
import { buildIdeVcsContextRows } from './cm-ide-history.ts';

export type ManifestEcosystem = 'go' | 'maven' | 'gradle' | 'npm' | 'cargo' | 'pypi';
export interface IdeDepVuln { id: string; cve?: string | null; severity: 'critical' | 'high' | 'medium' | 'low' | string; summary: string; fixed_version?: string | null; }
export interface IdeDepManagedLoc { path: string; line: number; }
export interface IdeDepItem {
  name: string; current_version: string; latest_version?: string | null; latest_patch?: string | null;
  indirect?: boolean; deprecated?: boolean; deprecation_note?: string | null;
  vulnerabilities?: IdeDepVuln[]; managed_in?: IdeDepManagedLoc | null; property_name?: string | null;
  line: number; end_line?: number; block_start_line?: number; block_end_line?: number; scope?: string | null;
}
export type RawRemoteDepItem = Omit<IdeDepItem, 'deprecated'> & { deprecated?: boolean | string | null };
export interface IdeDepsInspectResponse { ecosystem: string; manifest_path?: string; items?: RawRemoteDepItem[]; dependencies?: RawRemoteDepItem[]; actions?: string[]; }

const MANIFEST_NAMES = new Set(['go.mod', 'pom.xml', 'package.json', 'cargo.toml', 'requirements.txt', 'pyproject.toml', 'build.gradle', 'build.gradle.kts']);

export function isIdeManifestPath(filePath: string): boolean {
  const base = filePath.split('?')[0].split('/').pop()?.toLowerCase() ?? '';
  return MANIFEST_NAMES.has(base) || /^requirements(?:[-_.][\w-]+)?\.txt$/.test(base);
}

export function detectManifestEcosystem(filePath: string): ManifestEcosystem | null {
  const base = filePath.split('?')[0].split('/').pop()?.toLowerCase() ?? '';
  if (base === 'go.mod') return 'go';
  if (base === 'pom.xml') return 'maven';
  if (base === 'build.gradle' || base === 'build.gradle.kts') return 'gradle';
  if (base === 'package.json') return 'npm';
  if (base === 'cargo.toml') return 'cargo';
  return base === 'requirements.txt' || base === 'pyproject.toml' || /^requirements(?:[-_.][\w-]+)?\.txt$/.test(base) ? 'pypi' : null;
}

const KNOWN_DEPRECATED: Record<string, string> = {
  'github.com/golang/protobuf': 'Deprecated: use google.golang.org/protobuf instead.',
  'github.com/pkg/errors': 'Deprecated: use standard library errors.Join / fmt.Errorf("%w") instead.',
  'io/ioutil': 'Deprecated since Go 1.16: use io and os packages.',
  'gopkg.in/yaml.v2': 'Consider upgrading to gopkg.in/yaml.v3.',
  'request': 'Deprecated npm package: use fetch or undici.',
  'tslint': 'Deprecated: use typescript-eslint instead.',
  'junit:junit': 'Legacy JUnit 4: consider org.junit.jupiter:junit-jupiter.',
};

export function isVersionOutdated(current: string, latest?: string | null): boolean {
  if (!latest || !current) return false;
  const c = current.replace(/^[v^~>=<]+/, '').trim(), l = latest.replace(/^[v^~>=<]+/, '').trim();
  return Boolean(l && c && l !== c);
}

export function parseManifestDependencies(filePath: string, content: string): IdeDepItem[] {
  const eco = detectManifestEcosystem(filePath);
  if (!eco || !content) return [];
  const lines = content.split(/\r?\n/), items: IdeDepItem[] = [];
  if (eco === 'go') {
    let inReq = false, pendingDep: string | null = null;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim(), ln = i + 1;
      if (/^\/\/\s*Deprecated:/i.test(t)) { pendingDep = t.replace(/^\/\/\s*/, ''); continue; }
      if (/^require\s*\(\s*$/.test(t)) { inReq = true; pendingDep = null; continue; }
      if (inReq && t.startsWith(')')) { inReq = false; pendingDep = null; continue; }
      const m = inReq ? /^([A-Za-z0-9][\w./-]+)\s+(v[\w.+-]+)(?:\s*\/\/\s*(.*))?$/.exec(t) : /^require\s+([A-Za-z0-9][\w./-]+)\s+(v[\w.+-]+)(?:\s*\/\/\s*(.*))?$/.exec(t);
      if (m) {
        const name = m[1], trail = m[3] ?? '', inlineDep = /deprecated/i.test(trail) ? trail : null, note = pendingDep ?? inlineDep ?? KNOWN_DEPRECATED[name] ?? null;
        items.push({ name, current_version: m[2], indirect: /\bindirect\b/.test(trail), line: ln, end_line: ln, deprecated: Boolean(note), deprecation_note: note, vulnerabilities: [] });
      }
      pendingDep = null;
    }
  } else if (eco === 'maven') {
    let depStart = -1, group = '', artifact = '', version = '', scope = '', inMgmt = false;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim(), ln = i + 1;
      if (t.includes('<dependencyManagement>')) inMgmt = true;
      if (t.includes('</dependencyManagement>')) inMgmt = false;
      if (t.startsWith('<dependency>')) { depStart = ln; group = ''; artifact = ''; version = ''; scope = ''; continue; }
      if (depStart > 0) {
        const mg = /<groupId>([^<]+)<\/groupId>/.exec(t); if (mg) group = mg[1].trim();
        const ma = /<artifactId>([^<]+)<\/artifactId>/.exec(t); if (ma) artifact = ma[1].trim();
        const mv = /<version>([^<]+)<\/version>/.exec(t); if (mv) version = mv[1].trim();
        const ms = /<scope>([^<]+)<\/scope>/.exec(t); if (ms) scope = ms[1].trim();
        if (t.startsWith('</dependency>')) {
          if (group && artifact) {
            const coord = `${group}:${artifact}`, dir = filePath.includes('/') ? filePath.slice(0, filePath.lastIndexOf('/')) : '', parentDir = dir.includes('/') ? dir.slice(0, dir.lastIndexOf('/')) : '';
            items.push({ name: coord, current_version: version || 'managed', line: depStart, end_line: ln, scope: scope || (inMgmt ? 'management' : null), deprecated: Boolean(KNOWN_DEPRECATED[coord]), deprecation_note: KNOWN_DEPRECATED[coord] ?? null, managed_in: !version && !inMgmt && parentDir ? { path: `${parentDir}/pom.xml`, line: 1 } : null, vulnerabilities: [] });
          }
          depStart = -1;
        }
      }
    }
  } else if (eco === 'npm') {
    let sec: string | null = null;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim();
      const ln = i + 1;
      const mSec = /^"(dependencies|devDependencies|peerDependencies)"\s*:\s*\{/.exec(t);
      if (mSec) {
        sec = t.includes('}') ? null : mSec[1];
        continue;
      }
      if (sec && t.startsWith('}')) {
        sec = null;
        continue;
      }
      const mDep = sec ? /^"(@?[^"]+)"\s*:\s*"([^"]+)"/.exec(t) : null;
      if (mDep) {
        items.push({
          name: mDep[1],
          current_version: mDep[2],
          indirect: sec !== 'dependencies',
          scope: sec,
          line: ln,
          end_line: ln,
          deprecated: Boolean(KNOWN_DEPRECATED[mDep[1]]),
          deprecation_note: KNOWN_DEPRECATED[mDep[1]] ?? null,
          vulnerabilities: [],
        });
      }
    }
  } else if (eco === 'cargo') {
    let inDeps = false;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim(), ln = i + 1;
      if (/^\[(?:dev-|build-)?dependencies\]$/i.test(t)) { inDeps = true; continue; }
      if (t.startsWith('[')) { inDeps = false; continue; }
      const m = inDeps ? /^([A-Za-z0-9_-]+)\s*=\s*(?:"([^"]+)"|\{[^}]*version\s*=\s*"([^"]+)")/.exec(t) : null;
      if (m) items.push({ name: m[1], current_version: m[2] || m[3] || '*', line: ln, end_line: ln, vulnerabilities: [] });
    }
  } else if (eco === 'gradle') {
    for (let i = 0; i < lines.length; i++) {
      const m = /^\s*(?:implementation|api|compileOnly|runtimeOnly|testImplementation)\s*\(?\s*['"]([^:'"]+):([^:'"]+):([^'"]+)['"]/.exec(lines[i]);
      if (m) items.push({ name: `${m[1]}:${m[2]}`, current_version: m[3], line: i + 1, end_line: i + 1, vulnerabilities: [] });
    }
  } else if (eco === 'pypi') {
    const isPyProject = filePath.toLowerCase().endsWith('pyproject.toml');
    let inPoetry = false, inProjDeps = false;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim(), ln = i + 1;
      if (!t || t.startsWith('#')) continue;
      if (isPyProject) {
        if (/^\[tool\.poetry(?:\.group\.\w+)?\.dependencies\]$/i.test(t)) { inPoetry = true; inProjDeps = false; continue; }
        if (t.startsWith('[')) { inPoetry = false; inProjDeps = false; continue; }
        if (/^dependencies\s*=\s*\[/.test(t)) { inProjDeps = !t.includes(']'); continue; }
        if (inProjDeps && t.startsWith(']')) { inProjDeps = false; continue; }
        const mp = inPoetry ? /^([A-Za-z0-9_.-]+)\s*=\s*(?:"([^"]+)"|\{[^}]*version\s*=\s*"([^"]+)")/.exec(t) : inProjDeps ? /^['"]([A-Za-z0-9_.-]+)\s*(?:==|>=|~=|<=|\^)\s*([\w.*+-]+)['"]/.exec(t) : null;
        if (mp && mp[1] !== 'python') items.push({ name: mp[1], current_version: mp[2] || mp[3] || '*', line: ln, end_line: ln, vulnerabilities: [] });
      } else {
        const m = /^([A-Za-z0-9_.-]+)\s*(?:==|>=|~=|<=)\s*([\w.*+-]+)/.exec(t);
        if (m) items.push({ name: m[1], current_version: m[2], line: ln, end_line: ln, vulnerabilities: [] });
      }
    }
  }
  return items;
}

export function normalizeRemoteDepItem(r: RawRemoteDepItem): IdeDepItem {
  const depMsg = typeof r.deprecated === 'string' ? (r.deprecated.trim() || null) : (r.deprecation_note ?? null);
  const startLine = r.block_start_line ?? r.line, endLine = r.end_line ?? r.block_end_line ?? r.line;
  return { ...r, line: r.line >= 1 ? r.line : startLine, block_start_line: startLine, block_end_line: endLine, end_line: endLine, latest_patch: r.latest_patch ?? null, deprecated: Boolean(r.deprecated), deprecation_note: depMsg, vulnerabilities: r.vulnerabilities ?? [], managed_in: r.managed_in ?? null };
}
export function mergeDepItems(local: IdeDepItem[], rawRemote: RawRemoteDepItem[]): IdeDepItem[] {
  if (!rawRemote.length) return local;
  const remote = rawRemote.map(normalizeRemoteDepItem), byKey = new Map(remote.map((r) => [`${r.name}@${r.line}`, r] as const)), byPkg = new Map<string, IdeDepItem>();
  for (const r of remote) if (!byPkg.has(r.name)) byPkg.set(r.name, r);
  const merged = local.map((loc) => {
    const hit = byKey.get(`${loc.name}@${loc.line}`) ?? byPkg.get(loc.name);
    if (!hit) return loc;
    const blockStart = hit.block_start_line ?? loc.block_start_line ?? loc.line, blockEnd = Math.max(hit.end_line ?? hit.line, loc.end_line ?? loc.line);
    return { ...loc, ...hit, line: hit.line >= 1 ? hit.line : loc.line, block_start_line: blockStart, block_end_line: blockEnd, end_line: blockEnd, latest_patch: hit.latest_patch ?? loc.latest_patch ?? null, deprecated: Boolean(hit.deprecated || loc.deprecated), deprecation_note: hit.deprecation_note ?? loc.deprecation_note ?? null, vulnerabilities: hit.vulnerabilities?.length ? hit.vulnerabilities : (loc.vulnerabilities ?? []), managed_in: hit.managed_in ?? loc.managed_in ?? null };
  });
  for (const r of remote) if (!merged.some((m) => m.name === r.name && (m.line === r.line || (m.block_start_line ?? m.line) === (r.block_start_line ?? r.line)))) merged.push(r);
  return merged;
}
export async function inspectIdeDependencies(opts: { root: string; connectionId?: string | null; projectFolders?: string[]; path: string; content: string; signal?: AbortSignal }): Promise<IdeDepItem[]> {
  const local = parseManifestDependencies(opts.path, opts.content);
  if (!opts.root) return local;
  try {
    const { api } = await import('../api/client.ts');
    const resp = await api.post<IdeDepsInspectResponse>('/ide/deps/inspect', { target: { kind: opts.connectionId ? 'ssh' : 'local', root: opts.root, connection_id: opts.connectionId ?? null, additional_roots: opts.projectFolders }, path: opts.path, content: opts.content }, opts.signal);
    return mergeDepItems(local, resp.items ?? resp.dependencies ?? []);
  } catch { return local; }
}
export function normalizeIdeDepAction(action: string, eco?: ManifestEcosystem | null): string {
  if (action === 'update_single' || action === 'upgrade') return 'update_one';
  if (action === 'update_all') return 'update_all_latest';
  if (action === 'update_direct') return 'update_direct_latest';
  return action === 'sync' && eco === 'maven' ? 'maven_sync' : action === 'sync' && eco === 'gradle' ? 'gradle_sync' : action;
}
export function applyFallbackManifestVersionUpdate(content: string, dep: IdeDepItem, nextVersion: string): string {
  if (dep.property_name) {
    const esc = dep.property_name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), propRe = new RegExp(`(<${esc}>)([^<]+)(</${esc}>)`);
    if (propRe.test(content)) return content.replace(propRe, `$1${nextVersion}$3`);
  }
  const lines = content.split('\n'), s = Math.max(0, (dep.block_start_line ?? dep.line) - 1), e = Math.min(lines.length - 1, Math.max(s, (dep.end_line ?? dep.block_end_line ?? dep.line) - 1));
  for (let i = s; i <= e; i++) if (dep.current_version && dep.current_version !== 'managed' && lines[i].includes(dep.current_version)) { lines[i] = lines[i].replace(dep.current_version, nextVersion); return lines.join('\n'); }
  return content;
}
export function applyBatchFallbackManifestUpdates(content: string, deps: IdeDepItem[], mode: string): { updated: string; count: number } {
  const directOnly = mode.includes('direct'), patchOnly = mode.includes('patch');
  let cur = content, count = 0;
  for (const d of [...deps].sort((a, b) => b.line - a.line)) {
    if (directOnly && d.indirect) continue;
    const fixedVer = d.vulnerabilities?.find((v) => v.fixed_version)?.fixed_version ?? null;
    const target = patchOnly ? (d.latest_patch ?? fixedVer) : (d.latest_version ?? d.latest_patch ?? fixedVer);
    if (!target || !isVersionOutdated(d.current_version, target)) continue;
    const next = applyFallbackManifestVersionUpdate(cur, d, target);
    if (next !== cur) { cur = next; count++; }
  }
  return { updated: cur, count };
}
export async function executeIdeDepAction(opts: { root: string; connectionId?: string | null; projectFolders?: string[]; path: string; content: string; action: string; dep?: IdeDepItem | null; version?: string | null; deps?: IdeDepItem[] }): Promise<{ ok: boolean; updated_content?: string; output?: string; items?: IdeDepItem[] }> {
  const [{ api }, { toasts }] = await Promise.all([import('../api/client.ts'), import('../toast.svelte.ts')]);
  const eco = detectManifestEcosystem(opts.path);
  if (opts.action === 'vuln_scan') {
    const items = await inspectIdeDependencies({ root: opts.root, connectionId: opts.connectionId, projectFolders: opts.projectFolders, path: opts.path, content: opts.content });
    const cveCount = items.reduce((sum, d) => sum + (d.vulnerabilities?.length ?? 0), 0);
    if (cveCount > 0) toasts.error('Vulnerability scan complete', `${cveCount} security advisories found in manifest.`);
    else toasts.success('Vulnerability scan complete', '0 known CVEs found in manifest dependencies.');
    return { ok: true, items, output: `${cveCount} vulnerabilities` };
  }
  const normAction = normalizeIdeDepAction(opts.action, eco);
  try {
    const resp = await api.post<{ ok?: boolean; updated_content?: string; content?: string; output?: string; stdout?: string }>('/ide/deps/action', { target: { kind: opts.connectionId ? 'ssh' : 'local', root: opts.root, connection_id: opts.connectionId ?? null, additional_roots: opts.projectFolders }, path: opts.path, action: normAction, module: opts.dep?.name ?? undefined, version: opts.version ?? undefined, content: opts.content });
    const ok = resp.ok !== false, out = resp.output ?? resp.stdout, updated_content = resp.updated_content ?? resp.content;
    if (ok) { if (normAction === 'why') toasts.info(`go mod why ${opts.dep?.name ?? ''}`.trim(), (out ?? '').trim().slice(0, 220) || 'Module is required by build graph.'); else toasts.success('Dependency action complete', out?.slice(0, 120) || normAction); }
    return { ok, updated_content, output: out };
  } catch {
    if (normAction === 'update_one' && opts.dep && opts.version) {
      const updated = applyFallbackManifestVersionUpdate(opts.content, opts.dep, opts.version);
      if (updated !== opts.content) { toasts.success(`Updated ${opts.dep.name}`, `${opts.dep.current_version} → ${opts.version}`); return { ok: true, updated_content: updated }; }
    } else if (normAction === 'update_all_latest' || normAction === 'update_all_patch' || normAction === 'update_direct_latest' || normAction === 'update_direct_patch') {
      const { updated, count } = applyBatchFallbackManifestUpdates(opts.content, opts.deps?.length ? opts.deps : parseManifestDependencies(opts.path, opts.content), normAction);
      if (count > 0 && updated !== opts.content) { toasts.success(`Updated ${count} dependencies`, normAction); return { ok: true, updated_content: updated }; }
    }
    toasts.info('Dependency action queued', `Run ${normAction} when toolchain is ready.`);
    return { ok: false };
  }
}

const setManifestDepsEffect = StateEffect.define<IdeDepItem[]>();
const manifestDepsField = StateField.define<IdeDepItem[]>({ create: () => [], update: (v, tr) => { for (const e of tr.effects) if (e.is(setManifestDepsEffect)) return e.value; return v; } });
export const getManifestAllDeps = (state: EditorState): IdeDepItem[] => state.field(manifestDepsField, false) ?? [];
export const getManifestDepAtLine = (state: EditorState, line: number): IdeDepItem | null => getManifestAllDeps(state).find((d) => line >= (d.block_start_line ?? d.line) && line <= (d.end_line ?? d.block_end_line ?? d.line)) ?? null;

class ManifestCodeVisionWidget extends WidgetType {
  actions: Array<{ label: string; action: string }>; onRun: (action: string) => void;
  constructor(actions: Array<{ label: string; action: string }>, onRun: (action: string) => void) { super(); this.actions = actions; this.onRun = onRun; }
  eq(o: ManifestCodeVisionWidget): boolean { return this.actions.map((a) => a.action).join(',') === o.actions.map((a) => a.action).join(','); }
  toDOM(): HTMLElement {
    const bar = document.createElement('span'); bar.className = 'cm-ide-dep-cv-bar'; bar.dataset.testid = 'ide-dep-codevision-bar';
    for (const act of this.actions) {
      const btn = document.createElement('button'); btn.type = 'button'; btn.className = 'cm-ide-dep-cv-link'; btn.textContent = act.label; btn.dataset.depAction = act.action;
      btn.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); this.onRun(act.action); });
      bar.appendChild(btn);
    }
    return bar;
  }
}

class ManagedDepGutterMarker extends GutterMarker {
  target: IdeDepManagedLoc; onJump: (loc: IdeDepManagedLoc) => void;
  constructor(target: IdeDepManagedLoc, onJump: (loc: IdeDepManagedLoc) => void) { super(); this.target = target; this.onJump = onJump; }
  eq(o: ManagedDepGutterMarker): boolean { return this.target.path === o.target.path && this.target.line === o.target.line; }
  toDOM(): HTMLElement {
    const btn = document.createElement('button'); btn.type = 'button'; btn.className = 'cm-ide-dep-gutter-btn'; btn.dataset.testid = 'ide-dep-managed-jump';
    const tip = `Navigate to managed dependency in ${this.target.path.split('/').pop() ?? this.target.path}:${this.target.line}`;
    btn.title = tip; btn.setAttribute('aria-label', tip); btn.textContent = '🎯↑';
    btn.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); this.onJump(this.target); });
    return btn;
  }
}

class VulnShieldGutterMarker extends GutterMarker {
  tip: string;
  constructor(tip: string) { super(); this.tip = tip; }
  eq(o: VulnShieldGutterMarker): boolean { return this.tip === o.tip; }
  toDOM(): HTMLElement {
    const s = document.createElement('span'); s.className = 'cm-ide-dep-shield-marker'; s.dataset.testid = 'ide-dep-vuln-gutter';
    s.title = this.tip; s.setAttribute('role', 'img'); s.setAttribute('aria-label', this.tip); s.textContent = '🛡';
    return s;
  }
}

export interface CreateIdeDepsExtensionOptions {
  filePath: string; root: string; connectionId?: string | null; projectFolders?: string[];
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
  '.cm-ide-dep-vulnerable': { textDecoration: 'underline wavy var(--danger)', backgroundColor: 'color-mix(in srgb, var(--danger) 8%, transparent)', borderRadius: '2px' },
  '.cm-ide-dep-cv-bar': { display: 'inline-flex', alignItems: 'center', gap: '10px', marginInlineStart: '12px', userSelect: 'none' },
  '.cm-ide-dep-cv-link': { appearance: 'none', border: 'none', background: 'transparent', color: 'var(--accent-text)', fontFamily: 'var(--font-ui)', fontSize: 'var(--fs-xs)', cursor: 'pointer', padding: '0 2px' },
  '.cm-ide-dep-cv-link:hover': { textDecoration: 'underline' },
  '.cm-ide-dep-gutter': { width: '18px', userSelect: 'none' },
  '.cm-ide-dep-gutter-btn': { appearance: 'none', border: 'none', background: 'transparent', fontSize: 'var(--fs-xs)', padding: '0', cursor: 'pointer', lineHeight: '1' },
  '.cm-ide-dep-shield-marker': { fontSize: 'var(--fs-xs)', lineHeight: '1', cursor: 'help' },
});

export function createIdeDepsExtension(opts: CreateIdeDepsExtensionOptions): Extension {
  const eco = detectManifestEcosystem(opts.filePath);
  if (!eco) return [];
  const triggerAction = (view: EditorView, action: string, dep?: IdeDepItem | null, version?: string | null) => {
    void executeIdeDepAction({ root: opts.root, connectionId: opts.connectionId, projectFolders: opts.projectFolders, path: opts.filePath, content: view.state.doc.toString(), action, dep, version, deps: getManifestAllDeps(view.state) }).then((res) => {
      if (res.items && view.dom.isConnected) { view.dispatch({ effects: setManifestDepsEffect.of(res.items) }); opts.onDepsUpdated?.(res.items); }
      if (res.updated_content && res.updated_content !== view.state.doc.toString()) opts.onApplyContent(res.updated_content);
    });
  };
  const depGutter = gutter({
    class: 'cm-ide-dep-gutter',
    markers(view) {
      const deps = getManifestAllDeps(view.state);
      if (!deps.length) return RangeSet.empty;
      const markers: Range<GutterMarker>[] = [];
      for (const d of deps) {
        if (d.line < 1 || d.line > view.state.doc.lines) continue;
        const pos = view.state.doc.line(d.line).from;
        if (d.managed_in && opts.onJumpLocation) markers.push(new ManagedDepGutterMarker(d.managed_in, (loc) => opts.onJumpLocation?.({ path: loc.path, line: loc.line, column: 1 })).range(pos));
        else if (d.vulnerabilities?.length) markers.push(new VulnShieldGutterMarker(`${d.vulnerabilities[0].id}: ${d.vulnerabilities[0].summary}`).range(pos));
      }
      return RangeSet.of(markers.sort((a, b) => a.from - b.from), true);
    },
  });

  const plugin = ViewPlugin.fromClass(class {
    decorations: DecorationSet = Decoration.none;
    private timer: ReturnType<typeof setTimeout> | null = null;
    private abort: AbortController | null = null;
    constructor(view: EditorView) {
      const initial = parseManifestDependencies(opts.filePath, view.state.doc.toString());
      queueMicrotask(() => {
        if (!view.dom.isConnected) return;
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
      if (this.timer) clearTimeout(this.timer);
      this.abort?.abort();
    }
    scheduleInspect(view: EditorView, ms: number) {
      if (this.timer) clearTimeout(this.timer);
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
          (eco === 'pypi' && (ln === 1 || /^dependencies\s*=/.test(t) || /^\[tool\.poetry\.dependencies\]/.test(t)));
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
        if (d.line < 1 || d.line > doc.lines) continue;
        const startLn = doc.line(blkStart), endLn = doc.line(blkEnd);
        if (d.deprecated) {
          const artifact = d.name.includes(':') ? d.name.split(':')[1] : d.name;
          let depFrom = -1, depTo = -1;
          for (let l = blkStart; l <= blkEnd; l++) {
            const lo = doc.line(l), idx = lo.text.indexOf(artifact);
            if (idx >= 0) { depFrom = lo.from + idx; depTo = depFrom + artifact.length; break; }
          }
          if (depFrom < 0) { const anch = doc.line(d.line); depFrom = anch.from + (anch.text.length - anch.text.trimStart().length); depTo = anch.to; }
          if (depTo > depFrom) ranges.push(Decoration.mark({ class: 'cm-ide-dep-deprecated', attributes: { 'data-testid': 'ide-dep-deprecated' } }).range(depFrom, depTo));
        }
        if (isVersionOutdated(d.current_version, d.latest_version)) {
          for (let l = blkStart; l <= blkEnd; l++) {
            const lo = doc.line(l), vIdx = lo.text.indexOf(d.current_version);
            if (vIdx >= 0) { ranges.push(Decoration.mark({ class: 'cm-ide-dep-outdated', attributes: { 'data-testid': 'ide-dep-outdated' } }).range(lo.from + vIdx, lo.from + vIdx + d.current_version.length)); break; }
          }
        }
        if (d.vulnerabilities?.length && endLn.to > startLn.from) {
          ranges.push(Decoration.mark({ class: 'cm-ide-dep-vulnerable', attributes: { 'data-testid': 'ide-dep-vulnerable' } }).range(startLn.from + (startLn.text.length - startLn.text.trimStart().length), endLn.to));
        }
      }
      return ranges.length ? Decoration.set(ranges.sort((a, b) => a.from - b.from), true) : Decoration.none;
    }
  }, { decorations: (v) => v.decorations });

  const depHover = hoverTooltip((view, pos): Tooltip | null => {
    const ln = view.state.doc.lineAt(pos), dep = getManifestDepAtLine(view.state, ln.number);
    if (!dep) return null;
    const targetVer = dep.vulnerabilities?.find((v) => v.fixed_version)?.fixed_version ?? dep.latest_version ?? null;
    return {
      pos: ln.from, end: ln.to, above: true,
      create() {
        const dom = document.createElement('div'); dom.className = 'cm-ide-quick-doc cm-ide-dep-hover'; dom.dataset.testid = 'ide-dep-hover-card';
        const head = document.createElement('div'); head.className = 'cm-ide-qd-sig'; head.textContent = `${dep.name}  ${dep.current_version}${dep.indirect ? '  // indirect' : ''}`; dom.appendChild(head);
        const body = document.createElement('div'); body.className = 'cm-ide-qd-body';
        const notes: string[] = [];
        if (isVersionOutdated(dep.current_version, dep.latest_version)) notes.push(`Newer version is available: ${dep.latest_version}`);
        if (dep.deprecated) notes.push(dep.deprecation_note || `${dep.name} is deprecated.`);
        for (const v of dep.vulnerabilities ?? []) notes.push(`🛡 ${v.id} (${v.severity}): ${v.summary}${v.fixed_version ? ` — Fixed in ${v.fixed_version}` : ''}`);
        if (dep.managed_in) notes.push(`🎯↑ Managed in ${dep.managed_in.path.split('/').pop()}:${dep.managed_in.line}`);
        body.textContent = notes.length ? notes.join('\n') : `Declared ${eco} dependency (${dep.current_version}).`; dom.appendChild(body);
        const foot = document.createElement('div'); foot.className = 'cm-ide-qd-footer';
        const mkBtn = (label: string, fn: () => void) => {
          const b = document.createElement('button'); b.type = 'button'; b.className = 'cm-ide-qd-btn'; b.textContent = label;
          b.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); fn(); }); foot.appendChild(b);
        };
        if (targetVer && targetVer !== dep.current_version) mkBtn(`Update to ${targetVer}`, () => triggerAction(view, 'update_one', dep, targetVer));
        mkBtn('Update dependencies... ⌥⇧⏎', () => triggerAction(view, 'update_all_latest', dep));
        mkBtn('More actions... ⌥⏎', () => opts.onOpenIntentionMenu?.(dep.line, dep));
        dom.appendChild(foot);
        return { dom };
      },
    };
  }, { hoverTime: 220 });

  const ctxHandler = Prec.highest(EditorView.domEventHandlers({
    contextmenu(e, view) {
      if (!(e.target instanceof Element) || e.target.closest('.cm-gutters')) return false;
      const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
      const ln = pos != null ? view.state.doc.lineAt(pos).number : view.state.doc.lineAt(view.state.selection.main.head).number;
      const dep = getManifestDepAtLine(view.state, ln);
      const showMain = () => {
        const items: MenuItem[] = [
          { label: 'Show Context Actions', icon: 'bulb', hint: '⌥⏎', action: () => opts.onOpenIntentionMenu?.(ln, dep) },
          { label: 'Add to Agent Context', icon: 'assistant', action: () => opts.onAddAgentContext?.() },
          ...buildIdeVcsContextRows(e, showMain, { filePath: opts.filePath, readOnly: view.state.readOnly, onToggleBlame: opts.onToggleBlame }),
          { separator: true },
        ];
        if (eco === 'go') {
          items.push(
            { label: 'Go Mod Tidy', icon: 'refresh', action: () => triggerAction(view, 'tidy') },
            { label: 'Go Mod Vendor', icon: 'folder', action: () => triggerAction(view, 'vendor') },
            { label: 'Sync Go Module', icon: 'check', action: () => triggerAction(view, 'sync') },
          );
        } else if (eco === 'maven' || eco === 'gradle') {
          items.push(
            { label: eco === 'maven' ? 'Sync / Reload Maven' : 'Sync Gradle Project', icon: 'refresh', action: () => triggerAction(view, eco === 'maven' ? 'maven_sync' : 'gradle_sync') },
            { label: 'Dependency Tree', icon: 'layers', action: () => opts.onOpenDepAnalyzer?.() },
          );
        } else {
          items.push({ label: 'Update Dependencies', icon: 'refresh', action: () => triggerAction(view, 'update_all_latest') }, { label: 'Dependency Analyzer', icon: 'layers', action: () => opts.onOpenDepAnalyzer?.() });
        }
        const syn = new MouseEvent('contextmenu', { clientX: e.clientX, clientY: e.clientY, bubbles: true });
        void import('../contextmenu.svelte.ts').then(({ ctxMenu }) => ctxMenu.show(syn, items));
      };
      e.preventDefault();
      showMain();
      return true;
    },
  }));

  return [manifestDepsField, depTheme, Prec.high(depGutter), plugin, depHover, ctxHandler];
}
