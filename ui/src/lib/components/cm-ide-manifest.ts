// The build-manifest file rule, kept apart from cm-ide-deps-model so the
// eager editor can ask "is this a manifest?" without loading the deps model.

const MANIFEST_NAMES = new Set([
  'go.mod',
  'pom.xml',
  'package.json',
  'cargo.toml',
  'build.zig.zon',
  'cmakelists.txt',
  'vcpkg.json',
  'conanfile.txt',
  'directory.packages.props',
  'packages.config',
  'requirements.txt',
  'pyproject.toml',
  'build.gradle',
  'build.gradle.kts',
  'libs.versions.toml',
]);

export function isIdeManifestPath(filePath: string): boolean {
  const base = filePath.split('?')[0].split('/').pop()?.toLowerCase() ?? '';
  return (
    MANIFEST_NAMES.has(base) ||
    base.endsWith('.uproject') ||
    base.endsWith('.uplugin') ||
    base.endsWith('.csproj') ||
    base.endsWith('.fsproj') ||
    base.endsWith('.vbproj') ||
    /^requirements(?:[-_.][\w-]+)?\.txt$/.test(base)
  );
}
