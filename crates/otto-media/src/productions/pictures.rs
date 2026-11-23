//! Every piece's picture first: before any 3D shape is built, Otto Forge
//! draws each generated piece's concept picture on its own (`forge generate
//! --concept-only`), one after another on one warm image server for the
//! pass (`forge concept-server`, which runs under the machine's heavy lock and
//! stops on SIGTERM; on a Forge without it each picture loads the model itself). A person sees
//! every picture in minutes; each shape is then built from exactly its
//! picture (Forge draws the same picture a whole run would).

use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::cancel::CancelGuard;

use super::{copy_picture, forge_serves_concepts, forge_takes, picture_body, Shared, JOB_POLL};
use crate::gen3d::{read_optional_manifest, run_cli_argv, signal_group};
use crate::productions::progress::{preview_path, step, steps_of, waits_for_heavy_slot, DRAWING};
use crate::service::MediaService;
use crate::types::BUILTIN_OTTO_FORGE_ID;

/// The concept and shape jobs must see the same geometry requirements.
pub(super) fn visual_prompt(plan: &super::Plan, it: &super::Item) -> String {
    let mut prompt = it
        .details
        .prompt
        .as_ref()
        .unwrap_or(&it.description)
        .clone();
    if it.kind == "object" {
        // Keep an independently drawn occupant out of the cover's picture,
        // and keep that cover out of the occupant's picture.
        let named = |id: &str| plan.items.iter().find(|i| i.enabled && i.id == id);
        for placement in &plan.layout {
            let Some(rel) = &placement.rel else { continue };
            if rel.to == it.id && matches!(rel.how.as_str(), "on" | "in" | "under") {
                if let Some(occupant) = named(&placement.item) {
                    if rel.how == "under" {
                        remove_occupant_size(&mut prompt, &occupant.name);
                    }
                    remove_purpose(
                        &mut prompt,
                        " for ",
                        &super::super::scene::words(&occupant.name),
                    );
                }
            } else if placement.item == it.id && rel.how == "under" {
                if let Some(cover) = named(&rel.to) {
                    let words = super::super::scene::words(&cover.name);
                    if let Some(noun) = words.last() {
                        for clause in [" so ", " that fits under "] {
                            remove_purpose(&mut prompt, clause, std::slice::from_ref(noun));
                        }
                    }
                }
            }
        }
    }
    if it.kind == "object"
        && plan.items.iter().any(|actor| {
            actor.enabled
                && actor.kind == "character"
                && crate::productions::pose::effective(plan, actor)
                    .and_then(|pose| pose.plant)
                    .as_deref()
                    == Some(it.id.as_str())
        })
    {
        prompt.push_str(" The shaft ends in a bare pointed tip for pressing into the ground. Keep the lower shaft slender and omit a display stand or pedestal.");
    }
    prompt
}

/// "A teddy-sized rise" describes a fold, but the image generator draws a
/// second teddy there. Remove only size comparisons to this cover's occupant.
fn remove_occupant_size(prompt: &mut String, name: &str) {
    let name = name.to_ascii_lowercase();
    for word in std::iter::once(name.as_str()).chain(name.split_whitespace()) {
        if word.is_empty() {
            continue;
        }
        let comparison = format!("{word}-sized ");
        let lower = prompt.to_ascii_lowercase();
        for (at, _) in lower.rmatch_indices(comparison.as_str()) {
            if !lower[..at].ends_with(char::is_alphanumeric) {
                prompt.replace_range(at..at + comparison.len(), "");
            }
        }
    }
}

/// Keep the physical description on either side of a purpose clause. Only
/// clauses naming an independently placed scene piece can be removed.
fn remove_purpose(prompt: &mut String, introducer: &str, piece: &[String]) {
    let lower = prompt.to_ascii_lowercase();
    if piece.is_empty() {
        return;
    }
    for (start, _) in lower.rmatch_indices(introducer) {
        let end = lower[start..]
            .find([',', '.', ';', '!'])
            .map_or(lower.len(), |n| start + n);
        let clause = super::super::scene::words(&lower[start..end]);
        if clause.windows(piece.len()).any(|part| part == piece) {
            prompt.replace_range(start..end, "");
            break;
        }
    }
}

