// Otto School — the ambient-life director. PURE and deterministic (a seeded
// RNG, a caller-driven clock): the scene calls `step(dtMs)` each frame and
// reads back where every character stands, which way it faces and which
// animation clip it plays. unit/school-life.test.ts drives it with a fake
// clock — no three.js here.
//
// The rules, so the room stays TRUTHFUL while it feels alive:
//  • A kid's seat is its session. Working kids type, needs-you kids raise a
//    hand, failed kids slump, done kids sit back, kids idle for ASLEEP_MS doze
//    (head down), suspended seats are empty.
//  • A kid whose session just STOPPED working (its turn finished, it ended,
//    it now waits for you) stands up, cheers (Wave) and sits back down — a
//    real state change, never a random one; the headmaster gives a thumbs-up.
//  • Only FREE kids wander — idle ones, and ones whose "needs you" has gone
//    unanswered for BORED_MS (an agent that finished its turn waits at its
//    prompt; after a while it gets up, the ✋ still over its head). At most
//    ~⅓ of a room's free kids are up at once — to the windows, the bookshelf,
//    the board, the back-wall posters or a classmate's desk (to chat). The
//    moment their session starts working (or asks anew) they hurry back.
//  • A kid whose desk moved (someone joined / left) walks to the new desk.
//  • The headmaster patrols the lanes: needs-you kids first (walks over,
//    reads the screen, turns to you and waves — "this one wants you"), then
//    failed kids, then working kids he hasn't checked lately, and between
//    rounds points at the board.
//  • Kick-out: the kid stands up and walks out of the door (the headmaster
//    scolds); detention: the kid walks to the bench at the back and slumps.
//  • Reduced motion: nobody wanders or cheers, the headmaster stays at the
//    board, and a kick / detention completes at once.

import {
  DESK_OFFSET,
  aisleX,
  along,
  backLaneZ,
  benchSlot,
  doorInside,
  doorOutside,
  frontLaneZ,
  isAsleep,
  laneZ,
  lanePath,
  pathLength,
  type Kid,
  type P2,
  type Room,
} from './model.ts';
import { bookshelfAt, lecternAt } from './furnish.ts';
import {
  CALL_MS,
  CHEER_MS,
  FACE_BOARD,
  FACE_CLASS,
  HEAD_SPEED,
  HURRY_SPEED,
  INSPECT_MS,
  KID_SPEED,
  NOD_MS,
  POINT_MS,
  SCOLD_MS,
  SIT_MS,
  STAND_MS,
  THUMBS_MS,
  BORED_MS,
  cheerClip,
  rng,
  seatedClip,
  type ActorView,
  type KidClip,
} from './life/clips.ts';
import type { Goal, HeadActor, KidActor } from './life/actors.ts';

export * from './life/clips.ts';

const headingTo = (from: P2, to: P2): number => Math.atan2(to.x - from.x, to.z - from.z);

export interface LifeOpts {
  seed?: number;
  reducedMotion?: boolean;
  /** Wall clock (ms) — injected by tests. */
  now?: () => number;
}

export class Life {
  private room: Room | null = null;
  private kids = new Map<string, KidActor>();
  private head: HeadActor | null = null;
  private rand: () => number;
  private reduced: boolean;
  /** Last time (life clock, ms) the headmaster checked on each kid. */
  private visited = new Map<string, number>();
  private clock = 0;
  private now: () => number;

  constructor(opts: LifeOpts = {}) {
    this.rand = rng(opts.seed ?? 0x5c4001);
    this.reduced = !!opts.reducedMotion;
    this.now = opts.now ?? (() => Date.now());
  }

  /** May this kid leave its desk right now? */
  free(k: Kid): boolean {
    if (k.detention) return false;
    if (k.pose === 'idle') return true;
    if (k.pose !== 'needs-you') return false;
    const at = Date.parse(k.lastActiveAt);
    return Number.isFinite(at) && this.now() - at >= BORED_MS;
  }

