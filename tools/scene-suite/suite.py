#!/usr/bin/env python3
"""The Create (Smart mode) scene suite: does Create turn different
descriptions into scenes that are what was asked? See README.md.

  suite.py run    --run baseline [--scenes t01-pier,t02-breakfast] [--plans-from RUN]
                  level A: an isolated daemon of this checkout, the real planner, stand-in models
  suite.py remote --base http://127.0.0.1:7700 --token TOKEN --out DIR [--scenes ...]
                  any running daemon (its own engine): create, plan, make, collect, render, check
  suite.py owner  --run baseline-real    the owner's pier from his real finished models
  suite.py check  --run baseline         re-render and re-check a run
"""
import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
import checks  # noqa: E402
import glbparts  # noqa: E402
import standin  # noqa: E402

HOME = Path.home()
OUT = Path(os.environ.get("SCENE_SUITE_OUT", HOME / ".claude/projects/-Users-itziklavon-claude-ade/research/design/create-scene-suite"))
HEAVY = os.environ.get("HEAVY", str(HOME / "otto-forge/tools/heavy.sh"))
BLENDER = os.environ.get("BLENDER", "/Applications/Blender.app/Contents/MacOS/Blender")
OWNER_DB = HOME / "Library/Application Support/Otto/otto.db"
OWNER_BLOBS = HOME / "Library/Application Support/Otto/design/blobs"
OWNER_PRODUCTION = "01M7X5TJYE0K81CKMXDHSEQEP2"
PORT = int(os.environ.get("SCENE_SUITE_PORT", "7889"))
PASSWORD = "scene-suite-only-1"
REPLAY = "plan-replay"
TERMINAL = ("done", "failed", "cancelled")


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def sha16(data):
    return hashlib.sha256(data if isinstance(data, bytes) else data.encode()).hexdigest()[:16]


def load_corpus(path=None):
    return {s["id"]: s for s in json.loads(Path(path or HERE / "corpus.json").read_text())}


def maker_prompt(it):
    """What the executor sends Forge for an item: details.prompt, else the description."""
    p = (it.get("details") or {}).get("prompt")
    return p if p is not None else it["description"]


def library_asks(it):
    """The executor's two library asks for an object (head word, then the whole name)."""
    head = it["name"].split()[-1] if it["name"].split() else it["name"]
    return list(dict.fromkeys([f"{head}. {it['description']}", f"{it['name']}. {it['description']}"]))


BUILDER_NOTE = re.compile(r"because I can.t find|does not hold .* yet|stand next to it for now|cannot .* yet, so that was left out|could not put the scene")


def planned(plan):
    """A plan as the planner wrote it: no progress, no results, no maker notes."""
    p = json.loads(json.dumps(plan))
    for it in p.get("items", []):
        for k in ("status", "progress_pct", "message", "result", "previous_result"):
            it.pop(k, None)
    p["notes"] = [n for n in p.get("notes", []) if not BUILDER_NOTE.search(n)]
    return p


def save(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(obj if isinstance(obj, str) else json.dumps(obj, indent=1, ensure_ascii=False))


# --------------------------------------------------------------------- API
class Api:
    def __init__(self, base, token=None, ws=None):
        self.base, self.token, self.ws = base.rstrip("/"), token, ws

    def call(self, method, path, body=None, raw=False):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + "/api/v1" + path, data=data, method=method)
        req.add_header("Content-Type", "application/json")
        if self.token:
            req.add_header("Authorization", "Bearer " + self.token)
        for attempt in range(5):
            try:
                with urllib.request.urlopen(req, timeout=120) as r:
                    b = r.read()
                    return b if raw else (json.loads(b) if b else None)
            except urllib.error.HTTPError as e:
                raise RuntimeError(f"{method} {path} → {e.code} {e.read()[:500]!r}")
            except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
                if attempt == 4:
                    raise
                log("retrying", method, path, e)
                time.sleep(5)

    def workspace(self):
        if not self.ws:
            wss = self.call("GET", "/workspaces")
            self.ws = wss[0]["id"]
        return self.ws

    def production(self, pid):
        return self.call("GET", f"/design/productions/{pid}")

    def planner_turns(self, pid):
        ss = self.call("GET", "/sessions") or []
        ss = ss if isinstance(ss, list) else ss.get("sessions", [])
        return len([s for s in ss if (s.get("meta") or {}).get("production_id") == pid and s.get("provider") != REPLAY])