/// Forge reports this error only after rejecting the starting seed and its
/// successor. Its sampler seeds wrap modulo 2^32.
pub(super) fn next_concept_seed(error: &str, seed: Option<i64>) -> Option<i64> {
    error
        .starts_with("Concept anatomy still mismatches the requested body after two drawings:")
        .then(|| i64::from((seed.unwrap_or(42) as u32).wrapping_add(2)))
}

/// Forge ships rendered PNGs for catalog models as thumbnails/<pack>/<model>.png.
pub(super) async fn library_picture(bin: &Path, provenance: &serde_json::Value) -> Option<PathBuf> {
    if provenance["source"] != "library" {
        return None;
    }
    let (pack, model) = provenance["library"]["id"].as_str()?.split_once('/')?;
    let slug = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    };
    if !slug(pack) || !slug(model) {
        return None;
    }
    let bin = tokio::fs::canonicalize(bin).await.ok()?;
    let root = tokio::fs::canonicalize(bin.parent()?.parent()?.join("models/library/thumbnails"))
        .await
        .ok()?;
    let path = tokio::fs::canonicalize(root.join(pack).join(format!("{model}.png")))
        .await
        .ok()?;
    let meta = tokio::fs::metadata(&path).await.ok()?;
    (path.starts_with(&root) && meta.is_file() && (8..=2 * 1024 * 1024).contains(&meta.len()))
        .then_some(path)
}

impl MediaService {
    /// Library imports from earlier builds may have no saved thumbnail.
    pub(super) async fn imported_picture(
        &self,
        a: &otto_design::DesignArtifact,
        item: &crate::productions::Item,
    ) -> Option<PathBuf> {
        if a.meta["source"] != "library" && item.message.as_deref() != Some(super::FROM_LIBRARY) {
            return None;
        }
        let design = self.design_service();
        let v = match item.result.as_ref()?.version_id.as_deref() {
            Some(id) => design.resolve_version(a, id).await,
            None => design.head_version(a).await,
        }
        .ok()?;
        library_picture(Path::new(&self.forge_bin().await?), &v.provenance).await
    }

