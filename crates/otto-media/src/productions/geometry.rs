//! A made model's real shape for the scene: its triangles (from the GLB's
//! binary chunk), the surfaces a piece can stand on (an occupancy grid of
//! its up-facing triangles, seen from above), and rays for what the camera
//! sees. Everything here is in the model's own space unless it says world.

/// One triangle, three points.
pub type Tri = [[f64; 3]; 3];

/// Up (or down) facing: the triangle's normal within ~50° of vertical.
const FACING: f64 = 0.65;

/// The model seen from above, one cell a grid step wide: the highest
/// up-facing surface over each cell (where a piece stands), the lowest one
/// (a container's inner floor) and the lowest down-facing one (the
/// clearance under a table top).
pub struct Surface {
    min: [f64; 2],
    cell: f64,
    nx: usize,
    nz: usize,
    top: Vec<f64>,
    low: Vec<f64>,
    ceil: Vec<f64>,
    /// The standing level (the middle height of the tops pieces stand on),
    /// the lowest such top, and the model's height.
    pub max_top: f64,
    upper: f64,
    /// Its highest up-facing surface (a container's rim).
    rim: f64,
    height: f64,
}

impl Surface {
    /// `None` when the model has no triangles that face up.
    pub fn of(tris: &[Tri], min: [f64; 3], max: [f64; 3]) -> Option<Surface> {
        let span = (max[0] - min[0]).max(max[2] - min[2]);
        let cell = (span / 64.0).max(0.005);
        let nx = ((max[0] - min[0]) / cell).ceil() as usize + 1;
        let nz = ((max[2] - min[2]) / cell).ceil() as usize + 1;
        let mut s = Surface {
            min: [min[0], min[2]],
            cell,
            nx,
            nz,
            top: vec![f64::NAN; nx * nz],
            low: vec![f64::NAN; nx * nz],
            ceil: vec![f64::NAN; nx * nz],
            max_top: f64::NAN,
            upper: f64::NAN,
            rim: f64::NAN,
            height: (max[1] - min[1]).max(0.01),
        };
        for t in tris {
            let (u, v) = (sub(t[1], t[0]), sub(t[2], t[0]));
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len < 1e-12 || n[1].abs() < FACING * len {
                continue;
            }
            let up = n[1] > 0.0;
            // Every cell whose centre the triangle covers (seen from above),
            // and the cell of its centroid (a triangle smaller than a cell).
            let lo = [0, 2].map(|k| t.iter().map(|p| p[k]).fold(f64::MAX, f64::min));
            let hi = [0, 2].map(|k| t.iter().map(|p| p[k]).fold(f64::MIN, f64::max));
            let (i0, k0) = s.index(lo[0], lo[1]);
            let (i1, k1) = s.index(hi[0], hi[1]);
            let mut mark = |i: usize, k: usize, y: f64| {
                let c = k * s.nx + i;
                if up {
                    s.top[c] = if s.top[c].is_nan() {
                        y
                    } else {
                        s.top[c].max(y)
                    };
                    s.low[c] = if s.low[c].is_nan() {
                        y
                    } else {
                        s.low[c].min(y)
                    };
                } else {
                    s.ceil[c] = if s.ceil[c].is_nan() {
                        y
                    } else {
                        s.ceil[c].min(y)
                    };
                }
            };
            let centroid = [0, 1, 2].map(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.0);
            let (ci, ck) = (
                ((centroid[0] - min[0]) / cell) as usize,
                ((centroid[2] - min[2]) / cell) as usize,
            );
            mark(ci.min(nx - 1), ck.min(nz - 1), centroid[1]);
            for k in k0..=k1 {
                for i in i0..=i1 {
                    let (x, z) = (
                        min[0] + (i as f64 + 0.5) * cell,
                        min[2] + (k as f64 + 0.5) * cell,
                    );
                    if let Some(y) = height_at(t, x, z) {
                        mark(i, k, y);
                    }
                }
            }
        }
        // Where pieces stand: tops in the upper part of the model (a deck, a
        // table top, a seat; generated planks are uneven and may slope), not
        // a leg's foot; its level is their middle height.
        s.upper = min[1] + 0.4 * s.height;
        s.rim = s
            .top
            .iter()
            .copied()
            .filter(|y| !y.is_nan())
            .fold(f64::MIN, f64::max);
        let mut tops: Vec<f64> = s.top.iter().copied().filter(|y| *y >= s.upper).collect();
        tops.sort_by(f64::total_cmp);
        s.max_top = *tops.get(tops.len() / 2)?;
        Some(s)
    }

    /// One grid step, in the model's units.
    pub fn cell(&self) -> f64 {
        self.cell
    }

