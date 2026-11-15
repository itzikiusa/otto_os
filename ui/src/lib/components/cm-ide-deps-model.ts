// Pure manifest dependency parser, remote advisory merger, version-update fallbacks,
// and backend `/ide/deps/*` client helpers (`go.mod`, `pom.xml`, `build.gradle*`,
// `package.json`, `Cargo.toml`, `requirements.txt`, `pyproject.toml`).

export type ManifestEcosystem = 'go' | 'maven' | 'gradle' | 'npm' | 'cargo' | 'pypi';

export interface IdeDepVuln {
  id: string;
  cve?: string | null;
  severity: 'critical' | 'high' | 'medium' | 'low' | string;
  summary: string;
  fixed_version?: string | null;
}

export interface IdeDepManagedLoc {
  path: string;
  line: number;
}

export interface IdeDepItem {
  name: string;
  current_version: string;
  latest_version?: string | null;
  latest_patch?: string | null;
  indirect?: boolean;
  deprecated?: boolean;
  deprecation_note?: string | null;
  vulnerabilities?: IdeDepVuln[];
  managed_in?: IdeDepManagedLoc | null;
  property_name?: string | null;
  line: number;
  end_line?: number;
  block_start_line?: number;
  block_end_line?: number;
  scope?: string | null;
}

export type RawRemoteDepItem = Omit<IdeDepItem, 'deprecated'> & {
  deprecated?: boolean | string | null;
};

export interface IdeDepsInspectResponse {
  ecosystem: string;
  manifest_path?: string;
  items?: RawRemoteDepItem[];
  dependencies?: RawRemoteDepItem[];
  actions?: string[];
}

const MANIFEST_NAMES = new Set([
  'go.mod',
  'pom.xml',
  'package.json',
  'cargo.toml',
  'requirements.txt',
  'pyproject.toml',
  'build.gradle',
  'build.gradle.kts',
]);

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
  if (
    base === 'requirements.txt' ||
    base === 'pyproject.toml' ||
    /^requirements(?:[-_.][\w-]+)?\.txt$/.test(base)
  ) {
    return 'pypi';
  }
  return null;
}

const KNOWN_DEPRECATED: Record<string, string> = {
  'github.com/golang/protobuf': 'Deprecated: use google.golang.org/protobuf instead.',
  'github.com/pkg/errors': 'Deprecated: use standard library errors.Join / fmt.Errorf("%w") instead.',
  'io/ioutil': 'Deprecated since Go 1.16: use io and os packages.',
  'gopkg.in/yaml.v2': 'Consider upgrading to gopkg.in/yaml.v3.',
  request: 'Deprecated npm package: use fetch or undici.',
  tslint: 'Deprecated: use typescript-eslint instead.',
  'junit:junit': 'Legacy JUnit 4: consider org.junit.jupiter:junit-jupiter.',
};

export function isVersionOutdated(current: string, latest?: string | null): boolean {
  if (!latest || !current) return false;
  const currentClean = current.replace(/^[v^~>=<]+/, '').trim();
  const latestClean = latest.replace(/^[v^~>=<]+/, '').trim();
  return Boolean(latestClean && currentClean && latestClean !== currentClean);
}

