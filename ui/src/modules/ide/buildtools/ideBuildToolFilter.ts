// Pure filter helpers for the Maven / Gradle build tool window.
// Kept rune-free so `node --test` unit tests can verify filtering directly.

import type { IdeBuildToolModule } from '../store/ideTypes';

const DEFAULT_MAVEN_LIFECYCLE = ['clean', 'validate', 'compile', 'test', 'package', 'verify', 'install', 'site', 'deploy'];
const DEFAULT_GRADLE_LIFECYCLE = ['clean', 'assemble', 'compileJava', 'classes', 'test', 'check', 'jar', 'build', 'bootRun'];

export function matchesBuildToolQuery(text: string | null | undefined, rawQuery: string): boolean {
  const q = rawQuery.trim().toLowerCase();
  if (!q) return true;
  return Boolean(text && text.toLowerCase().includes(q));
}

export function moduleSelfMatchesBuildFilter(mod: IdeBuildToolModule, rawQuery: string): boolean {
  const q = rawQuery.trim().toLowerCase();
  if (!q) return true;
  return (
    matchesBuildToolQuery(mod.name, q) ||
    matchesBuildToolQuery(mod.artifact_id, q) ||
    matchesBuildToolQuery(mod.group_id, q) ||
    matchesBuildToolQuery(mod.project_coord, q)
  );
}

/** True when a module itself, any of its lifecycle phases, plugins/goals, dependencies,
 *  repositories, or nested child modules match `rawQuery`. */
export function moduleMatchesBuildFilter(mod: IdeBuildToolModule, rawQuery: string): boolean {
  const q = rawQuery.trim().toLowerCase();
  if (!q) return true;
  if (moduleSelfMatchesBuildFilter(mod, q)) return true;
  const lifecycle =
    mod.lifecycle && mod.lifecycle.length > 0
      ? mod.lifecycle
      : mod.tool === 'gradle'
        ? DEFAULT_GRADLE_LIFECYCLE
        : DEFAULT_MAVEN_LIFECYCLE;
  if (lifecycle.some((phase) => matchesBuildToolQuery(phase, q))) return true;
  if (
    (mod.plugins ?? []).some(
      (plug) =>
        matchesBuildToolQuery(plug.name, q) ||
        (plug.goals ?? []).some((goal) => matchesBuildToolQuery(goal, q)),
    )
  ) {
    return true;
  }
  if ((mod.dependencies ?? []).some((dep) => matchesBuildToolQuery(dep, q))) return true;
  if (
    (mod.repositories ?? []).some(
      (rep) => matchesBuildToolQuery(rep.id, q) || matchesBuildToolQuery(rep.url, q),
    )
  ) {
    return true;
  }
  return (mod.modules ?? mod.children ?? []).some((child) => moduleMatchesBuildFilter(child, q));
}
