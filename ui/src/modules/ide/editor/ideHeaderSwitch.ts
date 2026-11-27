/**
 * Pure C/C++ header ↔ source counterpart path resolution.
 *
 * Used both as a fallback when `clangd`'s `textDocument/switchSourceHeader` is
 * unavailable (e.g. cold startup or no `clangd` installed) and to normalize the
 * `file://` URI returned by `clangd` into a clean workspace path.
 */

const SOURCE_EXTS = ['cpp', 'cc', 'cxx', 'c++', 'c', 'mm', 'm', 'ixx', 'cppm'] as const;
const HEADER_EXTS = ['hpp', 'h', 'hxx', 'hh', 'h++', 'inl', 'ipp', 'tpp'] as const;

const SOURCE_EXT_SET = new Set<string>(SOURCE_EXTS);
const HEADER_EXT_SET = new Set<string>(HEADER_EXTS);

export function isCppHeaderOrSourcePath(filePath: string | null | undefined): boolean {
  if (!filePath) return false;
  const ext = extensionOf(filePath);
  return SOURCE_EXT_SET.has(ext) || HEADER_EXT_SET.has(ext);
}

export function isCppHeaderPath(filePath: string | null | undefined): boolean {
  return Boolean(filePath && HEADER_EXT_SET.has(extensionOf(filePath)));
}

export function isCppSourcePath(filePath: string | null | undefined): boolean {
  return Boolean(filePath && SOURCE_EXT_SET.has(extensionOf(filePath)));
}

/** Decode a `file://...` URI from clangd `textDocument/switchSourceHeader` into a filesystem path. */
export function decodeSwitchSourceHeaderUri(uri: string | null | undefined): string | null {
  if (!uri || typeof uri !== 'string') return null;
  const trimmed = uri.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith('file://')) {
    try {
      const parsed = new URL(trimmed);
      let decoded = decodeURIComponent(parsed.pathname);
      // Windows drive letter path `/C:/foo` -> `C:/foo`
      if (/^\/[A-Za-z]:\//.test(decoded)) decoded = decoded.slice(1);
      return decoded;
    } catch {
      return decodeURIComponent(trimmed.slice('file://'.length));
    }
  }
  return trimmed;
}

/**
 * Generate ordered candidate counterpart paths for a C/C++ header or source file.
 *
 * Covers:
 * - Same directory (`foo.cpp` ↔ `foo.hpp` / `foo.h`)
 * - CMake / Meson `src/` ↔ `include/` (both flat and `include/<project>/...`)
 * - Unreal Engine module layout `Private/` ↔ `Public/`
 * - Generic `source/` ↔ `headers/` / `inc/`
 */
