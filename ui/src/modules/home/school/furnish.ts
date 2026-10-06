// Otto School — where every kit piece goes. PURE: a Room / the corridor in,
// a list of placements (kit node name + world position + Y rotation) out.
// scene.ts clones `school-kit.glb` nodes onto these; unit/school-layout.test
// checks nothing blocks a walkway. Kit facing (CONTRACT.md): every piece's
// front / inside face points +Z at rotation 0; rotY θ turns it to
// (sin θ, cos θ) — so π/2 faces +X, −π/2 faces −X, π faces −Z.

import {
  CORRIDOR_HALF,
  DESK_OFFSET,
  SEG,
  aisleX,
  backLaneZ,
  benchSlot,
  frontLaneZ,
  laneZ,
  seatAt,
  toWorld,
  type P2,
  type Room,
} from './model.ts';

export type KitNode =
  | 'Workstation'
  | 'Chair'
  | 'TeacherDesk'
  | 'Board'
  | 'Bookshelf'
  | 'Plant'
  | 'Clock'
  | 'Bench'
  | 'Poster_1'
  | 'Poster_2'
  | 'Poster_3'
  | 'Poster_4'
  | 'Poster_5'
  | 'Poster_6'
  | 'Floor_Tile'
  | 'Wall'
  | 'Wall_Window'
  | 'Wall_Door'
  | 'Corridor_Floor'
  | 'Corridor_Wall'
  | 'Corridor_Door'
  | 'Ceiling_Light'
  // Decor (optional — the scene skips any the kit lacks).
  | 'Backpack'
  | 'Trashcan';

export const KIT_NODES: readonly KitNode[] = [
  'Workstation', 'Chair', 'TeacherDesk', 'Board', 'Bookshelf', 'Plant', 'Clock', 'Bench',
  'Poster_1', 'Poster_2', 'Poster_3', 'Poster_4', 'Poster_5', 'Poster_6',
  'Floor_Tile', 'Wall', 'Wall_Window', 'Wall_Door', 'Corridor_Floor', 'Corridor_Wall', 'Corridor_Door', 'Ceiling_Light',
];

export interface Placement {
  node: KitNode;
  x: number;
  y: number;
  z: number;
  rotY: number;
  /** What the piece belongs to: a desk's seat key (`r:c`), a door's room id… */
  tag?: string;
}

const PI = Math.PI;
const posterNode = (i: number): KitNode => `Poster_${(i % 6) + 1}` as KitNode;

/** The bookshelf's floor spot (room-local) — kids browse it (life.ts). */
export function bookshelfAt(room: Pick<Room, 'width' | 'depth'>): P2 {
  return { x: room.width / 2 - 0.22, z: -room.depth / 2 + 3.0 };
}

/** Where the headmaster stands to address the class (room-local). */
export function lecternAt(room: Pick<Room, 'width' | 'depth'>): P2 {
  return { x: -1.3, z: frontLaneZ(room) };
}

/** Every desk the room has — a full grid, so a half-empty class still looks
 *  like a classroom (empty desks keep a sleeping PC). */
export function deskGrid(room: Pick<Room, 'cols' | 'rows' | 'backRows' | 'depth'>): { row: number; col: number; seat: P2 }[] {
  const out: { row: number; col: number; seat: P2 }[] = [];
  for (let row = 0; row < room.rows + room.backRows; row++)
    for (let col = 0; col < room.cols; col++) out.push({ row, col, seat: seatAt(row, col, room.cols, room.depth) });
  return out;
}

