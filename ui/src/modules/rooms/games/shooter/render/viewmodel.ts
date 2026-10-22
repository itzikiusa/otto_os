/**
 * First-person weapon view: the held gun (from guns.ts) in gloved hands with
 * sleeves, drawn in its own small scene over the world (GameView.overlay) so
 * it never clips into walls. Hip pose low and right, aim-down-sights pose
 * with the sight on the crosshair; sway lags behind turning, bob follows the
 * stride, recoil kicks back and up and settles, reloads dip the gun, sprint
 * lowers it, and every shot shows a muzzle flash with a light on the hands.
 */
import * as THREE from 'three';
import { approach, clamp } from '../../common/math';
import { flashTexture } from '../../common/textures';
import type { Theme } from '../../common/view';
import { GUNS, type GunSpec } from './guns';
import type { Skin } from './soldiers';

const _c = new THREE.Color();
/** Shoulder-fired: the tube rides beside the eye instead of under it. */
const GUN_SHOULDER = GUNS.rocket;

export interface ViewmodelInput {
  gun: GunSpec | null; ads: number; speed: number; sprint: boolean; reload: boolean; grounded: boolean;
  /** Looking through a scope (the HUD draws the reticle): the gun is put away from the eye. */
  scoped?: boolean;
  /** View turn rates (rad/s) for sway, and the main camera's aspect. */
  yawRate: number; pitchRate: number; aspect: number;
}

export class Viewmodel {
  readonly scene = new THREE.Scene();
  readonly camera = new THREE.PerspectiveCamera(54, 1, 0.01, 10);
  private hemi: THREE.HemisphereLight; private sun: THREE.DirectionalLight; private glow: THREE.PointLight;
  private rig = new THREE.Group();
  private holder = new THREE.Group();
  private guns = new Map<GunSpec, THREE.Group>();
  private current: GunSpec | null = null;
  private flash: THREE.Mesh;
  private sleeve: THREE.MeshStandardMaterial; private glove: THREE.MeshStandardMaterial;
  private swatches = new Map<number, THREE.MeshStandardMaterial>();
  private box = new THREE.BoxGeometry(1, 1, 1);
  private limb = new THREE.CapsuleGeometry(0.5, 1, 3, 10);
  private resources: { dispose(): void }[] = [];
  private kickZ = 0; private kickPitch = 0; private kickYaw = 0; private flashT = 0;
  private swayX = 0; private swayY = 0; private bob = 0; private dip = 0; private lower = 0; private adsBlend = 0;

  constructor() {
    this.scene.visible = false;
    this.hemi = new THREE.HemisphereLight(0xffffff, 0x444444, 1.2);
    this.sun = new THREE.DirectionalLight(0xffffff, 1.6);
    this.sun.position.set(1, 2, 1.5);
    this.glow = new THREE.PointLight(0xffb060, 0, 3, 2);
    this.sleeve = new THREE.MeshStandardMaterial({ color: 0x5b6346, roughness: 0.9 });
    this.glove = new THREE.MeshStandardMaterial({ color: 0x2b2b2b, roughness: 0.7 });
    this.flash = new THREE.Mesh(new THREE.PlaneGeometry(0.22, 0.22), new THREE.MeshBasicMaterial({ map: flashTexture(), transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, fog: false }));
    this.flash.visible = false;
    this.rig.add(this.holder);
    this.scene.add(this.hemi, this.sun, this.glow, this.rig);
    this.resources.push(this.box, this.limb, this.sleeve, this.glove, this.flash.geometry, this.flash.material as THREE.Material, (this.flash.material as THREE.MeshBasicMaterial).map!);
  }

  /** Matches the world's light so the gun sits in the same time of day. */
  setLighting(theme: Theme): void {
    this.hemi.color.setHex(theme.ambientSky); this.hemi.groundColor.setHex(theme.ambientGround); this.hemi.intensity = theme.ambient * 1.6;
    this.sun.color.setHex(theme.sun); this.sun.intensity = Math.max(0.8, theme.sunIntensity * 0.9);
  }
  /** Sleeves and gloves in the local survivor's colors. */
  setSkin(s: Skin): void { this.sleeve.color.setHex(s.top); this.glove.color.setHex(s.gloves); }