    /// Draw the picture of every 3D piece that has none yet, in plan order,
    /// holding the GPU turn for the pass. Each lands as the item's preview
    /// at once (`made: "picture"`); one that fails is drawn by its 3D run.
    pub(super) async fn draw_pictures(&self, s: &Shared, guard: &CancelGuard) {
        let todo = self
            .todo(s, |i| {
                i.made.is_none() && matches!(i.kind.as_str(), "character" | "object")
            })
            .await;
        let Some(bin) = self.forge_bin().await else {
            return;
        };
        if todo.is_empty() || !forge_takes(&bin, "--concept-only").await {
            return;
        }
        let _turn = self.production_gpu.acquire().await.ok();
        let workdir = self
            .store()
            .require_3d_provider(BUILTIN_OTTO_FORGE_ID)
            .await
            .ok()
            .and_then(|f| f.cli_workdir)
            .filter(|d| !d.trim().is_empty())
            .map(PathBuf::from);
        let serves = forge_serves_concepts(&bin).await;
        let takes_body = forge_takes(&bin, "--body").await;
        let mut server = match serves {
            true => ConceptServer::start(Path::new(&bin), workdir.as_deref()),
            false => None,
        };
        let mut url = None;
        for (n, &ix) in todo.iter().enumerate() {
            if guard.is_cancelled() {
                break;
            }
            let (pid, it, prompt) = {
                let p = s.lock().await;
                (
                    p.id.clone(),
                    p.plan.items[ix].clone(),
                    visual_prompt(&p.plan, &p.plan.items[ix]),
                )
            };
            let of = steps_of(&it, !it.details.moves.is_empty());
            self.begin(s, ix, step(1, of, DRAWING)).await;
            if n == 0 {
                if let Some(server) = &mut server {
                    url = self.until_up(server, s, ix, guard).await;
                }
            }
            let to = preview_path(self.data_dir(), &pid, &it.id);
            let seed = it.seed.unwrap_or(42).to_string();
            let mut retry_seed = None;
            let drawn = match (to, tempfile::tempdir()) {
                (Some(to), Ok(tmp)) => {
                    let out = tmp.path().join(format!("{pid}-{}.png", it.id));
                    let mut args: Vec<String> = ["generate", "--prompt", &prompt, "--seed", &seed]
                        .into_iter()
                        .map(String::from)
                        .collect();
                    // The same picture a character's draft is built from (its recipe).
                    if serves && it.kind == "character" {
                        args.extend(["--kind".into(), "character".into()]);
                    }
                    if let Some(body) = picture_body(&it).filter(|_| takes_body) {
                        args.extend(["--body".into(), body.into()]);
                    }
                    args.extend(["--concept-only".into(), "--output".into()]);
                    args.push(out.to_string_lossy().into_owned());
                    if let Some(url) = &url {
                        args.extend(["--concept-server".to_string(), url.clone()]);
                    }
                    let limit = Duration::from_secs(1800);
                    let wd = workdir.as_deref();
                    match run_cli_argv(Path::new(&bin), &args, wd, limit, guard.flag(), None).await
                    {
                        Ok(()) => {
                            if let Some(checks) =
                                read_optional_manifest(&out).await.and_then(|mut m| {
                                    m.get_mut("concept_checks").map(serde_json::Value::take)
                                })
                            {
                                tracing::info!(production = %pid, item = %it.id, %checks, "concept drawing checks");
                            }
                            copy_picture(&out, &to).await
                        }
                        Err(error) => {
                            if let otto_core::Error::Upstream(message) = &error {
                                retry_seed = next_concept_seed(message, it.seed);
                            }
                            tracing::warn!(production = %pid, item = %it.id, %error, "concept drawing failed");
                            false
                        }
                    }
                }
                _ => false,
            };
            self.edit(s, |p| {
                let it = &mut p.plan.items[ix];
                (it.status, it.step, it.waiting) = ("waiting".into(), None, None);
                it.working_since = None;
                if drawn {
                    it.made = Some("picture".into());
                    (it.has_preview, it.preview_rev) = (true, it.preview_rev + 1);
                } else if let Some(seed) = retry_seed {
                    it.seed = Some(seed);
                }
            })
            .await;
        }
    }

    /// Wait for the image server to answer (it may wait for a heavy slot
    /// first: the piece says so); `None` when it ended or Stop came.
    async fn until_up(
        &self,
        server: &mut ConceptServer,
        s: &Shared,
        ix: usize,
        guard: &CancelGuard,
    ) -> Option<String> {
        let mut polls = 0u32;
        while !guard.is_cancelled() && matches!(server.child.try_wait(), Ok(None)) {
            if let Ok(url) = server.ready.try_recv() {
                self.edit(s, |p| p.plan.items[ix].waiting = None).await;
                return Some(url);
            }
            if polls.is_multiple_of(4) {
                let held = waits_for_heavy_slot(&format!("--listen-port {}", server.port)).await;
                let waits = held.then(|| "engine".to_string());
                self.edit(s, |p| p.plan.items[ix].waiting = waits).await;
            }
            polls += 1;
            tokio::time::sleep(JOB_POLL).await;
        }
        None
    }
}

/// `forge concept-server`: FLUX.2 klein loaded once for a pass, under the
/// heavy lock, on a free local port; it says `{"ready": true, "url": …}` on
/// its first line when it answers. Dropped: SIGTERM, on which Forge stops
/// the image server and exits.
struct ConceptServer {
    child: tokio::process::Child,
    port: u16,
    ready: tokio::sync::oneshot::Receiver<String>,
}