    fn index(&self, x: f64, z: f64) -> (usize, usize) {
        let i = ((x - self.min[0]) / self.cell)
            .floor()
            .clamp(0.0, (self.nx - 1) as f64);
        let k = ((z - self.min[1]) / self.cell)
            .floor()
            .clamp(0.0, (self.nz - 1) as f64);
        (i as usize, k as usize)
    }

    fn at(&self, grid: &[f64], x: f64, z: f64) -> Option<f64> {
        let (i, k) = self.index(x, z);
        let inside = x >= self.min[0] - 1e-9
            && z >= self.min[1] - 1e-9
            && x <= self.min[0] + self.nx as f64 * self.cell
            && z <= self.min[1] + self.nz as f64 * self.cell;
        Some(grid[k * self.nx + i]).filter(|y| inside && !y.is_nan())
    }

    /// Highest up-facing triangle directly beneath this point, including a
    /// low seat or branch with a taller back or twig elsewhere.
    pub fn top_at(&self, x: f64, z: f64) -> Option<f64> {
        self.at(&self.top, x, z)
    }

    /// Cell at the middle of the highest patch, for an explicitly named treetop
    /// or crown. Pick a real cell even when two branches are the same height.
    pub(super) fn crown(&self) -> [f64; 2] {
        let cells = || {
            self.top
                .iter()
                .enumerate()
                .filter(|(_, y)| **y >= self.rim - self.cell)
                .map(|(n, _)| {
                    [
                        self.min[0] + ((n % self.nx) as f64 + 0.5) * self.cell,
                        self.min[1] + ((n / self.nx) as f64 + 0.5) * self.cell,
                    ]
                })
        };
        let (sum, count) = cells().fold(([0., 0.], 0.), |(s, n), p| {
            ([s[0] + p[0], s[1] + p[1]], n + 1.)
        });
        let mid = sum.map(|v| v / count);
        let distance = |p: &[f64; 2]| (p[0] - mid[0]).hypot(p[1] - mid[1]);
        cells()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .unwrap_or(mid)
    }

    /// Where a piece may stand: a top in the upper part of the model.
    fn standable(&self, x: f64, z: f64) -> Option<f64> {
        self.top_at(x, z).filter(|y| *y >= self.upper)
    }

    /// The box (min x, min z, max x, max z) of every standable cell.
    pub fn footprint(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for k in 0..self.nz {
            for i in 0..self.nx {
                let (x, z) = (
                    self.min[0] + (i as f64 + 0.5) * self.cell,
                    self.min[1] + (k as f64 + 0.5) * self.cell,
                );
                if self.standable(x, z).is_some() {
                    b = [b[0].min(x), b[1].min(z), b[2].max(x), b[3].max(z)];
                }
            }
        }
        b
    }

