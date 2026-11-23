//! What a making production shows while it works: each item's step in
//! plain words (from the engine's own progress lines), whether it waits for
//! the engine, its concept picture as soon as it is drawn, and the one time
//! estimate, counted by lanes.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::Poll;

use super::{Item, Production, Step};

/// An item's 3D steps, a model's first three from Otto Forge's `stage` keys.
pub const DRAWING: &str = "Drawing the picture";
pub const SHAPING: &str = "Building the 3D shape";
pub const READYING: &str = "Getting it ready";
pub const SKELETON: &str = "Adding a skeleton";
pub const MOVES: &str = "Adding moves";
pub const LIBRARY: &str = "Looking in the library";

/// Which of a model's steps an engine `stage` key is (`""`: none came yet).
/// The first two are GPU work (FLUX.2, the matte, Pixal3D); from the third
/// on (the game-ready pass in Blender) it is CPU work.
pub fn generate_step(key: &str) -> (u32, &'static str) {
    match key {
        "" | "concept" | "matte" => (1, DRAWING),
        "generate" => (2, SHAPING),
        _ => (3, READYING),
    }
}

pub fn step(n: u32, of: u32, label: &str) -> Option<Step> {
    Some(Step {
        n,
        of,
        label: label.into(),
        fraction: None,
    })
}

/// How far Pixal3D is into building a shape (0–1), from the part banners
/// (`[3/6] shape SLAT flow …`) and flow bars (`[flow] [###…]  3/8 …`) it
/// writes to Forge's `generate.log`, weighed by where its time goes (the
/// shape and texture flows are most of it). `None`: no banner yet.
pub fn shape_fraction(log: &str) -> Option<f64> {
    // The share of the build done before each part starts (parts 0–6).
    const START: [f64; 7] = [0.0, 0.01, 0.02, 0.17, 0.55, 0.62, 0.85];
    let (mut part, mut flows, mut flow) = (None, 0, 0.0);
    for line in log.lines().map(str::trim) {
        let banner = line
            .strip_prefix('[')
            .and_then(|l| l.split_once("/6]"))
            .and_then(|(k, _)| k.parse::<usize>().ok())
            .filter(|k| *k <= 6);
        if banner.is_some() {
            (part, flows, flow) = (banner, 0, 0.0);
            continue;
        }
        let bar = line
            .strip_prefix("[flow] [")
            .and_then(|l| l.split_once(']'));
        let done = bar.and_then(|(_, rest)| {
            let (i, n) = rest.split_whitespace().next()?.split_once('/')?;
            Some((i.parse::<u32>().ok()?, n.parse::<u32>().ok()?.max(1)))
        });
        if let Some((i, n)) = done {
            flows += (i == 0) as u32;
            flow = f64::from(i) / f64::from(n);
        }
    }
    let k = part?;
    let end = START.get(k + 1).copied().unwrap_or(1.0);
    // The shape part runs two flows: the low one (a sixth of it), then the high one.
    let within = match (k, flows) {
        (3, 0 | 1) => flow * 0.15,
        (3, _) => 0.15 + flow * 0.85,
        _ => flow,
    };
    Some((START[k] + (end - START[k]) * within).clamp(0.0, 1.0))
}

/// A character's steps: the model's three, its skeleton, its moves.
pub fn steps_of(it: &Item, moves: bool) -> u32 {
    match (it.kind == "character", moves) {
        (false, _) => 3,
        (true, false) => 4,
        (true, true) => 5,
    }
}

/// Whether a Forge run waits for the machine's heavy-work lock
/// (`tools/heavy.sh`, two slots shared by every heavy job on this Mac). The
/// engine prints nothing while it waits, so the process table says it: one
/// of its `heavy.sh` runs (its work folder `marker` in the arguments) is
/// alive with no `lockf` holding a slot for it.
pub async fn waits_for_heavy_slot(marker: &str) -> bool {
    let ps = tokio::process::Command::new("/bin/ps")
        .args(["-Aww", "-o", "command="])
        .kill_on_drop(true)
        .output()
        .await;
    let Ok(out) = ps else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mine: Vec<&str> = text.lines().filter(|l| l.contains(marker)).collect();
    mine.iter().any(|l| l.contains("heavy.sh"))
        && !mine.iter().any(|l| l.starts_with("/usr/bin/lockf"))
}