  setReducedMotion(on: boolean): void {
    this.reduced = on;
    if (on) for (const a of this.kids.values()) if (a.mode !== 'gone' && a.mode !== 'leaving') this.snapHome(a);
  }

  /** Enter a classroom (null = the corridor): everybody starts at their desk. */
  setRoom(room: Room | null): void {
    this.room = room;
    this.kids.clear();
    this.visited.clear();
    this.head = null;
    if (!room) return;
    for (const k of [...room.kids, ...room.bench]) this.kids.set(k.id, this.spawn(k, room));
    const lec = lecternAt(room);
    this.head = { mode: 'lectern', pos: lec, lane: frontLaneZ(room), heading: 0, clip: 'Idle', path: [], dist: 0, timer: 2500 + this.rand() * 2000, target: null, resume: null };
  }

  /** Fresh data for the SAME room (statuses, joins, leaves, seat moves). */
  sync(room: Room): void {
    if (!this.room || this.room.id !== room.id) {
      this.setRoom(room);
      return;
    }
    this.room = room;
    const seen = new Set<string>();
    for (const k of [...room.kids, ...room.bench]) {
      seen.add(k.id);
      const a = this.kids.get(k.id);
      if (!a) {
        this.kids.set(k.id, this.spawn(k, room));
        continue;
      }
      const wasBench = a.kid.detention;
      const was = a.kid.pose;
      a.kid = k;
      if (a.mode === 'leaving' || a.mode === 'gone') continue;
      if (a.goal === 'bench' && !wasBench) {
        // On the way to detention: the archive hasn't landed yet (keep the
        // bench as home), or it just did (take the slot the model gave).
        if (k.detention) {
          a.home = k.seat;
          a.homeLane = this.laneOf(k, room);
        }
        continue;
      }
      const home = k.seat;
      const moved = Math.hypot(home.x - a.home.x, home.z - a.home.z) > 1e-3 || wasBench !== k.detention;
      a.home = home;
      a.homeLane = this.laneOf(k, room);
      if (moved) {
        if (this.reduced || a.mode !== 'seated') {
          // Mid-walk kids just route to the new desk on their way back.
          if (this.reduced) this.snapHome(a);
          else if (a.mode === 'visiting' || a.mode === 'walking') this.returnHome(a, HURRY_SPEED);
          else if (a.mode === 'celebrating') this.getUp(a, 'seat');
        } else this.getUp(a, 'seat');
        continue;
      }
      if (a.mode === 'seated') {
        if (was === 'working' && (k.pose === 'needs-you' || k.pose === 'idle' || k.pose === 'done') && !k.detention && !this.reduced) this.cheer(a);
        else a.clip = k.detention ? 'Sit_Slump' : this.restClip(k);
      } else if (a.mode === 'celebrating' && k.pose === 'working') {
        // Back to work mid-cheer: straight to the sit-down.
        a.timer = Math.min(a.timer, SIT_MS);
      } else if (!this.free(k) && (a.mode === 'walking' || a.mode === 'visiting') && a.goal === 'spot') {
        // Their agent woke up — hurry back to the desk.
        this.returnHome(a, HURRY_SPEED);
      }
    }
    for (const [id, a] of this.kids) {
      if (seen.has(id)) continue;
      // Gone from the data. A kid already walking out finishes its walk.
      if (a.mode !== 'leaving') this.kids.delete(id);
    }
    if (this.head?.target && !this.kids.has(this.head.target)) this.head.target = null;
  }

