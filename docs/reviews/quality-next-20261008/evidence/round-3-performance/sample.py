#!/usr/bin/env python3
"""Sample only a named test process and its descendants, retaining raw observations."""
import json, os, subprocess, sys, time
from pathlib import Path
out = Path(__file__).parent
log = (out / 'chromium-run.log').open('w')
command = [sys.argv[1], '--ignored', '--exact', 'live::session::http_chromium_tests::real_chromium_positive_http_bursts_preserve_bodies_input_and_frames', '--nocapture']
start = time.monotonic()
proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=os.environ)
samples = []
owned = {proc.pid}
while proc.poll() is None:
    rows = subprocess.check_output(['/bin/ps', '-axo', 'pid=,ppid=,rss=,%cpu=,command='], text=True).splitlines()
    parsed = []
    for row in rows:
        parts = row.split(None, 4)
        if len(parts) == 5:
            parsed.append(dict(pid=int(parts[0]), ppid=int(parts[1]), rss_kib=int(parts[2]), cpu=float(parts[3]), command=parts[4]))
    changed = True
    while changed:
        before = len(owned)
        owned.update(p['pid'] for p in parsed if p['ppid'] in owned)
        changed = before != len(owned)
    selected = [p for p in parsed if p['pid'] in owned]
    samples.append(dict(seconds=time.monotonic()-start, processes=selected))
    if time.monotonic()-start > 120:
        proc.kill()
        break
    time.sleep(0.5)
code = proc.wait()
log.close()
time.sleep(1)
alive = []
for pid in owned:
    try:
        os.kill(pid, 0)
        alive.append(pid)
    except ProcessLookupError:
        pass
(out/'resources.json').write_text(json.dumps(dict(command=command, test_pid=proc.pid, exit_code=code, elapsed_seconds=time.monotonic()-start, samples=samples, owned_pids=sorted(owned), remaining_pids=alive), indent=2)+'\n')
print(json.dumps(dict(exit_code=code, samples=len(samples), remaining_pids=alive)))
sys.exit(code)
