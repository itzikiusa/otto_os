"""Compare one encoded frame per chapter against the current Remotion QA still.
Run after npm run stills and npm run render-all. SSIM tolerates H.264 loss, while
catching stale illustrations, broken frames, and stale render/source combinations.
"""
import json,re,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[4]
OUT=ROOT/'marketing/videos/out-v2'
results=[]
for ep in json.loads((ROOT/'ui/src/lib/walkthroughs/catalog.json').read_text()):
 for ch in ep['chapters']:
  frame=round((ch['start']+ch['stepStarts'][1]+1.1)*30)
  proc=subprocess.run(['ffmpeg','-hide_banner','-ss',str(frame/30),'-i',str(OUT/ep['file']),'-i',str(OUT/'qa'/f"{ep['id']}-{ch['id']}.png"),'-lavfi','ssim','-frames:v','1','-an','-f','null','-'],capture_output=True,text=True)
  if proc.returncode:raise RuntimeError(proc.stderr)
  match=re.search(r'All:([0-9.]+)',proc.stderr)
  if not match:raise RuntimeError('No SSIM result for '+ch['id'])
  score=float(match[1]);assert score>.985,f"Stale or damaged frame: {ep['id']}/{ch['id']} SSIM={score}"
  results.append(dict(episode=ep['id'],chapter=ch['id'],frame=frame,ssim=score))
 print(ep['id'],'encoded frames match current source',flush=True)
(OUT/'qa/frame-verification.json').write_text(json.dumps(results,indent=2)+'\n')
print('Verified',len(results),'chapter frames; minimum SSIM',min(r['ssim'] for r in results))