  private swatch(hex: number): THREE.MeshStandardMaterial {
    let m = this.swatches.get(hex);
    if (!m) {
      _c.setHex(hex);
      const metal = _c.r + _c.g + _c.b > 0.9 && Math.abs(_c.r - _c.b) < 0.1;
      m = new THREE.MeshStandardMaterial({ color: hex, roughness: metal ? 0.35 : 0.7, metalness: metal ? 0.7 : 0.15 });
      this.swatches.set(hex, m); this.resources.push(m);
    }
    return m;
  }

  /** The gun with both hands on it (built once per item). */
  private build(spec: GunSpec): THREE.Group {
    const g = new THREE.Group();
    for (const [x, y, z, w, h, d, sw] of spec.parts) {
      // Stocks and anything else wholly behind the grip would fill the screen (the eye sits over it): left out.
      if (z + d / 2 < -0.05 && spec !== GUN_SHOULDER) continue;
      // Optics reach the sight line: from the eye they would be a solid block over the target (the crosshair stands in for the reticle).
      if (y + h / 2 > spec.sight - 0.01 && spec !== GUN_SHOULDER && spec !== GUNS.pickaxe && sw === 1 && h < 0.08) continue;
      const m = new THREE.Mesh(this.box, this.swatch(spec.colors[sw]));
      m.position.set(x, y, z); m.scale.set(w, h, d);
      g.add(m);
    }
    // Right hand on the grip, left hand under the fore-end; forearms run back out of view.
    const grip = spec.parts.find(p => p[1] < -0.02 && Math.abs(p[2]) < 0.06) ?? spec.parts[0];
    const fore = spec.parts.reduce((a, p) => (p[2] > a[2] && p[2] < spec.muzzle[2] - 0.08 ? p : a), spec.parts[0]);
    const hand = (x: number, y: number, z: number, roll: number) => {
      const glove = new THREE.Mesh(this.box, this.glove);
      glove.position.set(x, y, z); glove.scale.set(0.075, 0.09, 0.11); glove.rotation.z = roll;
      const arm = new THREE.Mesh(this.limb, this.sleeve);
      arm.scale.set(0.085, 0.24, 0.085);
      arm.rotation.x = Math.PI / 2 - 0.35; arm.rotation.z = roll * 0.6;
      arm.position.set(x + (x > 0 ? 0.05 : -0.03), y - 0.12, z - 0.24);
      g.add(glove, arm);
    };
    hand(-0.005, grip[1] - 0.01, grip[2], 0.2);
    if (spec.parts.length > 2) hand(0.025, fore[1] - 0.05, Math.min(fore[2], spec.muzzle[2] - 0.12), -0.3);
    this.flash.removeFromParent();
    // The pickaxe head is wide: carried smaller and lower so it frames the view instead of filling it.
    if (spec === GUNS.pickaxe) { g.scale.setScalar(0.6); g.position.set(0.05, -0.08, 0); }
    g.visible = false;
    this.holder.add(g);
    return g;
  }

  /** A shot from the local gun: kick and flash (`strength` 0..1, e.g. heavier for snipers / shotguns). */
  kick(strength: number): void {
    this.kickZ = Math.min(0.12, this.kickZ + 0.035 + strength * 0.05);
    this.kickPitch = Math.min(0.35, this.kickPitch + 0.05 + strength * 0.12);
    this.kickYaw += (Math.random() - 0.5) * 0.04 * (1 + strength);
    this.flashT = 0.045;
  }