  /** Kick out: walk to the door and leave. Returns the walk's duration (ms). */
  kick(id: string): number {
    const a = this.kids.get(id);
    const room = this.room;
    if (!a || !room) return 0;
    this.scold(a.pos);
    if (this.reduced) {
      a.mode = 'gone';
      return 0;
    }
    const exit = [...lanePath(room, a.home, a.homeLane, doorInside(room), backLaneZ(room)), doorOutside(room)];
    if (a.mode === 'seated') {
      this.getUp(a, 'leave', exit);
      return STAND_MS + (pathLength(exit) / KID_SPEED) * 1000;
    }
    const back = this.retrace(a);
    a.path = [...back, ...exit.slice(1)];
    a.dist = 0;
    a.speed = HURRY_SPEED;
    a.mode = 'leaving';
    a.goal = 'leave';
    a.clip = 'Walk';
    return (pathLength(a.path) / HURRY_SPEED) * 1000;
  }

  /** Put a kid back where it was (the delete failed). */
  restore(id: string): void {
    const a = this.kids.get(id);
    if (!a || !this.room) return;
    a.home = { ...a.kid.seat };
    a.homeLane = this.laneOf(a.kid, this.room);
    a.goal = a.kid.detention ? 'bench' : 'spot';
    this.snapHome(a);
  }

  /** Detention: walk to the first free bench slot and slump there. */
  detention(id: string): number {
    const a = this.kids.get(id);
    const room = this.room;
    if (!a || !room) return 0;
    const used = new Set([...this.kids.values()].filter((k) => k.kid.detention || k.goal === 'bench').map((k) => `${k.home.x.toFixed(2)}`));
    let slot = benchSlot(room, 0);
    for (let i = 0; i < 3; i++) {
      const s = benchSlot(room, i);
      if (!used.has(s.x.toFixed(2))) {
        slot = s;
        break;
      }
    }
    if (this.reduced) {
      a.home = slot;
      a.homeLane = frontLaneZ(room);
      a.mode = 'bench';
      a.pos = { ...slot };
      a.clip = 'Sit_Slump';
      a.heading = FACE_CLASS;
      return 0;
    }
    const path = lanePath(room, a.home, a.homeLane, slot, frontLaneZ(room));
    a.home = slot;
    a.homeLane = frontLaneZ(room);
    if (a.mode === 'seated') {
      this.getUp(a, 'bench', path);
      return STAND_MS + (pathLength(path) / KID_SPEED) * 1000 + SIT_MS;
    }
    a.path = [...this.retrace(a), ...path.slice(1)];
    a.dist = 0;
    a.speed = KID_SPEED;
    a.mode = 'walking';
    a.goal = 'bench';
    a.clip = 'Walk';
    return (pathLength(a.path) / KID_SPEED) * 1000 + SIT_MS;
  }

  /** The headmaster goes to check on `id` next (a click on "Check on"). */
  inspect(id: string): void {
    const h = this.head;
    if (!h || !this.kids.has(id) || this.reduced) return;
    this.headTo(id);
  }

  /** Advance the world. */
  step(dtMs: number): void {
    if (!this.room) return;
    const dt = Math.min(1000, Math.max(0, dtMs));
    this.clock += dt;
    let up = 0;
    let idle = 0;
    for (const a of this.kids.values()) {
      if (this.free(a.kid)) idle++;
      if (a.mode !== 'seated' && a.mode !== 'bench' && a.mode !== 'gone') up++;
    }
    const cap = Math.max(1, Math.floor(idle / 3));
    for (const a of this.kids.values()) {
      up += this.stepKid(a, dt, up < cap) ? 1 : 0;
    }
    if (this.head) this.stepHead(this.head, dt);
  }

  /** Is this kid gone (walked out)? Unknown ids count as gone. */
  isGone(id: string): boolean {
    const a = this.kids.get(id);
    return !a || a.mode === 'gone';
  }

  kidView(id: string): ActorView | null {
    const a = this.kids.get(id);
    if (!a) return null;
    const visible = a.mode !== 'gone' && !(a.mode === 'seated' && a.kid.pose === 'away' && !a.kid.detention);
    return { id, x: a.pos.x, z: a.pos.z, heading: a.heading, clip: a.clip, mode: a.mode, visible, seated: a.mode === 'seated' || a.mode === 'bench' };
  }

