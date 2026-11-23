// Encapsulates the Three.js viewport's inspection, shading, polygon picking,
// face highlight overlays, scale reference, and 3D pin screen-space projection.
// Keeps Scene3DViewport.svelte lean and strictly below its LOC ratchet.
import type * as THREE_NS from 'three';
import type { Three } from './build';
import {
  buildScaleReference,
  createShadingController,
  findMeshByUid,
  inspectSceneTopology,
  type MeshTopologyStats,
  type ShadingController,
  type ShadingMode,
  type SubMeshInfo,
} from './meshInspector';
import {
  defaultCameraForAnchor,
  isBackfacing,
  ndcToViewportPx,
  pin3dLabel,
  roundVec3,
  type CameraPose3d,
  type Pin3dAnchor,
  type ProjectedPin3d,
  type SelectionKind3d,
} from './pin3d';
import {
  createPolygonHighlight,
  extractWorldFaceTriangles,
  findMeshForAnchor,
  raycastPolygon,
  resolveMeshIdentity,
  type PolygonHighlightHandle,
} from './polygonTools';
import { RigPlayer, type RigInfo } from './rigPlayer';

export interface Pin3dInput {
  id: string;
  n: number;
  body: string;
  muted: boolean;
  anchor: Pin3dAnchor;
}

export class ViewportInspection {
  readonly THREE: Three;
  readonly shading: ShadingController;
  readonly hoverHighlight: PolygonHighlightHandle;
  readonly activeHighlight: PolygonHighlightHandle;
  readonly scaleRefGroup: THREE_NS.Group;
  readonly rig: RigPlayer;

  shadingMode: ShadingMode = 'shaded';
  wireframeOverlay = false;

  constructor(THREE: Three, helpers: THREE_NS.Group) {
    this.THREE = THREE;
    this.shading = createShadingController(THREE);
    this.hoverHighlight = createPolygonHighlight(THREE, 0x38bdf8, 0.36);
    this.activeHighlight = createPolygonHighlight(THREE, 0xf59e0b, 0.48);
    this.scaleRefGroup = buildScaleReference(THREE);
    this.scaleRefGroup.visible = false;
    helpers.add(this.hoverHighlight.group, this.activeHighlight.group, this.scaleRefGroup);
    this.rig = new RigPlayer(THREE, helpers);
  }

  syncRig(content: THREE_NS.Object3D): RigInfo {
    return this.rig.syncFromContent(content);
  }

  applyShading(content: THREE_NS.Object3D, mode: ShadingMode, wireOverlay: boolean): void {
    this.shadingMode = mode;
    this.wireframeOverlay = wireOverlay;
    this.shading.apply(content, mode, wireOverlay);
  }

  reapplyShading(content: THREE_NS.Object3D): void {
    if (this.shadingMode !== 'shaded' || this.wireframeOverlay) {
      this.shading.apply(content, this.shadingMode, this.wireframeOverlay);
    }
  }

  setScaleRef(visible: boolean): void {
    this.scaleRefGroup.visible = visible;
  }

  inspect(content: THREE_NS.Object3D): { meshes: SubMeshInfo[]; stats: MeshTopologyStats } {
    return inspectSceneTopology(this.THREE, content);
  }

  pickSurface(
    renderer: THREE_NS.WebGLRenderer,
    camera: THREE_NS.PerspectiveCamera,
    orbitTarget: THREE_NS.Vector3,
    content: THREE_NS.Object3D,
    clientX: number,
    clientY: number,
    kind: SelectionKind3d,
  ): Pin3dAnchor | null {
    const hit = raycastPolygon(this.THREE, renderer, camera, orbitTarget, content, clientX, clientY, kind);
    if (!hit) {
      this.hoverHighlight.setTriangles(null);
      return null;
    }
    if (hit.anchor.selection_kind === 'mesh') {
      this.hoverHighlight.setMeshHighlight(hit.mesh);
    } else if (hit.triangles.length >= 9) {
      this.hoverHighlight.setTriangles(hit.triangles);
    } else {
      this.hoverHighlight.setTriangles(null);
    }
    return hit.anchor;
  }

  clearHover(): void {
    this.hoverHighlight.setTriangles(null);
  }

