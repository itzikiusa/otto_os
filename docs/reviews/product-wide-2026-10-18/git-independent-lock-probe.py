import subprocess as sp,pathlib,tempfile,json,os
root=pathlib.Path(tempfile.mkdtemp(prefix='otto-stash-fixtures-'))
def git(p,*args,check=True):
 r=sp.run(['git','-C',str(p),*args],text=True,capture_output=True,env={**os.environ,'GIT_CONFIG_NOSYSTEM':'1','GIT_CONFIG_GLOBAL':'/dev/null'},timeout=10)
 if check and r.returncode:raise RuntimeError((args,r.stderr))
 return r

def init(name,flags=()):
 p=root/name;p.mkdir();git(p,'init','-q',*flags);git(p,'config','user.email','fixture@example.invalid');git(p,'config','user.name','Fixture')
 (p/'file').write_text('base\n');git(p,'add','file');git(p,'commit','-qm','base')
 for v in ['oldest','middle','newest']:
  (p/'file').write_text(v+'\n');git(p,'stash','push','-qm',v)
 return p

def stack(p):return git(p,'stash','list','--format=%H%x09%gs').stdout.splitlines()
results={}
p=init('locks');wt=root/'linked';git(p,'worktree','add','-qb','linked',str(wt))
before=stack(p);tip=git(p,'rev-parse','refs/stash').stdout.strip()
lock=sp.Popen(['git','-C',str(p),'update-ref','--stdin'],stdin=sp.PIPE,stdout=sp.PIPE,stderr=sp.PIPE,text=True)
lock.stdin.write('start\nverify refs/stash '+tip+'\nprepare\n');lock.stdin.flush()
assert lock.stdout.readline().strip()=='start: ok';assert lock.stdout.readline().strip()=='prepare: ok'
for where in [p,wt]:
 for args in [('stash','drop','stash@{0}'),('stash','push','-qm','while-locked')]:
  if args[1]=='push':(where/'file').write_text('external work\n')
  r=git(where,*args,check=False)
  results[str(where.name)+' '+args[1]]={'code':r.returncode,'mentions_lock':'refs/stash.lock' in r.stderr,'stack_unchanged':stack(p)==before,'working_file':(where/'file').read_text().strip()}
lock.stdin.write('abort\n');lock.stdin.flush();lock.communicate(timeout=5)
p=init('partial');before=stack(p);oldtip=git(p,'rev-parse','refs/stash').stdout.strip();git(p,'reflog','delete','--rewrite','stash@{0}')
results['reflog_written_target_old']={'stack':stack(p),'expected_remaining':before[1:],'ref_still_old':git(p,'rev-parse','refs/stash').stdout.strip()==oldtip,'default_apply_oid':git(p,'rev-parse','stash').stdout.strip(),'selector0_oid':git(p,'rev-parse','stash@{0}').stdout.strip()}
for name,flags in [('sha256',('--object-format=sha256',)),('reftable',('--ref-format=reftable',))]:
 try:
  q=init(name,flags);results[name]={'oid_length':len(git(q,'rev-parse','refs/stash').stdout.strip()),'stash_count':len(stack(q)),'ref_format':git(q,'rev-parse','--show-ref-format').stdout.strip()}
 except Exception as e:results[name]={'error':str(e)}
print(json.dumps({'root':str(root),'results':results},indent=2))
