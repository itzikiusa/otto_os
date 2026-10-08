"""Read only the isolated fixture's session count, without printing credentials."""
import json, pathlib, urllib.request
root = pathlib.Path(__file__).resolve().parents[5]
auth = root / 'ui/e2e/.auth-qualityr4perf'
meta = json.loads((auth / 'daemon.json').read_text())
state = json.loads((auth / 'state.json').read_text())
values = {x['name']: x['value'] for x in state['origins'][0]['localStorage']}
req = urllib.request.Request(f"http://127.0.0.1:{meta['port']}/api/v1/sessions", headers={'Authorization': 'Bearer ' + values['otto_token']})
with urllib.request.urlopen(req, timeout=3) as response:
    sessions = json.load(response)
print(json.dumps({'fixture_pid': meta['pid'], 'live_owned_terminals': sum(s.get('live', False) for s in sessions), 'session_count': len(sessions)}))