  highlightAnchor(content: THREE_NS.Object3D, anchor: Pin3dAnchor | null): void {
    if (!anchor) {
      this.activeHighlight.setTriangles(null);
      return;
    }
    const mesh = findMeshForAnchor(content, anchor);
    if (!mesh) {
      this.activeHighlight.setTriangles(null);
      return;
    }
    if (anchor.selection_kind === 'mesh') {
      this.activeHighlight.setMeshHighlight(mesh);
      return;
    }
    const faces = anchor.face_indices?.length
      ? anchor.face_indices
      : anchor.face_index != null
        ? [anchor.face_index]
        : [];
    if (faces.length > 0) {
      const ext = extractWorldFaceTriangles(this.THREE, mesh, faces);
      if (ext) {
        this.activeHighlight.setTriangles(ext.positions);
        return;
      }
    }
    this.activeHighlight.setMeshHighlight(mesh);
  }

  anchorForMeshUid(
    content: THREE_NS.Object3D,
    camera: THREE_NS.PerspectiveCamera,
    orbitTarget: THREE_NS.Vector3,
    uid: string,
  ): Pin3dAnchor | null {
    const mesh = findMeshByUid(content, uid);
    if (!mesh) return null;
    const { meshName, objectId } = resolveMeshIdentity(mesh);
    const box = new this.THREE.Box3().setFromObject(mesh);
    const center = box.getCenter(new this.THREE.Vector3());
    const camPose: CameraPose3d = {
      pos: roundVec3([camera.position.x, camera.position.y, camera.position.z]),
      target: roundVec3([orbitTarget.x, orbitTarget.y, orbitTarget.z]),
      fov: Math.round(camera.fov),
    };
    return {
      selection_kind: 'mesh',
      mesh_name: meshName,
      object_id: objectId,
      position: roundVec3([center.x, center.y, center.z]),
      normal: [0, 1, 0],
      camera: camPose,
    };
  }

  cameraForMeshUid(content: THREE_NS.Object3D, camera: THREE_NS.PerspectiveCamera, orbitTarget: THREE_NS.Vector3, uid: string): CameraPose3d | null {
    const mesh = findMeshByUid(content, uid);
    if (!mesh) return null;
    const box = new this.THREE.Box3().setFromObject(mesh);
    if (box.isEmpty()) return null;
    const center = box.getCenter(new this.THREE.Vector3());
    const size = box.getSize(new this.THREE.Vector3()).length() || 1;
    const dist = (size / 2) / Math.tan((camera.fov * Math.PI) / 360) * 1.4;
    const dir = camera.position.clone().sub(orbitTarget).normalize();
    if (!Number.isFinite(dir.length()) || dir.length() === 0) dir.set(1, 0.7, 1).normalize();
    const pos = center.clone().add(dir.multiplyScalar(Math.max(0.6, dist)));
    return {
      pos: roundVec3([pos.x, pos.y, pos.z]),
      target: roundVec3([center.x, center.y, center.z]),
      fov: camera.fov,
    };
  }

  toggleMeshVisible(content: THREE_NS.Object3D, uid: string): boolean {
    const mesh = findMeshByUid(content, uid);
    if (!mesh) return false;
    mesh.visible = !mesh.visible;
    return true;
  }

  cameraForAnchor(anchor: Pin3dAnchor): CameraPose3d | null {
    return defaultCameraForAnchor(anchor);
  }

  projectPins(
    camera: THREE_NS.PerspectiveCamera,
    width: number,
    height: number,
    inputs: readonly Pin3dInput[],
  ): ProjectedPin3d[] {
    if (inputs.length === 0 || width <= 0 || height <= 0) return [];
    const camPos: [number, number, number] = [camera.position.x, camera.position.y, camera.position.z];
    const v = new this.THREE.Vector3();
    return inputs.map((item) => {
      const p = item.anchor.position;
      v.set(p[0], p[1], p[2]).project(camera);
      const { x, y, inBounds } = ndcToViewportPx(v.x, v.y, v.z, width, height);
      return {
        id: item.id,
        n: item.n,
        x,
        y,
        visible: inBounds,
        backfacing: isBackfacing(p, item.anchor.normal, camPos),
        muted: item.muted,
        label: pin3dLabel(item.anchor),
        body: item.body,
        anchor: item.anchor,
      };
    });
  }

  dispose(): void {
    this.rig.dispose();
    this.hoverHighlight.dispose();
    this.activeHighlight.dispose();
    this.shading.dispose();
    this.scaleRefGroup.traverse((o) => {
      const ls = o as THREE_NS.LineSegments;
      ls.geometry?.dispose();
      (ls.material as THREE_NS.Material | undefined)?.dispose();
    });
  }
}
