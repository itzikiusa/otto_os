# forge pose

`forge pose` holds a rigged character in a pose that fits its scene: sitting on a seat of a given height, kneeling, crouching, lying, curled up, holding out a hand palm up, and looking at a point. It works on the output of `forge rig`. The pose is added to the GLB as a clip, played by default. It runs on the CPU in under half a second per character, with no Blender and no GPU.

```sh
bin/forge pose --mesh rigged.glb --pose sit --seat-height 0.35 --look-at 0,0.3,1.1 --output posed.glb
bin/forge pose --mesh giant.glb --pose stand,hold-out --hand right --look-at 0.5,4,3 --output posed.glb
bin/forge pose --mesh fox.glb --pose sit --seat-height 0.65 --look-at 0,1.6,1.0 --output posed.glb
```

| Option | Meaning |
| --- | --- |
| `--pose` | One body pose, `stand`, `sit`, `kneel`, `crouch`, `lie` or `curl`, optionally followed by `,hold-out` (an open palm, up) or `,hold` (a closed fist holding something). `hold-out` or `hold` on its own means standing. Neither goes with `lie` or `curl`, and the two cannot be combined. |
| `--seat-height` | Metres from the character's floor to the top of the seat, in the model's own scale. Required for a humanoid `sit`. For a four-legged body it raises the surface it sits or lies on (a crate). |
| `--look-at` | `x,y,z` in metres, measured from the point on the floor under the character's origin. `x` is the character's left, `y` is up and `z` is the way it faces. The neck turns 40 % of the way and the head the rest. The look is capped at 75° to either side, 60° up and 45° down. |
| `--hand` | `left` or `right` for `hold-out` or `hold`. The default is `right`. |
| `--hold-style` | `raise` (default): a stick, spoon or sword held up and forward. `carry`: a lantern or bucket hanging from the fist, held forward and a little out from the body so it can be seen. |
| `--hold-toward` | `x,y,z`, same frame as `--look-at`. For `raise` only: the forearm turns halfway towards the point and the wrist up to 65°, so the handle points at it. Example: a stick over a fire 1 m ahead is `0,0,1`. |

Otto's uniform argv (`--mesh --image --prompt --output --poly-budget --seed`) is accepted; empty values mean "not given".

## What each body can do

| Body (from `forge rig`) | Poses |
| --- | --- |
| Humanoid (Unity names) | all: stand, sit, kneel, crouch, lie, curl, hold-out, hold, look-at |
| Four legs (body plan `quadruped`) | stand, sit (on the haunches), lie (a sphinx), curl, look-at |
| Any other plan (two-legged bird, many legs, snake, blob) | stand, and look-at when it has a head |

Any other request exits with `{"error": "pose 'sit' is not supported for a … body (supported here: …)"}`. No model is written in that case.

## How the poses are made

Each bone is aimed along its joint-to-child direction in world space. Angles are never added to the rest pose. SkinTokens rest poses vary (T, A, crouched), and their glTF bone tails are guesses.