class Daemon(Api):
    """A copy of a debug ottod on its own port and temp data dir, env -i, the
    usage engine off from the first real boot (a bootstrap boot with an empty
    HOME turns it off). Its Otto Forge is \`forge_bin\`."""

    def __init__(self, ottod, forge_bin):
        super().__init__(f"http://127.0.0.1:{PORT}")
        base = "/private/tmp/claude-501" if Path("/private/tmp/claude-501").is_dir() else None
        self.root = Path(tempfile.mkdtemp(prefix="scene-suite.", dir=base))
        self.bin = self.root / "ottod"
        shutil.copy2(ottod, self.bin)
        for d in ("data", "tmp", "ws", "empty-home", "troots", "plugins", "scratches", "context", "ide-cache"):
            (self.root / d).mkdir()
        self.forge_bin = forge_bin
        self.proc = None

    def env(self, home):
        r = self.root
        return {
            "HOME": str(home), "USER": os.environ.get("USER", "user"), "LOGNAME": os.environ.get("USER", "user"),
            "SHELL": "/bin/zsh", "LANG": "en_US.UTF-8", "TMPDIR": str(r / "tmp"),
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "OTTO_DATA_DIR": str(r / "data"), "OTTO_PORT": str(PORT),
            "OTTO_SECRETS": "file", "OTTO_SECRETS_ALLOW_PLAINTEXT": "1", "OTTO_SELF_IMPROVE": "off",
            "OTTO_CLI_UPDATE": "0", "OTTO_TAILSCALE_CLI": str(r / "no-tailscale"),
            "OTTO_PLUGINS_HOME": str(r / "plugins"), "OTTO_SCRATCHES_HOME": str(r / "scratches"),
            "OTTO_CONTEXT_ROOT": str(r / "context"), "OTTO_IDE_BUILD_CACHE_DIR": str(r / "ide-cache"),
            **({"OTTO_TRANSCRIPT_ROOTS": str(r / "troots")} if home != HOME else {}),
        }

    def start(self, home):
        logf = open(self.root / "ottod.log", "ab")
        self.proc = subprocess.Popen([str(self.bin)], env=self.env(home), stdout=logf, stderr=subprocess.STDOUT,
                                     cwd=str(self.root), start_new_session=True)
        for _ in range(240):
            try:
                urllib.request.urlopen(self.base + "/api/v1/meta", timeout=2).read()
                return
            except Exception:
                if self.proc.poll() is not None:
                    raise SystemExit(f"ottod exited at boot, see {self.root / 'ottod.log'}")
                time.sleep(0.5)
        raise SystemExit("ottod did not answer in 120 s")

    def stop(self):
        if not self.proc or self.proc.poll() is not None:
            return
        self.proc.send_signal(signal.SIGTERM)
        try:
            self.proc.wait(60)
        except subprocess.TimeoutExpired:
            os.killpg(self.proc.pid, signal.SIGKILL)
            self.proc.wait(10)

    def boot(self):
        self.start(self.root / "empty-home")
        self.token = self.call("POST", "/onboarding/root", {"password": PASSWORD, "display_name": "Suite"})["token"]
        self.call("PUT", "/usage/config", {"enabled": False})
        self.stop()
        self.start(HOME)
        self.token = self.call("POST", "/auth/login", {"username": "root", "password": PASSWORD})["token"]
        st = self.call("GET", "/usage/status")
        if st.get("enabled"):
            raise SystemExit(f"usage engine is on: {st}")
        self.ws = self.call("POST", "/workspaces", {"name": "Scene suite", "root_path": str(self.root / "ws")})["id"]
        self.call("PATCH", "/design/3d/providers/otto-forge", {"cli_binary": self.forge_bin})
        # The replay agent: its "CLI" copies the plan the suite left for it.
        self.call("PUT", "/settings", {"providers": {REPLAY: {
            "cmd": "/bin/sh", "args": ["-c", "printf 'ready> '; cp ../next-plan.json plan.json; exec cat"]}}})
        log("daemon up", self.base, "data", self.root / "data")

    def replay(self, plan):
        (self.root / "data/productions").mkdir(parents=True, exist_ok=True)
        (self.root / "data/productions/next-plan.json").write_text(json.dumps(planned(plan)))

    def cleanup(self, keep=False):
        self.stop()
        if not keep:
            shutil.rmtree(self.root, ignore_errors=True)


