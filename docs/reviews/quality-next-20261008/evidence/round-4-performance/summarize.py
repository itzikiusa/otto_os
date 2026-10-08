"""Summarize the unchanged rendered-terminal fixture's raw JSON, nearest-rank p95."""
import json, math, pathlib, statistics
root = pathlib.Path(__file__).parent
raw = json.loads((root / 'rendered-terminal-load.json').read_text())
def dist(values):
    values = sorted(values)
    return {'n': len(values), 'min': min(values), 'mean': statistics.mean(values),
            'sd': statistics.pstdev(values), 'p50': values[math.ceil(len(values)*.5)-1],
            'p95': values[math.ceil(len(values)*.95)-1], 'max': max(values)} if values else None
out = {'engine': raw['engine'], 'protocol': raw['protocol'], 'fatal': raw['fatal'], 'phases': []}
for phase in raw['phases']:
    n = phase['concurrency']
    row = {'concurrency': n, 'load_seconds': phase['load_seconds'], 'recovery_seconds': phase['recovery_seconds'],
           'elapsed_seconds': phase['elapsed_seconds'], 'markers_sent': sum(x['sent'] for x in phase['rows']),
           'markers_rendered': sum(x['rendered'] for x in phase['rows']),
           'missed_ticks': sum(x['missed'] for x in phase['rows']),
           'input_bytes': sum(x['bytes'] for x in phase['rows']),
           'renderers': sorted(set(x['renderer'] for x in phase['rows'])),
           'latency_ms': dist([v for x in phase['rows'] for v in x['latencies']]),
           'per_terminal_latency_ms': [x['latency_ms'] for x in phase['rows']],
           'frame_ms': phase['frame_ms'], 'remaining': phase['remaining'], 'protocol': phase['protocol'],
           'peak_pending_bytes': max(x['peakPending'] for x in phase['rows']),
           'peak_queued_bytes': max(x['peakQueued'] for x in phase['rows']), 'resources': []}
    for phase_name in ['rendered-load', 'quiet-recovery']:
        samples = [x for x in raw['samples'] if x['concurrency']==n and x['phase']==phase_name]
        for proc in sorted(set(x['process'] for x in samples)):
            s = [x for x in samples if x['process']==proc]
            row['resources'].append({'phase': phase_name, 'process': proc,
                'cpu_percent': dist([x['cpu_percent'] for x in s]), 'rss_mib': dist([x['rss_mb'] for x in s]),
                'rss_first_mib': s[0]['rss_mb'], 'rss_last_mib': s[-1]['rss_mb']})
    out['phases'].append(row)
(root / 'summary.json').write_text(json.dumps(out, indent=2)+'\n')
print(json.dumps({**out, 'phases': [{k:v for k,v in p.items() if k!='resources'} for p in out['phases']]},indent=2))
