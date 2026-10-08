# Rooms Arcade asset contract

Binary glTF 2.0, meters, +Y up, **+Z forward**. All model roots have identity
transform. Runtime code positions/rotates the root. Root names are stable;
internal mesh names and glTF accessor indices are implementation details.

## Fighters

`fighter-azure.glb`, `fighter-ember.glb`: root **Fighter**, floor at Y=0;
W×H×D = **0.969×1.847×1.501 m**, including the forward-pointing rifle.

Exact clips: **Idle**, **Run**, **Shoot**, **Death**. Idle is two seconds, Run
0.8 seconds, Shoot one third second, Death 1.1667 seconds. Idle/Run loop;
Shoot/Death play once. Death should clamp at completion. Clips are in place;
the simulation moves the root. The articulation is rigid Object3D animation,
not a skeleton/skinned mesh: regular `clone(true)` plus an independent
`AnimationMixer` per fighter works. Crossfade locomotion actions and restore
Idle/Run after Shoot.

Pivots: `Hips`, `Torso`, `Head`, `UpperArmL`, `UpperArmR`, `ForearmL`,
`ForearmR`, `UpperLegL`, `UpperLegR`, `LowerLegL`, `LowerLegR`.
`PulseRifle` is attached to the right forearm. **Muzzle** is a child marker at
the muzzle mouth; use its world position for visual muzzle flash only.
Simulation still determines shot origin and collision.

Team material name **Azure** / **Ember** can be cloned and recolored. Emissive
**CyanLight**, **AmberLight** and visor **Glass** are separate material slots.
Do not mutate shared material instances when recoloring only one actor.

## Karts

`kart-azure.glb`, `kart-ember.glb`: root **Kart**, ground Y≈0;
W×H×D = **1.812×1.030×2.484 m**.

Exact wheel pivots: **WheelFL**, **WheelFR**, **WheelRL**, **WheelRR**.
Rotate local X for rolling. Set front wheel steering around local Y; combine
with wheel rolling in a quaternion or wrapper pivot rather than overwriting
one angle with the other. Wheel pivot centers: X=±0.75, Y=0.31, Z=+0.67 (front)
and −0.66 (rear). Tire radius including tread ≈0.331m. **DriverSocket** accepts a separately loaded roster driver (see below). The
steering wheel and seat are modeled; karts contain no baked driver or clips.

## Rifle

`rifle.glb`: root **Rifle**, receiver-centered, +Z forward, length 0.933 m;
size 0.184×0.405×0.933 m. The local receiver center is Y=0; grip extends to
Y=−0.215. This explicit hand-space origin makes first-person camera placement
straightforward. **Muzzle** marker at (0, 0.025, 0.58). No clips.

## Environment kit

`environment-kit.glb` contains exactly twenty reusable top-level roots. Each
root is at world zero; extract/clone the named root, **do not display the whole
file as a scene**. Mesh detail is joined per root with shared material slots.

| Root | W×H×D, meters | Use |
|---|---|---|
| CargoCrate | 1.46×1.46×1.44 | Reinforced cargo cover, latches, warning bands |
| Barrier | 3.2×1.46×0.85 | Armor cover with luminous trim |
| StationColumn | 1.4×3.95×1.4 | Pillar with illuminated corners |
| StationWall | 4.0×3.0×0.408 | Paneled station bulkhead |
| Reactor | 2.2×2.95×2.2 | Dark core, luminous channels and confinement rings |
| FoundryFurnace | 2.3×3.6×1.93 | Hot grate, heavy frame, twin chimneys |
| PipeRack | 3.1×2.4×0.55 | Copper piping and couplers |
| DesertRock | 3.014×3.03×2.107 | Weathered clustered sandstone |
| DesertArch | 7.1×5.489×2.171 | Large weathered arch |
| PalmTree | 4.626×3.982×4.697 | Segmented trunk, feathered fronds, coconuts |
| PineTree | 2.574×3.998×2.547 | Layered evergreen |
| BroadleafTree | 3.878×4.526×3.833 | Trunk, branch canopy clusters |
| CoastalRock | 2.414×1.437×1.848 | Pale coastal rock cluster |
| NeonTower | 3.5×8.15×2.9 | Window grid, neon corners |
| NeonSign | 2.8×2.55×0.7 | Illuminated directional chevrons |
| RaceGantry | 10.1×4.575×1.5 | Checkered gantry and start lamps |
| TireStack | 0.86×0.7×0.86 | Three stacked tires |
| TrackBarrier | 3.0×0.85×0.7 | Race safety barrier |
| BoostPad | 2.4×0.067×3.0 | Emissive chevrons on a plate |
| Pickup | 0.94×0.94×0.94 | Floating energy orb and orbit rings |