/// Otto Forge's work folder for a job: `<forge>/out/work/<job>-…` (the job's
/// output is named by its id) holding `file` (generate draws `concept.png`
/// first and logs Pixal3D's steps to `generate.log`).
pub async fn work_file(forge_bin: &Path, job_id: &str, file: &str) -> Option<PathBuf> {
    let work = forge_bin.parent()?.parent()?.join("out").join("work");
    let prefix = format!("{job_id}-");
    let mut dir = tokio::fs::read_dir(work).await.ok()?;
    while let Ok(Some(e)) = dir.next_entry().await {
        let found = e.path().join(file);
        if e.file_name().to_string_lossy().starts_with(&prefix)
            && tokio::fs::metadata(&found).await.is_ok_and(|m| m.is_file())
        {
            return Some(found);
        }
    }
    None
}

/// Where a production keeps its items' pictures (not the planner's folder,
/// which an agent writes in).
pub fn preview_path(data_dir: &Path, production_id: &str, item_id: &str) -> Option<PathBuf> {
    let dir =
        otto_core::paths::confine_join(&data_dir.join("media").join("productions"), production_id)?;
    otto_core::paths::confine_join(&dir, &format!("{item_id}.png"))
}

/// Where a character draft's finishing inputs are kept between its draft
/// and its game-ready pass (Forge's `<draft>.glb.finish/` and its manifest).
pub fn finish_dir(data_dir: &Path, production_id: &str, item_id: &str) -> Option<PathBuf> {
    let dir =
        otto_core::paths::confine_join(&data_dir.join("media").join("productions"), production_id)?;
    otto_core::paths::confine_join(&dir, &format!("{item_id}.finish"))
}

/// Run every future at once, polled in the order given (so they queue on a
/// fair semaphore in that order), until all are done.
pub async fn join_all<F: Future<Output = ()>>(futs: Vec<F>) {
    let mut futs: Vec<Option<Pin<Box<F>>>> = futs.into_iter().map(|f| Some(Box::pin(f))).collect();
    std::future::poll_fn(|cx| {
        for slot in futs.iter_mut() {
            if slot
                .as_mut()
                .is_some_and(|f| f.as_mut().poll(cx).is_ready())
            {
                *slot = None;
            }
        }
        match futs.iter().all(Option::is_none) {
            true => Poll::Ready(()),
            false => Poll::Pending,
        }
    })
    .await
}

// Measured on the real engine (an M5 Pro, one GPU job at a time; Otto
// Forge's speed report and the owner's pier made by Smart mode), seconds: a
// concept picture on the warm image server (and the server's start), a 3D
// shape (Pixal3D, 145–550 by how much detail the piece has; a character's
// run also gets ready inside it), an object's game-ready pass (CPU, 35–145),
// a skeleton (GPU, 40–300) and moves (CPU), a sound, music, a line.
const PICTURE_S: i64 = 30;
const SERVER_S: i64 = 10;
const SHAPE_S: i64 = 250;
const CHARACTER_S: i64 = 350;
const READY_S: i64 = 80;
const CHARACTER_READY_S: i64 = 80;
const SKELETON_S: i64 = 120;
const MOVES_S: i64 = 45;
const LIBRARY_S: i64 = 2;
const VOICE_S: i64 = 4;
const SOUND_S: i64 = 100;
const MUSIC_S: i64 = 150;

/// THE time estimates (seconds) of what is still to be made, the same for
/// the plan (before Start: every enabled item not made) and the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eta {
    /// Until every piece's picture is drawn.
    pub pictures: i64,
    /// Until the scene can be looked at: every piece has a shape.
    pub scene: i64,
    /// Until everything is made.
    pub all: i64,
}

