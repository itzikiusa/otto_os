"""Original stereo score, Foley and narration mix. Deterministic; no stock audio.
Requires numpy and ffmpeg. Rendered mixes are generated assets (48kHz PCM).
"""
import json, math, subprocess, tempfile, wave
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[4]
PUBLIC=ROOT/'marketing/videos/public'
CAT=ROOT/'ui/src/lib/walkthroughs/catalog.json'
SR=48000

def hz(midi):return 440*2**((midi-69)/12)
def add(track,signal,at,gain=1,pan=0):
    start=round(at*SR);length=min(len(signal),len(track)-start)
    if length<=0:return
    track[start:start+length,0]+=signal[:length]*gain*math.sqrt((1-pan)/2)
    track[start:start+length,1]+=signal[:length]*gain*math.sqrt((1+pan)/2)
def tone(note,seconds,decay=3):
    t=np.arange(int(seconds*SR))/SR;freq=hz(note)
    env=(1-np.exp(-t*70))*np.exp(-t*decay)
    return (np.sin(2*np.pi*freq*t)+.22*np.sin(2*np.pi*freq*2*t)+.07*np.sin(2*np.pi*freq*3*t))*env

def main():
 for ep in json.loads(CAT.read_text()):
    duration=ep['duration'];n=round(duration*SR);music=np.zeros((n,2),np.float32);mix=np.zeros_like(music)
    rng=np.random.default_rng(1809+ep['number']);beat=60/96
    chords=[[50,57,60,64],[53,60,64,67],[48,55,62,64],[55,62,65,69]]
    # Slow harmonic field: detuned sine partials with overlapping one-second fades.
    for bar in range(math.ceil(duration/(beat*16))):
        chord=chords[(bar+ep['number'])%4];at=bar*beat*16;length=min(beat*16+1.2,duration-at)
        t=np.arange(round(length*SR))/SR;env=np.minimum(1,t/1.5)*np.minimum(1,(length-t)/1.5)
        for i,note in enumerate(chord):
            freq=hz(note);pad=(np.sin(2*np.pi*freq*t)+.35*np.sin(2*np.pi*freq*1.002*t)+.1*np.sin(2*np.pi*freq*2*t))*env
            add(music,pad,at,.017,(i-1.5)/2.5)
    motif=[0,2,1,3,2,1,0,2]
    for k in range(math.ceil(duration/beat)):
        at=k*beat;chord=chords[(k//16+ep['number'])%4]
        if k%2==0:
            bell=tone(chord[motif[(k//2)%8]]+12,2.3,2.6)
            add(music,bell,at,.038,(-.45 if k%4==0 else .45))
            add(music,bell,at+beat*.75,.010,.55)
        if k%4==0:
            add(music,tone(chord[0]-12,2.3,1.6),at,.07)
            t=np.arange(int(.3*SR))/SR
            kick=np.sin(2*np.pi*(48*t+5*(1-np.exp(-t*30))))*np.exp(-t*18)
            add(music,kick,at,.045)
        if k%2==1:
            noise=rng.normal(0,1,int(.09*SR));noise=np.diff(noise,prepend=0)*np.exp(-np.arange(len(noise))/SR*60)
            add(music,noise,at,.005,.3)
    # Gentle stereo air transition and short, low-level interaction cues.
    for ch in ep['chapters']:
        at=ch['start']-.45;t=np.arange(int(.9*SR))/SR
        noise=rng.normal(0,1,len(t));smooth=np.convolve(noise,np.ones(18)/18,mode='same')
        air=smooth*np.sin(np.pi*t/.9)**2
        add(music,air,at,.045,-.3);add(music,air,at+.025,.04,.3)
        for i,start in enumerate(ch['stepStarts']):
            cue=tone([79,81,86][i],.35,16)
            add(music,cue,ch['start']+start+.35,.032,[-.2,0,.2][i])
        for start,voice in zip(ch['stepStarts'],ch['voice']):
            raw=subprocess.check_output(['ffmpeg','-v','error','-i',str(PUBLIC/voice['file']),'-af','loudnorm=I=-19:TP=-3:LRA=7','-f','f32le','-ac','1','-ar',str(SR),'-'])
            narration=np.frombuffer(raw,dtype=np.float32)
            at=ch['start']+start;add(mix,narration,at,1.25)
            a=max(0,round((at-.2)*SR));b=min(n,round((at+len(narration)/SR+.3)*SR))
            env=np.ones(b-a,np.float32)*.36;fade=min(int(.2*SR),(b-a)//2)
            env[:fade]=np.linspace(1,.36,fade);env[-fade:]=np.linspace(.36,1,fade)
            music[a:b]*=env[:,None]
    fade=np.minimum(1,np.arange(n)/SR/2)*np.minimum(1,(n-np.arange(n))/SR/2.5)
    mix+=music;mix*=fade[:,None]
    peak=float(np.max(np.abs(mix)))
    if peak>.95:mix*=.95/peak
    dest=PUBLIC/'studio-v2/audio'/f"{ep['id']}.wav"
    with tempfile.TemporaryDirectory(prefix='otto-score-') as tmp:
        raw=Path(tmp)/'mix.wav'
        with wave.open(str(raw),'wb') as w:w.setnchannels(2);w.setsampwidth(2);w.setframerate(SR);w.writeframes((mix*32767).astype('<i2').tobytes())
        subprocess.run(['ffmpeg','-v','error','-y','-i',str(raw),'-af','loudnorm=I=-16:TP=-1.5:LRA=9','-ar',str(SR),'-c:a','pcm_s16le',str(dest)],check=True)
    print(ep['id'],duration,'seconds, mix peak before mastering',round(peak,4),flush=True)
if __name__=='__main__':main()
