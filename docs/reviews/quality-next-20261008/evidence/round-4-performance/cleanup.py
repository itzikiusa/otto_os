"""Read-only teardown audit for the isolated fixture's sampled owned processes."""
import json, pathlib, subprocess
root=pathlib.Path(__file__).resolve().parents[5]
out=pathlib.Path(__file__).parent
raw=json.loads((out/'rendered-terminal-load.json').read_text())
meta=json.loads((root/'ui/e2e/.auth-qualityr4perf/daemon.json').read_text())
pids=sorted({pid for sample in raw['samples'] if sample['process']!='load-driver' for pid in sample['pids']})
r=subprocess.run(['ps','-p',','.join(map(str,pids)),'-o','pid=,lstart=,comm='],capture_output=True,text=True)
listeners={}
for port in [7896,5296]:
    q=subprocess.run(['lsof','-nP',f'-iTCP:{port}','-sTCP:LISTEN'],capture_output=True,text=True)
    listeners[str(port)]=q.stdout.strip()
result={'fixture':meta,'sampled_owned_pids':pids,'remaining_sampled_processes':r.stdout.strip(),'fixture_data_exists':pathlib.Path(meta['dataDir']).exists(),'listeners':listeners,'note':'Read-only check after fixture teardown; no process was killed by this audit. Raw process sampling excludes user applications and distinguishes WebKit by owned resource coalition.'}
(out/'cleanup.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
if result['remaining_sampled_processes'] or result['fixture_data_exists'] or any(listeners.values()):
    raise SystemExit('Owned fixture cleanup needs inspection')
