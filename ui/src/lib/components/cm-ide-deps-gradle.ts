// Gradle (`build.gradle`, `build.gradle.kts`) and Gradle Version Catalog
// (`gradle/libs.versions.toml`) dependency parser used by `cm-ide-deps-model.ts`.

import type { IdeDepItem } from './cm-ide-deps-model.ts';

export function parseGradleManifestDependencies(filePath: string, lines: readonly string[]): IdeDepItem[] {
  const items: IdeDepItem[] = [];
  const base = filePath.split('?')[0].split('/').pop()?.toLowerCase() ?? '';
  if (base === 'libs.versions.toml') {
    const versions = new Map<string, string>();
    let section = '';
    for (const raw of lines) {
      const t = raw.trim();
      if (t.startsWith('[') && t.endsWith(']')) { section = t.slice(1, -1).trim().toLowerCase(); continue; }
      if (section === 'versions') {
        const mv = /^([A-Za-z0-9_.-]+)\s*=\s*"([^"]+)"/.exec(t);
        if (mv) versions.set(mv[1], mv[2]);
      }
    }
    section = '';
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim();
      if (!t || t.startsWith('#')) continue;
      if (t.startsWith('[') && t.endsWith(']')) { section = t.slice(1, -1).trim().toLowerCase(); continue; }
      if (section !== 'libraries') continue;
      let coord = '';
      let ver = '';
      let propName: string | null = null;
      const mShort = /^[A-Za-z0-9_.-]+\s*=\s*"([^":]+:[^":]+)(?::([^"]+))?"/.exec(t);
      const mMod = /module\s*=\s*"([^":]+:[^":]+)(?::([^"]+))?"/.exec(t);
      const mGn = /group\s*=\s*"([^"]+)".*name\s*=\s*"([^"]+)"/.exec(t);
      if (mShort) { coord = mShort[1]; ver = mShort[2] ?? ''; }
      else if (mMod) { coord = mMod[1]; ver = mMod[2] ?? ''; }
      else if (mGn) { coord = `${mGn[1]}:${mGn[2]}`; }
      if (coord) {
        if (!ver) {
          const mRef = /version\.ref\s*=\s*"([^"]+)"/.exec(t);
          const mIn = /version\s*=\s*"([^"]+)"/.exec(t);
          if (mRef) { propName = mRef[1]; ver = versions.get(mRef[1]) ?? '*'; }
          else if (mIn) { ver = mIn[1]; }
        }
        items.push({
          name: coord,
          current_version: ver || '*',
          property_name: propName,
          line: i + 1,
          end_line: i + 1,
          vulnerabilities: [],
        });
      }
    }
    return items;
  }

  for (let i = 0; i < lines.length; i++) {
    const m = /^\s*(?:implementation|api|compileOnly|runtimeOnly|testImplementation|testRuntimeOnly|kapt|ksp)\s*\(?\s*['"]([^:'"]+):([^:'"]+):([^'"]+)['"]/.exec(
      lines[i],
    );
    if (m) {
      items.push({
        name: `${m[1]}:${m[2]}`,
        current_version: m[3],
        indirect: /^\s*test/.test(lines[i]),
        line: i + 1,
        end_line: i + 1,
        vulnerabilities: [],
      });
      continue;
    }
    const mKt = /^\s*(?:implementation|api|compileOnly|runtimeOnly|testImplementation|testRuntimeOnly)\s*\(\s*kotlin\s*\(\s*"([^"]+)"(?:\s*,\s*"([^"]+)")?\s*\)\s*\)/.exec(
      lines[i],
    );
    if (mKt) {
      items.push({
        name: `org.jetbrains.kotlin:kotlin-${mKt[1]}`,
        current_version: mKt[2] ?? 'plugin',
        indirect: /^\s*test/.test(lines[i]),
        line: i + 1,
        end_line: i + 1,
        vulnerabilities: [],
      });
    }
  }
  return items;
}