Rocks/arch intentionally extend 0.19–0.40m below the origin to bed into terrain;
crate corners extend 3cm below ground. Pickup rests visually above its origin
(minY=0.13). Other roots sit approximately on the floor. Complete exact bounds
are machine-recorded in `validation.json`; collision boxes stay authoritative
in game simulation and must not be inferred from decorative geometry.

## Chooser images

`arena-preview.png` and `kart-preview.png`: 1280×720 actual Blender renders of
the shipped fighters/karts and environments. Source contact sheets are under
`ui/assets-src/room-games/previews/`.

## Budget and manifest

Eight GLBs, including the roster and adventure kit, remain below the combined 20 MiB budget, with zero textures. Two chooser images are
loaded outside gameplay. Total public folder must remain below 20 MiB.
`manifest.json` records SHA-256, bytes, triangle count, mesh/material count,
exact node names and animation clips for each GLB. `validation.json` independently
re-imports the GLBs through Blender and measures geometry/roots/clip presence.

## Character roster and adventure scenery (experience revision)

`drivers.glb` is an independent, reusable roster. **DriverFox**, **DriverPanda**,
**DriverRabbit**, and **DriverRobot** are top-level roots at identity transform.
Their origin is the **seated pelvis**, not the floor. Attach exactly one root at
identity transform to the kart's **DriverSocket**, located at kart local
`(0, 0.67, -0.15)`. Kart GLBs no longer contain a baked helmeted driver. Driver
hands reach `(±0.25, -0.029, 0.365)` relative to the socket, matching the steering
wheel. Head pivots are **FoxHead**, **PandaHead**, **RabbitHead**, **RobotHead**;
fox additionally has **FoxTail**. Rotate the head subtly around Y for gaze,
Z for lean. There are no baked driver clips. The roster is under 1.5 MiB and
shares materials. Each actor should clone its hierarchy before posing pivots.

Portraits: **driver-fox.png**, **driver-panda.png**, **driver-rabbit.png**,
**driver-robot.png**. These are 640×640 transparent renders of the shipped roots.
Fox has orange fur/cream cheeks, pointed ears, a tail and scarf; panda has
round black ears/eye patches; rabbit has long ears/whiskers/teeth; robot has a
friendly digital face and antenna. Runtime palette recoloring must not recolor
fur or facial materials.

`adventure-kit.glb` supplies **Cottage**, **Barn**, **Windmill**, **Fence**,
**BridgeRail**, **CoralFan**, **CoralCluster**, **Seaweed**, **Fish**, **ReefRock**,
and **Buoy**. Roots are identity transforms, +Z front, grounded to minimum Y=0.
Exact per-root bounds are measured in `validation.json`. **WindmillSails** is
an optional child pivot; rotate local Z to turn sails. Fish faces +Z; animate
its root along a path. The scenery is visual decoration; simulation route and
collision definitions remain authoritative.

Adventure library measured bounds (W×H×D, meters):

| Root | Dimensions |
|---|---|
| Barn | 6.935×4.369×5.57 |
| BridgeRail | 4.16×1.2×0.15 |
| Buoy | 0.74×1.7×0.74 |
| CoralCluster | 1.495×1.032×1.497 |
| CoralFan | 2.87×2.803×0.294 |
| Cottage | 5.581×4.31×4.825 |
| Fence | 3.16×1.2×0.15 |
| Fish | 0.365×0.57×0.875 |
| ReefRock | 3.083×1.744×2.296 |
| Seaweed | 1.361×1.82×0.28 |
| Windmill | 7.4×8.5×2.52 |
