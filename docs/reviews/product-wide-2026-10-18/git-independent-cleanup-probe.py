from pathlib import Path
exec(Path(__file__).with_name('git-independent-lock-probe.py').read_text().split('results={}')[0])
results={}
for mode in ['normal','packed','same-oid-arrival','different-oid-arrival']:
 p=init(mode)
 other=stack(p)[1].split('\t')[0]
 git(p,'stash','drop','stash@{2}');git(p,'stash','drop','stash@{1}')
 old=git(p,'rev-parse','refs/stash').stdout.strip()
 git(p,'update-ref','refs/otto/recovery/test/0',old)
 if mode=='packed':
  git(p,'branch','must-survive');git(p,'pack-refs','--all')
 # This reproduces native set_reflog(empty) without rewriting ref storage.
 git(p,'reflog','delete','--rewrite','stash@{0}')
 if mode=='same-oid-arrival':
  git(p,'stash','store','-m','intervening object',other)
  git(p,'stash','store','-m','new same object',old)
 if mode=='different-oid-arrival':
  (p/'file').write_text('new external work\n');git(p,'stash','push','-qm','new different object')
 proc=sp.Popen(['git','-C',str(p),'update-ref','--stdin'],stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,text=True)
 proc.stdin.write('start\ndelete refs/stash '+old+'\nprepare\n');proc.stdin.flush()
 started=proc.stdout.readline().strip();prepared=proc.stdout.readline().strip()
 live=git(p,'reflog','show','--format=%H','refs/stash',check=False)
 if prepared=='prepare: ok':
  decision='commit' if live.returncode==0 and not live.stdout.strip() else 'abort'
  proc.stdin.write(decision+'\n');proc.stdin.flush();stdout,stderr=proc.communicate(timeout=5)
 else:
  decision='prepare-refused';stdout,stderr=proc.communicate(timeout=5)
 results[mode]={'decision':decision,'stack':stack(p),'stash_ref_exists':git(p,'show-ref','--verify','refs/stash',check=False).returncode==0,'recovery_object':git(p,'cat-file','-t',old).stdout.strip(),'unrelated_branch_exists':git(p,'show-ref','--verify','refs/heads/must-survive',check=False).returncode==0 if mode=='packed' else None}
print(json.dumps({'root':str(root),'results':results},indent=2))