- **Sit.** The thighs lie along the seat, and the body is lowered or raised until the lowest skin of the hips and thighs is on the seat top. Each shin then reaches for the floor with the foot flat in its rest orientation, or hangs straight down when the knee is higher than the shin is long (`contact.feet: "dangle"`).
- **Kneel.** The left knee goes down and the right foot forward, then the lowest skin is put on the floor.
- **Crouch.** Two-segment leg IK, with the ankles under and slightly ahead of the hips and the spine leaned forward.
- **Lie.** On the back.
- **Curl.** On the right side, with the knees drawn to the chest, the shins tucked, the arms to the knees and the head bowed. A tail on a two-legged rig (a cat) is laid round the feet.
- **Hold-out.** The arm goes forward and down and the forearm forward. The hand is rolled about its length until the palm normal points up. The palm normal comes from the index, middle and little finger roots. A rig without finger joints keeps its hand roll and reports the normal as up.
- **Four legs, sit (a dog's sit).**
  - The body pitches nose-up about the hind hips, at 15–50°.
  - The hind legs fold flat: the thigh goes forward, the shin back, and the foot lies flat. The shin's slope is chosen so the hind feet come down as low as the rump.
  - The rump rests on the surface: the hind hips, the pelvis and the tail root.
  - Of those angles, the one is kept where the straight front legs come closest to standing on the same surface.
  - Front legs that would still reach below it lean forward by the difference (up to 40°), so the paws stay planted ahead of the chest. Before, the body tipped nearly upright.
  - The tail, including a tail tip the body plan named as a separate limb, slopes back down as steeply as it can without going under the surface.
  - Before/after stills: `out/speed-evidence/poses/qsit/sheet_quad_sit.png`. They show the fox from the Codex picture, the fox rig made sitting (`fox4`) and the standing fox.
- **Four legs, lie and curl.** A rig modelled sitting up is first tipped forward about its hind hips until its back is within 10° of level. Lie is on the belly: the front legs make a sphinx, the hind legs fold under, the tail lies along the floor and the head dips 10°. Curl is a sleeping ball: the front legs fold back under the chest and the spine bends 150° round to the left. The head turns with it and dips 20°, and the tail wraps round. `bounds` gives the posed size, for fitting under a table.

The clip is a 3-second breathing loop of about 1° in the chest. It never moves the hips, the feet or the root. It is inserted first in the GLB's animation list, and every existing clip (idle, walk …) is kept.

## Holding things (`hold`)

The upper arm goes forward and down, and the forearm forward. For `raise` the forearm points up 20°, turned halfway towards `--hold-toward` when given. For `carry` it is level and a little out from the body. The fingers then fold into a loose fist: about 70° at the knuckle, 80° at the middle joint, 55° at the tip, with the thumb laid across. The hand is rolled, and then bent at the wrist (up to 65°), so the handle line through the fist points:
- for `raise`: up and forward, or at the target;
- for `carry`: with the thumb on top, so the load hangs below.

A rig without finger joints only turns its hand bone, and the grip is the middle of that bone.

`grip` in the manifest:
- `point`: the centre of the closed fist, where a handle passes, in the model's frame.
- `axis`: the unit handle direction, signed so the free end leaves along +axis. For `raise` it runs out of the thumb side, towards the target. For `carry` it is straight down, the way the load hangs.
- `up`: out of the top of the fist (the thumb side).
- `reach.tip_1m`: the end of a 1 m stick held at `point` along `axis`.
- `reach.toward_m`: the shortest distance from the `--hold-toward` point to that 1 m stick (`null` without a target).

## Manifest

`<output>.manifest.json` has the usual Forge keys plus the keys below. Points are in the output model's own glTF frame, in metres with the same origin as the input.

```json
{
  "pose": {"names": ["sit", "hold-out"], "clip": "sit", "body": "humanoid", "seat_height_m": 0.35,
           "look_at": [0, 0.3, 1.1], "hand": "right"},
  "default_clip": "sit",
  "contact": {"floor_y": -0.599, "seat": [x, y, z], "feet": "touch"},
  "palm": {"hand": "right", "point": [x, y, z], "normal": [x, y, z]},
  "head": {"point": [x, y, z], "gaze": [x, y, z]},
  "bounds": {"min": [x, y, z], "max": [x, y, z]},
  "check": {"pass": true, "floor_penetration_m": 0.0, "seat_penetration_m": 0.0, "collapsed_share": 0.0014,
            "torn_share": 0.0, "reasons": [], "cut_triangles": 0}
}
```

- **`contact.seat`** is where the body rests: the middle of the lowest hip and thigh skin, at the seat top. For four legs it is the hind hips on the surface. Put it on the seat object's top centre.
- **`contact.feet`** is `touch`, `dangle` or `none` (lying).
- Radial percussion poses record the two striking tips in `contact.sticks` and every articulated arm tip in `limb_tips`. Each position is in model space, in metres, at the opening animation frame.
- **`palm.point`** is on the visible palm skin, 1 mm out, 62 % of the way from the wrist to the middle finger's root (the palm's hollow), in the posed mesh as a viewer shows the pose clip's first frame. It is where a ray coming down the palm normal from a hand's length above meets the skin. Put the carried object's base there. A centred model needs half its height added along `palm.normal`. `palm.normal` is the skin's normal there, averaged over a small ring of rays, pointing away from the back of the hand.
- **`head.gaze`** is a unit direction.

## Self-check

Before writing, the posed skin is measured:
- skin under the floor, or under the raised surface;
- for a sit, the deepest skin inside the seat block under the resting part of the body;
- the share of skin triangles that keep less than 20 % of their area;
- torn skin: triangles with an edge stretched past 3 times its rest length and longer by over 5 % of the height.

The limits are 2 % of the character's height, 3 % of its height, 2 % of the triangles and 1.5 % torn triangles. A torn triangle that fuses two body parts four or more bones apart fails the check on its own.