  update(dt: number, i: ViewmodelInput): void {
    this.camera.aspect = i.aspect;
    this.camera.updateProjectionMatrix();
    if (i.gun !== this.current) {
      if (this.current) this.guns.get(this.current)!.visible = false;
      this.current = i.gun;
      if (i.gun) {
        let g = this.guns.get(i.gun);
        if (!g) { g = this.build(i.gun); this.guns.set(i.gun, g); }
        g.visible = true;
        g.add(this.flash);
        this.flash.position.set(i.gun.muzzle[0], i.gun.muzzle[1], i.gun.muzzle[2] + 0.02);
        // Swapping weapons: the new one comes up from below.
        this.dip = 1;
      }
    }
    const spec = i.gun;
    if (!spec) return;
    this.adsBlend = approach(this.adsBlend, clamp(i.ads, 0, 1), 18, dt);
    const a = this.adsBlend;
    // Sway lags against the turn (less in the sights), bob rides the stride.
    this.swayX = approach(this.swayX, clamp(-i.yawRate * 0.012, -0.05, 0.05) * (1 - a * 0.8), 10, dt);
    this.swayY = approach(this.swayY, clamp(i.pitchRate * 0.01, -0.04, 0.04) * (1 - a * 0.8), 10, dt);
    const moving = i.grounded ? clamp(i.speed / 6, 0, 1.3) : 0;
    this.bob += dt * (6 + i.speed * 1.2) * (moving > 0.05 ? 1 : 0);
    const bobX = Math.sin(this.bob) * 0.012 * moving * (1 - a * 0.85), bobY = -Math.abs(Math.cos(this.bob)) * 0.014 * moving * (1 - a * 0.85);
    this.lower = approach(this.lower, i.sprint && a < 0.2 ? 1 : 0, 9, dt);
    this.dip = approach(this.dip, i.reload ? 1 : 0, i.reload ? 9 : 6, dt);
    // Recoil recovers.
    this.kickZ = approach(this.kickZ, 0, 14, dt); this.kickPitch = approach(this.kickPitch, 0, 12, dt); this.kickYaw = approach(this.kickYaw, 0, 10, dt);
    // Hip: low right; sights: centred with the sight line on the view axis.
    const hipX = 0.15, hipY = -0.15 - spec.sight * 0.4, hipZ = -0.34;
    const shoulder = spec === GUN_SHOULDER;
    // Iron sights sit just under the crosshair and a little further out, so the receiver frames the
    // target instead of covering the lower half of the view.
    const adsX = shoulder ? 0.14 : 0, adsY = shoulder ? -0.1 : -spec.sight - 0.028, adsZ = (shoulder ? -0.1 : -0.42) - Math.max(0, spec.muzzle[2] - 0.45) * 0.2;
    this.holder.visible = !(i.scoped && a > 0.8);
    this.rig.position.set(
      hipX + (adsX - hipX) * a + this.swayX + bobX + this.lower * 0.04,
      hipY + (adsY - hipY) * a + this.swayY + bobY - this.dip * 0.12 - this.lower * 0.06,
      hipZ + (adsZ - hipZ) * a + this.kickZ,
    );
    // The gun points down -z (it is modelled toward +z); sprint cants it, reloads tip it down.
    this.rig.rotation.set(-this.kickPitch + this.dip * 0.6 + this.lower * 0.55 - this.swayY * 2, Math.PI + this.kickYaw - this.lower * 0.5 + this.swayX * 2 * (1 - a), (1 - a) * 0.04 + this.lower * 0.35, 'YXZ');
    // Muzzle flash and a warm light on the hands.
    this.flashT -= dt;
    const firing = this.flashT > 0;
    this.flash.visible = firing;
    if (firing) { this.flash.rotation.z = Math.random() * Math.PI; this.flash.scale.setScalar(0.8 + Math.random() * 0.7); }
    this.glow.intensity = firing ? 6 : 0;
    this.glow.position.set(this.rig.position.x, this.rig.position.y + 0.05, this.rig.position.z - 0.35);
  }

  dispose(): void { for (const r of this.resources) r.dispose(); }
}
