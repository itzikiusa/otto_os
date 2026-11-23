# Fit: holding, hands, reaches and a measured physics check

`bin/forge-fit` makes a rigged character hold rigid props, closes its hands around them, authors short
interactions with inverse kinematics, and measures whether any animated GLB "fits": feet on the floor, knees
and elbows bending the right way, hands outside the torso, the prop in the hand. "It fits" is a set of numbers
with thresholds, not an opinion. Every command follows the Forge stage contract: `<output>.manifest.json`
(tools, licences, timings, all measurements), JSON progress lines on stderr, a final `{"error": ...}` and a
non-zero exit on failure, SIGTERM/SIGINT kill the Blender process group, the work directory is removed
(keep it with `FORGE_KEEP_WORK=1`), and every Blender run takes a shared heavy slot (`tools/heavy.sh`).

`check` on a body that is not a humanoid (four legs, eight, one, none) runs `forge/fit/plan_check.py` instead of
Blender. It applies the same foot, ground and loop limits to every leg the body plan finds, and adds
`leg_swing_m` for walk and run. Distances are in the GLB's metres unless `--height` is given. See
[Rig and animate any body](engine.md#rig-and-animate-any-body).

## Measured on this machine (2026-11-20)

All numbers are from the files in `out/fit-evidence/` (see its README); metres at the declared height.

| Case | Result |
| --- | --- |
| Hand intake, procedural fixture | passes: every chain moves only its own skin, 0 web edges, each chain drives the finger it is named after |
| Hand intake, text-generated knight | **fails** both hands: fused webs (9-36 skin edges between neighbouring fingers stretch 2.9-4.0x when one finger curls), middle finger drags ring/index by 42-46% of its own motion, `RightIndexProximal` has no vertex mainly following it |
| Hand intake, warrior `rigged-handfit.glb` | every chain drives its intended finger (the Codex fix holds); still **fails**: `LeftRingDistal` and `RightThumbProximal` own no vertex, ring/little webs stretch 2.0-3.7x |
| hold, fixture + procedural sword | wrap 306 deg around the grip; 4 fingers and the thumb within 4 mm; max penetration 0.8 mm; held through the walk clip with contact fraction 1.0 and 0 m drift at keys |
| hold, knight + procedural sword | wrap 306 deg; 4 fingers + thumb in contact; **penetration 4.8 mm** (limit 3 mm) on the crowded wedge fingers; contact fraction 1.0 and drift 0 through idle, walk, chop and slice |
| hold, knight + generated "longsword" | the generated object has a leather grip at both ends and a paper-thin bent blade; grip 7 cm thick at 1 m (0.75 palm widths radius); wrap only 169 deg, **penetration 9.1 mm** |
| reach pull_sword, knight | reach shortfall 0 after compensation (up to 44 deg extra spine bend and 0.40 m hip drop: a deep squat); both hands keep contact (fraction 1.0) from the grab to the end; sword ownership change jump 0.9 micrometre; feet drift 5.6 mm; fails: penetration 4.3 / 3.7 mm, an elbow bent 73 deg sideways at the grab, forearms 8.8 cm inside the thigh proxy while squatting |
| reach baton_pass, knight | passes all measures; ownership change at release: 0 m jump |
| jumps (KayKit via animlib, grounded) | soles never below the floor; knees bend forward only (max flexion 110-132 deg, 0 deg hyperextension, at most 4.4 deg sideways); slide while planted 4.7-4.9 cm (fails 2 cm); hips acceleration 76-113 m/s^2 at take-off/landing at 30 Hz (fails 60) |
| correct on the jumps | grounding (animlib's) was tried per clip: rejected on all pure jumps (slide got worse, 4.9 -> 5.2 cm), kept on the jump-chop (slide 27.3 -> 14.3 cm); no hinge or arm step was needed |
| shoot_bow, warrior + procedural bow/quiver/arrow | nock on the string within 0.05 mm over 147 nocked samples; shaft on the rest within 0.7 mm; draw fingers within 1.2 mm of the string; release position jump 0.04 mm; draw 0.48 m -> 50.7 m/s; flight fits a parabola with g = 9.810 m/s^2 (residual 4 micrometres), shaft within 0.09 deg of the velocity; string overshoot 12% of brace; quiver does not intersect the body. Fails: right hand 8 cm short of the arrow at the take and 19-37 cm short while lifting it out of the quiver; an elbow-plane flip of 133 deg in one frame during the reach over the shoulder; elbow folded to 165 deg; bow-hand contact fraction 0.80 and 18 mm penetration while returning to rest |
| shoot_bow, warrior + generated bow/quiver/arrow | the toy bow's leather grip ring is about 20 cm across at 1.5 m: the hand closes inside the ring (wrap 111 deg, 20 mm penetration); shaft-to-rest 11.5 mm; the rest as above |

Tests: `tools/heavy.sh tests/fit/run.sh` (8 tests, about 70 s holding one heavy slot) and `--mutations` (11 switched-off features, each detected by its test).

## Thresholds and why

| Measure | Limit | Why |
| --- | --- | --- |
| foot penetration | 1.0 cm | a sole visibly sinks into a flat floor at about a centimetre; shoe soles of generated meshes are rarely flatter than that |
| foot slide while planted | 5.0 cm | planted = sole within 2% of the height above the floor AND moving under 0.5 m/s for at least 0.1 s; a common belt speed is removed for in-place gaits. Round 2: was 2 cm; the hand-made baseline (Quaternius Ranger, native UAL sword attack) pivots on the ball of the foot and moves the sole marker 4.1 cm |
| foot drift (authored interactions only) | 1.0 cm | the IK plants the feet; any drift is a bug |
| knee hyperextension / flexion | 5 / 160 deg | human knees hyperextend about 5 deg; "backwards" only counts when the bend lies within 45 deg of the anatomical plane (a sideways bend of a nearly straight leg has no reliable sign) |
| knee and elbow sideways bend, elbow hyperextension | reported only | Round 2: the hand-made baseline reaches 70-90 deg sideways (arms overhead, wide lunges) and 30 deg elbow "hyperextension" on a visibly normal fast swing; these measures do not separate good from bad on hand-made rigs, so they no longer pass or fail |
| elbow flexion | 155 deg | past typical active flexion |
| elbow-plane flip | 60 deg per 60 Hz frame | the baseline's fast sword swings reach 51; a real flip is 90-180 |
| wrist rotation | 90 deg per 60 Hz frame | the baseline's combos reach 69 deg/frame (about 4000 deg/s); a one-frame flip is near 180 |
| limb inside torso or thighs | 3 cm | convex per-bone proxies overestimate concave bodies; the baseline reaches 2.8 cm with hands at the hips |
| hips acceleration | 150 m/s^2 at 30 Hz | the baseline's leaping combo reaches 112 m/s^2; library jumps 76-113; a pop between keys is far above |
| loop seam | 1 cm | judged on clips whose name says they loop (loop, walk, run, idle) |
| held prop through the body | 1.5 cm | every body vertex except the holding hands and forearms against the prop's closed proxy; the baseline's crouching combo grazes the thigh by 1.1 cm |
| held prop drift between keys | 3 mm | the prop node against the hand bone at half frames, as an engine interpolates them; held props are baked at 4x from the baked bone chain |
| prop penetration / contact gap / contact fraction | 3 mm / 5 mm / 90% | contact is skin within 5 mm of the closed proxy; intended patches are those in contact when the hand takes the prop |
| grip wrap / fingers in contact | 200 deg / 3 fingers + thumb | angular coverage of the contact skin around the grip axis |
| prop wobble relative to the hand | 2 mm, at keys | a rigidly held prop does not move in the hand; half-frame drift is reported separately |
| nock to string / shaft to rest / draw fingers to string | 2 / 5 / 6 mm | |
| release jump / flight residual / tangent | 1 mm / 1 cm / 2 deg | |
| string forward overshoot | 50% of brace height | past that the string would hit the bow hand's wrist |


## Commands

| Command | What it does |
| --- | --- |
| `forge-fit hands --character RIGGED.glb --output R.json [--renders DIR]` | Hand intake test (below). Exit 0 with `"pass": false` when a hand fails. |
| `forge-fit hold --character RIGGED.glb --prop PROP.glb --hand right|left [--class sword] [--prop-length 1.0] [--grip auto|x,y,z --grip-axis x,y,z] --output OUT.glb` | Grip found on the prop, palm pressed to it, fingers closed to contact; prop parented to the hand bone; clips `hand_open`, `hand_closed`, `grip`, `grip_close` plus every input clip with the grip layered on. Refuses a character whose hands fail intake unless `--accept-hands`. |
| `forge-fit fist --character RIGGED.glb --output OUT.glb` | `hand_open`, `hand_closed`, `fist_close`, `fist_open` for both hands, fingers closed against the palm. |
| `forge-fit check --mesh ANIMATED.glb [--prop-node NAME] --output REPORT.json` | Every clip sampled at 120 Hz; pass/fail per measure; `REPORT.png` contact sheet of the worst frames (full body beside a close-up). |
| `forge-fit correct --mesh ANIMATED.glb --output OUT.glb` | Hinge clamp, grounding (animlib's), arm pushed out of the torso; each kept only if check improves and nothing else gets worse. |
| `forge-fit reach --character RIGGED.glb --targets pull_sword|shoot_bow|baton_pass|FILE.json [--prop name=PATH] --output OUT.glb` | Runs an interaction template (below) and bakes one clip; props are their own animated nodes. |

`--height` (default 1.8 m) declares the character's real height. The GLB is never rescaled; instead the
scene-units-per-metre factor `s = rest skin height / height` converts every threshold and every prop (props are read
in metres, or set to `--prop-length`). The knight is 0.875 units tall, so 1 m of sword is 0.486 units on it.

## Hand intake test (`hands`, also run by `hold`)

Finger bone names are not trusted. Chains are first relabelled by geometry: the chain whose root is nearest the
wrist is the thumb; the others are ordered along the knuckle line starting next to the thumb (index, middle,
ring, little). The report lists any renaming. Then each chain is curled on its own to its anatomical limits and
the deformed skin is compared with the rest skin. A chain fails when:

- no skin vertex mainly follows one of its bones (empty bone group);
- its own skin spans less than half of the chain length (stub or missing digit);
- the skin it moves lies nearest another digit's bones (it curls the wrong finger);
- a neighbouring digit's skin moves more than 35% as far as its own tip (it drags its neighbour);
- skin edges joining it to a neighbour stretch more than 1.8x (a fused web).

The report also gives, per joint, the offset of the joint centre from its finger cross-section centre in units
of the cross-section radius (a named joint is an association, not a measured pivot). `--renders` writes open,
half-closed and closed views of each hand from the palm and in profile on a green background, with the
camera clipped so the thigh does not hide the palm.

## Holding

1. **Grip on the prop.** The prop is sampled densely on its surface (vertices plus a barycentric grid, so long
   low-poly faces count) and sliced into 48-64 slabs along its principal axis. For a sword the widest slab is the
   guard, the shorter side is the handle, and the grip is the longest run of slabs no wider than 1.35x the
   thinnest; its radius is the median slab half-width. `--grip x,y,z --grip-axis x,y,z` overrides it.
2. **Closed proxy.** Penetration is only meaningful against a closed volume, so every prop is represented by the
   union of convex hulls of overlapping slabs (Blender's own convex hull). Signed distance: negative inside
   (depth, from the hull planes), positive outside (exact nearest-surface distance from a BVH).
3. **Palm socket.** From the hand and finger joints: wrist W, knuckle centre K, the knuckle line a (little to
   index), the finger direction d and the palm normal n (the side the thumb tip is on). The grip axis is laid
   along a in the curl hollow behind the knuckles (`K + 0.22 (W - K)`), the blade or the bow's depth axis
   rolled to a chosen hand axis, the hand next to the guard, then the prop slides along -n until the palm skin
   touches it.
4. **Close.** Each finger joint flexes, proximal to distal, about a hinge axis `segment x n` within MCP 90,
   PIP 105, DIP 80 degrees (thumb 45/55/75), stopping at the first penetration deeper than 0.4 mm (real), found
   by 3-degree steps and bisection to 0.1 degree. The thumb is first swung to lie along the grip and then closes
   towards the grip axis (opposition). A segment that starts inside the prop opens first (up to 40 degrees).
5. **Report.** Per skin patch (each digit's pad and segments, the palm): gap (+) or penetration (-) against the
   closed proxy, contact when within 4 mm (real).

Grip kinds: `power` (palm contact, all digits) and `hook` (the object across the middle phalanges, only index,
middle and ring intermediate and distal joints flex; used for a bowstring and for taking an arrow).

## Sockets and anchors (for other tools)

Character sockets are rest-pose frames attached to a bone (z first, x second):

| Socket | Bone | Frame |
| --- | --- | --- |
| `right_hand`, `left_hand` | hand | the fitted grip: z along the grip axis, x along the prop's roll axis |
| `back` | UpperChest (or Chest) | origin on the back skin behind the joint; z tilted 25 degrees from up towards the draw-hand shoulder; x pointing backwards |
| `hip_left`, `hip_right` | Hips | origin on the side skin; z down, x backwards |
| `neck` | Neck | origin at the neck joint; z up, x forwards |

Prop anchors (prop-local; vectors named `*_axis`). Found from geometry; any can be overridden.

| Class | Anchors |
| --- | --- |
| sword | `grip`, `grip_axis` (towards the blade), `grip_length`, `grip_radius`, `guard`, `tip`, `pommel`, `roll_axis` (blade width) |
| baton | `grip` (centre), `grip_axis`, `end_a`, `end_b` |
| bow | `grip` (riser centre), `grip_axis` (towards the upper tip), `upper_tip`, `lower_tip`, `depth_axis` (riser towards the string, i.e. the archer), `side_axis`, `nock_rest` (string middle at rest), `arrow_rest` (top of the grip, on the string-side face; the arrow lies beside it by the riser half thickness plus the arrow radius, on the archer's view-left), `brace` |
| quiver | `opening`, `opening_axis` (out of the opening), `bottom`, `radius` |
| arrow | `nock`, `tip`, `shaft_axis` (nock to tip), `grip` (6% of the length from the nock) |

A generated bowstring is never trusted: a separate mesh object with "string" in its name is deleted, and fused
vertices lying on the straight tip-to-tip line (within 0.6% of the bow length, away from the tips) are cut out.
The string is rebuilt as nodes under the bow: `bow_string_nock` (an empty) and `bow_string_upper`/`_lower`
(unit-length segment meshes, tip at the origin, scaled along z to the nock), so drawing the nock pulls two
straight segments. Anchors are exported as child empties named `<prop>.<anchor>`.

## Interaction templates (`reach`)

A template is JSON: targets relative to anchors and sockets over time, executed by one routine. Bundled:
`forge/fit/templates/pull_sword.json`, `shoot_bow.json`, `baton_pass.json`.

```json
{
  "name": "pull_sword", "duration": 2.0, "fps": 60, "aim_yaw_deg": 0,
  "props": {"sword": {"class": "sword", "file": "sword.glb",
            "start": {"place": {"anchor": "tip", "axis": "grip_axis", "z": "down", "x": "left",
                                "at": {"base": "floor", "add": [["forward", 0.40, "m"], ["down", 0.30, "m"]]}}}}},
  "grips": {"right_grip": {"hand": "right", "prop": "sword", "kind": "power", "reverse": true, "slot": 0}},
  "order": ["right_hand", "sword", "left_hand", "head"],
  "tracks": {
    "hips":  [{"t": 0, "offset": [0, 0, 0]}, {"t": 0.6, "offset": [0, -0.03, -0.07]}],
    "spine": [{"t": 0, "bend": 0}, {"t": 0.6, "bend": 28}],
    "sword": [{"t": 0, "static": true}, {"t": 0.62, "follow": "right_hand"}],
    "right_hand": [{"t": 0, "rest": true}, {"t": 0.55, "grip": "right_grip"},
                   {"t": 1.6, "carry": {"grip": "right_grip", "place": {"anchor": "grip", "z": "down", "x": "left",
                                        "at": {"base": "joint:chest", "add": [["forward", 0.32, "m"]]}}}}]
  },
  "events": [{"t": 0.62, "attach": "sword", "to": "right_hand"}]
}
```

- Keys interpolate with smoothstep; each key value is evaluated at the current time, so a blend between a rest
  pose and a moving target never jumps.
- Hand values: `rest` (the FK hand under the current spine), `grip` (the hand where it holds the prop now;
  frozen at the moment the hand takes ownership), `carry` (the hand follows a placement of the held prop),
  `draw` (the hook on the string at a point: `string_rest` or any point), `point`.
- Prop values: `static`, `place`, `follow: <hand|socket:NAME|prop>` (relative transform captured at the key time, so
  ownership changes are continuous), `nocked` (nock on the string, shaft through the rest), `release`.
- Points: `floor`, `joint:KEY`, `prop:NAME.ANCHOR`, `socket:NAME`, `string:nock`, plus `add` terms
  `[direction, amount, unit]` with units `m`, `H` (height), `arml`/`armr` (arm length), `len:PROP`, `r:PROP`.
- Directions: `up down forward back left right aim aim_left aim_right`, `prop:NAME.AXIS`, `joint:A>B`, a leading `-`
  negates; `aim` is forward turned left by `aim_yaw_deg`.
- Body tracks: `hips` (offset in H units: right, forward, up; turn in degrees), `spine` (turn, bend forward,
  lean; spread over Spine/Chest/UpperChest), `head` (look at a point or along a direction; 40% neck, 60% head),
  `elbow_l`/`elbow_r` (pole direction in the body frame: outward, forward, up).
- Feet are always planted on their rest positions with leg IK; the solver reports any unreachable target
  (`max_reach_shortfall_m`).

Arms and legs use an analytic two-bone IK on joint heads rather than Blender's IK constraint: on these AI rigs
the bone tails are arbitrary (the knight's forearm tail is 3 cm from its wrist), and the constraint drives the
tail. The spine, hips and head are FK. The solved pose is written as keys for every bone and every prop.

## Archery model

- **Draw:** the draw hand's hook relation is fitted on a 1.5 mm string proxy; the string's nock is the hook
  point, the arrow's nock is the string's nock, and the shaft points through the arrow rest, by construction;
  `check` measures all three on the exported nodes.
- **Release:** draw length x = distance of the nock from its rest point. Linear force-draw curve: stored energy
  F x / 2; the arrow receives efficiency eta of it, so v0 = sqrt(eta F x / m). Defaults: F = 178 N (40 lb), eta
  0.75, m = 25 g (template `params`). During the power stroke the arrow and nock accelerate uniformly along the
  shaft (duration 2x/v0, keyed every 2 ms) so position and velocity are continuous at release and at separation;
  then a ballistic arc under g = 9.81 m/s^2 (scaled by s), the shaft kept along the velocity. The string then
  overshoots the brace line by at most 20% of the brace height and settles (12 Hz, 40 ms decay).
- The arrow is its own animated node for the whole clip, so an engine can hand it to its projectile system at
  the release frame (time in the manifest).

## Round 2: template hands, functional props, calibration on hand-made assets

**Hand intake, recalibrated.** Webs and neighbour drag are judged at a grip curl (60% of each joint's limit), not at
the maximal isolated curl. At that curl the hand-made Quaternius bases and the Ranger reach at most 1.75x web
stretch and 0.05 neighbour drag; the limits are 2.2x and 0.15. The generated knight (webs 2.4-4.3x, drag 0.24-0.41)
and warrior (webs up to 2.7x, drag up to 0.21) still fail. Empty bone groups, stub digits and chains that curl
another finger's skin fail as before.

**Template hands** (`hands --replace`, and hold/reach when intake fails; `--keep-hands` turns it off). The
original skin that mainly follows the hand and finger bones is cut at the wrist. The new hand is, in order of
preference: a library wrist-cut hand (`--hand-source male|female`, default male, from
`models/library/derived/characters/<sex>/{left,right}.glb`; any GLB or directory with left/right GLBs whose
finger joints use Unity or Mixamo names also works), carried bone by bone onto the character's own joints (a
similarity per segment: thickness from the palm-width ratio, length from the segment ratio clamped to 0.7-1.4 of
it; the palm scaled in width and length; fingertip joints fold into the distal bone), or the procedural fallback
(`--hand-source procedural`, styles simple|gloved|chunky: Skin + Subdivision shells grown along the joints, one per
digit plus a palm and wrist cuff, palm and finger colours sampled separately from the original texture, rigid
per-shell weights blended only near joints). A library hand that would tear on the target skeleton (0.5% of its
edges stretched over 2.5x) is refused and the manifest says so; the procedural hand is then used.

**Functional props.** `forge-fit prop-intake --class sword|bow|arrow|quiver --prop P.glb` reports, in palm widths and
metres, whether a hand can use the prop as its class says, plus its anchors in the file's own frame:
sword (grip 0.15-0.6 palm widths thick and at least 0.9 long, guard wider than twice the grip, guard not in the
middle 38-62% of the length, blade within 2% of straight), bow (grip at most 0.7 palm widths, 0.9-2.0 m), arrow
(shaft within 1% of straight and under 0.2 palm widths, head under 6 cm), quiver (opening radius 2.5-9 cm).
`forge-fit prop --class C [--like GEN.glb] --height-m H` builds a parametric prop sized for the character, styled from
the generated one (region colours by position along the axis; a metal part whose sampled colour is strongly
saturated keeps the steel default; clamped shape ratios). hold and reach run intake on every prop and use a
parametric stand-in when it fails, unless `--keep-prop`; the manifest says which and why. Library props are passed with
`--prop` like any file (library first, parametric second, generated only if it passes).

**Motion.** Elbow and knee bend planes are continuous in time (the offset may turn at most 12 deg per 60 Hz frame);
the clavicle swings up to 35 deg towards targets above the shoulder that the arm cannot reach, rate-limited to
4 deg per frame; compensation bends the spine first (35 deg) and only then drops the hips, capped at 10% of the
height; held props are baked at 4x per key from the baked bone chain; hold picks a wrist carry offset (0 or
+-25 deg in two axes) that keeps the prop out of the body through the layered clips; correct gained
`smooth_root` (hip keys blended with a 50 ms Gaussian around acceleration spikes, kept only if it helps).

## Limits (what does not work yet)

- **Hand geometry decides the ceiling.** The knight's wedge fingers are fused by skin webs; closing one finger stretches the
  web 3-4x and drags its neighbour, so a "closed" hand reads as a crumpled mitten with faceted shards, and the closest
  achievable grip still penetrates the handle by 4.8 mm. The fit refuses such a hand unless `--accept-hands` is given.
- **Only the hand is collision-checked against a held prop.** Guard against forearm, blade against thigh, bow against the
  body are not measured (the quiver against the body is).
- **Two-handed hold** measures contact for the hand the prop is parented to only; the second hand's contact is not in the
  check report of a hold file (it is in the reach outputs, where both grips are recorded).
- **Reach targets beyond the arm are not reached**: the solver squats and bends only for targets below the shoulder and
  reports the shortfall; over-the-shoulder reaches (the quiver) and the arrow lift stay short on the warrior.
- **IK quality:** analytic two-bone IK with a pole per key; elbow plane flips can still happen when a pole swings while the
  arm is straight (measured: 133 deg/frame in shoot_bow).
- **No limb bend on the bow**, string drawn as two straight segments.
- **Props as separate animated nodes drift between keys** relative to the hand (2.8 cm at half frames for the arrow during
  a fast lift) because an engine interpolates a node and a bone chain differently; bake at a higher rate or parent in-engine.
- Profile views in `hands --renders` can be hidden by the hip in A-pose; the palm views are the informative ones.
