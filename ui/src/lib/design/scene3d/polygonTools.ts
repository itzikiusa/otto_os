// Three.js polygon / triangle / region / mesh raycast extraction and visual
// highlight overlay builder. Lazy-three safe (accepts the `THREE` namespace,
// zero static `import * as THREE`).
import type * as THREE_NS from 'three';
import { USERDATA_ID, USERDATA_KIND, type Three } from './build';
import {
  normalizeFaceIndices,
  roundVec3,
  type CameraPose3d,
  type Pin3dAnchor,
  type SelectionKind3d,
} from './pin3d';

export interface PolygonHitResult {
  anchor: Pin3dAnchor;
  mesh: THREE_NS.Mesh;
  /** World-space triangle vertices for immediate overlay rendering. */
  triangles: Float32Array;
}

/** Walk up to the owning `scene3d` object id (if any) and best readable mesh name. */
export function resolveMeshIdentity(obj: THREE_NS.Object3D): {
  meshName: string;
  objectId: string | null;
} {
  let objectId: string | null = null;
  let cur: THREE_NS.Object3D | null = obj;
  while (cur) {
    const id = cur.userData?.[USERDATA_ID] as string | undefined;
    const kind = cur.userData?.[USERDATA_KIND] as string | undefined;
    if (id && kind === 'object') {
      objectId = id;
      break;
    }
    cur = cur.parent;
  }
  const ownName = obj.name?.trim();
  const parentName = obj.parent?.name?.trim();
  const meshName =
    ownName && ownName !== 'placeholder'
      ? ownName
      : parentName && parentName !== 'content'
        ? parentName
        : objectId ?? 'Mesh';
  return { meshName, objectId };
}

/** Read the 3 vertex indices of triangle `faceIndex` in `geo`. */
export function faceVertexIndices(
  geo: THREE_NS.BufferGeometry,
  faceIndex: number,
): [number, number, number] | null {
  const pos = geo.getAttribute('position');
  if (!pos) return null;
  const idx = geo.getIndex();
  if (idx) {
    const base = faceIndex * 3;
    if (base + 2 >= idx.count) return null;
    return [idx.getX(base), idx.getX(base + 1), idx.getX(base + 2)];
  }
  const base = faceIndex * 3;
  if (base + 2 >= pos.count) return null;
  return [base, base + 1, base + 2];
}

/** Extract world-space triangle positions for one or more face indices on `mesh`,
 *  nudged slightly along the triangle normal to prevent z-fighting. */
export function extractWorldFaceTriangles(
  THREE: Three,
  mesh: THREE_NS.Mesh,
  faceIndices: readonly number[],
  offset = 0.0025,
): { positions: Float32Array; centroid: [number, number, number]; normal: [number, number, number] } | null {
  const geo = mesh.geometry as THREE_NS.BufferGeometry | undefined;
  const posAttr = geo?.getAttribute('position');
  if (!geo || !posAttr) return null;

  mesh.updateWorldMatrix(true, false);
  const m = mesh.matrixWorld;
  const validFaces = normalizeFaceIndices(faceIndices);
  if (validFaces.length === 0) return null;

  const verts: number[] = [];
  const vA = new THREE.Vector3();
  const vB = new THREE.Vector3();
  const vC = new THREE.Vector3();
  const e1 = new THREE.Vector3();
  const e2 = new THREE.Vector3();
  const fn = new THREE.Vector3();
  const avgCenter = new THREE.Vector3();
  const avgNormal = new THREE.Vector3();
  let count = 0;

  for (const fi of validFaces) {
    const tri = faceVertexIndices(geo, fi);
    if (!tri) continue;
    vA.fromBufferAttribute(posAttr, tri[0]).applyMatrix4(m);
    vB.fromBufferAttribute(posAttr, tri[1]).applyMatrix4(m);
    vC.fromBufferAttribute(posAttr, tri[2]).applyMatrix4(m);
    e1.subVectors(vB, vA);
    e2.subVectors(vC, vA);
    fn.crossVectors(e1, e2);
    if (fn.lengthSq() > 1e-12) fn.normalize();
    else fn.set(0, 1, 0);

    avgCenter.add(vA).add(vB).add(vC);
    avgNormal.add(fn);
    count++;

    const ox = fn.x * offset;
    const oy = fn.y * offset;
    const oz = fn.z * offset;
    verts.push(vA.x + ox, vA.y + oy, vA.z + oz, vB.x + ox, vB.y + oy, vB.z + oz, vC.x + ox, vC.y + oy, vC.z + oz);
  }

  if (count === 0) return null;
  avgCenter.divideScalar(count * 3);
  if (avgNormal.lengthSq() > 1e-12) avgNormal.normalize();
  else avgNormal.set(0, 1, 0);

  return {
    positions: new Float32Array(verts),
    centroid: roundVec3([avgCenter.x, avgCenter.y, avgCenter.z]),
    normal: roundVec3([avgNormal.x, avgNormal.y, avgNormal.z]),
  };
}