# --------------------------------------------------------------- one scene
class Timeline:
    """Every change of the production's and its items' status, progress and
    message, timestamped: the per-item, per-step timings."""

    def __init__(self, path):
        self.path = path
        self.events = json.loads(path.read_text()) if path.exists() else []
        self.last = {}

    def see(self, p):
        now = round(time.time(), 1)
        rows = [("production", p["status"], None, p.get("message"))] + [
            (i["id"], i["status"], i.get("progress_pct"), i.get("message")) for i in p["plan"]["items"]]
        changed = False
        for key, st, pct, msg in rows:
            if self.last.get(key) != (st, pct, msg):
                self.last[key] = (st, pct, msg)
                self.events.append({"t": now, "item": key, "status": st, "pct": pct, "message": msg})
                changed = True
        if changed:
            save(self.path, self.events)

    def timings(self):
        """Per item: when it started and ended, its seconds, and each step
        (each distinct message while working) with its seconds."""
        out = {}
        for key in dict.fromkeys(e["item"] for e in self.events):
            ev = [e for e in self.events if e["item"] == key]
            work = [e for e in ev if e["status"] in ("working", "planning", "running")]
            if not work:
                continue
            t0 = work[0]["t"]
            end = next((e["t"] for e in ev if e["t"] >= t0 and e["status"] not in ("working", "planning", "running", "waiting", "ready")), None)
            steps, cur = [], None
            for e in ev:
                if e["t"] < t0 or (end and e["t"] > end):
                    continue
                label = e["message"] or e["status"]
                if cur is None or label != cur["step"]:
                    cur = {"step": label, "from": e["t"]}
                    steps.append(cur)
            for a, b in zip(steps, steps[1:] + [{"from": end}]):
                a["secs"] = round((b["from"] or a["from"]) - a["from"], 1)
            out[key] = {"start": t0, "end": end, "secs": round(end - t0, 1) if end else None,
                        "final": ev[-1]["status"], "steps": steps}
        return out


def poll(api, pid, until, limit_s, tl):
    t0 = time.time()
    while True:
        p = api.production(pid)
        tl.see(p)
        if p["status"] in until:
            return p
        if time.time() - t0 > limit_s:
            raise TimeoutError(f"production {pid} still {p['status']} after {limit_s} s")
        time.sleep(2)


def wait_no_make(api):
    """Never two makes at once: wait while any production of the workspace runs."""
    while True:
        rows = api.call("GET", f"/design/productions?workspace_id={api.workspace()}") or []
        busy = [r["id"] for r in rows if r["status"] == "running"]
        if not busy:
            return
        log("waiting: a production is making already", busy)
        time.sleep(30)


def fill_standins(plan, standins, sdir):
    """Stand-in models for every 3D item, by its maker prompt."""
    (sdir / "parts").mkdir(parents=True, exist_ok=True)
    made = {}
    for it in plan["items"]:
        if it["kind"] not in ("character", "object"):
            continue
        k = sha16(maker_prompt(it))
        if k not in made:
            parts, joints, shape = standin.standin(it)
            (standins / f"{k}.glb").write_bytes(standin.glb(parts, joints))
            made[k] = {"shape": shape, "parts": parts, "joints": joints}
        (sdir / "parts" / f"{it['id']}.parts.json").write_text(json.dumps(made[k]))


