// Commit-graph lane layout for GraphView: which column each commit's node
// lands on, the gutter segments to draw, and lane colours / branch names.
// Pure (no Svelte, no DOM) so it can be unit-tested and reused; GraphView
// owns the reactive inputs (commits, lane names) and the memo between passes.
import type { CommitInfo } from '../../lib/api/types';

// Lane palette: the categorical series tokens (graphics only, tuned per
// scheme for >= 3:1 on --surface) — the old fixed hex set was dark-only and
// washed out on light.
export const PALETTE = [
  'var(--cat-1)', 'var(--cat-2)', 'var(--cat-3)',
  'var(--cat-4)', 'var(--cat-5)', 'var(--cat-6)',
];

export interface LaneRow {
  commit: CommitInfo;
  col: number;       // which column this commit's node lands on
  lines: LaneLine[]; // segments to draw in the SVG gutter
  color: string;     // node color
  // Whether the node's OWN column carries a line above / below it. Without
  // these the gutter drew only the half below every node, so the lane you are
  // actually following rendered as a dashed line while pass-through lanes
  // (drawn full-height as 'vert') rendered solid — exactly backwards.
  // above = some lane was already awaiting this sha (false at a branch tip,
  // where a line upwards would point at nothing).
  // below = the commit has a first parent (false at a root / end of history,
  // where the old code still trailed a stub into empty space).
  hasAbove: boolean;
  hasBelow: boolean;
  /** Branch this node's LANE belongs to, or null when the lane's tip carries no
   *  branch ref (a merged branch whose ref was deleted is genuinely unnamed —
   *  we never guess one). Drives the hover tooltip on the lane. */
  branch: string | null;
}

export interface LaneLine {
  fromCol: number;
  toCol: number;
  color: string;
  // vert      = straight continuation in a lane
  // merge-in  = this node's extra parent heading down into another lane
  // converge  = a duplicate-awaiting lane from above folding into this node
  kind: 'vert' | 'merge-in' | 'branch-out' | 'converge';
  /** Branch of the lane this SEGMENT belongs to (same rule as LaneRow.branch),
   *  so hovering any line — not just a node — names its branch. */
  branch: string | null;
}

export const LANE_W = 18; // pixels per lane column
export const NODE_R = 5;  // node radius
// Gutter width is capped (see gutterWidth), so lanes past this column are
// never visible: their segments aren't stored or drawn at all (a 60-lane
// `--all` fan-out used to allocate one segment per lane per row).
export const MAX_GUTTER_W = 260;
export const MAX_DRAW_COLS = Math.floor((MAX_GUTTER_W - LANE_W / 2) / LANE_W) + 1;

// Hard cap on lane count. Past this we stop opening brand-new lanes for extra
// (merge) parents and route them to the node's own column, so a pathological
// `--all` fan-out degrades gracefully instead of exploding sideways.
export const MAX_LANES = 24;

// Resumable lane layout. The pass below is O(rows × lanes); a "load more"
// page used to re-run it over EVERY loaded row (50k rows for a 10k append).
// The lane state at the end of the last pass is kept here (plain, not
// $state — the derived owns it), and a pass whose `commits` merely EXTENDS
// the previous list (same first/last objects at the same positions — pages
// are appended with `concat`, so the objects are shared) resumes from there.
// Anything else (reload, cache restore) lays out from scratch.
//
// A reload SPLICE (graph-splice.ts: a fresh first page + the held tail,
// whose objects are shared) lays out the new page from scratch and keeps
// going only until it CONVERGES: at each checkpoint of the previous pass
// (one per CHECKPOINT_EVERY rows) whose commit is the same object at the
// shifted position, an identical lane state means every row below is what
// the previous pass computed — those rows and the final state are reused.
// A splice after paging to 50k lays out ~the page, not 50k rows.
export const CHECKPOINT_EVERY = 1000;
export interface LaneCheckpoint {
  /** Row index whose layout starts from this state. */
  row: number;
  lanes: (string | null)[];
  laneColors: number[];
  laneBranch: (string | null)[];
  colorIdx: number;
  /** Widest lane count over the rows from `row` to the next checkpoint. */
  segWidest: number;
}
export interface LaneMemo {
  checkpoints: LaneCheckpoint[];
  names: Map<string, string>;
  n: number;
  first: CommitInfo | undefined;
  last: CommitInfo | undefined;
  rows: LaneRow[];
  lanes: (string | null)[];
  laneColors: number[];
  laneBranch: (string | null)[];
  colorIdx: number;
  widest: number;
}
/** One layout pass: `commits` laid out against `names` (sha → branch, see
 *  GraphView's `laneNames`), resuming from / converging onto `memo` (the
 *  previous pass) when it can. Returns the rows, the widest lane count, the
 *  memo for the next pass and how many rows this pass actually computed
 *  (the E2E perf probe). An empty `commits` has no memo and lays out nothing. */