/** Raycast against `content` and build a structured `Pin3dAnchor` for the chosen selection kind. */
export function raycastPolygon(
  THREE: Three,
  renderer: THREE_NS.WebGLRenderer,
  camera: THREE_NS.PerspectiveCamera,
  orbitTarget: THREE_NS.Vector3,
  content: THREE_NS.Object3D,
  clientX: number,
  clientY: number,
  selectionKind: SelectionKind3d,
): PolygonHitResult | null {
  const rect = renderer.domElement.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  const ndc = new THREE.Vector2(
    ((clientX - rect.left) / rect.width) * 2 - 1,
    -((clientY - rect.top) / rect.height) * 2 + 1,
  );
  const ray = new THREE.Raycaster();
  ray.setFromCamera(ndc, camera);
  const hits = ray.intersectObject(content, true);
  for (const h of hits) {
    const mesh = h.object as THREE_NS.Mesh;
    if (!mesh.isMesh || !mesh.visible || mesh.name === 'placeholder') continue;
    const { meshName, objectId } = resolveMeshIdentity(mesh);
    const camPose: CameraPose3d = {
      pos: roundVec3([camera.position.x, camera.position.y, camera.position.z]),
      target: roundVec3([orbitTarget.x, orbitTarget.y, orbitTarget.z]),
      fov: Math.round(camera.fov),
    };

    const faceIndex = typeof h.faceIndex === 'number' && h.faceIndex >= 0 ? h.faceIndex : null;
    const geo = mesh.geometry as THREE_NS.BufferGeometry | undefined;
    const verts = faceIndex != null && geo ? faceVertexIndices(geo, faceIndex) : null;

    let worldNormal: [number, number, number] = [0, 1, 0];
    if (h.face?.normal) {
      const nMatrix = new THREE.Matrix3().getNormalMatrix(mesh.matrixWorld);
      const wn = h.face.normal.clone().applyMatrix3(nMatrix).normalize();
      worldNormal = roundVec3([wn.x, wn.y, wn.z]);
    }

    if (selectionKind === 'mesh') {
      const box = new THREE.Box3().setFromObject(mesh);
      const center = box.getCenter(new THREE.Vector3());
      return {
        mesh,
        triangles: new Float32Array(0),
        anchor: {
          selection_kind: 'mesh',
          mesh_name: meshName,
          object_id: objectId,
          position: roundVec3([center.x, center.y, center.z]),
          normal: worldNormal,
          camera: camPose,
        },
      };
    }

    if ((selectionKind === 'polygon' || selectionKind === 'region') && faceIndex != null) {
      const extracted = extractWorldFaceTriangles(THREE, mesh, [faceIndex]);
      if (extracted) {
        return {
          mesh,
          triangles: extracted.positions,
          anchor: {
            selection_kind: selectionKind === 'region' ? 'region' : 'polygon',
            mesh_name: meshName,
            object_id: objectId,
            face_index: faceIndex,
            face_indices: [faceIndex],
            vertices: verts,
            position: extracted.centroid,
            normal: extracted.normal,
            camera: camPose,
          },
        };
      }
    }

    // Fallback: exact surface point
    return {
      mesh,
      triangles: new Float32Array(0),
      anchor: {
        selection_kind: 'point',
        mesh_name: meshName,
        object_id: objectId,
        face_index: faceIndex,
        vertices: verts,
        position: roundVec3([h.point.x, h.point.y, h.point.z]),
        normal: worldNormal,
        camera: camPose,
      },
    };
  }
  return null;
}