    /// The height a base `half` (x, z half sizes) centred at (x, z) stands
    /// at, when at least 20 of its 25 points are over standable surface
    /// within `tol` of each other: the base touches real surface, never
    /// air (a gap between planks is allowed).
    pub fn support(&self, x: f64, z: f64, half: [f64; 2], tol: f64) -> Option<f64> {
        let mut ys = Vec::with_capacity(25);
        for a in 0..5 {
            for b in 0..5 {
                let px = x + half[0] * (a as f64 / 2.0 - 1.0);
                let pz = z + half[1] * (b as f64 / 2.0 - 1.0);
                let y = self.standable(px, pz);
                // Its corners and middle always over surface (no overhang).
                if y.is_none() && (a % 4 == 0 && b % 4 == 0 || a == 2 && b == 2) {
                    return None;
                }
                ys.extend(y);
            }
        }
        // A gap between planks is fine; a base mostly over air is not.
        if ys.len() < 20 {
            return None;
        }
        let (lo, hi) = ys
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), y| (l.min(*y), h.max(*y)));
        (hi - lo <= tol).then_some(hi)
    }

    /// A container's inner floor at (x, z): the surface there lies well
    /// below the container's rim (`None`: a lid or a solid top, nothing goes in).
    pub fn cavity(&self, x: f64, z: f64) -> Option<f64> {
        let floor = self.at(&self.top, x, z)?;
        (floor < self.rim - 0.15 * self.height).then_some(floor)
    }

    /// The clearance under the model over a base `half` at (x, z): the lowest
    /// down-facing surface above `from` (`None`: open sky; about 0 where a
    /// leg stands).
    pub fn clearance(&self, x: f64, z: f64, half: [f64; 2], from: f64) -> Option<f64> {
        let mut least: Option<f64> = None;
        for a in 0..5 {
            for b in 0..5 {
                let px = x + half[0] * (a as f64 / 2.0 - 1.0);
                let pz = z + half[1] * (b as f64 / 2.0 - 1.0);
                // A leg's foot faces down at the floor: no room there.
                if let Some(r) = self.at(&self.ceil, px, pz).map(|c| (c - from).max(0.0)) {
                    least = Some(least.map_or(r, |l: f64| l.min(r)));
                }
            }
        }
        least
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The triangle's height over (x, z) seen from above, when it covers it.
fn height_at(t: &Tri, x: f64, z: f64) -> Option<f64> {
    let (a, b, c) = (t[0], t[1], t[2]);
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
    let l3 = 1.0 - l1 - l2;
    (l1 >= -1e-9 && l2 >= -1e-9 && l3 >= -1e-9).then(|| l1 * a[1] + l2 * b[1] + l3 * c[1])
}

/// Whether the segment from `o` along `d` (0 < t < 1) passes through the triangle.
pub fn hits(o: [f64; 3], d: [f64; 3], t: &Tri) -> bool {
    hit_distance(o, d, t).is_some()
}

/// Fraction of a segment to its first intersection with the triangle.
pub(super) fn hit_distance(o: [f64; 3], d: [f64; 3], t: &Tri) -> Option<f64> {
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let p = [
        d[1] * e2[2] - d[2] * e2[1],
        d[2] * e2[0] - d[0] * e2[2],
        d[0] * e2[1] - d[1] * e2[0],
    ];
    let det = e1[0] * p[0] + e1[1] * p[1] + e1[2] * p[2];
    if det.abs() < 1e-12 {
        return None;
    }
    let s = sub(o, t[0]);
    let u = (s[0] * p[0] + s[1] * p[1] + s[2] * p[2]) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = [
        s[1] * e1[2] - s[2] * e1[1],
        s[2] * e1[0] - s[0] * e1[2],
        s[0] * e1[1] - s[1] * e1[0],
    ];
    let v = (d[0] * q[0] + d[1] * q[1] + d[2] * q[2]) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let along = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) / det;
    (along > 1e-6 && along < 1.0 - 1e-6).then_some(along)
}

/// Whether the segment from `o` along `d` (0 < t < 1) meets the box.
pub fn meets_box(o: [f64; 3], d: [f64; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for k in 0..3 {
        if d[k].abs() < 1e-12 {
            if o[k] < min[k] || o[k] > max[k] {
                return false;
            }
            continue;
        }
        let (a, b) = ((min[k] - o[k]) / d[k], (max[k] - o[k]) / d[k]);
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
        if lo > hi {
            return false;
        }
    }
    true
}

/// Where a placed piece's surface is, as the cubes of a grid over its world
/// box that it passes through (about 32 across its longest side): what a ray
/// from the camera meets, without missing the gaps between a thinned-out set
/// of its triangles.
pub struct Occupancy {
    min: [f64; 3],
    cell: f64,
    dims: [usize; 3],
    full: Vec<bool>,
}

impl Occupancy {
    pub fn of(tris: &[Tri], min: [f64; 3], max: [f64; 3]) -> Occupancy {
        let span = (0..3).map(|k| max[k] - min[k]).fold(1e-6, f64::max);
        let cell = span / 32.0;
        let dims = [0, 1, 2].map(|k| (((max[k] - min[k]) / cell).ceil() as usize).clamp(1, 33));
        let full = vec![false; dims[0] * dims[1] * dims[2]];
        let mut g = Occupancy {
            min,
            cell,
            dims,
            full,
        };
        for t in tris {
            // Points over the triangle no further apart than half a cube.
            let edge = (0..3)
                .map(|i| {
                    let d = sub(t[(i + 1) % 3], t[i]);
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                })
                .fold(0.0, f64::max);
            let n = ((2.0 * edge / cell).ceil() as usize).clamp(1, 64);
            for i in 0..=n {
                for j in 0..=n - i {
                    let (a, b) = (i as f64 / n as f64, j as f64 / n as f64);
                    let p = [0, 1, 2]
                        .map(|k| t[0][k] + a * (t[1][k] - t[0][k]) + b * (t[2][k] - t[0][k]));
                    if let Some(at) = g.at(p) {
                        g.full[at] = true;
                    }
                }
            }
        }
        g
    }

    fn at(&self, p: [f64; 3]) -> Option<usize> {
        let mut at = 0;
        for k in [2, 1, 0] {
            let c = ((p[k] - self.min[k]) / self.cell).floor();
            if c < 0.0 || c as usize >= self.dims[k] {
                return None;
            }
            at = at * self.dims[k] + c as usize;
        }
        Some(at)
    }

    /// Whether the segment from `o` along `d` passes through the surface
    /// before its last cube and a half (a piece held in a hand is not hidden
    /// by the hand it touches).
    pub fn crosses(&self, o: [f64; 3], d: [f64; 3]) -> bool {
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-9);
        let end = 1.0 - 1.5 * self.cell / len;
        let step = 0.5 * self.cell / len;
        let mut t = 0.0;
        while t < end {
            if self
                .at([0, 1, 2].map(|k| o[k] + t * d[k]))
                .is_some_and(|i| self.full[i])
            {
                return true;
            }
            t += step;
        }
        false
    }
}

