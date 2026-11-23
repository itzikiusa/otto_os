# Create scene suite

Checks whether Create (Smart mode) turns different descriptions into scenes that
are what was asked. The corpus (`corpus.json`) holds 22 descriptions written the
way a person types them, each with what it asks for, written down before any run:
the things, where they are (on / in / under / beside / held / left of …), sizes,
facing, quoted lines and their speakers, and the setting and details it states or
clearly implies ("missing: water" is a failure).

## Run it

```sh
# Any running daemon and its own engine (the real productions): one scenario at a
# time, never two makes at once, resumable (a finished scenario is skipped; an
# unfinished one is picked up by its production id).
python3 tools/scene-suite/suite.py remote --base http://127.0.0.1:7700 --token <bearer> --out <dir> \
    --scenes t01-pier,t02-breakfast,t03-campfire,t04-giant-mouse,t05-moon,t06-under-the-sea,t07-snowman,t08-animal-band,t09-desert-road,t10-bedtime
#   --workspace <id> (default: the first), --provider/--model (default: the daemon's default agent),
#   --skip-audio (leave sound, music and nonverbal lines out), --scenarios <file> (default: corpus.json)

# Level A on this checkout: an isolated daemon, the real planner, stand-in models
/Users/itziklavon/otto-forge/tools/heavy.sh cargo build -p ottod
python3 tools/scene-suite/suite.py run --run baseline                       # plans with codex (the owner's agent)
python3 tools/scene-suite/suite.py run --run after --plans-from baseline    # the same plans again, no planner turns
python3 tools/scene-suite/suite.py owner --run owner-real                   # the owner's pier from his real models
python3 tools/scene-suite/suite.py check --run baseline                     # re-render and re-check a run
```

Per scenario (`<out>/<id>/`): `plan-planned.json` (as the planner wrote it), `plan.json`
(after making, with every item's status, result and the builder's notes), `production-*.json`,
`scene.json`, `models/*.glb`, `timeline.json` (every status/progress/message change),
`result.json` (items, per-item and per-step timings, check results), `voices.json`,
`jobs-3d.json`, `camera.png` / `camera_labels.png` (the scene's own camera), `top.png`,
`vis.json`, `checks.json`; `<out>/summary.md` is the scenario × check table.

Results of `run`/`owner`/`check` go to `~/.claude/projects/-Users-itziklavon-claude-ade/research/design/create-scene-suite/<run>/`
(`SCENE_SUITE_OUT` moves them): per scene the plan, the production, the scene
document, `checks.json`, `result.json` and the pictures (`camera.png`,
`camera_labels.png`, `top.png`); `summary.md` is the scene × check table.

## How

- An isolated copy of `target/debug/ottod` on port 7889 with a temp data dir,
  `env -i`, usage engine off (a bootstrap boot with an empty HOME turns it off).
  The real daemon, the app and its data are never touched; the owner's plan and
  models are only read (sqlite `-readonly`, blob files).
- Level A: the REAL planner (codex by default, as the owner used) writes each
  plan; the built-in Otto Forge provider points at `fake-forge.sh` (the e2e fake
  engine's seam), which hands back a stand-in model per prompt from
  `standin.py`: boxes at the planned size with a plausible shape per kind (a person
  with a Forge-like skeleton and hands, a table on legs, an open basket…). The real
  executor and scene builder run unchanged. `--plans-from` replays saved plans
  through a stand-in agent (no planner turns).
- Level B: the same with the installed Otto Forge (`real`), or with the owner's
  finished models and his fox's real rig failure (`owner`). Sound and music are
  left out (the suite cannot judge them).
- `checks.py` measures the placed pieces (stand-in parts exactly; a real model by
  its bounds box) and `render_blender.py` renders each scene from its own camera
  and from the top and counts what the camera sees (Blender, through heavy.sh).