  kidViews(): ActorView[] {
    return [...this.kids.keys()].map((id) => this.kidView(id)!).filter(Boolean);
  }

  headView(): ActorView | null {
    const h = this.head;
    if (!h) return null;
    return { id: 'headmaster', x: h.pos.x, z: h.pos.z, heading: h.heading, clip: h.clip, mode: h.mode, visible: true, seated: false };
  }

  headTarget(): string | null {
    return this.head?.target ?? null;
  }

  // ── Kids ─────────────────────────────────────────────────────────────────

  private laneOf(k: Kid, room: Room): number {
    return k.detention ? frontLaneZ(room) : laneZ(room, k.row);
  }

  private spawn(k: Kid, room: Room): KidActor {
    return {
      kid: k,
      mode: k.detention ? 'bench' : 'seated',
      pos: { ...k.seat },
      heading: k.detention ? FACE_CLASS : FACE_BOARD,
      clip: k.detention ? 'Sit_Slump' : this.restClip(k),
      path: [],
      dist: 0,
      speed: KID_SPEED,
      timer: 0,
      // First wander rolls come quickly, so the room visibly lives.
      next: 1500 + this.rand() * 7000,
      goal: 'spot',
      spot: null,
      home: { ...k.seat },
      homeLane: this.laneOf(k, room),
    };
  }

  /** The seated clip for a kid's real state (dozing after ASLEEP_MS idle). */
  private restClip(k: Kid): KidClip {
    return seatedClip(k.pose, isAsleep(k, this.now()));
  }

  /** Stand, cheer, sit — at the desk (the chair slides back meanwhile). */
  private cheer(a: KidActor): void {
    a.mode = 'celebrating';
    a.timer = STAND_MS + CHEER_MS + SIT_MS;
    a.clip = 'Stand_Up';
    a.pos = { ...a.home };
    a.heading = FACE_BOARD;
    const h = this.head;
    if (h && (h.mode === 'lectern' || h.mode === 'pointing')) {
      h.mode = 'cheering';
      h.clip = 'ThumbsUp';
      h.timer = THUMBS_MS;
      h.heading = headingTo(h.pos, a.home);
    }
  }

  private snapHome(a: KidActor): void {
    a.mode = a.kid.detention || a.goal === 'bench' ? 'bench' : 'seated';
    a.pos = { ...a.home };
    a.heading = a.mode === 'bench' ? FACE_CLASS : FACE_BOARD;
    a.clip = a.mode === 'bench' ? 'Sit_Slump' : this.restClip(a.kid);
    a.path = [];
    a.dist = 0;
    a.goal = a.mode === 'bench' ? 'bench' : 'spot';
  }

  private getUp(a: KidActor, goal: Goal, path?: P2[]): void {
    a.mode = 'standing';
    a.clip = 'Stand_Up';
    a.timer = STAND_MS;
    a.goal = goal;
    a.speed = KID_SPEED;
    a.path = path ?? [];
    a.dist = 0;
  }

  /** Path from where the kid is back to the start of its current path. */
  private retrace(a: KidActor): P2[] {
    if (a.path.length === 0) return [{ ...a.pos }];
    // Points already passed (by distance), reversed, starting at `pos`.
    const passed: P2[] = [];
    let d = 0;
    for (let i = 1; i < a.path.length; i++) {
      const seg = Math.hypot(a.path[i].x - a.path[i - 1].x, a.path[i].z - a.path[i - 1].z);
      if (d + seg > a.dist) break;
      d += seg;
      passed.push(a.path[i]);
    }
    return [{ ...a.pos }, ...passed.slice(0, Math.max(0, passed.length - (a.mode === 'visiting' ? 1 : 0))).reverse(), a.path[0]];
  }

