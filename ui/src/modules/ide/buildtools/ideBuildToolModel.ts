// Maven & Gradle multi-module model loader, fallback manifest parser, and goal runner.
import { api, ApiError } from '../../../lib/api/client';
import { shortenHomePath } from '../../../lib/homePath';
import { normalizeRootPath } from '../shell/ideProjectHelpers';
import {
  ide,
  ideBasename,
  type IdeBuildToolModule,
  type IdeBuildToolProfile,
  type IdeFileReadResp,
  type IdeFsTreeResp,
} from '../ideStore.svelte';

export interface LoadedBuildToolWorkspace {
  tool: 'maven' | 'gradle';
  profiles: IdeBuildToolProfile[];
  springProfiles: string[];
  modules: IdeBuildToolModule[];
}

export const MAVEN_LIFECYCLE = ['clean', 'validate', 'compile', 'test', 'package', 'verify', 'install', 'site', 'deploy'];
export const GRADLE_LIFECYCLE = ['clean', 'assemble', 'compileJava', 'classes', 'test', 'check', 'jar', 'build', 'bootRun'];

function extractXmlTag(xml: string, tag: string): string | null {
  const withoutParent = xml.replace(/<parent>[\s\S]*?<\/parent>/gi, '').replace(/<build>[\s\S]*?<\/build>/gi, '');
  const re = new RegExp(`<${tag}>\\s*([^<]+?)\\s*</${tag}>`, 'i');
  return withoutParent.match(re)?.[1]?.trim() ?? xml.match(re)?.[1]?.trim() ?? null;
}

function extractXmlList(blockRegex: RegExp, itemRegex: RegExp, xml: string): string[] {
  const out: string[] = [];
  let bm: RegExpExecArray | null;
  const bRe = new RegExp(blockRegex.source, 'gi');
  while ((bm = bRe.exec(xml)) !== null) {
    const iRe = new RegExp(itemRegex.source, 'gi');
    let im: RegExpExecArray | null;
    while ((im = iRe.exec(bm[1] ?? '')) !== null) {
      const val = (im[1] ?? '').trim();
      if (val && !out.includes(val)) out.push(val);
    }
  }
  return out;
}

function parsePomDeps(xml: string): string[] {
  const out: string[] = [];
  const depBlocks = xml.match(/<dependency>[\s\S]*?<\/dependency>/gi) ?? [];
  for (let i = 0; i < depBlocks.length && out.length < 40; i++) {
    const b = depBlocks[i];
    const g = b.match(/<groupId>\s*([^<]+?)\s*<\/groupId>/i)?.[1]?.trim() ?? '';
    const a = b.match(/<artifactId>\s*([^<]+?)\s*<\/artifactId>/i)?.[1]?.trim() ?? '';
    const v = b.match(/<version>\s*([^<]+?)\s*<\/version>/i)?.[1]?.trim() ?? '';
    if (a) out.push([g, a, v].filter(Boolean).join(':'));
  }
  return out;
}

function parsePomPlugins(xml: string): { name: string; goals: string[] }[] {
  const defaults: { name: string; goals: string[] }[] = [
    { name: 'compiler', goals: ['compiler:compile', 'compiler:testCompile'] },
    { name: 'surefire', goals: ['surefire:test'] },
    { name: 'dependency', goals: ['dependency:tree', 'dependency:sources'] },
  ];
  const seen = new Set(defaults.map((d) => d.name));
  const blocks = xml.match(/<plugin>[\s\S]*?<\/plugin>/gi) ?? [];
  for (let i = 0; i < blocks.length; i++) {
    const raw = blocks[i].match(/<artifactId>\s*([^<]+?)\s*<\/artifactId>/i)?.[1]?.trim() ?? '';
    const short = raw.replace(/^maven-|-plugin$/g, '');
    if (!short || seen.has(short)) continue;
    seen.add(short);
    const goals = short === 'spring-boot'
      ? ['spring-boot:run', 'spring-boot:repackage', 'spring-boot:build-info']
      : [`${short}:help`, `${short}:run`];
    defaults.unshift({ name: short, goals });
  }
  return defaults;
}

