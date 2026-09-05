"""Record each instruction separately; derive edit timing from the actual performance.
Requires edge-tts in the interpreter environment. Only the fictional narration is sent.
Existing recordings are content-addressed and reused; no credentials are read.
"""
import asyncio, hashlib, json, subprocess
from pathlib import Path
import edge_tts
ROOT=Path(__file__).resolve().parents[4]
CAT=ROOT/'ui/src/lib/walkthroughs/catalog.json'
OUT=ROOT/'marketing/videos/public/studio-v2/audio/voice'
VOICE='en-US-AndrewNeural'
async def main():
    OUT.mkdir(parents=True,exist_ok=True)
    catalog=json.loads(CAT.read_text())
    sem=asyncio.Semaphore(3)
    async def record(text):
        key=hashlib.sha256((VOICE+'|-4%|'+text).encode()).hexdigest()[:20]
        dest=OUT/(key+'.mp3')
        async with sem:
            if not dest.exists():
                for attempt in range(3):
                    try:
                        tmp=dest.with_suffix('.tmp.mp3')
                        await edge_tts.Communicate(text,VOICE,rate='-4%').save(str(tmp))
                        tmp.replace(dest)
                        break
                    except Exception:
                        if attempt==2: raise
                        await asyncio.sleep(2)
            duration=float(subprocess.check_output(['ffprobe','-v','error','-show_entries','format=duration','-of','csv=p=0',str(dest)],text=True))
            return dict(file='studio-v2/audio/voice/'+dest.name,duration=duration)
    for ep in catalog:
        for ch in ep['chapters']:
            ch['voice']=await asyncio.gather(*(record(t) for t in ch['steps']))
            starts=[]; cursor=1.2
            for v in ch['voice']:
                starts.append(round(cursor,3)); cursor+=max(5.2,v['duration']+0.8)
            ch['stepStarts']=starts
            ch['duration']=round((cursor+0.8)*30+0.999)//30+1
            print(ep['id'],ch['id'],ch['duration'],'seconds',flush=True)
        cursor=ep['introSeconds']
        for ch in ep['chapters']:
            ch['start']=cursor;cursor+=ch['duration']
        ep['duration']=cursor+ep['outroSeconds']
        # Persist completed episodes, allowing a failed remote generation to resume.
        CAT.write_text(json.dumps(catalog,indent=2,ensure_ascii=False)+'\n')
    print('Narration ready:',sum(e['duration'] for e in catalog),'seconds',flush=True)
asyncio.run(main())