  private returnHome(a: KidActor, speed: number): void {
    const room = this.room!;
    // Back the way we came (to where the walk started); when that isn't home
    // any more (the desk moved), lanes from there to the new desk.
    const back = this.retrace(a);
    const start = back[back.length - 1];
    const route =
      Math.hypot(start.x - a.home.x, start.z - a.home.z) < 1e-3 ? [] : lanePath(room, start, a.path[1]?.z ?? a.homeLane, a.home, a.homeLane).slice(1);
    a.path = [...back, ...route].filter((p, i, arr) => i === 0 || Math.hypot(p.x - arr[i - 1].x, p.z - arr[i - 1].z) > 1e-3);
    a.dist = 0;
    a.speed = speed;
    a.mode = 'returning';
    a.goal = a.kid.detention ? 'bench' : 'seat';
    a.clip = 'Walk';
    a.spot = null;
  }

  /** Pick a place to wander to. */
  private wanderTarget(a: KidActor): { path: P2[]; face: number; clip: KidClip; ms: number } | null {
    const room = this.room!;
    const r = this.rand();
    const dwell = 5000 + this.rand() * 8000;
    let spot: P2;
    let lane: number;
    let face: number;
    let clip: KidClip;
    if (r < 0.28) {
      // The windows (west wall).
      const z = -room.depth / 2 + 2.2 + this.rand() * Math.max(0.5, room.depth - 4.6);
      spot = { x: aisleX(room, -1) - 0.2, z };
      lane = z;
      face = -Math.PI / 2;
      clip = 'Look_Around';
    } else if (r < 0.48) {
      const b = bookshelfAt(room);
      spot = { x: aisleX(room, 1) + 0.05, z: b.z };
      lane = b.z;
      face = Math.PI / 2;
      clip = 'Idle_Stand';
    } else if (r < 0.66) {
      spot = { x: (this.rand() - 0.5) * 3, z: frontLaneZ(room) - 0.3 };
      lane = frontLaneZ(room);
      face = Math.PI;
      clip = 'Look_Around';
    } else if (r < 0.78) {
      spot = { x: (this.rand() - 0.5) * (room.width - 4), z: backLaneZ(room) + 0.3 };
      lane = backLaneZ(room);
      face = 0;
      clip = 'Look_Around';
    } else {
      // A classmate at its desk.
      const mates = [...this.kids.values()].filter((m) => m !== a && m.mode === 'seated' && !m.kid.detention && m.kid.pose !== 'away');
      if (mates.length === 0) return null;
      const m = mates[Math.floor(this.rand() * mates.length)];
      lane = m.homeLane;
      spot = { x: m.home.x - 0.6, z: lane };
      face = headingTo(spot, m.home);
      clip = 'Talk';
    }
    return { path: lanePath(room, a.home, a.homeLane, spot, lane), face, clip, ms: dwell };
  }