/// A turn (3×3, rows) of a model's own space.
pub type Turn = [[f64; 3]; 3];

pub fn turn_point(r: &Turn, v: [f64; 3]) -> [f64; 3] {
    r.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

/// Heading of the sides of a rectangular deck after levelling. Opposite and
/// perpendicular faces vote for the same axis when their angles are quadrupled.
/// A circular or irregular rim has no dominant pair of directions.
pub(super) fn deck_axis(tris: &[Tri], level: &Turn) -> Option<f64> {
    let (mut x, mut z, mut total) = (0.0, 0.0, 0.0);
    for t in tris {
        let [a, b, c] = t.map(|v| turn_point(level, v));
        let (u, v) = (sub(b, a), sub(c, a));
        let nx = u[1] * v[2] - u[2] * v[1];
        let ny = u[2] * v[0] - u[0] * v[2];
        let nz = u[0] * v[1] - u[1] * v[0];
        let area = nx.hypot(nz);
        if area <= 2.0 * ny.abs() {
            continue;
        }
        let (sin, cos) = (4.0 * nz.atan2(nx)).sin_cos();
        x += area * cos;
        z += area * sin;
        total += area;
    }
    (total > 0.0 && x.hypot(z) > 0.65 * total).then(|| 0.25 * z.atan2(x))
}

/// How a model's top slopes: the plane of its upward faces in its upper part
/// (above 40 % of its height), weighed by their area, as the turn that
/// levels it and its slope in degrees. `None`: no such faces.
pub fn levelling(tris: &[Tri], min: [f64; 3], max: [f64; 3]) -> Option<(Turn, f64)> {
    let upper = min[1] + 0.4 * (max[1] - min[1]);
    let mut n = [0.0; 3];
    for t in tris {
        let (a, b) = (
            [0, 1, 2].map(|k| t[1][k] - t[0][k]),
            [0, 1, 2].map(|k| t[2][k] - t[0][k]),
        );
        let c = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        let mid = (t[0][1] + t[1][1] + t[2][1]) / 3.0;
        if len > 0.0 && c[1] / len > 0.7 && mid >= upper {
            n = [0, 1, 2].map(|k| n[k] + c[k]);
        }
    }
    level_normal(n)
}

/// A thin sheet may be drawn almost upright. Its broad faces on either
/// side agree on a plane, even when none of them points near straight up.
pub fn sheet_turn(tris: &[Tri]) -> Option<Turn> {
    let area = |t: &Tri| {
        let a = [0, 1, 2].map(|k| t[1][k] - t[0][k]);
        let b = [0, 1, 2].map(|k| t[2][k] - t[0][k]);
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| a[k] * b[k]).sum::<f64>();
    let reference = tris
        .iter()
        .map(area)
        .max_by(|a, b| dot(*a, *a).total_cmp(&dot(*b, *b)))?;
    let mut normal = [0.0; 3];
    for n in tris.iter().map(area) {
        let sign = dot(n, reference).signum();
        normal = [0, 1, 2].map(|k| normal[k] + sign * n[k]);
    }
    if normal[1] < 0.0 {
        normal = normal.map(|v| -v);
    }
    level_normal(normal).map(|(turn, _)| turn)
}

fn level_normal(n: [f64; 3]) -> Option<(Turn, f64)> {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len == 0.0 {
        return None;
    }
    let n = n.map(|x| x / len);
    let slope = n[1].clamp(-1.0, 1.0).acos().to_degrees();
    // The turn taking n to up (Rodrigues, about n × up).
    let (vx, vz) = (-n[2], n[0]);
    let s2 = vx * vx + vz * vz;
    let k = if s2 > 1e-12 { (1.0 - n[1]) / s2 } else { 0.0 };
    let turn = [
        [1.0 - k * vz * vz, -vz, k * vx * vz],
        [vz, 1.0 - k * (vx * vx + vz * vz), -vx],
        [k * vx * vz, vx, 1.0 - k * vx * vx],
    ];
    Some((turn, slope))
}