/** Find a Three.js mesh inside `content` matching a comment's `mesh_name` / `object_id`. */
export function findMeshForAnchor(
  content: THREE_NS.Object3D,
  anchor: Pin3dAnchor,
): THREE_NS.Mesh | null {
  let fallback: THREE_NS.Mesh | null = null;
  const wantName = anchor.mesh_name?.trim() || '';
  const wantObj = anchor.object_id?.trim() || '';
  content.traverse((o) => {
    const m = o as THREE_NS.Mesh;
    if (!m.isMesh || m.name === 'placeholder') return;
    const id = resolveMeshIdentity(m);
    if (wantName && id.meshName === wantName && (!wantObj || id.objectId === wantObj)) {
      fallback = m;
    } else if (!fallback && wantName && id.meshName === wantName) {
      fallback = m;
    } else if (!fallback && wantObj && id.objectId === wantObj) {
      fallback = m;
    }
  });
  return fallback;
}

export interface PolygonHighlightHandle {
  group: THREE_NS.Group;
  setTriangles(positions: Float32Array | null): void;
  setMeshHighlight(mesh: THREE_NS.Mesh | null): void;
  dispose(): void;
}

/** Manages a reusable non-z-fighting triangle fill + outline + optional mesh bbox highlight. */
export function createPolygonHighlight(
  THREE: Three,
  colorHex: number,
  opacity = 0.42,
): PolygonHighlightHandle {
  const group = new THREE.Group();
  group.name = 'polygon-highlight';
  group.renderOrder = 20;

  let fillMesh: THREE_NS.Mesh | null = null;
  let edgeLines: THREE_NS.LineSegments | null = null;
  let meshBox: THREE_NS.BoxHelper | null = null;

  const fillMat = new THREE.MeshBasicMaterial({
    color: colorHex,
    transparent: true,
    opacity,
    side: THREE.DoubleSide,
    depthTest: true,
    depthWrite: false,
    polygonOffset: true,
    polygonOffsetFactor: -2,
    polygonOffsetUnits: -2,
  });
  const edgeMat = new THREE.LineBasicMaterial({
    color: colorHex,
    transparent: true,
    opacity: 0.95,
    depthTest: true,
  });

  function clearFaces(): void {
    if (fillMesh) {
      fillMesh.removeFromParent();
      fillMesh.geometry.dispose();
      fillMesh = null;
    }
    if (edgeLines) {
      edgeLines.removeFromParent();
      edgeLines.geometry.dispose();
      edgeLines = null;
    }
  }

  function clearMeshBox(): void {
    if (meshBox) {
      meshBox.removeFromParent();
      meshBox.dispose();
      meshBox = null;
    }
  }

  return {
    group,
    setTriangles(positions) {
      clearFaces();
      clearMeshBox();
      if (!positions || positions.length < 9) return;
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
      geo.computeVertexNormals();
      fillMesh = new THREE.Mesh(geo, fillMat);
      fillMesh.renderOrder = 20;
      const wireGeo = new THREE.WireframeGeometry(geo);
      edgeLines = new THREE.LineSegments(wireGeo, edgeMat);
      edgeLines.renderOrder = 21;
      group.add(fillMesh, edgeLines);
    },
    setMeshHighlight(mesh) {
      clearFaces();
      clearMeshBox();
      if (!mesh) return;
      meshBox = new THREE.BoxHelper(mesh, colorHex);
      meshBox.renderOrder = 20;
      group.add(meshBox);
    },
    dispose() {
      clearFaces();
      clearMeshBox();
      fillMat.dispose();
      edgeMat.dispose();
    },
  };
}
