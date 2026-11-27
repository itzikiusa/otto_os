// Zig (`build.zig.zon`) dependency parser used by `cm-ide-deps-model.ts`.

import type { IdeDepItem } from './cm-ide-deps-model.ts';

function stripZigLineComment(raw: string): string {
  let inStr = false;
  for (let j = 0; j < raw.length; j++) {
    if (inStr) {
      if (raw[j] === '\\') { j++; continue; }
      if (raw[j] === '"') inStr = false;
    } else if (raw[j] === '"') {
      inStr = true;
    } else if (raw[j] === '/' && raw[j + 1] === '/') {
      return raw.slice(0, j);
    }
  }
  return raw;
}

export function parseZigManifestDependencies(lines: readonly string[]): IdeDepItem[] {
  const items: IdeDepItem[] = [];
  let inDeps = false;
  let depDepth = 0;
  let curName = '';
  let curStart = 0;
  let curUrl = '';
  let curPath = '';
  let curHash = '';
  const flushDep = (endLn: number) => {
    if (!curName) return;
    let ver = '*';
    if (curPath) {
      ver = `path:${curPath}`;
    } else if (curUrl) {
      const seg = curUrl.split('/').pop() ?? curUrl;
      ver = seg.replace(/\.(?:tar\.(?:gz|xz|zst)|tgz|zip)$/i, '') || curUrl;
    } else if (curHash) {
      ver = curHash.slice(0, 12);
    }
    items.push({
      name: curName,
      current_version: ver,
      line: curStart,
      end_line: endLn,
      block_start_line: curStart,
      block_end_line: endLn,
      vulnerabilities: [],
    });
    curName = '';
    curUrl = '';
    curPath = '';
    curHash = '';
  };

  for (let i = 0; i < lines.length; i++) {
    const t = stripZigLineComment(lines[i]).trim();
    const ln = i + 1;
    if (!t) continue;
    if (!inDeps) {
      if (/^\.dependencies\s*=\s*\.\{/.test(t)) {
        inDeps = true;
        depDepth = 1;
      }
      continue;
    }
    if (depDepth === 1) {
      const mDep = /^\.(?:"([^"]+)"|([A-Za-z_]\w*))\s*=\s*\.\{/.exec(t);
      if (mDep) {
        curName = mDep[1] || mDep[2];
        curStart = ln;
        curUrl = '';
        curPath = '';
        curHash = '';
        if (t.includes('}')) {
          const mu = /\.url\s*=\s*"([^"]+)"/.exec(t);
          const mp = /\.path\s*=\s*"([^"]+)"/.exec(t);
          const mh = /\.hash\s*=\s*"([^"]+)"/.exec(t);
          if (mu) curUrl = mu[1];
          if (mp) curPath = mp[1];
          if (mh) curHash = mh[1];
          flushDep(ln);
        } else {
          depDepth = 2;
        }
        continue;
      }
      if (t.startsWith('}')) {
        inDeps = false;
        depDepth = 0;
      }
    } else if (depDepth >= 2) {
      const mu = /^\.url\s*=\s*"([^"]+)"/.exec(t);
      if (mu) curUrl = mu[1];
      const mp = /^\.path\s*=\s*"([^"]+)"/.exec(t);
      if (mp) curPath = mp[1];
      const mh = /^\.hash\s*=\s*"([^"]+)"/.exec(t);
      if (mh) curHash = mh[1];
      const opens = (t.match(/\{/g) ?? []).length;
      const closes = (t.match(/\}/g) ?? []).length;
      depDepth += opens - closes;
      if (depDepth <= 1) {
        depDepth = 1;
        flushDep(ln);
      }
    }
  }
  return items;
}