**Fused skin is cut, not stretched.** A generated mesh can join parts that were modelled touching, such as a hanging forearm and the thigh. When a pose pulls them apart, the triangles between them become long spikes. Before the check, every torn triangle whose corners follow bones four or more apart is left out of the written mesh, and `check.cut_triangles` counts them. This changes the mesh for every clip in the GLB; at rest those triangles sit in the gap between the touching parts. A small slit can show where they were.

The owner's 2 m keeper robot (no finger joints) had 30 such triangles between the right forearm and hand and the right thigh. Carry, raise and hold-out all tore them into spikes from the hand to the hip. Its weights were not the cause: no torn triangle had a weight on a far bone. With the cut, all three poses pass with `torn_share` 0. Stills from one camera, before and after: `out/speed-evidence/poses/stretch/sheet_keeper_tear.png`. Body poses fold skin inside the body at hips and knees. On the real rigs, the highest torn share is 1.09 %, on the fox curl, and that is the reason for the 1.5 % limit. A pose over any limit exits 1 with `{"error": "pose not written: …"}` and writes no GLB. The manifest is still written, with `check.pass: false` and the reasons.

Measured on the real campfire children, the four-legged fox and an 8 m giant (2026-11-21): skin never went under the floor or into the seat, and 0–0.30 % of the triangles collapsed. The pose itself took 0.05–0.13 s, and the whole `bin/forge pose` call 0.21–0.29 s per character, including Python start-up. Stills: `out/speed-evidence/poses/final/sheet_forge_pose_final.png`.

## Limits

- **Which end is the head** comes from the body plan (forge/bodyplan.py). The end whose last joints carry two or more side branches (ears or horns) is the head, before toes, height and girth are considered. That cue fixed the standing fox whose raised bushy tail had been read as its head. A rig with no ears, a raised tail and paws pointing backwards can still be read backwards.
- **No finger curl:** a held-out hand stays open.
- **One kneel** (left knee down).
- **Lie is on the back only;** curl lies on the right side.
- **The fist is a fixed loose fist.** It does not fit the handle's thickness, and a thick handle passes through the fingers.
- **The standing-fox evidence rig lies and curls poorly.** Its rear legs fold only partly, so its body stays high.

## Tests

```sh
engines/assetfurnace/.venv/bin/python -m unittest tests.pose.test_pose
```

Twenty-three tests on synthetic skinned rigs: a Unity-named humanoid with finger roots, the body-plan test quadruped, a six-legged body and a snake. No packs, Blender or GPU are needed. The real giant and child rigs are used when `out/speed-evidence/poses/inputs` holds them.
- **Sit:** the hips are on the seat within 1 cm and the soles on the floor within 2 cm, also with twisted joint frames. A high seat leaves the feet dangling.
- **Look-at:** the gaze is within 10° of the target.
- **Hold-out:** the palm skin's normal points up (dot with up above 0.85), the palm point is at the hand and the hand is held forward, on both hands.
- **Palm point on the skin** (synthetic, both hands; the real 8 m giant and a child), checked on the GLB's first clip, as a viewer plays it:
  - the manifest point equals the first frame's palm within 1 mm;
  - a sphere at the point plus 2 cm along the normal is clear of the skin in 26 directions;
  - straight below it the skin is 2.1 cm away;
  - the point lies between the wrist and the finger roots;
  - the normal points out of the palm.
- **Every humanoid pose** passes its check and stays above the floor.
- **Hold:**
  - raise closes the fist (the finger skin comes nearer the hand) and aims the handle at a target on the floor ahead;
  - carry points the axis down, puts the thumb on top and holds the fist away from the hips and below the shoulder;
  - hold works with a seat;
  - bad combinations are refused;
  - a rig without finger joints still gets a grip.
- **Four legs:** sit raises the head and looks up; on a crate it rests on the crate top; lie and curl lower the body; human poses are refused.
- **Other plans** are refused except stand.
- **A collapsed skin** fails the check.
- **Torn skin**, on a rig whose hands hang against its thighs:
  - a forearm fused to the thigh fails the check, and exactly the fused triangles are cut;
  - thigh skin weighted to the forearm is refused;
  - a clean rig tears nothing in carry, raise, hold-out, crouch or curl;
  - cut triangles leave the written mesh.
- **The stage:** clips are kept with the pose first, the manifest has the keys and depth above, a failed check writes no model, bad input gives clean errors, and the real `bin/forge` runs.
