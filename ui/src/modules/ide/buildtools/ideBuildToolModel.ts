// Maven & Gradle multi-module model loader, fallback manifest parser, and goal runner (#76 §1).

import { api } from '../../../lib/api/client';
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

async function readLocalOrTargetFile(rootDir: string, fullPath: string): Promise<string | null> {
  try {
    const q = ide.target.kind === 'local'
      ? new URLSearchParams({ kind: 'local', root: rootDir, path: fullPath }).toString()
      : ide.targetQuery({ path: fullPath });
    const res = await api.get<IdeFileReadResp>(`/ide/fs/file?${q}`);
    return res.content ?? '';
  } catch {
    return null;
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
  } catch {
    return [];
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
    tool: 'maven',
    root_dir: cleanDir,
    manifest_path: pomPath,
    lifecycle: MAVEN_LIFECYCLE,
    plugins: parsePomPlugins(xml),
    dependencies: parsePomDeps(xml),
    repositories: repoIds.length > 0 ? repoIds : ['central (https://repo.maven.apache.org/maven2)'],
    modules: childModules,
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
    return {
      name: inc,
      group_id: group,
      artifact_id: ideBasename(rel),
      version,
      tool: 'gradle',
      root_dir: subRoot,
      manifest_path: `${subRoot}/build.gradle`,
      lifecycle: GRADLE_LIFECYCLE,
      plugins: [{ name: 'verification', goals: ['test', 'check'] }],
      dependencies: [],
      repositories: ['mavenCentral()'],
      modules: [],
    };
  });

  return {
    name: rootName,
    group_id: group,
    artifact_id: rootName,
    version,
    tool: 'gradle',
    root_dir: cleanDir,
    manifest_path: manifestPath,
    lifecycle: GRADLE_LIFECYCLE,
    plugins: [
      { name: 'application', goals: ['bootRun', 'run'] },
      { name: 'verification', goals: ['test', 'check'] },
      { name: 'help', goals: ['dependencies', 'projects', 'tasks'] },
    ],
    dependencies: deps,
    repositories: ['mavenCentral()'],
    modules: subMods,
  };
}

export async function loadBuildToolWorkspaceModel(): Promise<LoadedBuildToolWorkspace> {
  const roots = ide.projectRoots.map((r) => r.path);
  try {
    const resp = await api.post<{
      tool?: 'maven' | 'gradle' | null;
      profiles?: IdeBuildToolProfile[];
      spring_profiles?: string[];
      modules?: IdeBuildToolModule[];
    }>('/ide/build-tools/model', { target: ide.targetPayload, roots });
    if (resp.modules && resp.modules.length > 0) {
      const tool = resp.modules[0].tool ?? resp.tool ?? 'maven';
      ide.detectedBuildToolOverride = tool;
      return {
        tool,
        profiles: resp.profiles ?? [],
        springProfiles: resp.spring_profiles ?? [],
        modules: resp.modules,
      };
    }
  } catch { /* fallback to manifest parser */ }

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

export function formatHomeShortPath(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~');
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
