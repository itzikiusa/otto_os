# Deviations from CONTRACT.md

Node names, clip names and file names are exactly as contracted. Where the
contract left something open or could not be met literally, this is what the
build does:

1. **Painted planes' UV convention** (`Screen`, `BoardTitle`, `NamePlate`,
   `DoorSign`): the exported glTF `TEXCOORD_0` has (0,0) at the bottom-left
   and (1,1) at the top-right *as seen* — literally as the contract states.
   In three.js that means a runtime texture needs `texture.flipY = true` (the
   default for `CanvasTexture`/`Texture`; GLTFLoader's own maps use `false`).
   The embedded default maps (`textures/ph_*`) are stored upside down so they
   look right with GLTFLoader's `flipY = false`. `build_school.py --verify`
   checks the orientation from the .glb bytes.
2. **`Board` origin** = x-centre, **floor level** (y = 0), wall surface
   (z = 0); the board spans y 0.9–2.3. (Contract: "origin at the wall surface,
   bottom edge 0.9 m above floor" — read as "place the origin where the wall
   meets the floor".)
3. **`Ceiling_Light` origin** = centre of the emitting face (y = 0, facing
   down); the housing rises to y = 0.1. Place it at `ceilingHeight − 0.1`.
4. **`Stand_Up` / `Sit_Down` are in place**: the kid stands up on the chair
   origin (the desk is in front, so there is nowhere else to stand without
   root motion). Slide the kid ≈0.45 m along ±X into the aisle while the clip
   plays (and back for `Sit_Down`).
5. **Kids are Quaternius CC0 characters** on a 62-joint rig (elbows, knees,
   fingers) with a 1.6× head for kid proportions. The head joint's rest
   scale is also keyed in every clip, because some importers (Blender's)
   drop rest scale on joints. `Sit_*` hips and wrists are measured on the
   skinned mesh by `--verify`. The hips sit at y ≈ 0.42 on the seat, and the
   wrists are at the keyboard, whose centre moved to z = −0.20 in the
   Workstation (0.35 m from the chair) so a 1.1 m kid can reach it. Feet
   dangle, as they would for a small child on a 0.42 m seat. The `Walk`
   clip is the pack's walk, in place (≈0.9 m/s).
6. **`Bench` seat top is at 0.42** (contract ≈0.45 high), the same as the
   `Chair`, so the `Sit_*` clips sit flush on it. Its front faces +Z.
7. **`Corridor_Wall` lockers** stand 0.14 m proud of the inside face
   (z 0…0.14); everything else on walls stays within a few cm.
8. **Headmaster height**: 1.6 m to the top of the head; the mortarboard adds
   ≈0.1 m. The un-animated rest pose is the source file's bind pose (play
   `Idle` on load). The original face morph targets (Angry/Surprised/Sad) are
   kept.
9. **Extras** (not required, harmless): `school-kit.glb` also has the
   decor nodes `Backpack`, `Trashcan`, `Plant_Desk`, `Book`, `Book_Open`,
   `Water_Bottle` (floor/desk origin, front +Z) and `Corkboard` (origin at
   the wall-surface centre). `ui/public/school/env.hdr` is a 1k CC0 HDRI for
   `RGBELoader`. The kid files carry a `ChestDecal` child under the `Chest`
   joint. `Clock` has child nodes
   `Clock_HourHand` / `Clock_MinuteHand` (pivot at the centre, rotate about Z;
   rest = 10:10). Kid clips are stored alphabetically in the file.
10. Total shipped size is ≈9.6 MB (the lead raised the budget to 15 MB).
11. Small spheres (apple, globe, mouse) are flat-shaded: smooth normals at the
   poles differ in the last bits between Blender runs, which made rebuilds
   non-reproducible.
