# Otto School — asset sources & licences

Everything shipped in `ui/public/school/` is built from the files in this
folder. Every third-party input is **CC0 1.0** (public domain dedication);
nothing CC-BY or GPL is used.

| Source | Author | Licence | Used for | Files in `base/` / `env/` / `textures/` |
|---|---|---|---|---|
| [Hoodie Character](https://poly.pizza/m/gKLBoRsyKe) (Ultimate Animated Character pack) | Quaternius | CC0 1.0 | kid body, rig and the Idle/Walk/Wave source clips | `base/quaternius/hoodie.glb` |
| [Casual Character](https://poly.pizza/m/kZ3DmIoGip), [Animated Woman](https://poly.pizza/m/qJ2gsTUBHL), [Animated Woman](https://poly.pizza/m/nIItLV9nxS), [Punk](https://poly.pizza/m/djXoqejw6w), [Adventurer](https://poly.pizza/m/ZwF0K7WBmu) | Quaternius | CC0 1.0 | kid heads (extracted by `prep_base.py`) | `base/quaternius/{casual,woman1,woman2,punk,adventurer}.glb` |
| [RobotExpressive](https://github.com/mrdoob/three.js/tree/dev/examples/models/gltf/RobotExpressive) | Tomás Laulhé (Quaternius), modifications by Don McCurdy | CC0 1.0 | Headmaster Otto | `base/robot-expressive/RobotExpressive.glb` |
| [Houseplant](https://poly.pizza/m/VtJh4Irl4w), [Houseplant](https://poly.pizza/m/f6GPjbEgg0), [Backpack](https://poly.pizza/m/2g9Jm7kvIU), [Trashcan Small](https://poly.pizza/m/i7HDuYDLkx), [Book](https://poly.pizza/m/LC0w7VI75u), [Open Book](https://poly.pizza/m/FsCIGEfTEs), [Water Bottle](https://poly.pizza/m/KpxDpidn1Z) | Quaternius | CC0 1.0 | `Plant`, `Plant_Desk`, `Backpack`, `Trashcan`, `Book`, `Book_Open`, `Water_Bottle` | `base/props/{houseplant2,houseplant,backpack,trash,book,openbook,bottle}.glb` |
| [Wall Corkboard](https://poly.pizza/m/U8yQZ9l0HZ) | CreativeTrio | CC0 1.0 | `Corkboard` | `base/props/corkboard.glb` |
| [Empty Play Room](https://polyhaven.com/a/empty_play_room) HDRI, 1k | Greg Zaal, Jarod Guest (Poly Haven) | CC0 1.0 | `env.hdr` (runtime IBL; preview lighting) | `env/empty_play_room_1k.hdr` |
| [Oak Veneer 01](https://polyhaven.com/a/oak_veneer_01) diffuse, downscaled to 512² | Poly Haven | CC0 1.0 | wood (teacher desk, bookshelf, board frame, bench) | `textures/wood_oak.jpg` |
| Provider marks (`decals/*.svg`) | Otto (redrawn from `ui/src/lib/components/ProviderIcon.svelte`; grok and shell marks drawn here) | same as the Otto repo | kid chest decals | `textures/decal_*.png` |
| Generated textures | made for this project with Codex CLI image generation (prompts in `TEXTURE_PROMPTS.md`) | owned by the project | posters, chalk art, board, floors, walls, lockers, window view, screen wallpaper | `textures/*.jpg` |

Everything else — desks, chairs, monitors, keyboards, board, bookshelf and
books, clock, bench, walls, windows, doors, lockers, lights, the mortarboard
and bow tie, and every authored animation clip — is modelled or keyframed by
`blender/build_school.py` and its `lib/` modules.
