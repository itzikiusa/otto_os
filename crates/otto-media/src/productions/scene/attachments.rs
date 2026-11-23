//! Resolve support and attachment chains after placing the free models.
use super::*;

pub(super) struct Attachments<'m> {
    pub(super) groups: BTreeMap<String, Vec<String>>,
    pub(super) supports: BTreeMap<&'m str, String>,
    pub(super) socketed: Vec<(&'m Model, Link)>,
}

pub(super) fn place<'m>(
    plan: &Plan,
    models: &'m [Model],
    spots: &mut BTreeMap<&'m str, Spot>,
    scales: &BTreeMap<String, [f64; 3]>,
    geo: &BTreeMap<&'m str, Surface>,
    link: impl Fn(&str) -> Option<Link>,
    notes: &mut Vec<String>,
) -> Attachments<'m> {
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);
    let name = |id: &str| item(id).map_or(id.to_string(), |i| i.name.clone());
    // "the The Monster" never: a name's own leading "The" goes.
    let bare = |id: &str| bare_name(&name(id)).to_string();
    let layout = |id: &str| plan.layout.iter().find(|p| p.item == id);
    let holder = |id: &str| holder_of(plan, models, id);
    let spot = |m: &Model| Spot::for_model(m, scales[m.item.as_str()]);
    let slot_of = |id: &str| layout(id).map_or((0, 0), |p| slots(&p.place));
    // Attached models (see [`Link`]), grouped with their target and placed
    // after it, a chain in order (the fox on the crate on the pier; one whose
    // target was not made stands free):
    // - on: ON its top, one over the spot it names (else the middle),
    //   several each where it says (else by its `place`) on the target's
    //   footprint a margin in, packed like the ground grid, never into each other;
    // - in: inside it, resting on its floor; under: below it on its floor;
    // - beside / behind / in front of: on its floor next to it on that side
    //   (toward the middle when it says none), several in a row outward;
    // - above: over it; around: in a ring around it;
    // - held / worn: at that bone once every piece has turned (below).
    let mut tucked = Vec::new();
    let mut supports = BTreeMap::new();
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut riders, mut socketed) = (Vec::new(), Vec::new());
    let mut row: BTreeMap<(String, (i32, i32)), f64> = BTreeMap::new();
    let mut pending: Vec<(&Model, Link)> = models
        .iter()
        .filter_map(|m| Some((m, link(&m.item)?)))
        .collect();
    loop {
        let (ready, wait): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|(_, l)| spots.contains_key(l.to.as_str()));
        pending = wait;
        if ready.is_empty() {
            break;
        }
        let mut surfaces: BTreeMap<String, Vec<(&Model, Link)>> = BTreeMap::new();
        let mut rings: BTreeMap<String, Vec<&Model>> = BTreeMap::new();
        for (m, l) in ready {
            groups.entry(l.to.clone()).or_default().push(m.item.clone());
            let t = spots[l.to.as_str()];
            let own = spot(m);
            let [thx, thz] = t.half();
            let [ohx, ohz] = own.half();
            // A spot inside the target's footprint (its place words, a margin in).
            let within = |s: Option<(i32, i32)>| {
                let (x, z) = s.unwrap_or((0, 0));
                [
                    t.center[0] + x as f64 * (thx - ohx).max(0.0) * 0.8,
                    t.center[1] + z as f64 * (thz - ohz).max(0.0) * 0.8,
                ]
            };
            let at = |center: [f64; 2], floor: f64| Spot {
                center,
                floor,
                rot_y: t.rot_y,
                ..own
            };
            // The target's real shape decides: in needs a real cavity (else
            // the piece sits on top), under needs real clearance (else it
            // stands beside), each said plainly.
            let sf = geo.get(l.to.as_str());
            let spot = in_spot(&l, sf, |s| t.to_model(within(s)));
            let here = t.to_model(within(spot));
            let base = [ohx.max(ohz) / t.scale[0], ohx.max(ohz) / t.scale[2]];
            if let Some(s) = lay_textile(
                &l,
                (item(&m.item), item(&l.to)),
                sf,
                (own, t),
                within(l.spot),
                supports.contains_key(l.to.as_str()),
            ) {
                if s.tilt.is_some() {
                    tucked.push((m, l.clone()));
                }
                spots.insert(&m.item, s);
                if l.how == How::On {
                    supports.insert(m.item.as_str(), l.to.clone());
                }
                riders.push((m, l.to.clone()));
                continue;
            }
            let (how, said) = real_how(&l, sf, &t, here, base, own.size(1));
            if let Some(said) = said {
                notes.push(
                    said.replace("{it}", &bare(&m.item))
                        .replace("{to}", &bare(&l.to)),
                );
            }
            if matches!(how, How::On | How::In) {
                supports.insert(m.item.as_str(), l.to.clone());
            }
            match how {
                How::On
                    if words(&name(&l.to)).iter().any(|w| w == "stand")
                        && words(&name(&m.item))
                            .iter()
                            .any(|w| w == "microphone" || w == "mic") =>
                {
                    let mount = microphone_mount(models, &l.to, t);
                    spots.insert(&m.item, at([mount[0], mount[2]], mount[1]));
                }
                How::On
                    if socketed.iter().any(|(parent, link): &(&Model, Link)| {
                        parent.item == l.to
                            && fist(models, link).is_some()
                            && !item(&parent.item).is_some_and(|i| i.kind == "character")
                    }) =>
                {
                    // Its definitive position is the tip of the gripped model,
                    // after the hand is posed. It requires no flat surface.
                    spots.insert(&m.item, at(t.center, t.top()));
                    continue;
                }
                How::On => {
                    surfaces.entry(l.to.clone()).or_default().push((m, l));
                    continue;
                }
                How::Around => {
                    rings.entry(l.to.clone()).or_default().push(m);
                    continue;
                }
                How::Socket | How::Head | How::EnclosedHead => {
                    // Fit it now: its children stand on the fitted surface.
                    // After facing is resolved the entire chain moves to the socket.
                    let held = holder(&m.item).is_some_and(|c| c.id == l.to);
                    let (fitted, _) = fit_worn(models, &l, t, own, held);
                    let mut pending = at(t.center, t.floor);
                    pending.scale = fitted.scale;
                    spots.insert(&m.item, pending);
                    socketed.push((m, l));
                    continue;
                }
                How::In => {
                    // Resting on its inner floor (a closed one hides it).
                    let floor = match sf.and_then(|sf| sf.cavity(here[0], here[1])) {
                        Some(low) => t.world_y(low),
                        None => t.floor + (0.05 * (t.top() - t.floor)).min(0.03),
                    };
                    spots.insert(&m.item, at(within(spot), floor));
                }
                How::Under => {
                    spots.insert(&m.item, at(within(l.spot), t.floor));
                }
                How::Above => {
                    let label = name(&m.item);
                    let tokens = words(&label);
                    let column = tokens
                        .iter()
                        .any(|w| matches!(w.as_str(), "trail" | "stream" | "column" | "plume"));
                    let medium = tokens
                        .iter()
                        .any(|w| matches!(w.as_str(), "bubble" | "smoke" | "steam"));
                    let source = if column && medium {
                        // Animated lids can cover the cavity in the GLB rest pose.
                        // Their hinge gives the mouth's height when opened.
                        let hinge = models
                            .iter()
                            .find(|model| model.item == l.to)
                            .and_then(|model| model.joints.get("lid_hinge"))
                            .filter(|_| words(&name(&l.to)).iter().any(|w| w == "open"));
                        hinge.map(|p| p[1]).or_else(|| sf?.cavity(here[0], here[1]))
                    } else {
                        None
                    };
                    let floor = source.map_or_else(
                        || t.top() + overhead_clearance(&label, plan, models, scales),
                        |y| t.world_y(y),
                    );
                    spots.insert(&m.item, at(within(l.spot), floor));
                }
                How::Beside => {
                    Neighbours {
                        plan,
                        models,
                        geo,
                        spots,
                        supports: &mut supports,
                        rows: &mut row,
                        notes,
                    }
                    .place(m, &l, own, t, groups[&l.to].len() == 1);
                    continue;
                }
            }
            riders.push((m, l.to.clone()));
        }
        for (target, kids) in rings {
            let t = spots[target.as_str()];
            for (i, m) in kids.iter().enumerate() {
                let own = spot(m);
                let a = std::f64::consts::TAU * i as f64 / kids.len() as f64;
                let r = t.radius() + GAP / 2.0 + own.radius();
                let center = [t.center[0] + r * a.sin(), t.center[1] + r * a.cos()];
                spots.insert(
                    &m.item,
                    Spot {
                        center,
                        floor: t.floor.max(0.0),
                        ..own
                    },
                );
            }
        }
        for (target, kids) in surfaces {
            let t = spots[target.as_str()];
            let slot = surface_slots(&kids, |id| slot_of(id), |id| name(id));
            let mut cells: BTreeMap<(i32, i32), Vec<(&Model, Spot)>> = BTreeMap::new();
            for ((m, _), s) in kids.iter().zip(slot) {
                cells.entry(s).or_default().push((m, spot(m)));
            }
            let packed = kids.iter().all(|(_, l)| l.packed);
            let (placed, said) = on_top(t, geo.get(target.as_str()), &cells, packed, |id| {
                let actor = item(id).filter(|i| i.kind == "character")?;
                let description = actor.description.to_ascii_lowercase();
                let crown = actor.details.body.as_deref() == Some("bird")
                    && (description.contains("top of")
                        || words(&description)
                            .iter()
                            .any(|w| w == "atop" || w == "treetop"))
                    && layout(id)
                        .and_then(|p| p.rel.as_ref())
                        .and_then(|r| r.spot.as_deref())
                        .is_none_or(|spot| slots(spot) == (0, 0))
                    && t.size(1) > t.size(0).max(t.size(2));
                Some((models.iter().find(|m| m.item == id)?, crown))
            });
            for (id, note) in said {
                notes.push(
                    note.replace("{it}", &bare(id))
                        .replace("{to}", &bare(&target)),
                );
            }
            spots.extend(placed);
        }
    }
    // Never silently gone: a piece whose chain never reaches a placed one
    // (two attached to each other) stands free, with a note.
    for (m, l) in pending {
        let s = Spot {
            center: [0.0, 0.0],
            ..spot(m)
        };
        spots.insert(&m.item, s);
        notes.push(format!(
            "I could not put the {} with the {}, so it stands on its own.",
            bare(&m.item),
            bare(&l.to)
        ));
    }
    line_up(plan, spots, &supports, &groups);
    face_targets(plan, models, spots, riders);
    for (m, l) in tucked {
        let cover_id = l.to.as_str();
        let Some(bed_id) = supports.get(cover_id).map(String::as_str) else {
            continue;
        };
        let (Some(fabric), Some(mattress), Some(it)) =
            (geo.get(cover_id), geo.get(bed_id), item(&m.item))
        else {
            continue;
        };
        if let Some((toy, cover)) = bedding::rest(
            m,
            it,
            &l,
            spots[m.item.as_str()],
            (spots[cover_id], fabric),
            (spots[bed_id], mattress),
        ) {
            spots.insert(m.item.as_str(), toy);
            if let Some(target) = spots.get_mut(cover_id) {
                *target = cover;
            }
        }
    }
    Attachments {
        groups,
        supports,
        socketed,
    }
}
