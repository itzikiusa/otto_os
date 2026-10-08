# Rooms Arcade asset contract

Binary glTF 2.0, meters, +Y up, **+Z forward**. All model roots have identity
transform. Runtime code positions/rotates the root. Root names are stable;
internal mesh names and glTF accessor indices are implementation details.

## Fighters

`fighter-azure.glb`, `fighter-ember.glb`: root **Fighter**, floor at Y=0;
W×H×D = **0.914×1.827×1.239 m**, including the forward-pointing rifle.

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
W×H×D = **1.812×1.298×2.505 m**.

Exact wheel pivots: **WheelFL**, **WheelFR**, **WheelRL**, **WheelRR**.
Rotate local X for rolling. Set front wheel steering around local Y; combine
with wheel rolling in a quaternion or wrapper pivot rather than overwriting
one angle with the other. Wheel pivot centers: X=±0.75, Y=0.31, Z=+0.67 (front)
and −0.66 (rear). Tire radius ≈0.313m. **DriverHead** is an independent pivot
for subtle look/lean feedback. Driver and steering wheel are modeled; karts
have no baked animation clips.

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
| StationWall | 4×3×0.409 | Paneled station bulkhead |
| Reactor | 2.2×2.95×2.2 | Luminous core with confinement rings |
| FoundryFurnace | 2.3×3.6×1.93 | Hot grate, heavy frame, twin chimneys |
| PipeRack | 3.1×2.4×0.55 | Copper piping and couplers |
| DesertRock | 3.014×3.03×2.107 | Weathered clustered sandstone |
| DesertArch | 7.1×5.49×2.17 | Large weathered arch |
| PalmTree | 4.27×3.97×4.33 | Segmented trunk, layered fronds, coconuts |
| PineTree | 2.57×4×2.55 | Layered evergreen |
| BroadleafTree | 3.88×4.53×3.83 | Trunk, branch canopy clusters |
| CoastalRock | 2.414×1.437×1.848 | Pale coastal rock cluster |
| NeonTower | 3.5×8.15×2.9 | Window grid, neon corners |
| NeonSign | 2.8×2.55×0.7 | Illuminated directional chevrons |
| RaceGantry | 10.1×4.575×1.5 | Checkered gantry and start lamps |
| TireStack | 0.86×0.7×0.86 | Three stacked tires |
| TrackBarrier | 3×0.85×0.7 | Race safety barrier |
| BoostPad | 2.4×0.067×3 | Emissive chevrons on a plate |
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

Six GLBs together are about 4 MiB, with zero textures. Two chooser images are
loaded outside gameplay. Total public folder must remain below 20 MiB.
`manifest.json` records SHA-256, bytes, triangle count, mesh/material count,
exact node names and animation clips for each GLB. `validation.json` independently
re-imports the GLBs through Blender and measures geometry/roots/clip presence.