  /** Returns true when this kid just got up (counts toward the cap). */
  private stepKid(a: KidActor, dt: number, mayGetUp: boolean): boolean {
    switch (a.mode) {
      case 'seated': {
        a.pos.x = a.home.x;
        a.pos.z = a.home.z;
        a.heading = FACE_BOARD;
        a.clip = this.restClip(a.kid);
        if (this.reduced || !this.free(a.kid)) return false;
        a.next -= dt;
        if (a.next > 0) return false;
        a.next = 7000 + this.rand() * 14000;
        if (!mayGetUp || this.rand() > 0.65) return false;
        const t = this.wanderTarget(a);
        if (!t) return false;
        this.getUp(a, 'spot', t.path);
        a.spot = { face: t.face, clip: t.clip, ms: t.ms };
        return true;
      }
      case 'standing': {
        a.timer -= dt;
        if (a.timer > 0) return false;
        if (a.goal === 'seat') {
          // Moving desks: route from the OLD desk's lane to the new one.
          const room = this.room!;
          a.path = lanePath(room, a.pos, a.path[1]?.z ?? a.homeLane, a.home, a.homeLane);
          if (a.path.length < 2) a.path = [a.pos, a.home];
        }
        a.mode = a.goal === 'leave' ? 'leaving' : a.goal === 'spot' ? 'walking' : a.goal === 'bench' ? 'walking' : 'returning';
        a.clip = 'Walk';
        a.dist = 0;
        return false;
      }
      case 'walking':
      case 'returning':
      case 'leaving': {
        a.dist += (a.speed * dt) / 1000;
        const at = along(a.path, a.dist);
        a.pos = at.p;
        if (!at.done) {
          a.heading = at.heading;
          return false;
        }
        if (a.mode === 'leaving') {
          a.mode = 'gone';
          return false;
        }
        if (a.mode === 'walking' && a.goal === 'spot' && a.spot) {
          a.mode = 'visiting';
          a.heading = a.spot.face;
          a.clip = a.spot.clip;
          a.timer = a.spot.ms;
          return false;
        }
        // Arrived home (desk or bench): sit down.
        a.mode = 'sitting';
        a.clip = 'Sit_Down';
        a.heading = a.goal === 'bench' || a.kid.detention ? FACE_CLASS : FACE_BOARD;
        a.timer = SIT_MS;
        a.pos = { ...a.home };
        return false;
      }
      case 'visiting': {
        a.timer -= dt;
        if (a.timer <= 0) this.returnHome(a, KID_SPEED);
        return false;
      }
      case 'celebrating': {
        a.timer -= dt;
        if (a.timer > 0) {
          a.clip = cheerClip(a.timer);
          return false;
        }
        a.mode = 'seated';
        a.clip = this.restClip(a.kid);
        a.next = 6000 + this.rand() * 10000;
        return false;
      }
      case 'sitting': {
        a.timer -= dt;
        if (a.timer > 0) return false;
        a.mode = a.goal === 'bench' || a.kid.detention ? 'bench' : 'seated';
        a.clip = a.mode === 'bench' ? 'Sit_Slump' : this.restClip(a.kid);
        a.next = 9000 + this.rand() * 14000;
        a.path = [];
        a.spot = null;
        return false;
      }
      default:
        return false;
    }
  }

  // ── Headmaster ───────────────────────────────────────────────────────────

  private scold(at: P2): void {
    const h = this.head;
    if (!h || this.reduced) return;
    if (h.mode !== 'scolding') h.resume = { mode: h.mode, clip: h.clip, timer: Math.max(h.timer, 300) };
    h.mode = 'scolding';
    h.clip = 'Scold';
    h.timer = SCOLD_MS;
    h.heading = headingTo(h.pos, at);
  }

  /** Inspection spot for a kid: in the lane behind its chair, a little to
   *  the side, facing the monitor over the kid's shoulder. */
  private inspectSpot(a: KidActor): { spot: P2; lane: number; face: number } {
    const lane = a.homeLane;
    const spot = { x: a.home.x + 0.5, z: lane };
    const screen = { x: a.home.x, z: a.home.z - DESK_OFFSET };
    return { spot, lane, face: headingTo(spot, screen) };
  }

  private headTo(id: string): void {
    const h = this.head!;
    const a = this.kids.get(id)!;
    const room = this.room!;
    const t = this.inspectSpot(a);
    h.path = lanePath(room, h.pos, h.lane, t.spot, t.lane);
    h.dist = 0;
    h.mode = 'walking';
    h.clip = 'Walk';
    h.target = id;
    h.lane = t.lane;
    h.resume = null;
  }

