// Extracted browser export, base64 encoding, and thumbnail rendering helpers
// for Studio3D.svelte (keeps Studio3D.svelte strictly below its LOC cap).
import type * as THREE_NS from 'three';
import * as dapi from '../../../lib/api/design';
import type { DesignArtifact } from '../../../lib/api/types';
import { exportObjectToGlb, exportObjectToUsdz, glbFileName } from '../../../lib/design/scene3d';

export interface ExportableViewport {
  snapshotPng(): Promise<Blob | null>;
  turntableSheet(frames?: number, cell?: number): Promise<Blob | null>;
  contentRoot(): THREE_NS.Object3D | null;
}

export function saveBlob(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 5000);
}

export function bytesToB64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

export async function exportViewportAs(
  vp: ExportableViewport,
  title: string,
  kind: 'glb' | 'usdz' | 'png' | 'sheet',
): Promise<void> {
  if (kind === 'png') {
    const b = await vp.snapshotPng();
    if (b) saveBlob(b, glbFileName(title, 'png'));
    return;
  }
  if (kind === 'sheet') {
    const b = await vp.turntableSheet(8);
    if (b) saveBlob(b, glbFileName(`${title}-turntable`, 'png'));
    return;
  }
  const root = vp.contentRoot();
  if (!root) return;
  const b = kind === 'glb' ? await exportObjectToGlb(root) : await exportObjectToUsdz(root);
  saveBlob(b, glbFileName(title, kind));
}

export async function renderAndPutThumbnail(vp: ExportableViewport, artifactId: string): Promise<void> {
  const b = await vp.snapshotPng();
  if (!b) return;
  try {
    const img = await createImageBitmap(b);
    const w = Math.min(640, img.width);
    const h = Math.round((img.height / img.width) * w);
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    c.getContext('2d')?.drawImage(img, 0, 0, w, h);
    const small = await new Promise<Blob | null>((res) => c.toBlob(res, 'image/png'));
    if (small) await dapi.putThumbnail(artifactId, small);
  } catch {
    /* a thumbnail is a cache — never interrupt editing over it */
  }
}

export async function uploadGlbFileArtifact(
  file: File,
  parent: DesignArtifact,
): Promise<DesignArtifact> {
  const ext = file.name.toLowerCase().endsWith('.gltf') ? 'gltf' : 'glb';
  const bytes = new Uint8Array(await file.arrayBuffer());
  const res = await dapi.createArtifact({
    workspace_id: parent.workspace_id,
    project_id: parent.project_id ?? undefined,
    studio: '3d',
    format: ext,
    title: file.name.replace(/\.(glb|gltf)$/i, ''),
    content_b64: bytesToB64(bytes),
    message: `Imported for ${parent.title}`,
  });
  return res.artifact;
}

export async function listImportable3dModels(parent: DesignArtifact): Promise<DesignArtifact[]> {
  try {
    const f = parent.project_id ? { project_id: parent.project_id } : { workspace_id: parent.workspace_id };
    const [glb, gltf] = await Promise.all([
      dapi.listArtifacts({ ...f, format: 'glb', limit: 50 }),
      dapi.listArtifacts({ ...f, format: 'gltf', limit: 50 }),
    ]);
    return [...glb, ...gltf];
  } catch {
    return [];
  }
}

export function anchorEventNearLeftPanel(): MouseEvent {
  const anchor = document.querySelector<HTMLElement>('[data-testid="s3d-left"]');
  const rect = anchor?.getBoundingClientRect();
  const rtl = !!anchor && getComputedStyle(anchor).direction === 'rtl';
  return {
    clientX: rect ? (rtl ? rect.right - 40 : rect.left + 40) : 240,
    clientY: (rect?.top ?? 120) + 40,
    preventDefault() {},
    stopPropagation() {},
  } as unknown as MouseEvent;
}
