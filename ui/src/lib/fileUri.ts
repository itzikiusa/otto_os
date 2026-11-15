// Shared RFC 3986 `file://` URI encoder/decoder and broad-root safety helper.
// Mirrors `otto_core::paths::{path_to_file_uri, file_uri_to_path, is_broad_non_project_root}`.

function utf8Encode(str: string): number[] {
  if (typeof TextEncoder !== 'undefined') {
    return Array.from(new TextEncoder().encode(str));
  }
  const out: number[] = [];
  for (let i = 0; i < str.length; i++) {
    const cp = str.codePointAt(i)!;
    if (cp > 0xffff) i++;
    if (cp < 0x80) out.push(cp);
    else if (cp < 0x800) out.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
    else if (cp < 0x10000) out.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
    else out.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3f), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
  }
  return out;
}

function utf8Decode(bytes: number[]): string {
  if (typeof TextDecoder !== 'undefined') {
    return new TextDecoder('utf-8', { fatal: false }).decode(new Uint8Array(bytes));
  }
  let out = '';
  for (let i = 0; i < bytes.length; ) {
    const b0 = bytes[i];
    if (b0 < 0x80) {
      out += String.fromCharCode(b0);
      i += 1;
    } else if ((b0 & 0xe0) === 0xc0 && i + 1 < bytes.length && (bytes[i + 1] & 0xc0) === 0x80) {
      const cp = ((b0 & 0x1f) << 6) | (bytes[i + 1] & 0x3f);
      out += cp >= 0x80 ? String.fromCharCode(cp) : '\ufffd';
      i += 2;
    } else if ((b0 & 0xf0) === 0xe0 && i + 2 < bytes.length && (bytes[i + 1] & 0xc0) === 0x80 && (bytes[i + 2] & 0xc0) === 0x80) {
      const cp = ((b0 & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f);
      out += cp >= 0x800 && (cp < 0xd800 || cp > 0xdfff) ? String.fromCharCode(cp) : '\ufffd';
      i += 3;
    } else if ((b0 & 0xf8) === 0xf0 && i + 3 < bytes.length && (bytes[i + 1] & 0xc0) === 0x80 && (bytes[i + 2] & 0xc0) === 0x80 && (bytes[i + 3] & 0xc0) === 0x80) {
      const cp = ((b0 & 0x07) << 18) | ((bytes[i + 1] & 0x3f) << 12) | ((bytes[i + 2] & 0x3f) << 6) | (bytes[i + 3] & 0x3f);
      out += cp >= 0x10000 && cp <= 0x10ffff ? String.fromCodePoint(cp) : '\ufffd';
      i += 4;
    } else {
      out += '\ufffd';
      i += 1;
    }
  }
  return out;
}

/** Decode `%XX` byte sequences properly as UTF-8 (never `String.fromCharCode` Latin-1 corruption). */
export function percentDecodeUtf8(input: string): string {
  if (!input.includes('%')) return input;
  const bytes: number[] = [];
  for (let i = 0; i < input.length; i++) {
    const code = input.charCodeAt(i);
    if (code === 0x25 /* % */ && i + 2 < input.length) {
      const hex = input.slice(i + 1, i + 3);
      if (/^[0-9A-Fa-f]{2}$/.test(hex)) {
        bytes.push(Number.parseInt(hex, 16));
        i += 2;
        continue;
      }
    }
    const encoded = utf8Encode(input[i]);
    for (let j = 0; j < encoded.length; j++) bytes.push(encoded[j]);
  }
  return utf8Decode(bytes);
}

/** Encode an absolute filesystem path into an RFC 3986 `file://` URI, percent-encoding
 *  spaces, `#`, `?`, `%`, brackets, and non-ASCII UTF-8 bytes so LSP servers and URI
 *  parsers never truncate fragments/queries or reject whitespace. */
export function pathToFileUri(filePath: string): string {
  if (filePath.startsWith('file://')) return filePath;
  let normalized = filePath.replace(/\\/g, '/');
  if (!normalized.startsWith('/')) normalized = `/${normalized}`;
  let out = 'file://';
  for (let i = 0; i < normalized.length; i++) {
    const ch = normalized[i];
    const isWinDriveColon =
      i === 2 && ch === ':' && normalized[0] === '/' && /^[A-Za-z]$/.test(normalized[1]);
    if (/^[A-Za-z0-9/\-._~]$/.test(ch) || isWinDriveColon) {
      out += ch;
    } else {
      const codePoint = normalized.codePointAt(i)!;
      const charStr = String.fromCodePoint(codePoint);
      if (codePoint > 0xffff) i++;
      const bytes = utf8Encode(charStr);
      for (let b = 0; b < bytes.length; b++) {
        out += `%${bytes[b].toString(16).toUpperCase().padStart(2, '0')}`;
      }
    }
  }
  return out;
}

/** Convert a `file://` URI (`file:///path`, `file://localhost/path`, `file:///C:/path`)
 *  or plain path back into a normalized filesystem path with full UTF-8 percent-decoding. */
export function fileUriToPath(uriOrPath: string): string {
  if (!uriOrPath) return '';
  if (!uriOrPath.startsWith('file:')) return uriOrPath;
  let rest = uriOrPath.slice('file:'.length);
  if (rest.startsWith('//localhost/')) {
    rest = rest.slice('//localhost'.length);
  } else if (rest.startsWith('///')) {
    rest = rest.slice(2);
  } else if (rest.startsWith('//')) {
    rest = rest.slice(1);
  }
  const decoded = percentDecodeUtf8(rest);
  if (/^\/[A-Za-z]:\//.test(decoded)) return decoded.slice(1);
  return decoded;
}

const BROAD_SYSTEM_ROOTS = new Set([
  '/',
  '/Users',
  '/home',
  '/System',
  '/Library',
  '/Applications',
  '/Volumes',
  '/Network',
  '/cores',
  '/dev',
  '/etc',
  '/private/etc',
  '/opt',
  '/proc',
  '/sbin',
  '/bin',
  '/usr',
  '/tmp',
  '/private/tmp',
  '/var',
  '/private/var',
  '/var/folders',
  '/private/var/folders',
]);

/** Client-side check matching `otto_core::paths::is_broad_non_project_root`:
 *  rejects `/`, `/Users`, `/tmp`, and direct `$HOME` (`/Users/<username>` or `/home/<username>`)
 *  so an accidental `$HOME/.git` dotfile repo never hijacks IDE Git or `Open in Git Module`,
 *  while allowing normal project subdirectories like `/Users/<username>/koala-common-go`. */
export function isBroadNonProjectRootClient(rawPath: string | null | undefined): boolean {
  const trimmed = (rawPath ?? '').trim();
  if (!trimmed) return true;
  const norm = trimmed.length > 1 ? trimmed.replace(/\/+$/, '') : trimmed;
  if (BROAD_SYSTEM_ROOTS.has(norm)) return true;
  if (/^\/Users\/[^/]+$/.test(norm) || /^\/home\/[^/]+$/.test(norm)) return true;
  return false;
}

/** Normalize macOS `/private/var` <-> `/var` and `/private/tmp` <-> `/tmp` firmlink/symlink
 *  prefixes for path prefix comparisons in the UI. */
export function normalizeMacPrivatePath(p: string): string {
  const trimmed = p.replace(/\/+$/, '') || '/';
  if (trimmed === '/private/tmp' || trimmed.startsWith('/private/tmp/')) {
    return trimmed.slice('/private'.length);
  }
  if (trimmed === '/private/var' || trimmed.startsWith('/private/var/')) {
    return trimmed.slice('/private'.length);
  }
  return trimmed;
}