  private pickNext(): string | null {
    const now = this.clock;
    const seated = [...this.kids.entries()].filter(([, a]) => a.mode === 'seated' && a.kid.pose !== 'away');
    const due = (id: string, every: number) => now - (this.visited.get(id) ?? -Infinity) > every;
    const needs = seated.filter(([id, a]) => a.kid.pose === 'needs-you' && due(id, 15000));
    if (needs.length) return needs[0][0];
    const failed = seated.filter(([id, a]) => a.kid.pose === 'error' && due(id, 30000));
    if (failed.length) return failed[0][0];
    const working = seated.filter(([id, a]) => a.kid.pose === 'working' && due(id, 40000));
    if (working.length && this.rand() < 0.75) return working[Math.floor(this.rand() * working.length)][0];
    const any = seated.filter(([id]) => due(id, 60000));
    if (any.length && this.rand() < 0.35) return any[Math.floor(this.rand() * any.length)][0];
    return null;
  }

  private stepHead(h: HeadActor, dt: number): void {
    const room = this.room!;
    if (this.reduced) {
      h.mode = 'lectern';
      h.pos = lecternAt(room);
      h.heading = 0;
      h.clip = 'Idle';
      return;
    }
    switch (h.mode) {
      case 'walking': {
        h.dist += (HEAD_SPEED * dt) / 1000;
        const at = along(h.path, h.dist);
        h.pos = at.p;
        if (!at.done) {
          h.heading = at.heading;
          return;
        }
        if (h.target && this.kids.has(h.target)) {
          const a = this.kids.get(h.target)!;
          h.mode = 'inspecting';
          h.clip = 'Inspect_Screen';
          h.heading = this.inspectSpot(a).face;
          h.timer = INSPECT_MS;
        } else if (h.target) {
          // The kid left while we walked over: think about what's next.
          h.target = null;
          h.mode = 'nodding';
          h.clip = 'Idle';
          h.timer = 300;
        } else {
          // Arrived at the lectern / board.
          h.mode = 'pointing';
          h.clip = 'Point_Board';
          h.heading = Math.PI;
          h.timer = POINT_MS;
        }
        return;
      }
      case 'inspecting': {
        h.timer -= dt;
        if (h.timer > 0) return;
        if (h.target) this.visited.set(h.target, this.clock);
        if (h.target && this.kids.get(h.target)?.kid.pose === 'needs-you') {
          // Turn to the room (the camera's side) and wave: "this one wants you".
          h.mode = 'calling';
          h.clip = 'Wave';
          h.heading = 0;
          h.timer = CALL_MS;
          return;
        }
        h.mode = 'nodding';
        h.clip = 'Nod';
        h.timer = NOD_MS;
        return;
      }
      case 'scolding': {
        h.timer -= dt;
        if (h.timer > 0) return;
        const r = h.resume;
        h.resume = null;
        if (r && r.mode === 'walking' && h.path.length) {
          h.mode = 'walking';
          h.clip = 'Walk';
        } else {
          h.mode = 'nodding';
          h.clip = 'Idle';
          h.timer = 400;
        }
        return;
      }
      case 'nodding':
      case 'calling':
      case 'cheering':
      case 'lectern':
      case 'pointing': {
        h.timer -= dt;
        if (h.timer > 0) return;
        const next = this.pickNext();
        if (next) {
          this.headTo(next);
          return;
        }
        if (h.mode === 'pointing') {
          h.mode = 'lectern';
          h.clip = 'Idle';
          h.heading = 0;
          h.timer = 2500 + this.rand() * 3000;
          return;
        }
        // Back to the board.
        const lec = lecternAt(room);
        if (Math.hypot(h.pos.x - lec.x, h.pos.z - lec.z) < 0.05) {
          h.mode = 'pointing';
          h.clip = 'Point_Board';
          h.heading = Math.PI;
          h.timer = POINT_MS;
          return;
        }
        h.path = lanePath(room, h.pos, h.lane, lec, frontLaneZ(room));
        h.lane = frontLaneZ(room);
        h.dist = 0;
        h.mode = 'walking';
        h.clip = 'Walk';
        h.target = null;
        return;
      }
    }
  }
}