/// One GPU job at a time, one CPU job beside it: library lookups, every
/// picture (`pictures`: drawn first; else inside each 3D run), every shape
/// one after another (an object's game-ready pass overlaps the next shape,
/// so only the last one's is left after them), then the skeletons and
/// sounds one after another on the GPU, the last moves on the CPU. Spoken
/// lines take the GPU for seconds. A working item counts by what is left
/// of it; a piece the library has is only known once looked up, so it
/// counts as made, and so does one reused from the project (it is in the
/// scene at once). Sound and music count only when their engine is
/// installed (they are skipped otherwise).
pub fn eta(p: &Production, sound: bool, music: bool, pictures: bool) -> Eta {
    let (mut pics, mut shapes, mut lib, mut tail) = (0, 0, 0, 0);
    let (mut after, mut moves, mut last_ready, mut voice) = (0, 0, 0, 0);
    let reused = |i: &Item| i.status == "waiting" && i.details.reuse.is_some();
    for it in p.plan.items.iter().filter(|i| i.enabled && !reused(i)) {
        let left = match it.status.as_str() {
            "waiting" => 100,
            "working" => 100 - it.progress_pct.clamp(0, 100),
            "done" => 0,
            _ if p.status == "ready" => 100,
            _ => 0,
        };
        let share = |s: i64| s * left / 100;
        match it.kind.as_str() {
            "character" | "object" => {
                let character = it.kind == "character";
                let made = it.made.as_deref();
                if made.is_none() && pictures {
                    pics += share(PICTURE_S);
                }
                if matches!(made, None | Some("picture")) {
                    lib += LIBRARY_S;
                    shapes += share(if character { CHARACTER_S } else { SHAPE_S });
                    shapes += i64::from(!pictures) * share(PICTURE_S);
                    // A character's shape is done when its run is (ready inside it).
                    last_ready = character as i64 * share(CHARACTER_READY_S);
                }
                if !character && matches!(made, None | Some("picture") | Some("draft")) {
                    tail = share(READY_S);
                }
                if character {
                    after += share(SKELETON_S);
                    if !it.details.moves.is_empty() {
                        moves = moves.max(share(MOVES_S));
                    }
                }
            }
            "voice_line" if it.details.nonverbal == Some(true) && sound => after += share(SOUND_S),
            "voice_line" => voice += share(VOICE_S),
            "sound" if sound => after += share(SOUND_S),
            "music" if music => after += share(MUSIC_S),
            _ => {}
        }
    }
    let drawn = if pics > 0 { pics + SERVER_S } else { 0 };
    let scene = lib + drawn + shapes + last_ready;
    Eta {
        pictures: if pics > 0 { lib + drawn } else { 0 },
        scene: if shapes > 0 { scene } else { 0 },
        all: scene + voice + tail.max(after + moves),
    }
}

#[cfg(test)]
mod tests {
    use super::shape_fraction;

    /// Pixal3D's own lines (from a real `generate.log`) move a shape's
    /// fraction forward: its parts by where the time goes, the shape part's
    /// low flow then its high flow.
    #[test]
    fn pixal3d_lines_say_how_far_the_shape_is() {
        let parts = [
            "[0/6] Pixal3D single view: /x/matted.png",
            "[2/6] SS proj conditioning + flow",
            "      [flow] [....................]  0/8    0.0s  starting",
            "      [flow] [##########..........]  4/8   18.5s  ~19s left",
            "[3/6] shape SLAT flow (LR 512 -> upsample -> HR 1024 cascade, max_tok=49152)",
            "      [flow] [....................]  0/8    0.0s  starting",
            "      [flow] [####################]  8/8   20.5s  ~0s left",
            "      [flow] [....................]  0/8    0.0s  starting",
            "      [flow] [##########..........]  4/8  108.5s  ~108s left",
            "[5/6] texture SLAT flow (HR 1024, NAF@1024) + PBR decode",
        ];
        let at = |n: usize| shape_fraction(&parts[..n].join("\n")).unwrap();
        assert_eq!(shape_fraction("loading weights"), None);
        assert_eq!(at(1), 0.0);
        assert!((at(4) - (0.02 + 0.15 * 0.5)).abs() < 1e-9);
        // The low flow done is a sixth of the shape part; half the high flow more.
        assert!((at(7) - (0.17 + 0.38 * 0.15)).abs() < 1e-9);
        assert!((at(9) - (0.17 + 0.38 * (0.15 + 0.85 * 0.5))).abs() < 1e-9);
        assert_eq!(at(10), 0.62);
        let mut seen: Vec<f64> = (1..=parts.len()).map(at).collect();
        let sorted = seen.clone();
        seen.sort_by(f64::total_cmp);
        assert_eq!(seen, sorted, "never backwards");
    }
}