/** Everything inside one classroom, in WORLD coordinates. */
export function furnishRoom(room: Room): Placement[] {
  const W = room.width;
  const D = room.depth;
  const out: Placement[] = [];
  const put = (node: KitNode, p: P2, rotY: number, y = 0, tag?: string) => {
    const w = toWorld(room, p);
    out.push({ node, x: w.x, y, z: w.z, rotY, tag });
  };
  const nx = W / SEG;
  const nz = D / SEG;

  // Floor.
  for (let i = 0; i < nx; i++) for (let j = 0; j < nz; j++) put('Floor_Tile', { x: -W / 2 + SEG / 2 + i * SEG, z: -D / 2 + SEG / 2 + j * SEG }, 0);

  // Walls: board wall (north, faces +Z), back wall with the door (faces −Z),
  // windows to the west (faces +X), plain east wall (faces −X).
  for (let i = 0; i < nx; i++) {
    const x = -W / 2 + SEG / 2 + i * SEG;
    put('Wall', { x, z: -D / 2 }, 0);
    put(i === nx - 1 ? 'Wall_Door' : 'Wall', { x, z: D / 2 }, PI, 0, i === nx - 1 ? `door:${room.id}` : undefined);
  }
  for (let j = 0; j < nz; j++) {
    const z = -D / 2 + SEG / 2 + j * SEG;
    put(j === 0 || j === nz - 1 ? 'Wall' : 'Wall_Window', { x: -W / 2, z }, PI / 2);
    put('Wall', { x: W / 2, z }, -PI / 2);
  }

  // Front of the class.
  put('Board', { x: 0, z: -D / 2 + 0.02 }, 0, 0, `board:${room.id}`);
  put('Clock', { x: W / 2 - 1.3, z: -D / 2 + 0.02 }, 0, 2.45);
  put('TeacherDesk', { x: -W / 2 + 1.9, z: -D / 2 + 0.9 }, 0);
  put('Plant', { x: W / 2 - 0.45, z: -D / 2 + 0.45 }, 0);

  // Side + back furniture.
  put('Bookshelf', bookshelfAt(room), -PI / 2);
  put('Plant', { x: W / 2 - 0.45, z: D / 2 - 0.45 }, 0);
  put('Bench', { x: benchSlot(room, 1).x, z: benchSlot(room, 1).z }, 0);

  // Posters: back wall (skipping the door) at eye height, east wall too.
  let p = 0;
  for (let i = 0; i < nx - 1; i++) put(posterNode(p++), { x: -W / 2 + SEG / 2 + i * SEG, z: D / 2 - 0.02 }, PI, 1.75);
  for (let j = 1; j < nz - 1; j += 1) {
    const z = -D / 2 + SEG / 2 + j * SEG;
    if (Math.abs(z - bookshelfAt(room).z) < 1.4) continue;
    put(posterNode(p++), { x: W / 2 - 0.02, z }, -PI / 2, 1.75);
  }

  // Desks: the workstation sits DESK_OFFSET toward the board from the chair;
  // both turned π so the monitor faces the kid and the kid faces the board.
  // Every third desk has a backpack dropped beside the chair.
  for (const d of deskGrid(room)) {
    const tag = `${d.row}:${d.col}`;
    put('Workstation', { x: d.seat.x, z: d.seat.z - DESK_OFFSET }, PI, 0, tag);
    put('Chair', d.seat, PI, 0, tag);
    if ((d.row * 7 + d.col * 3) % 3 === 0) put('Backpack', { x: d.seat.x + 0.45, z: d.seat.z - 0.1 }, PI + ((d.row + d.col) % 2 ? 0.5 : -0.4));
  }
  put('Trashcan', { x: W / 2 - 0.4, z: D / 2 - 2.6 }, 0);
  return out;
}

/** The corridor (world coordinates): floor, lockers, one door per room. */
export function furnishCorridor(rooms: readonly Pick<Room, 'id' | 'doorX'>[], length: number): Placement[] {
  const out: Placement[] = [];
  const doors = new Map(rooms.map((r) => [r.doorX, r.id] as const));
  const n = Math.round(length / SEG);
  for (let i = 0; i < n; i++) {
    const x = SEG / 2 + i * SEG;
    for (const z of [-CORRIDOR_HALF + SEG / 2, CORRIDOR_HALF - SEG / 2]) out.push({ node: 'Corridor_Floor', x, y: 0, z, rotY: 0 });
    const room = doors.get(x);
    out.push(
      room
        ? { node: 'Corridor_Door', x, y: 0, z: -CORRIDOR_HALF, rotY: 0, tag: `door:${room}` }
        : { node: 'Corridor_Wall', x, y: 0, z: -CORRIDOR_HALF, rotY: 0 },
    );
    out.push({ node: 'Corridor_Wall', x, y: 0, z: CORRIDOR_HALF, rotY: PI });
    if (i % 2 === 0) out.push({ node: 'Ceiling_Light', x, y: 2.95, z: 0, rotY: 0 });
  }
  for (const z of [-CORRIDOR_HALF + SEG / 2, CORRIDOR_HALF - SEG / 2]) {
    out.push({ node: 'Corridor_Wall', x: 0, y: 0, z, rotY: PI / 2 });
    out.push({ node: 'Corridor_Wall', x: length, y: 0, z, rotY: -PI / 2 });
  }
  return out;
}

/** World-space boxes (x0, z0, x1, z1) that must stay clear for people to
 *  walk — the lanes and aisles life.ts routes along (tests check furniture
 *  never lands on one). */
export function walkways(room: Room): { x0: number; z0: number; x1: number; z1: number }[] {
  const half = 0.25;
  const rows = room.rows + room.backRows;
  const lanes: number[] = [frontLaneZ(room), backLaneZ(room)];
  for (let r = 0; r < rows; r++) lanes.push(laneZ(room, r));
  const out: { x0: number; z0: number; x1: number; z1: number }[] = [];
  for (const z of lanes) out.push({ x0: room.cx + aisleX(room, -1), z0: room.cz + z - half, x1: room.cx + aisleX(room, 1), z1: room.cz + z + half });
  for (const side of [-1, 1] as const) {
    const x = room.cx + aisleX(room, side);
    out.push({ x0: x - half, z0: room.cz + frontLaneZ(room), x1: x + half, z1: room.cz + backLaneZ(room) });
  }
  return out;
}
