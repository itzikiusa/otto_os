#!/usr/bin/env python3
"""Spawn claude in a plain PTY (no Otto), watch how long it lives.
Usage: ptytest.py [clean|dirty] [seconds]"""
import os, pty, select, sys, time, signal

mode = sys.argv[1] if len(sys.argv) > 1 else "dirty"
secs = int(sys.argv[2]) if len(sys.argv) > 2 else 20

env = dict(os.environ)
if mode == "clean":
    for k in list(env):
        if k.startswith(("CLAUDE", "ANTHROPIC", "MCP", "OTEL")):
            env.pop(k)

pid, fd = pty.fork()
if pid == 0:
    os.environ.clear()
    os.environ.update(env)
    os.execvp("claude", ["claude"])

start = time.time()
out = b""
exited = None
while time.time() - start < secs:
    r, _, _ = select.select([fd], [], [], 0.5)
    if r:
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            chunk = b""
        if chunk:
            out += chunk
            # answer cursor-position queries so claude isn't blocked on them
            if b"\x1b[6n" in chunk:
                os.write(fd, b"\x1b[10;1R")
        else:
            exited = "eof"
            break
    w = os.waitpid(pid, os.WNOHANG)
    if w[0] == pid:
        exited = f"waitpid status={w[1]} (signal={w[1] & 0x7f}, code={w[1] >> 8})"
        break

alive = exited is None
print(f"mode={mode} alive_after={time.time()-start:.1f}s alive={alive} exited={exited}")
tail = out[-600:].decode("utf-8", "replace")
print("--- output tail ---")
print(tail)
if alive:
    os.kill(pid, signal.SIGKILL)
    os.waitpid(pid, 0)