export function parseManifestDependencies(filePath: string, content: string): IdeDepItem[] {
  const eco = detectManifestEcosystem(filePath);
  if (!eco || !content) return [];
  const lines = content.split(/\r?\n/);
  const items: IdeDepItem[] = [];

  if (eco === 'go') {
    let inReq = false;
    let pendingDep: string | null = null;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim();
      const ln = i + 1;
      if (/^\/\/\s*Deprecated:/i.test(t)) {
        pendingDep = t.replace(/^\/\/\s*/, '');
        continue;
      }
      if (/^require\s*\(\s*$/.test(t)) {
        inReq = true;
        pendingDep = null;
        continue;
      }
      if (inReq && t.startsWith(')')) {
        inReq = false;
        pendingDep = null;
        continue;
      }
      const m = inReq
        ? /^([A-Za-z0-9][\w./-]+)\s+(v[\w.+-]+)(?:\s*\/\/\s*(.*))?$/.exec(t)
        : /^require\s+([A-Za-z0-9][\w./-]+)\s+(v[\w.+-]+)(?:\s*\/\/\s*(.*))?$/.exec(t);
      if (m) {
        const name = m[1];
        const trail = m[3] ?? '';
        const inlineDep = /deprecated/i.test(trail) ? trail : null;
        const note = pendingDep ?? inlineDep ?? KNOWN_DEPRECATED[name] ?? null;
        items.push({
          name,
          current_version: m[2],
          indirect: /\bindirect\b/.test(trail),
          line: ln,
          end_line: ln,
          deprecated: Boolean(note),
          deprecation_note: note,
          vulnerabilities: [],
        });
      }
      pendingDep = null;
    }
  } else if (eco === 'maven') {
    let depStart = -1;
    let group = '';
    let artifact = '';
    let version = '';
    let scope = '';
    let inMgmt = false;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim();
      const ln = i + 1;
      if (t.includes('<dependencyManagement>')) {
        inMgmt = true;
      }
      if (t.includes('</dependencyManagement>')) {
        inMgmt = false;
      }
      if (t.startsWith('<dependency>')) {
        depStart = ln;
        group = '';
        artifact = '';
        version = '';
        scope = '';
        continue;
      }
      if (depStart > 0) {
        const mg = /<groupId>([^<]+)<\/groupId>/.exec(t);
        if (mg) {
          group = mg[1].trim();
        }
        const ma = /<artifactId>([^<]+)<\/artifactId>/.exec(t);
        if (ma) {
          artifact = ma[1].trim();
        }
        const mv = /<version>([^<]+)<\/version>/.exec(t);
        if (mv) {
          version = mv[1].trim();
        }
        const ms = /<scope>([^<]+)<\/scope>/.exec(t);
        if (ms) {
          scope = ms[1].trim();
        }
        if (t.startsWith('</dependency>')) {
          if (group && artifact) {
            const coord = `${group}:${artifact}`;
            const dir = filePath.includes('/') ? filePath.slice(0, filePath.lastIndexOf('/')) : '';
            const parentDir = dir.includes('/') ? dir.slice(0, dir.lastIndexOf('/')) : '';
            items.push({
              name: coord,
              current_version: version || 'managed',
              line: depStart,
              end_line: ln,
              scope: scope || (inMgmt ? 'management' : null),
              deprecated: Boolean(KNOWN_DEPRECATED[coord]),
              deprecation_note: KNOWN_DEPRECATED[coord] ?? null,
              managed_in: !version && !inMgmt && parentDir ? { path: `${parentDir}/pom.xml`, line: 1 } : null,
              vulnerabilities: [],
            });
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
      const t = lines[i].trim();
      const ln = i + 1;
      if (/^\[(?:dev-|build-)?dependencies\]$/i.test(t)) {
        inDeps = true;
        continue;
      }
      if (t.startsWith('[')) {
        inDeps = false;
        continue;
      }
      const m = inDeps
        ? /^([A-Za-z0-9_-]+)\s*=\s*(?:"([^"]+)"|\{[^}]*version\s*=\s*"([^"]+)")/.exec(t)
        : null;
      if (m) {
        items.push({
          name: m[1],
          current_version: m[2] || m[3] || '*',
          line: ln,
          end_line: ln,
          vulnerabilities: [],
        });
      }
    }
  } else if (eco === 'gradle') {
    for (let i = 0; i < lines.length; i++) {
      const m = /^\s*(?:implementation|api|compileOnly|runtimeOnly|testImplementation)\s*\(?\s*['"]([^:'"]+):([^:'"]+):([^'"]+)['"]/.exec(
        lines[i],
      );
      if (m) {
        items.push({
          name: `${m[1]}:${m[2]}`,
          current_version: m[3],
          line: i + 1,
          end_line: i + 1,
          vulnerabilities: [],
        });
      }
    }
  } else if (eco === 'pypi') {
    const isPyProject = filePath.toLowerCase().endsWith('pyproject.toml');
    let inPoetry = false;
    let inProjDeps = false;
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim();
      const ln = i + 1;
      if (!t || t.startsWith('#')) {
        continue;
      }
      if (isPyProject) {
        if (/^\[tool\.poetry(?:\.group\.\w+)?\.dependencies\]$/i.test(t)) {
          inPoetry = true;
          inProjDeps = false;
          continue;
        }
        if (t.startsWith('[')) {
          inPoetry = false;
          inProjDeps = false;
          continue;
        }
        if (/^dependencies\s*=\s*\[/.test(t)) {
          inProjDeps = !t.includes(']');
          continue;
        }
        if (inProjDeps && t.startsWith(']')) {
          inProjDeps = false;
          continue;
        }
        const mp = inPoetry
          ? /^([A-Za-z0-9_.-]+)\s*=\s*(?:"([^"]+)"|\{[^}]*version\s*=\s*"([^"]+)")/.exec(t)
          : inProjDeps
            ? /^['"]([A-Za-z0-9_.-]+)\s*(?:==|>=|~=|<=|\^)\s*([\w.*+-]+)['"]/.exec(t)
            : null;
        if (mp && mp[1] !== 'python') {
          items.push({
            name: mp[1],
            current_version: mp[2] || mp[3] || '*',
            line: ln,
            end_line: ln,
            vulnerabilities: [],
          });
        }
      } else {
        const m = /^([A-Za-z0-9_.-]+)\s*(?:==|>=|~=|<=)\s*([\w.*+-]+)/.exec(t);
        if (m) {
          items.push({
            name: m[1],
            current_version: m[2],
            line: ln,
            end_line: ln,
            vulnerabilities: [],
          });
        }
      }
    }
  }
  return items;
}

export function normalizeRemoteDepItem(r: RawRemoteDepItem): IdeDepItem {
  const depMsg =
    typeof r.deprecated === 'string' ? r.deprecated.trim() || null : (r.deprecation_note ?? null);
  const startLine = r.block_start_line ?? r.line;
  const endLine = r.end_line ?? r.block_end_line ?? r.line;
  return {
    ...r,
    line: r.line >= 1 ? r.line : startLine,
    block_start_line: startLine,
    block_end_line: endLine,
    end_line: endLine,
    latest_patch: r.latest_patch ?? null,
    deprecated: Boolean(r.deprecated),
    deprecation_note: depMsg,
    vulnerabilities: r.vulnerabilities ?? [],
    managed_in: r.managed_in ?? null,
  };
}

export function mergeDepItems(local: IdeDepItem[], rawRemote: RawRemoteDepItem[]): IdeDepItem[] {
  if (!rawRemote.length) return local;
  const remote = rawRemote.map(normalizeRemoteDepItem);
  const byKey = new Map(remote.map((r) => [`${r.name}@${r.line}`, r] as const));
  const byPkg = new Map<string, IdeDepItem>();
  for (const r of remote) {
    if (!byPkg.has(r.name)) {
      byPkg.set(r.name, r);
    }
  }
  const merged = local.map((loc) => {
    const hit = byKey.get(`${loc.name}@${loc.line}`) ?? byPkg.get(loc.name);
    if (!hit) return loc;
    const blockStart = hit.block_start_line ?? loc.block_start_line ?? loc.line;
    const blockEnd = Math.max(hit.end_line ?? hit.line, loc.end_line ?? loc.line);
    return {
      ...loc,
      ...hit,
      line: hit.line >= 1 ? hit.line : loc.line,
      block_start_line: blockStart,
      block_end_line: blockEnd,
      end_line: blockEnd,
      latest_patch: hit.latest_patch ?? loc.latest_patch ?? null,
      deprecated: Boolean(hit.deprecated || loc.deprecated),
      deprecation_note: hit.deprecation_note ?? loc.deprecation_note ?? null,
      vulnerabilities: hit.vulnerabilities?.length ? hit.vulnerabilities : (loc.vulnerabilities ?? []),
      managed_in: hit.managed_in ?? loc.managed_in ?? null,
    };
  });
  for (const r of remote) {
    const exists = merged.some(
      (m) =>
        m.name === r.name &&
        (m.line === r.line || (m.block_start_line ?? m.line) === (r.block_start_line ?? r.line)),
    );
    if (!exists) {
      merged.push(r);
    }
  }
  return merged;
}

export async function inspectIdeDependencies(opts: {
  root: string;
  connectionId?: string | null;
  projectFolders?: string[];
  path: string;
  content: string;
  signal?: AbortSignal;
}): Promise<IdeDepItem[]> {
  const local = parseManifestDependencies(opts.path, opts.content);
  if (!opts.root) return local;
  try {
    const { api } = await import('../api/client.ts');
    const resp = await api.post<IdeDepsInspectResponse>(
      '/ide/deps/inspect',
      {
        target: {
          kind: opts.connectionId ? 'ssh' : 'local',
          root: opts.root,
          connection_id: opts.connectionId ?? null,
          additional_roots: opts.projectFolders,
        },
        path: opts.path,
        content: opts.content,
      },
      opts.signal,
    );
    return mergeDepItems(local, resp.items ?? resp.dependencies ?? []);
  } catch {
    return local;
  }
}

export function normalizeIdeDepAction(action: string, eco?: ManifestEcosystem | null): string {
  if (action === 'update_single' || action === 'upgrade') return 'update_one';
  if (action === 'update_all') return 'update_all_latest';
  if (action === 'update_direct') return 'update_direct_latest';
  if (action === 'sync' && eco === 'maven') return 'maven_sync';
  if (action === 'sync' && eco === 'gradle') return 'gradle_sync';
  return action;
}

export function applyFallbackManifestVersionUpdate(
  content: string,
  dep: IdeDepItem,
  nextVersion: string,
): string {
  if (dep.property_name) {
    const esc = dep.property_name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const propRe = new RegExp(`(<${esc}>)([^<]+)(</${esc}>)`);
    if (propRe.test(content)) {
      return content.replace(propRe, `$1${nextVersion}$3`);
    }
  }
  const lines = content.split('\n');
  const s = Math.max(0, (dep.block_start_line ?? dep.line) - 1);
  const e = Math.min(
    lines.length - 1,
    Math.max(s, (dep.end_line ?? dep.block_end_line ?? dep.line) - 1),
  );
  for (let i = s; i <= e; i++) {
    if (
      dep.current_version &&
      dep.current_version !== 'managed' &&
      lines[i].includes(dep.current_version)
    ) {
      lines[i] = lines[i].replace(dep.current_version, nextVersion);
      return lines.join('\n');
    }
  }
  return content;
}

export function applyBatchFallbackManifestUpdates(
  content: string,
  deps: IdeDepItem[],
  mode: string,
): { updated: string; count: number } {
  const directOnly = mode.includes('direct');
  const patchOnly = mode.includes('patch');
  let cur = content;
  let count = 0;
  for (const d of [...deps].sort((a, b) => b.line - a.line)) {
    if (directOnly && d.indirect) {
      continue;
    }
    const fixedVer = d.vulnerabilities?.find((v) => v.fixed_version)?.fixed_version ?? null;
    const target = patchOnly ? (d.latest_patch ?? fixedVer) : (d.latest_version ?? d.latest_patch ?? fixedVer);
    if (!target || !isVersionOutdated(d.current_version, target)) {
      continue;
    }
    const next = applyFallbackManifestVersionUpdate(cur, d, target);
    if (next !== cur) {
      cur = next;
      count++;
    }
  }
  return { updated: cur, count };
}

export async function executeIdeDepAction(opts: {
  root: string;
  connectionId?: string | null;
  projectFolders?: string[];
  path: string;
  content: string;
  action: string;
  dep?: IdeDepItem | null;
  version?: string | null;
  deps?: IdeDepItem[];
}): Promise<{ ok: boolean; updated_content?: string; output?: string; items?: IdeDepItem[] }> {
  const [{ api }, { toasts }] = await Promise.all([
    import('../api/client.ts'),
    import('../toast.svelte.ts'),
  ]);
  const eco = detectManifestEcosystem(opts.path);
  if (opts.action === 'vuln_scan') {
    const items = await inspectIdeDependencies({
      root: opts.root,
      connectionId: opts.connectionId,
      projectFolders: opts.projectFolders,
      path: opts.path,
      content: opts.content,
    });
    const cveCount = items.reduce((sum, d) => sum + (d.vulnerabilities?.length ?? 0), 0);
    if (cveCount > 0) {
      toasts.error('Vulnerability scan complete', `${cveCount} security advisories found in manifest.`);
    } else {
      toasts.success('Vulnerability scan complete', '0 known CVEs found in manifest dependencies.');
    }
    return { ok: true, items, output: `${cveCount} vulnerabilities` };
  }
  const normAction = normalizeIdeDepAction(opts.action, eco);
  try {
    const resp = await api.post<{
      ok?: boolean;
      updated_content?: string;
      content?: string;
      output?: string;
      stdout?: string;
    }>('/ide/deps/action', {
      target: {
        kind: opts.connectionId ? 'ssh' : 'local',
        root: opts.root,
        connection_id: opts.connectionId ?? null,
        additional_roots: opts.projectFolders,
      },
      path: opts.path,
      action: normAction,
      module: opts.dep?.name ?? undefined,
      version: opts.version ?? undefined,
      content: opts.content,
    });
    const ok = resp.ok !== false;
    const out = resp.output ?? resp.stdout;
    const updatedContent = resp.updated_content ?? resp.content;
    if (ok) {
      if (normAction === 'why') {
        const whyBody = (out ?? '').trim().slice(0, 220) || 'Module is required by build graph.';
        toasts.info(`go mod why ${opts.dep?.name ?? ''}`.trim(), whyBody);
      } else {
        toasts.success('Dependency action complete', out?.slice(0, 120) || normAction);
      }
    }
    return { ok, updated_content: updatedContent, output: out };
  } catch {
    if (normAction === 'update_one' && opts.dep && opts.version) {
      const updated = applyFallbackManifestVersionUpdate(opts.content, opts.dep, opts.version);
      if (updated !== opts.content) {
        toasts.success(`Updated ${opts.dep.name}`, `${opts.dep.current_version} → ${opts.version}`);
        return { ok: true, updated_content: updated };
      }
    } else if (
      normAction === 'update_all_latest' ||
      normAction === 'update_all_patch' ||
      normAction === 'update_direct_latest' ||
      normAction === 'update_direct_patch'
    ) {
      const sourceDeps = opts.deps?.length
        ? opts.deps
        : parseManifestDependencies(opts.path, opts.content);
      const { updated, count } = applyBatchFallbackManifestUpdates(opts.content, sourceDeps, normAction);
      if (count > 0 && updated !== opts.content) {
        toasts.success(`Updated ${count} dependencies`, normAction);
        return { ok: true, updated_content: updated };
      }
    }
    toasts.info('Dependency action queued', `Run ${normAction} when toolchain is ready.`);
    return { ok: false };
  }
}