export function headerSourceCandidates(filePath: string): string[] {
  const norm = filePath.replace(/\\/g, '/');
  const slash = norm.lastIndexOf('/');
  const dir = slash >= 0 ? norm.slice(0, slash) : '';
  const base = slash >= 0 ? norm.slice(slash + 1) : norm;
  const dot = base.lastIndexOf('.');
  if (dot <= 0) return [];

  const stem = base.slice(0, dot);
  const ext = base.slice(dot + 1).toLowerCase();
  const isHeader = HEADER_EXT_SET.has(ext);
  const isSource = SOURCE_EXT_SET.has(ext);
  if (!isHeader && !isSource) return [];

  const targetExts: readonly string[] = isHeader ? SOURCE_EXTS : HEADER_EXTS;
  const candidateDirs: string[] = [];
  const pushDir = (d: string): void => {
    const clean = d.replace(/\/+$/, '');
    if (!candidateDirs.includes(clean)) candidateDirs.push(clean);
  };

  pushDir(dir);

  const dirPairs: ReadonlyArray<[string, string]> = isHeader
    ? [
        ['/include/', '/src/'],
        ['/include', '/src'],
        ['/Public/', '/Private/'],
        ['/Public', '/Private'],
        ['/public/', '/private/'],
        ['/public', '/private'],
        ['/inc/', '/src/'],
        ['/inc', '/src'],
        ['/headers/', '/src/'],
        ['/headers', '/src'],
        ['/include/', '/source/'],
        ['/include', '/source'],
      ]
    : [
        ['/src/', '/include/'],
        ['/src', '/include'],
        ['/Private/', '/Public/'],
        ['/Private', '/Public'],
        ['/private/', '/public/'],
        ['/private', '/public'],
        ['/src/', '/inc/'],
        ['/src', '/inc'],
        ['/src/', '/headers/'],
        ['/src', '/headers'],
        ['/source/', '/include/'],
        ['/source', '/include'],
      ];

  for (const [fromSeg, toSeg] of dirPairs) {
    const idx = dir.lastIndexOf(fromSeg);
    if (idx < 0) continue;
    const swapped = dir.slice(0, idx) + toSeg + dir.slice(idx + fromSeg.length);
    pushDir(swapped);
    // Also handle `include/<proj>/foo.hpp` -> `src/foo.cpp` (dropping one nested segment under include)
    if (isHeader && fromSeg === '/include/') {
      const tail = dir.slice(idx + fromSeg.length);
      const nextSlash = tail.indexOf('/');
      if (nextSlash >= 0) {
        pushDir(`${dir.slice(0, idx)}/src/${tail.slice(nextSlash + 1)}`);
      } else {
        pushDir(`${dir.slice(0, idx)}/src`);
      }
    }
  }

  const out: string[] = [];
  const seen = new Set<string>();
  for (const d of candidateDirs) {
    for (const targetExt of targetExts) {
      const candidate = d ? `${d}/${stem}.${targetExt}` : `${stem}.${targetExt}`;
      if (candidate !== norm && !seen.has(candidate)) {
        seen.add(candidate);
        out.push(candidate);
      }
    }
  }
  return out;
}

/**
 * Pick the best existing counterpart from a list of known project file paths.
 * Prefers exact candidate order, then falls back to same-stem counterpart anywhere
 * in the workspace (useful for `include/<pkg>/foo.hpp` ↔ `src/foo.cpp`).
 */
export function matchHeaderSourceFromList(filePath: string, knownPaths: readonly string[]): string | null {
  const candidates = headerSourceCandidates(filePath);
  if (candidates.length === 0) return null;
  const byNorm = new Map<string, string>();
  for (const p of knownPaths) {
    byNorm.set(p.replace(/\\/g, '/'), p);
  }
  for (const cand of candidates) {
    const hit = byNorm.get(cand);
    if (hit) return hit;
  }

  // Fallback: same filename stem with counterpart extension (e.g. include/mylib/math.hpp <-> src/math.cpp)
  const norm = filePath.replace(/\\/g, '/');
  const base = norm.slice(norm.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  if (dot <= 0) return null;
  const stem = base.slice(0, dot).toLowerCase();
  const wantExts = isCppHeaderPath(filePath) ? SOURCE_EXT_SET : HEADER_EXT_SET;
  let best: { path: string; score: number } | null = null;
  const srcParts = norm.split('/');

  for (const p of knownPaths) {
    const pn = p.replace(/\\/g, '/');
    if (pn === norm) continue;
    const pb = pn.slice(pn.lastIndexOf('/') + 1);
    const pd = pb.lastIndexOf('.');
    if (pd <= 0) continue;
    if (pb.slice(0, pd).toLowerCase() !== stem) continue;
    const pext = pb.slice(pd + 1).toLowerCase();
    if (!wantExts.has(pext)) continue;
    // Score by shared directory prefix length
    const pParts = pn.split('/');
    let shared = 0;
    while (shared < srcParts.length - 1 && shared < pParts.length - 1 && srcParts[shared] === pParts[shared]) {
      shared += 1;
    }
    if (!best || shared > best.score) best = { path: p, score: shared };
  }
  return best?.path ?? null;
}

function extensionOf(filePath: string): string {
  const clean = filePath.split('?')[0].replace(/\\/g, '/');
  const base = clean.slice(clean.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
}