def produce(api, scene, sdir, *, provider=None, model=None, replay=None, standins=None, skip_audio=False,
            plan_limit=15 * 60, make_limit=6 * 3600, textured=True, extra=None, render=True):
    """One scene end to end, resumable: create → plan → start → make →
    collect (plan, results, models, timings, voices) → render → check."""
    res_path = sdir / "result.json"
    res = json.loads(res_path.read_text()) if res_path.exists() else {}
    if res.get("done"):
        log(scene["id"], "already finished, skipped")
        return res
    sdir.mkdir(parents=True, exist_ok=True)
    tl = Timeline(sdir / "timeline.json")
    pid_file = sdir / "production-id.txt"
    res.update({"scene": scene["id"], "covers": scene.get("covers", []), "description": scene["description"], **(extra or {})})
    if pid_file.exists():
        pid = pid_file.read_text().strip()
        log(scene["id"], "resuming production", pid)
    else:
        if replay is not None:
            api.replay(replay)
        body = {"workspace_id": api.workspace(), "description": scene["description"], "use": scene.get("use", "pc")}
        if replay is not None:
            body["provider"] = REPLAY
        elif provider:
            body["provider"] = provider
        if model and replay is None:
            body["model"] = model
        log(scene["id"], "planning", "(replayed plan)" if replay is not None else f"({provider or 'default agent'})")
        pid = api.call("POST", "/design/productions", body)["id"]
        pid_file.write_text(pid)
        res["planned_by"] = res.get("planned_by") or ("replay" if replay is not None else (provider or "default") + (f" / {model}" if model else ""))
    p = poll(api, pid, ("ready",) + TERMINAL, plan_limit, tl)
    if "planner_turns" not in res:
        res["planner_turns"] = api.planner_turns(pid)
    if not (sdir / "plan-planned.json").exists():
        save(sdir / "production-planned.json", p)
        save(sdir / "plan-planned.json", planned(p["plan"]))
    res.update(plan_status=p["status"] if p["status"] != "done" else "ready", plan_message=p.get("message"))
    if p["status"] in ("failed", "cancelled") and not any(i.get("result") for i in p["plan"]["items"]):
        log(scene["id"], "plan", p["status"], p.get("message"))
        res["done"] = True
        save(res_path, res)
        return res
    if p["status"] == "ready":
        if standins is not None:
            fill_standins(p["plan"], standins, sdir)
        quiet = [i["id"] for i in p["plan"]["items"]
                 if skip_audio and (i["kind"] in ("sound", "music") or (i.get("details") or {}).get("nonverbal"))]
        if quiet:
            api.call("PATCH", f"/design/productions/{pid}", {"items": [{"id": i, "enabled": False} for i in quiet]})
            res["left_out"] = quiet
        wait_no_make(api)
        log(scene["id"], "making")
        api.call("POST", f"/design/productions/{pid}/start")
    p = poll(api, pid, TERMINAL, make_limit, tl)
    collect(api, p, scene, sdir, standins is not None)
    res.update(make_status=p["status"], make_message=p.get("message"),
               items={i["id"]: [i["kind"], i["status"], i.get("message")] for i in p["plan"]["items"]},
               timings=tl.timings())
    save(res_path, res)
    if render:
        render_and_check(sdir, textured)
    else:
        check_only(sdir)
    res = json.loads(res_path.read_text())
    res["done"] = True
    save(res_path, res)
    log(scene["id"], "made", p["status"], p.get("message") or "")
    return res


def collect(api, p, scene, sdir, have_parts):
    save(sdir / "production-made.json", p)
    save(sdir / "plan.json", p["plan"])
    save(sdir / "expect.json", scene.get("expect", {}))
    if p.get("scene_artifact_id"):
        save(sdir / "scene.json", json.loads(api.call("GET", f"/design/artifacts/{p['scene_artifact_id']}/content", raw=True)))
    (sdir / "models").mkdir(parents=True, exist_ok=True)
    (sdir / "parts").mkdir(parents=True, exist_ok=True)
    voices = {}
    for it in p["plan"]["items"]:
        r = it.get("result") or {}
        if it["kind"] in ("character", "object") and r.get("artifact_id") and r.get("version_id"):
            b = api.call("GET", f"/design/artifacts/{r['artifact_id']}/versions/{r['version_id']}/content", raw=True)
            (sdir / "models" / f"{it['id']}.glb").write_bytes(b)
            if not (have_parts and (sdir / "parts" / f"{it['id']}.parts.json").exists()):
                (sdir / "parts" / f"{it['id']}.parts.json").write_text(json.dumps(glbparts.parts(b)))
        if r.get("clip_id"):
            try:
                c = api.call("GET", f"/product/tts/clips/{r['clip_id']}")
                voices[it["id"]] = {k: c.get(k) for k in ("kind", "provider_id", "voice_id", "voice", "voice_name", "character_name", "duration_ms", "title") if k in c}
            except RuntimeError as e:
                voices[it["id"]] = {"error": str(e)}
    save(sdir / "voices.json", voices)
    try:
        jobs = api.call("GET", f"/design/3d/jobs?workspace_id={api.workspace()}&limit=200") or []
        made = {(i.get("result") or {}).get("artifact_id") for i in p["plan"]["items"]}
        save(sdir / "jobs-3d.json", [j for j in jobs if j.get("artifact_id") in made or j.get("source_artifact_id") in made])
    except RuntimeError as e:
        save(sdir / "jobs-3d.json", {"error": str(e)})