export function layoutLanes(
  commits: CommitInfo[],
  names: Map<string, string>,
  memo: LaneMemo | null,
): { rows: LaneRow[]; widest: number; memo: LaneMemo | null; laidOut: number } {
  if (commits.length === 0) return { rows: [], widest: 1, memo: null, laidOut: 0 };
  const m = memo;
  const resume =
    m !== null && m.names === names && m.n > 0 && m.n <= commits.length &&
    commits[0] === m.first && commits[m.n - 1] === m.last;
  // Splice convergence candidates: the previous pass's checkpoints, by the
  // row they map to now (`d` = how far the shared tail moved).
  const d = m ? commits.length - m.n : 0;
  let converge: Map<number, LaneCheckpoint> | null = null;
  /** No reuse at or above this row: a commit whose lane NAME changed (a
   *  ref moved) sits there, and old rows below would carry the stale name. */
  let convergeFloor = 0;
  if (!resume && m !== null && m.n > 0 && commits[commits.length - 1] === m.last && commits[0] !== m.first) {
    converge = new Map();
    for (const cp of m.checkpoints) {
      const at = cp.row + d;
      if (cp.row > 0 && at > 0 && at < commits.length && commits[at] === m.rows[cp.row]?.commit) converge.set(at, cp);
    }
    if (converge.size === 0) converge = null;
    else if (m.names !== names) {
      const changed = new Set<string>();
      for (const [k, v] of names) if (m.names.get(k) !== v) changed.add(k);
      for (const k of m.names.keys()) if (!names.has(k)) changed.add(k);
      for (let i = commits.length - 1; i >= 0 && changed.size > 0; i--) {
        if (changed.has(commits[i].sha)) {
          convergeFloor = i + 1;
          break;
        }
      }
    }
  }

  // lanes[i] = sha of the commit expected next in lane i (null = free).
  // laneColors[i] = palette index for lane i — kept index-aligned with lanes[]
  // through every push / pop / null so colors never drift off their lane.
  const lanes: (string | null)[] = resume ? m.lanes : [];
  const laneColors: number[] = resume ? m.laneColors : [];
  // laneBranch[i] = branch name this lane is drawing, kept index-aligned with
  // lanes[] exactly like laneColors. A lane inherits its name from the branch
  // ref that starts it (its tip) and CARRIES IT DOWN through the lane's
  // commits, which is what makes hovering a line mid-history meaningful.
  const laneBranch: (string | null)[] = resume ? m.laneBranch : [];

  let colorIdx = resume ? m.colorIdx : 0;
  let widest = resume ? m.widest : 1;

  function laneColorAt(i: number): string {
    return PALETTE[(laneColors[i] ?? 0) % PALETTE.length];
  }

  /** Pick a palette slot that no OTHER currently-active lane is using, so two
   *  branches on screen at the same time never share a color. Walks forward
   *  from `colorIdx` so consecutive branches still cycle through the palette
   *  rather than clustering on one hue. Past PALETTE.length concurrent lanes
   *  there is nothing free left to pick, so fall back to plain round-robin. */
  function freshColor(exclude: number): number {
    const inUse = new Set<number>();
    for (let i = 0; i < lanes.length; i++) {
      if (i === exclude || lanes[i] === null) continue;
      const c = laneColors[i];
      if (c !== undefined) inUse.add(c);
    }
    for (let step = 0; step < PALETTE.length; step++) {
      const cand = (colorIdx + step) % PALETTE.length;
      if (!inUse.has(cand)) {
        colorIdx = cand + 1;
        return cand;
      }
    }
    return colorIdx++ % PALETTE.length;
  }

  function allocateLane(sha: string | null): number {
    // prefer reusing a free slot
    const free = lanes.indexOf(null);
    if (free !== -1) {
      lanes[free] = sha;
      // ALWAYS re-pick the color. A recycled slot used to keep whatever color
      // the previous (unrelated) branch had, so two different branches showed
      // up in the same color within one view.
      laneColors[free] = freshColor(free);
      // Same reasoning for the name: a recycled slot must not inherit the
      // previous branch's label, or the tooltip would misattribute the lane.
      laneBranch[free] = null;
      return free;
    }
    lanes.push(sha);
    laneColors.push(freshColor(lanes.length - 1));
    laneBranch.push(null);
    return lanes.length - 1;
  }

  /** The branch a commit's refs name, if any (see `laneNames`). Tags, stashes
   *  and a detached HEAD are NOT branches and never name a lane. */
  function branchOf(commit: CommitInfo): string | null {
    return names.get(commit.sha) ?? null;
  }

  // A copy: the previous `rows` array is what consumers hold — mutating it
  // would hand them the same reference and skip their recompute.
  let rows: LaneRow[] = resume ? m.rows.slice() : [];
  const from = resume ? m.n : 0;
  let laidOut = commits.length - from;
  let checkpoints: LaneCheckpoint[] = resume ? m.checkpoints.slice() : [];
  let lastCp = checkpoints.length ? checkpoints[checkpoints.length - 1] : null;
  let adopted = false;

  function sameState(cp: LaneCheckpoint): boolean {
    if (cp.colorIdx % PALETTE.length !== colorIdx % PALETTE.length) return false;
    if (cp.lanes.length !== lanes.length) return false;
    for (let i = 0; i < lanes.length; i++) {
      if (cp.lanes[i] !== lanes[i] || cp.laneColors[i] !== laneColors[i] || cp.laneBranch[i] !== laneBranch[i]) return false;
    }
    return true;
  }

  for (let ci = from; ci < commits.length; ci++) {
    if (converge !== null && ci >= convergeFloor && m !== null) {
      const cp = converge.get(ci);
      if (cp && sameState(cp)) {
        // Converged: rows ci… are the previous pass's rows cp.row…, and so
        // is the state after them. Re-base the old checkpoints onto the new
        // row numbers (their state arrays are never mutated — shared).
        rows = rows.concat(m.rows.slice(cp.row));
        for (const old of m.checkpoints) {
          if (old.row < cp.row) continue;
          checkpoints.push({ ...old, row: old.row + d });
          if (old.segWidest > widest) widest = old.segWidest;
        }
        lanes.length = 0;
        lanes.push(...m.lanes);
        laneColors.length = 0;
        laneColors.push(...m.laneColors);
        laneBranch.length = 0;
        laneBranch.push(...m.laneBranch);
        colorIdx = m.colorIdx;
        laidOut = ci;
        adopted = true;
        break;
      }
    }
    if (lastCp === null || ci - lastCp.row >= CHECKPOINT_EVERY) {
      lastCp = {
        row: ci,
        lanes: lanes.slice(),
        laneColors: laneColors.slice(),
        laneBranch: laneBranch.slice(),
        colorIdx,
        segWidest: lanes.length,
      };
      checkpoints.push(lastCp);
    }
    const commit = commits[ci];
    // Find this commit's column (which lane is "expecting" this sha)
    let col = lanes.indexOf(commit.sha);
    // A lane already awaiting this sha means the row above continues down into
    // this node, so its own column needs the segment ABOVE the node drawn.
    // A fresh allocation is a branch TIP — nothing above it to connect to.
    const hasAbove = col !== -1;
    if (col === -1) {
      col = allocateLane(commit.sha);
    }
    const color = laneColorAt(col);

    // Name the lane from this commit's own branch refs when it has any. At a
    // tip that's the branch starting the lane; further down it re-asserts the
    // name (and corrects a lane that was opened by a merge, whose branch isn't
    // knowable until a decorated commit shows up in it).
    const ownBranch = branchOf(commit);
    if (ownBranch) laneBranch[col] = ownBranch;
    const branch = laneBranch[col] ?? null;

    // Build line segments for BEFORE this node (connecting to previous)
    const lines: LaneLine[] = [];

    // Collapse duplicate-awaiting lanes: more than one lane can await the SAME
    // parent sha (shared history / repeated merges in `--all`). `indexOf` only
    // resolved the first; the rest used to run parallel forever and pile up
    // into a "wall" of dangling verticals. Converge them onto `col` and free
    // the lane so the gutter can shrink back.
    for (let i = 0; i < lanes.length; i++) {
      if (i === col) continue;
      if (lanes[i] === commit.sha) {
        if (i < MAX_DRAW_COLS || col < MAX_DRAW_COLS) {
          lines.push({ fromCol: i, toCol: col, color: laneColorAt(i), kind: 'converge', branch: laneBranch[i] ?? null });
        }
        lanes[i] = null;
      }
    }

    // Vertical continuations for all active lanes (before node) — only the
    // columns the capped gutter can show.
    const drawn = Math.min(lanes.length, MAX_DRAW_COLS);
    for (let i = 0; i < drawn; i++) {
      if (i === col) continue;
      if (lanes[i] !== null) {
        lines.push({ fromCol: i, toCol: i, color: laneColorAt(i), kind: 'vert', branch: laneBranch[i] ?? null });
      }
    }

    // Now update lanes: replace this lane with first parent
    const [firstParent, ...extraParents] = commit.parents;
    if (firstParent) {
      lanes[col] = firstParent;
    } else {
      lanes[col] = null; // root commit or end of history
    }

    // For additional parents (merge commits): find existing lane or create one
    for (const parent of extraParents) {
      const existingLane = lanes.indexOf(parent);
      if (existingLane !== -1) {
        // already tracked — draw merge-in line
        lines.push({ fromCol: col, toCol: existingLane, color, kind: 'merge-in', branch: laneBranch[existingLane] ?? null });
      } else if (lanes.length >= MAX_LANES && lanes.indexOf(null) === -1) {
        // Safety cap: no free lane and we're at the ceiling — don't open a new
        // lane for this merge parent (it goes untracked); just draw a stub
        // converging on the node's own column so the merge is still visible.
        // Degrades gracefully instead of widening the gutter without bound.
        lines.push({ fromCol: col, toCol: col, color, kind: 'merge-in', branch });
      } else {
        const newLane = allocateLane(parent);
        lines.push({ fromCol: col, toCol: newLane, color: laneColorAt(newLane), kind: 'merge-in', branch: laneBranch[newLane] ?? null });
      }
    }

    // Trim trailing free lanes so the gutter shrinks back as branches close
    // (keeps laneColors index-aligned by popping its tail in lockstep).
    while (lanes.length && lanes[lanes.length - 1] === null) {
      lanes.pop();
      laneColors.pop();
      laneBranch.pop();
    }

    if (lanes.length > widest) widest = lanes.length;
    if (lanes.length > lastCp.segWidest) lastCp.segWidest = lanes.length;

    // A root commit (or the end of loaded history with no parent recorded) has
    // nothing below it — without this the gutter trailed a stub off the oldest
    // commit into empty space.
    rows.push({ commit, col, lines, color, hasAbove, hasBelow: !!firstParent, branch });
  }

  const next: LaneMemo = {
    checkpoints,
    names,
    n: commits.length,
    first: commits[0],
    last: commits[commits.length - 1],
    rows,
    lanes,
    laneColors,
    laneBranch,
    colorIdx,
    widest,
  };
  return { rows, widest, memo: next, laidOut };
}
