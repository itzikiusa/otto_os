// SkeletonHelper overlay + AnimationMixer playback controller for rigged/animated
// GLB/glTF models. Lazy-three safe (accepts THREE namespace, zero static three import).
import type * as THREE_NS from 'three';
import type { Three } from './build';

export interface AnimClipInfo {
  index: number;
  name: string;
  duration: number;
}

export interface RigInfo {
  hasSkeleton: boolean;
  boneCount: number;
  clips: AnimClipInfo[];
}

export const EMPTY_RIG_INFO: RigInfo = { hasSkeleton: false, boneCount: 0, clips: [] };

export function formatAnimDuration(secs: number): string {
  if (!Number.isFinite(secs) || secs <= 0) return '0.0s';
  return `${secs.toFixed(1)}s`;
}

export class RigPlayer {
  readonly THREE: Three;
  readonly skeletonGroup: THREE_NS.Group;
  private mixer: THREE_NS.AnimationMixer | null = null;
  /** Every other animated model of a scene: each plays its own first clip (its default: a pose, idle). */
  private others: { mixer: THREE_NS.AnimationMixer; action: THREE_NS.AnimationAction }[] = [];
  private clips: THREE_NS.AnimationClip[] = [];
  private activeAction: THREE_NS.AnimationAction | null = null;
  private clipIndex = 0;
  private playing = false;
  private showSkeleton = false;
  private rigInfo: RigInfo = EMPTY_RIG_INFO;

  constructor(THREE: Three, helpers: THREE_NS.Group) {
    this.THREE = THREE;
    this.skeletonGroup = new THREE.Group();
    this.skeletonGroup.name = 'skeleton-helpers';
    this.skeletonGroup.visible = false;
    helpers.add(this.skeletonGroup);
  }

  get info(): RigInfo {
    return this.rigInfo;
  }

  get isAnimating(): boolean {
    return this.playing && (this.activeAction !== null || this.others.length > 0);
  }

  /** Scan `content` for bones/skinned meshes and `userData.ottoAnimations` clips. */
  syncFromContent(content: THREE_NS.Object3D): RigInfo {
    this.clearHelpers();
    this.stopMixer();

    const boneRoots: THREE_NS.Object3D[] = [];
    let boneCount = 0;
    let clipRoot: THREE_NS.Object3D | null = null;
    const foundClips: THREE_NS.AnimationClip[] = [];
    const otherRoots: [THREE_NS.Object3D, THREE_NS.AnimationClip][] = [];

    content.traverse((o) => {
      if ((o as THREE_NS.Bone).isBone) boneCount++;
      const sm = o as THREE_NS.SkinnedMesh;
      if (sm.isSkinnedMesh && sm.skeleton?.bones.length) {
        const rootBone = sm.skeleton.bones[0].parent ?? sm.skeleton.bones[0];
        if (!boneRoots.includes(rootBone)) boneRoots.push(rootBone);
      }
      const anims = o.userData?.ottoAnimations as THREE_NS.AnimationClip[] | undefined;
      if (Array.isArray(anims) && anims.length > 0) {
        if (foundClips.length === 0) {
          clipRoot = o;
          foundClips.push(...anims);
        } else otherRoots.push([o, anims[0]]);
      }
    });

    for (const root of boneRoots) {
      const sh = new this.THREE.SkeletonHelper(root);
      sh.renderOrder = 15;
      this.skeletonGroup.add(sh);
    }
    this.skeletonGroup.visible = this.showSkeleton && boneRoots.length > 0;

    this.clips = foundClips;
    if (clipRoot && foundClips.length > 0) {
      this.mixer = new this.THREE.AnimationMixer(clipRoot);
      const clamped = Math.min(Math.max(0, this.clipIndex), foundClips.length - 1);
      this.clipIndex = clamped;
      this.activateClip(clamped);
    }
    // Not only the first: every animated model stands in its own first clip (a pose holds from frame 0).
    this.others = otherRoots.map(([root, clip]) => {
      const mixer = new this.THREE.AnimationMixer(root);
      const action = mixer.clipAction(clip);
      action.play();
      action.paused = !this.playing;
      mixer.update(0);
      return { mixer, action };
    });

    this.rigInfo = {
      hasSkeleton: boneCount > 0 || boneRoots.length > 0,
      boneCount,
      clips: foundClips.map((c, index) => ({
        index,
        name: c.name?.trim() || `Clip ${index + 1}`,
        duration: Math.round(c.duration * 100) / 100,
      })),
    };
    return this.rigInfo;
  }

  setSkeletonVisible(visible: boolean): void {
    this.showSkeleton = visible;
    this.skeletonGroup.visible = visible && this.skeletonGroup.children.length > 0;
  }

  setPlayback(clipIndex: number, playing: boolean): void {
    this.playing = playing;
    if (!this.mixer || this.clips.length === 0) return;
    const clamped = Math.min(Math.max(0, clipIndex), this.clips.length - 1);
    if (clamped !== this.clipIndex || !this.activeAction) {
      this.clipIndex = clamped;
      this.activateClip(clamped);
    }
    if (this.activeAction) this.activeAction.paused = !playing;
    for (const o of this.others) o.action.paused = !playing;
  }

  step(dt: number): boolean {
    if (!this.playing || (!this.activeAction && this.others.length === 0)) return false;
    if (this.activeAction) this.mixer?.update(dt);
    for (const o of this.others) o.mixer.update(dt);
    return true;
  }

  private activateClip(index: number): void {
    if (!this.mixer) return;
    this.mixer.stopAllAction();
    const clip = this.clips[index];
    if (!clip) {
      this.activeAction = null;
      return;
    }
    const action = this.mixer.clipAction(clip);
    action.reset();
    action.play();
    action.paused = !this.playing;
    this.activeAction = action;
    this.mixer.update(0);
  }

  private clearHelpers(): void {
    for (const child of [...this.skeletonGroup.children]) {
      child.removeFromParent();
      const sh = child as THREE_NS.SkeletonHelper;
      sh.dispose?.();
    }
  }

  private stopMixer(): void {
    for (const o of this.others) o.mixer.stopAllAction();
    this.others = [];
    this.mixer?.stopAllAction();
    this.mixer = null;
    this.activeAction = null;
    this.clips = [];
  }

  dispose(): void {
    this.clearHelpers();
    this.stopMixer();
    this.skeletonGroup.removeFromParent();
  }
}
