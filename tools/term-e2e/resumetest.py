#!/usr/bin/env python3
"""Does `claude --continue` re-render old content at the CURRENT width, or
replay it at the width it was originally printed?

Phase 1: run claude in a 60-col PTY in an isolated cwd, get a response, kill.
Phase 2: `claude --continue` in a 150-col PTY, capture the replay, and check
the wrap width of the replayed exchange.
"""
import os, pty, select, shutil, signal, struct, sys, tempfile, termios, time, fcntl

CWD = tempfile.mkdtemp(prefix="otto-resume-test-")

def run_claude(args, cols, rows, drive, timeout):
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(CWD)
        os.environ["TERM"] = "xterm-256color"
        os.execvp("claude", ["claude", *args])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    out = b""
    start = time.time()
    state = {"step": 0}
    while time.time() - start < timeout:
        r, _, _ = select.select([fd], [], [], 0.5)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            if b"\x1b[6n" in chunk:
                os.write(fd, b"\x1b[20;1R")
        drive(fd, out, state, time.time() - start)
        if state.get("done"):
            break
    try:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    except OSError:
        pass
    return out

def phase1_drive(fd, out, state, elapsed):
    txt = out.decode("utf-8", "replace").lower()
    if state["step"] == 0 and ("trust" in txt or "allow external imports" in txt) and elapsed > 3:
        os.write(fd, b"\r")
        state["step"] = 1
        state["t"] = elapsed
    elif state["step"] in (0, 1) and elapsed > 12:
        os.write(fd, b"Reply with exactly RESUME_WIDTH_PROBE_1234567890_ABCDEFGHIJKLMNOPQRSTUVWXYZ_END and nothing else.")
        state["step"] = 2
        state["t"] = elapsed
    elif state["step"] == 2 and elapsed - state["t"] > 1.5:
        os.write(fd, b"\r")
        state["step"] = 3
    elif state["step"] == 3:
        # The echo already contains the marker once; wait for the RESPONSE
        # (a second occurrence) then linger so the session file is written.
        flat = txt.replace("\n", "").replace("\r", "")
        if flat.count("resume_width_probe") >= 2:
            state.setdefault("seen", elapsed)
            if elapsed - state["seen"] > 12:
                state["done"] = True

def phase2_drive(fd, out, state, elapsed):
    txt = out.decode("utf-8", "replace")
    if "RESUME_WIDTH_PROBE" in txt:
        state.setdefault("seen", elapsed)
        if elapsed - state["seen"] > 6:
            state["done"] = True
    if elapsed > 60:
        state["done"] = True

print(f"cwd: {CWD}")
print("--- phase 1: claude @60 cols ---")
out1 = run_claude([], 60, 40, phase1_drive, 120)
got = "RESUME_WIDTH_PROBE" in out1.decode("utf-8", "replace")
print(f"phase1 bytes={len(out1)} marker_seen={got}")
if not got:
    tail = out1[-800:].decode("utf-8", "replace")
    print(tail)
    sys.exit(1)

slug = CWD.replace("/", "-")
proj = os.path.expanduser("~/.claude/projects")
hits = [d for d in os.listdir(proj) if CWD.split("/")[-1] in d] if os.path.isdir(proj) else []
print(f"session dirs for this cwd: {hits}")

time.sleep(2)
print("--- phase 2: claude --continue @150 cols ---")
out2 = run_claude(["--continue"], 150, 40, phase2_drive, 90)
txt2 = out2.decode("utf-8", "replace")
print(f"phase2 bytes={len(out2)}")
if len(out2) < 2000:
    print("--- raw phase2 output ---")
    print(txt2[-1500:])

# The marker is 62 chars; at 60 cols it MUST have been split in phase 1.
# If the replay re-renders at current width (150), the marker appears intact
# on one line; if it replays stored rendering, it appears split.
flat = txt2.replace("\r", "")
intact = "RESUME_WIDTH_PROBE_1234567890_ABCDEFGHIJKLMNOPQRSTUVWXYZ_END" in "".join(
    l for l in flat.split("\n")
)
present = "RESUME_WIDTH_PROBE" in flat
print(f"replay contains probe: {present}")
print(f"probe INTACT on a single replayed line (⇒ re-rendered at current width): {intact}")
for line in flat.split("\n"):
    if "RESUME_WIDTH_PROBE" in line or "_END" in line:
        print(f"  |{line.strip()[:140]}")
shutil.rmtree(CWD, ignore_errors=True)