def render_and_check(sdir, textured):
    render_many([sdir], textured, sdir / "render-job.json")
    check_only(sdir)


def render_many(sdirs, textured, job_file):
    """One Blender run (through the heavy lock) for many scenes."""
    jobs = []
    for sdir in sdirs:
        if (sdir / "scene.json").exists():
            plan = json.loads((sdir / "plan.json").read_text())
            jobs.append({"doc": str(sdir / "scene.json"), "out": str(sdir), "textured": textured,
                         "models": {m.stem: str(m) for m in (sdir / "models").glob("*.glb")},
                         "labels": {i["id"]: i["name"] for i in plan["items"]}})
    if not jobs:
        return
    save(job_file, jobs)
    log("rendering", len(jobs), "scene(s) in Blender (waits for a heavy slot)")
    r = subprocess.run([HEAVY, BLENDER, "-b", "--factory-startup", "--python", str(HERE / "render_blender.py"), "--",
                        str(job_file)], capture_output=True, text=True, cwd=str(REPO))
    Path(str(job_file) + ".log").write_text(r.stdout[-20000:] + r.stderr[-5000:])
    if r.returncode:
        log("render failed, see", str(job_file) + ".log")


def check_only(sdir):
    res_path = sdir / "result.json"
    res = json.loads(res_path.read_text()) if res_path.exists() else {"scene": sdir.name}
    if (sdir / "scene.json").exists():
        res["checks"] = checks.main(sdir)
    save(res_path, res)
    return res


NAMES = ["setting_and_details", "no_intersections", "plan_relations", "described_relations", "plan_places", "sizes",
         "supported", "ground_size", "framing", "visible", "facing", "notes_true", "plan_has_what_was_asked", "plan_voices"]


def summary(out, ids):
    lines = ["| scene | " + " | ".join(NAMES) + " | all |", "|---" * (len(NAMES) + 2) + "|"]
    passed = 0
    rows = [json.loads((out / s / "result.json").read_text()) for s in ids if (out / s / "result.json").exists()]
    for r in rows:
        c = r.get("checks")
        if not c:
            lines.append(f"| {r['scene']} | " + " | ".join(["—"] * len(NAMES)) + f" | no scene ({r.get('plan_status') or r.get('make_status')}) |")
            continue
        cells = []
        for n in NAMES:
            v = c.get(n, {}).get("pass")
            cell = "–" if v is None else ("pass" if v else "**FAIL**")
            if n == "setting_and_details" and c.get(n):
                cell += " " + c[n]["details"][0].split(" ")[1]
            cells.append(cell)
        ok = all(c.get(n, {}).get("pass") is not False for n in NAMES)
        passed += ok
        lines.append(f"| {r['scene']} | " + " | ".join(cells) + f" | {'PASS' if ok else 'FAIL'} |")
    lines.append(f"\n{passed} of {len(rows)} scenes pass every check.")
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    log(f"{passed} of {len(rows)} scenes pass every check; table in {out / 'summary.md'}")


def locked(out):
    """One runner per output folder (so never two makes from it at once)."""
    out.mkdir(parents=True, exist_ok=True)
    f = open(out / ".runner.lock", "w")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        raise SystemExit(f"another runner is working in {out}")
    return f