impl ConceptServer {
    fn start(forge_bin: &Path, workdir: Option<&Path>) -> Option<Self> {
        // Its own free port: asked of the system, then handed to the server.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .ok()?
            .local_addr()
            .ok()?
            .port();
        let mut cmd = tokio::process::Command::new(forge_bin);
        cmd.args(["concept-server", "--port", &port.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        if let Some(dir) = workdir {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().ok()?;
        let stdout = child.stdout.take()?;
        let (tx, ready) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let said: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
                if let (Some(true), Some(url)) = (said["ready"].as_bool(), said["url"].as_str()) {
                    let _ = tx.send(url.to_string());
                    return;
                }
            }
        });
        Some(Self { child, port, ready })
    }
}

impl Drop for ConceptServer {
    fn drop(&mut self) {
        if let Some(pid) = self.child.id() {
            signal_group(pid, libc::SIGTERM);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_cover_is_drawn_without_the_separate_toy_tucked_under_it() {
        let plan: super::super::Plan = serde_json::from_value(json!({
            "title":"Bedtime",
            "items":[
                {"kind":"object","id":"cover","name":"Striped blanket","description":"","details":{"prompt":"A striped green blanket with a raised edge for tucking in the plush rabbit, finished with yellow trim."}},
                {"kind":"object","id":"bunny","name":"Plush Rabbit","description":"","details":{"prompt":"A plush rabbit lying on its back with its head raised slightly so its face can show above the blanket edge, with embroidered whiskers."}}
            ],
            "layout":[{"item":"bunny","rel":{"to":"cover","how":"under"}}]
        })).unwrap();
        let text = visual_prompt(&plan, &plan.items[0]);
        assert!(!text.contains("rabbit"), "{text}");
        assert!(
            text.contains("striped green blanket") && text.contains("yellow trim"),
            "{text}"
        );
        let rabbit = visual_prompt(&plan, &plan.items[1]);
        assert!(!rabbit.contains("blanket"), "{rabbit}");
        assert!(
            rabbit.contains("on its back")
                && rabbit.contains("head raised slightly")
                && rabbit.contains("embroidered whiskers"),
            "{rabbit}"
        );
    }

    #[test]
    fn a_size_reference_does_not_draw_a_second_occupant_in_the_cover() {
        let mut plan: super::super::Plan = serde_json::from_value(json!({
            "title":"Bedroom",
            "items":[
                {"kind":"object","id":"sheet","name":"Patchwork quilt","description":"","details":{"prompt":"A child's patchwork quilt made of faded cotton squares with little running stitches, spread flat with a small teddy-sized rise near the head end and the top edge folded down."}},
                {"kind":"object","id":"toy","name":"Teddy bear","description":"A little honey-brown stuffed teddy bear resting on its back, with round ears, a stitched nose and a soft flat belly that fits under a quilt."}
            ],
            "layout":[{"item":"toy","rel":{"to":"sheet","how":"under"}}]
        })).unwrap();
        let original = plan.items[0].details.prompt.clone().unwrap();
        assert_eq!(
            visual_prompt(&plan, &plan.items[0]),
            original.replace("teddy-sized ", "")
        );
        assert_eq!(
            visual_prompt(&plan, &plan.items[1]),
            plan.items[1]
                .description
                .replace(" that fits under a quilt", "")
        );
        plan.items[1].enabled = false;
        assert_eq!(visual_prompt(&plan, &plan.items[0]), original);
    }

    #[test]
    fn a_planted_pole_has_a_ground_tip_in_the_visual_request() {
        let mut plan: super::super::Plan = serde_json::from_value(json!({
            "title": "Moon",
            "items": [
                {"kind":"character","id":"astronaut","name":"Astronaut","description":"","details":{"pose":{"kind":"stand","plant":"flag"}}},
                {"kind":"object","id":"flag","name":"Flag","description":"", "details":{"prompt":"Blue flag on a silver pole with its base at ground level."}}
            ]
        })).unwrap();
        let flag = &plan.items[1];
        let prompt = visual_prompt(&plan, flag);
        assert!(
            prompt.contains("Blue flag") && prompt.contains("bare pointed tip"),
            "{prompt}"
        );
        plan.items[0].enabled = false;
        assert_eq!(
            visual_prompt(&plan, &plan.items[1]),
            plan.items[1].details.prompt.as_deref().unwrap()
        );
    }
}