/** Only a missing file/route is "nothing there"; transport/auth/server errors reach the window's Retry state. */
function isNotFound(e: unknown): boolean {
  return e instanceof ApiError && e.status === 404;
}

async function readLocalOrTargetFile(rootDir: string, fullPath: string): Promise<string | null> {
  try {
    const q = ide.target.kind === 'local'
      ? new URLSearchParams({ kind: 'local', root: rootDir, path: fullPath }).toString()
      : ide.targetQuery({ path: fullPath });
    const res = await api.get<IdeFileReadResp>(`/ide/fs/file?${q}`);
    return res.content ?? '';
  } catch (e) {
    if (isNotFound(e)) return null;
    throw e;
  }
}

async function discoverSpringProfiles(rootDir: string): Promise<string[]> {
  const resDir = `${rootDir}/src/main/resources`;
  try {
    const q = ide.target.kind === 'local'
      ? new URLSearchParams({ kind: 'local', root: rootDir, path: resDir, show_hidden: 'false' }).toString()
      : ide.targetQuery({ path: resDir, show_hidden: false });
    const tr = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${q}`);
    const out: string[] = [];
    for (const e of tr.entries ?? []) {
      const m = e.name.match(/^application-([\w-]+)\.(?:ya?ml|properties)$/i);
      if (m?.[1] && !out.includes(m[1])) out.push(m[1]);
    }
    return out;
  } catch (e) {
    if (isNotFound(e)) return [];
    throw e;
  }
}

async function parseMavenModuleRecursive(
  repoRoot: string,
  modDir: string,
  depth: number,
  collectedProfiles: Map<string, IdeBuildToolProfile>,
  springSet: Set<string>,
): Promise<IdeBuildToolModule | null> {
  const cleanDir = normalizeRootPath(modDir);
  const pomPath = `${cleanDir}/pom.xml`;
  const xml = await readLocalOrTargetFile(repoRoot, pomPath);
  if (xml === null) return null;

  const artifactId = extractXmlTag(xml, 'artifactId') || ideBasename(cleanDir);
  const groupId = extractXmlTag(xml, 'groupId');
  const version = extractXmlTag(xml, 'version');
  const displayName = extractXmlTag(xml, 'name') || artifactId;
  const profIds = extractXmlList(/<profiles>([\s\S]*?)<\/profiles>/, /<profile>[\s\S]*?<id>\s*([^<]+?)\s*<\/id>[\s\S]*?<\/profile>/, xml);
  for (const id of profIds) {
    if (!collectedProfiles.has(id)) collectedProfiles.set(id, { id, name: id, kind: 'maven', source: displayName });
  }
  const repoIds = extractXmlList(/<repositories>([\s\S]*?)<\/repositories>/, /<repository>[\s\S]*?<id>\s*([^<]+?)\s*<\/id>[\s\S]*?<\/repository>/, xml);
  const sp = await discoverSpringProfiles(cleanDir);
  for (const s of sp) springSet.add(s);

  const childModules: IdeBuildToolModule[] = [];
  if (depth < 2) {
    const subNames = extractXmlList(/<modules>([\s\S]*?)<\/modules>/, /<module>\s*([^<]+?)\s*<\/module>/, xml);
    for (let i = 0; i < subNames.length && i < 25; i++) {
      const subDir = `${cleanDir}/${subNames[i].replace(/^\.\//, '').replace(/\/+$/, '')}`;
      const sub = await parseMavenModuleRecursive(repoRoot, subDir, depth + 1, collectedProfiles, springSet);
      if (sub) childModules.push(sub);
    }
  }

  return {
    name: displayName,
    group_id: groupId,
    artifact_id: artifactId,
    version,
    project_coord: [groupId, artifactId].filter(Boolean).join(':') || displayName,
    tool: 'maven',
    kind: 'maven',
    root_dir: cleanDir,
    root: cleanDir,
    location_display: shortenHomePath(cleanDir),
    manifest_path: pomPath,
    lifecycle: MAVEN_LIFECYCLE,
    plugins: parsePomPlugins(xml),
    dependencies: parsePomDeps(xml),
    repositories: repoIds.length > 0
      ? repoIds.map((id) => ({ id, url: id }))
      : [{ id: 'central', url: 'https://repo.maven.apache.org/maven2' }],
    modules: childModules,
    children: childModules,
  };
}

async function parseGradleModule(repoRoot: string, springSet: Set<string>): Promise<IdeBuildToolModule | null> {
  const cleanDir = normalizeRootPath(repoRoot);
  const candidates = ['build.gradle.kts', 'build.gradle', 'settings.gradle.kts', 'settings.gradle'];
  let manifestPath = '';
  let content = '';
  for (const c of candidates) {
    const txt = await readLocalOrTargetFile(cleanDir, `${cleanDir}/${c}`);
    if (txt !== null) {
      if (!manifestPath) manifestPath = `${cleanDir}/${c}`;
      content += `\n${txt}`;
    }
  }
  if (!manifestPath) return null;
  const rootName = content.match(/rootProject\.name\s*=\s*['"]([^'"]+)['"]/)?.[1] || ideBasename(cleanDir);
  const group = content.match(/\bgroup\s*=\s*['"]([^'"]+)['"]/)?.[1] ?? null;
  const version = content.match(/\bversion\s*=\s*['"]([^'"]+)['"]/)?.[1] ?? null;
  const deps = [...content.matchAll(/(?:implementation|api|testImplementation|runtimeOnly)\s*\(?\s*['"]([^'"]+)['"]/g)].map((m) => m[1]).slice(0, 40);
  const includes = [...content.matchAll(/include\s*\(?([^)\n]+)\)?/g)]
    .flatMap((m) => [...m[1].matchAll(/['"]:?([^'"]+)['"]/g)].map((x) => x[1]));
  const sp = await discoverSpringProfiles(cleanDir);
  for (const s of sp) springSet.add(s);

  const subMods: IdeBuildToolModule[] = includes.slice(0, 25).map((inc) => {
    const rel = inc.replace(/:/g, '/');
    const subRoot = `${cleanDir}/${rel}`;
    const art = ideBasename(rel);
    return {
      name: inc,
      group_id: group,
      artifact_id: art,
      version,
      project_coord: [group, art].filter(Boolean).join(':') || inc,
      tool: 'gradle',
      kind: 'gradle',
      root_dir: subRoot,
      root: subRoot,
      location_display: shortenHomePath(subRoot),
      manifest_path: `${subRoot}/build.gradle`,
      lifecycle: GRADLE_LIFECYCLE,
      plugins: [{ name: 'verification', goals: ['test', 'check'] }],
      dependencies: [],
      repositories: [{ id: 'mavenCentral()', url: 'https://repo.maven.apache.org/maven2' }],
      modules: [],
      children: [],
    };
  });

  return {
    name: rootName,
    group_id: group,
    artifact_id: rootName,
    version,
    project_coord: [group, rootName].filter(Boolean).join(':') || rootName,
    tool: 'gradle',
    kind: 'gradle',
    root_dir: cleanDir,
    root: cleanDir,
    location_display: shortenHomePath(cleanDir),
    manifest_path: manifestPath,
    lifecycle: GRADLE_LIFECYCLE,
    plugins: [
      { name: 'application', goals: ['bootRun', 'run'] },
      { name: 'verification', goals: ['test', 'check'] },
      { name: 'help', goals: ['dependencies', 'projects', 'tasks'] },
    ],
    dependencies: deps,
    repositories: [{ id: 'mavenCentral()', url: 'https://repo.maven.apache.org/maven2' }],
    modules: subMods,
    children: subMods,
  };
}

interface RawBuildToolProject {
  name: string;
  group_id?: string | null;
  artifact_id?: string | null;
  version?: string | null;
  tool?: 'maven' | 'gradle';
  kind?: 'maven' | 'gradle';
  project_coord?: string | null;
  root_dir?: string;
  root?: string;
  location_display?: string | null;
  manifest_path: string;
  lifecycle?: string[];
  plugins?: (string | { name: string; goals?: string[] })[];
  dependencies?: string[];
  repositories?: (string | { id: string; url?: string })[];
  modules?: RawBuildToolProject[];
  children?: RawBuildToolProject[];
}

function normalizePluginList(
  raw: (string | { name: string; goals?: string[] })[] | undefined,
  tool: 'maven' | 'gradle',
): { name: string; goals: string[] }[] {
  const byName = new Map<string, string[]>();
  const addGoal = (plug: string, goal: string): void => {
    const cur = byName.get(plug) ?? [];
    if (goal && !cur.includes(goal)) cur.push(goal);
    byName.set(plug, cur);
  };
  for (const item of raw ?? []) {
    if (typeof item !== 'string') {
      const cur = byName.get(item.name) ?? [];
      for (const g of item.goals ?? []) if (!cur.includes(g)) cur.push(g);
      byName.set(item.name, cur);
      continue;
    }
    const s = item.trim();
    if (!s) continue;
    const parts = s.split(':');
    if (parts.length === 2 && !parts[0].includes('.')) {
      addGoal(parts[0], s);
    } else if (parts.length >= 2) {
      const short = parts[1].replace(/^maven-|-plugin$/g, '') || parts[1];
      const goals = short === 'spring-boot'
        ? ['spring-boot:run', 'spring-boot:repackage']
        : [`${short}:help`, `${short}:run`];
      for (const g of goals) addGoal(short, g);
    } else {
      const short = s.replace(/^maven-|-plugin$/g, '');
      addGoal(short, tool === 'gradle' && short.includes('spring') ? 'bootRun' : short);
    }
  }
  return [...byName.entries()].map(([name, goals]) => ({ name, goals }));
}

function normalizeRepoList(raw: (string | { id: string; url?: string })[] | undefined): { id: string; url: string }[] {
  return (raw ?? []).map((r) => {
    if (typeof r !== 'string') return { id: r.id, url: r.url || r.id };
    const paren = r.match(/^([^\s(]+)\s*\(([^)]+)\)$/);
    if (paren) return { id: paren[1], url: paren[2] };
    const kv = r.match(/^([^=:]+?)\s*[=:]\s*(https?:\/\/.+)$/);
    if (kv) return { id: kv[1].trim(), url: kv[2].trim() };
    return { id: r, url: r };
  });
}

function normalizeRawProject(p: RawBuildToolProject, fallbackTool: 'maven' | 'gradle'): IdeBuildToolModule {
  const tool = p.tool ?? p.kind ?? fallbackTool;
  const rootDir = normalizeRootPath(p.root_dir ?? p.root ?? '');
  const children = (p.modules ?? p.children ?? []).map((c) => normalizeRawProject(c, tool));
  return {
    name: p.name,
    group_id: p.group_id ?? null,
    artifact_id: p.artifact_id ?? null,
    version: p.version ?? null,
    project_coord: p.project_coord ?? ([p.group_id, p.artifact_id].filter(Boolean).join(':') || p.name),
    tool,
    kind: tool,
    root_dir: rootDir,
    root: rootDir,
    location_display: p.location_display ?? shortenHomePath(rootDir),
    manifest_path: p.manifest_path,
    lifecycle: p.lifecycle && p.lifecycle.length > 0 ? p.lifecycle : (tool === 'gradle' ? GRADLE_LIFECYCLE : MAVEN_LIFECYCLE),
    plugins: normalizePluginList(p.plugins, tool),
    dependencies: p.dependencies ?? [],
    repositories: normalizeRepoList(p.repositories),
    modules: children,
    children,
  };
}

export async function loadBuildToolWorkspaceModel(): Promise<LoadedBuildToolWorkspace> {
  const roots = ide.projectRoots.map((r) => r.path);
  try {
    const resp = await api.post<{
      has_maven?: boolean;
      has_gradle?: boolean;
      tool?: 'maven' | 'gradle' | null;
      profiles?: IdeBuildToolProfile[];
      spring_profiles?: string[];
      projects?: RawBuildToolProject[];
      modules?: RawBuildToolProject[];
    }>('/ide/build-tools/model', { target: ide.targetPayload, roots });
    const rawList = resp.projects ?? resp.modules ?? [];
    if (rawList.length > 0 || resp.has_maven || resp.has_gradle) {
      const fallbackTool: 'maven' | 'gradle' = resp.has_maven ? 'maven' : resp.has_gradle ? 'gradle' : (resp.tool ?? 'maven');
      const modules = rawList.map((p) => normalizeRawProject(p, fallbackTool));
      const tool: 'maven' | 'gradle' = resp.has_maven ? 'maven' : resp.has_gradle ? 'gradle' : (modules[0]?.tool ?? fallbackTool);
      const buildProfiles: IdeBuildToolProfile[] = [];
      const springSet = new Set<string>(resp.spring_profiles ?? []);
      for (const prof of resp.profiles ?? []) {
        if (prof.kind === 'spring') {
          if (prof.id) springSet.add(prof.id);
        } else if (prof.id) {
          buildProfiles.push({
            id: prof.id,
            name: prof.name ?? prof.id,
            kind: prof.kind ?? tool,
            source: prof.source_project ?? prof.source ?? null,
            source_project: prof.source_project ?? prof.source ?? null,
            active_by_default: prof.active_by_default,
          });
        }
      }
      if (modules.length > 0 || resp.has_maven || resp.has_gradle) {
        ide.detectedBuildToolOverride = tool;
        return { tool, profiles: buildProfiles, springProfiles: [...springSet], modules };
      }
    }
  } catch (e) {
    if (!isNotFound(e)) throw e; // a daemon without the model route falls back to the manifest parser
  }

  const profilesMap = new Map<string, IdeBuildToolProfile>();
  const springSet = new Set<string>();
  const modules: IdeBuildToolModule[] = [];
  for (let i = 0; i < roots.length && i < 20; i++) {
    const mvn = await parseMavenModuleRecursive(roots[i], roots[i], 0, profilesMap, springSet);
    if (mvn) { modules.push(mvn); continue; }
    const grd = await parseGradleModule(roots[i], springSet);
    if (grd) modules.push(grd);
  }
  const tool: 'maven' | 'gradle' = modules[0]?.tool ?? ide.buildToolKind ?? 'maven';
  if (modules.length > 0) ide.detectedBuildToolOverride = tool;
  return {
    tool,
    profiles: [...profilesMap.values()],
    springProfiles: [...springSet],
    modules,
  };
}


export function executeBuildToolGoal(mod: IdeBuildToolModule, rawGoal: string, mode: 'run' | 'debug'): void {
  const goals = rawGoal.trim().split(/\s+/).filter(Boolean);
  if (goals.length === 0) return;
  const args = [...goals];
  if (mod.tool === 'maven') {
    if (ide.activeMavenProfiles.length > 0) args.push(`-P${ide.activeMavenProfiles.join(',')}`);
    if (ide.buildSkipTests) args.push('-DskipTests');
    if (ide.buildOffline) args.push('-o');
    if (ide.activeSpringProfiles.length > 0) {
      args.push(`-Dspring-boot.run.profiles=${ide.activeSpringProfiles.join(',')}`);
    }
  } else {
    if (ide.activeMavenProfiles.length > 0) args.push(`-Pprofile=${ide.activeMavenProfiles.join(',')}`);
    if (ide.buildSkipTests) args.push('-x', 'test');
    if (ide.buildOffline) args.push('--offline');
  }
  const envOverrides: Record<string, string> = {};
  if (ide.activeSpringProfiles.length > 0) {
    envOverrides.SPRING_PROFILES_ACTIVE = ide.activeSpringProfiles.join(',');
  }
  if (mode === 'debug') {
    void ide.startDebug({
      language: 'java',
      launch: { program: mod.manifest_path, args, cwd: mod.root_dir, env: envOverrides },
    });
    return;
  }
  void ide.startRun({
    adhoc: {
      name: `${mod.name} [${goals.join(' ')}]`,
      runner: mod.tool === 'gradle' ? 'gradle' : 'maven',
      cwd: mod.root_dir,
      target_path: mod.manifest_path,
      args,
      env_overrides: envOverrides,
    },
  });
}