# ---------------------------------------------------------------- commands
def owner_plan():
    out = subprocess.run(["sqlite3", "-readonly", str(OWNER_DB),
                          f"select plan_json from media_productions where id='{OWNER_PRODUCTION}'"],
                         capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def ottod_path(a):
    p = Path(a.ottod or REPO / "target/debug/ottod")
    if not p.is_file():
        raise SystemExit(f"no ottod at {p}: build it with {HEAVY} cargo build -p ottod")
    return p


def scenes_of(a, corpus):
    ids = a.scenes.split(",") if a.scenes else list(corpus)
    for s in ids:
        if s not in corpus:
            raise SystemExit(f"unknown scene {s}; the scenario file has {', '.join(corpus)}")
    return ids


def cmd_run(a):
    """Level A: an isolated daemon of this checkout; the real planner (or a
    replayed plan); stand-in models through the stand-in Forge."""
    corpus = load_corpus(a.scenarios)
    out = Path(a.out) if a.out else OUT / a.run
    lock = locked(out)  # noqa: F841
    fb = Path(tempfile.mkdtemp(prefix="scene-suite-forge."))
    shutil.copy2(HERE / "fake-forge.sh", fb / "forge")
    (fb / "standins").mkdir()
    d = Daemon(ottod_path(a), str(fb / "forge"))
    turns = 0
    try:
        d.boot()
        for sid in scenes_of(a, corpus):
            scene, sdir = corpus[sid], out / sid
            if json.loads((sdir / "result.json").read_text()).get("done") if (sdir / "result.json").exists() else False:
                log(sid, "already finished, skipped")
                continue
            # A production of an earlier daemon is gone with it: start this scene over.
            (sdir / "production-id.txt").unlink(missing_ok=True)
            replay = None
            if a.plans_from:
                replay = json.loads((OUT / a.plans_from / sid / "plan-planned.json").read_text())
            elif scene.get("plan_from") == "owner":
                replay = owner_plan()
            if replay is None and turns >= a.max_turns:
                log(sid, "skipped: the planner-turn cap is spent")
                continue
            res = produce(d, scene, sdir, provider=a.provider, model=a.model, replay=replay, standins=fb / "standins",
                          make_limit=30 * 60, textured=False, render=False,
                          extra={"planned_by": "replay of " + (a.plans_from or "the owner's real plan")} if replay is not None else None)
            turns += res.get("planner_turns", 0) if replay is None else 0
            log("planner turns so far", turns)
    finally:
        d.cleanup(keep=a.keep)
        shutil.rmtree(fb, ignore_errors=True)
    ids = [s for s in scenes_of(a, corpus) if (out / s).exists()]
    if not a.no_render:
        render_many([out / s for s in ids if not (out / s / "camera.png").exists()], False, out / "render-jobs.json")
    for s in ids:
        check_only(out / s)
    summary(out, ids)


def cmd_remote(a):
    """Any running daemon and its own engine: one scene at a time, resumable."""
    corpus = load_corpus(a.scenarios)
    out = Path(a.out)
    lock = locked(out)  # noqa: F841
    api = Api(a.base, a.token, a.workspace)
    for sid in scenes_of(a, corpus):
        try:
            produce(api, corpus[sid], out / sid, provider=a.provider, model=a.model, skip_audio=a.skip_audio,
                    textured=True, extra={"daemon": a.base})
        except (RuntimeError, TimeoutError) as e:
            log(sid, "ERROR", e)
            save(out / sid / "error.txt", str(e))
    summary(out, scenes_of(a, corpus))


def cmd_owner(a):
    """Level B: the owner's pier rebuilt from his real finished models (read-only
    copies out of the installed app's data): the stand-in Forge hands back his
    real outputs per prompt, his library pieces as library pieces, and his fox's
    real rig failure."""
    corpus = load_corpus(a.scenarios)
    scene = corpus["t01-pier"]
    out = Path(a.out) if a.out else OUT / a.run
    lock = locked(out)  # noqa: F841
    plan = owner_plan()
    fb = Path(tempfile.mkdtemp(prefix="scene-suite-forge."))
    shutil.copy2(HERE / "fake-forge.sh", fb / "forge")
    st = fb / "standins"
    st.mkdir()
    for it in plan["items"]:
        r = it.get("result") or {}
        if it["kind"] not in ("character", "object") or not r.get("version_id"):
            continue
        row = subprocess.run(["sqlite3", "-readonly", str(OWNER_DB),
                              f"select blob_sha256, provenance_json from design_versions where id='{r['version_id']}'"],
                             capture_output=True, text=True, check=True).stdout.strip()
        blob, prov = row.split("|", 1)
        b = (OWNER_BLOBS / blob).read_bytes()
        prov = json.loads(prov)
        if prov.get("source") == "library":
            for ask in library_asks(it):
                (st / f"lib-{sha16(ask)}.glb").write_bytes(b)
                (st / f"lib-{sha16(ask)}.json").write_text(json.dumps(prov["library"]))
        else:
            (st / f"{sha16(maker_prompt(it))}.glb").write_bytes(b)
        if it["status"] == "failed" and "skeleton" in (it.get("message") or ""):
            (st / f"rigfail-{sha16(b)}.txt").write_text(it["message"].split(": ", 1)[1])
    d = Daemon(ottod_path(a), str(fb / "forge"))
    try:
        d.boot()
        (out / scene["id"] / "production-id.txt").unlink(missing_ok=True)
        produce(d, scene, out / scene["id"], replay=plan, skip_audio=True, textured=True,
                extra={"planned_by": "the owner's real plan (codex, gpt-6.2-astra)", "models": "the owner's real finished models"})
    finally:
        d.cleanup(keep=a.keep)
        shutil.rmtree(fb, ignore_errors=True)


def cmd_check(a):
    corpus = load_corpus(a.scenarios)
    out = Path(a.out) if a.out else OUT / a.run
    for sid in scenes_of(a, corpus):
        sdir = out / sid
        if not sdir.exists():
            continue
        if corpus[sid].get("expect") is not None:
            save(sdir / "expect.json", corpus[sid]["expect"])
    ids = [s for s in scenes_of(a, corpus) if (out / s).exists()]
    if not a.no_render:
        render_many([out / s for s in ids], a.textured, out / "render-jobs.json")
    for s in ids:
        check_only(out / s)
    summary(out, ids)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["run", "remote", "owner", "check"])
    ap.add_argument("--run", help="the run's folder name under " + str(OUT))
    ap.add_argument("--out", help="the output folder (instead of --run)")
    ap.add_argument("--scenarios", help="the scenario file (default: corpus.json beside this script)")
    ap.add_argument("--scenes", help="comma-separated scenario ids (default: all of the file)")
    ap.add_argument("--plans-from", help="run: replay the plans of this earlier run (no planner turns)")
    ap.add_argument("--provider", default=None, help="the planning agent (run: default codex, as the owner's; remote: the daemon's default)")
    ap.add_argument("--model", help="the planning agent's model (default: its own default)")
    ap.add_argument("--max-turns", type=int, default=24, help="run: stop planning new scenes past this many planner turns")
    ap.add_argument("--ottod", help="run/owner: the debug ottod to copy (default <repo>/target/debug/ottod)")
    ap.add_argument("--keep", action="store_true", help="run/owner: keep the temp data dir")
    ap.add_argument("--base", help="remote: the daemon, e.g. http://127.0.0.1:7700")
    ap.add_argument("--token", help="remote: a bearer token of that daemon")
    ap.add_argument("--workspace", help="remote: the workspace id (default: the first one)")
    ap.add_argument("--skip-audio", action="store_true", help="remote: leave sound, music and nonverbal lines out")
    ap.add_argument("--textured", action="store_true", help="check: render with the models' textures")
    ap.add_argument("--no-render", action="store_true", help="run/check: no pictures (no Blender)")
    a = ap.parse_args()
    if a.cmd == "run" and a.provider is None:
        a.provider = "codex"
    if a.cmd != "remote" and not (a.run or a.out):
        ap.error("--run or --out is required")
    if a.cmd == "remote" and not (a.base and a.out):
        ap.error("remote needs --base, --token and --out")
    {"run": cmd_run, "remote": cmd_remote, "owner": cmd_owner, "check": cmd_check}[a.cmd](a)


if __name__ == "__main__":
    main()
