// Read a made piece's skeleton and moves from its model file: only the start
// of the file (a GLB's JSON part comes first); the rest of the download is
// cancelled. Cached per artifact and version.
import { baseUrl, getToken } from '../../../lib/api/client';
import { glbJson, rigFromGltf, type GlbRig } from './rig';

const cache = new Map<string, Promise<GlbRig | null>>();

/** Read a made piece's skeleton and moves (cached per artifact and version). */
export function readRig(artifactId: string, versionId?: string | null): Promise<GlbRig | null> {
  const key = `${artifactId}@${versionId ?? 'head'}`;
  let p = cache.get(key);
  if (!p) {
    p = fetchRig(artifactId, versionId).catch(() => null);
    cache.set(key, p);
  }
  return p;
}

async function fetchRig(id: string, versionId?: string | null): Promise<GlbRig | null> {
  const enc = encodeURIComponent;
  const path = versionId ? `/design/artifacts/${enc(id)}/versions/${enc(versionId)}/content` : `/design/artifacts/${enc(id)}/content`;
  const token = getToken();
  const resp = await fetch(`${baseUrl()}/api/v1${path}`, { headers: token ? { Authorization: `Bearer ${token}` } : {} });
  if (!resp.ok || !resp.body) return null;
  const reader = resp.body.getReader();
  let buf = new Uint8Array(0);
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (value) {
        const next = new Uint8Array(buf.length + value.length);
        next.set(buf);
        next.set(value, buf.length);
        buf = next;
        const json = glbJson(buf);
        if (json) return rigFromGltf(json as Parameters<typeof rigFromGltf>[0]);
        if (buf.length >= 20 && new DataView(buf.buffer).getUint32(0, true) !== 0x46546c67) return null;
      }
      if (done) return null;
    }
  } finally {
    void reader.cancel().catch(() => {});
  }
}
