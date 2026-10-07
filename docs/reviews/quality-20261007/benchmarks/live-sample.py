#!/usr/bin/env python3
"""Read-only installed Otto CPU/RSS snapshot. No API calls, arguments or secrets."""
import argparse,ctypes,json,os,pathlib,re,statistics,subprocess,tempfile,time

def snapshot():
    raw=subprocess.check_output(['ps','-axo','pid=,ppid=,lstart=,%cpu=,rss=,comm='],text=True,env={**os.environ,'LC_ALL':'C'})
    rows=[]
    for line in raw.splitlines():
        m=re.match(r'\s*(\d+)\s+(\d+)\s+(\w{3}\s+\w{3}\s+\d+\s+\d\d:\d\d:\d\d\s+\d{4})\s+([\d.]+)\s+(\d+)\s+(.+)$',line)
        if m:
            pid,parent,start,cpu,rss,exe=m.groups()
            rows.append({'pid':int(pid),'parent':int(parent),'start':start,'cpu_percent':float(cpu),'rss_mb':int(rss)/1024,'exe':exe})
    return rows

def coalition(pid):
    # Same macOS proc_pidinfo layout/ownership criterion as resource.rs.
    lib=ctypes.CDLL('/usr/lib/libproc.dylib')
    buffer=(ctypes.c_uint64*5)()
    lib.proc_pidinfo.argtypes=[ctypes.c_int,ctypes.c_int,ctypes.c_uint64,ctypes.c_void_p,ctypes.c_int]
    size=lib.proc_pidinfo(pid,20,0,buffer,ctypes.sizeof(buffer))
    return buffer[0] if size==ctypes.sizeof(buffer) and buffer[0] else None

def summarize(rows):
    """One observation per label/timestamp, summing all its live processes.

    A WebContent label may have several owned PIDs. Comparing those PIDs as
    successive time samples manufactures RSS drops and divides aggregate CPU.
    Keep identities/raw rows, and report population changes alongside totals.
    """
    buckets={}
    for row in rows:
        key=(row['process'],row['at'])
        group=buckets.setdefault(key, {'process':row['process'],'at':row['at'],
            'cpu_percent':0.0,'rss_mb':0.0,'instances':[]})
        group['cpu_percent']+=row['cpu_percent']
        group['rss_mb']+=row['rss_mb']
        group['instances'].append({'pid':row['pid'],'start':row['start']})
    series=sorted(buckets.values(),key=lambda row:(row['process'],row['at']))
    summary=[]
    for label in sorted({r['process'] for r in series}):
        values=[r for r in series if r['process']==label]
        summary.append({'process':label,'samples':len(values),
            'process_instances':len({(p['pid'],p['start']) for r in values for p in r['instances']}),
            'min_simultaneous_processes':min(len(r['instances']) for r in values),
            'max_simultaneous_processes':max(len(r['instances']) for r in values),
            'observed_seconds':values[-1]['at']-values[0]['at'],
            'mean_cpu':statistics.mean(r['cpu_percent'] for r in values),
            'max_cpu':max(r['cpu_percent'] for r in values),
            'min_rss_mb':min(r['rss_mb'] for r in values),
            'max_rss_mb':max(r['rss_mb'] for r in values),
            'first_rss_mb':values[0]['rss_mb'],'last_rss_mb':values[-1]['rss_mb']})
    return series,summary

def write_report(report, target):
    report['series'],report['summary']=summarize(report['samples'])
    report['summary_method']='Sum CPU/RSS per process label and timestamp, then summarize those timestamp totals; raw process rows retained unchanged.'
    # Require a new path so regenerated evidence cannot erase its original.
    with target.open('x') as output: output.write(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'artifact':str(target),'summary':report['summary']},indent=2))

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--seconds',type=int,default=60)
    p.add_argument('--summarize',type=pathlib.Path,help='Recompute an existing report without sampling or process inspection')
    p.add_argument('--output',type=pathlib.Path,help='New artifact path, required with --summarize')
    args=p.parse_args()
    if args.summarize:
        if not args.output: p.error('--summarize requires --output')
        report=json.loads(args.summarize.read_text())
        report['derived_from']=str(args.summarize)
        write_report(report,args.output)
        return
    if not 10<=args.seconds<=120: raise SystemExit('Choose 10–120 seconds')
    first=snapshot()
    # The listener identifies the main installed daemon; holders/MCP children
    # can share its executable and are explicitly not matched by name alone.
    listeners=subprocess.check_output(['lsof','-nP','-iTCP:7700','-sTCP:LISTEN','-t'],text=True).split()
    daemons=[r for r in first if str(r['pid']) in listeners and (r['exe'].endswith('/Otto/bin/ottod') or r['exe'].endswith('/Otto.app/Contents/MacOS/ottod'))]
    if len(daemons)!=1: raise SystemExit('Cannot identify exactly one installed daemon; no sampling performed')
    daemon=daemons[0]
    shells=[r for r in first if r['exe'].endswith('/Otto.app/Contents/MacOS/otto-desktop')]
    owners={r['pid']:(r['start'], 'daemon' if r['pid']==daemon['pid'] else 'desktop') for r in [daemon,*shells]}
    owned_coalitions={c for c in (coalition(r['pid']) for r in shells) if c is not None}
    rows=[]
    for _ in range(args.seconds):
        current=snapshot(); bypid={r['pid']:r for r in current}; at=time.time()
        def descendant(pid):
            for _ in range(64):
                if pid==daemon['pid']: return True
                row=bypid.get(pid)
                if not row or pid<=1: return False
                pid=row['parent']
            return False
        for row in current:
            label=None
            owned=owners.get(row['pid'])
            if owned and owned[0]==row['start']: label=owned[1]
            elif bypid.get(daemon['pid'],{}).get('start')==daemon['start'] and descendant(row['pid']):
                name=pathlib.Path(row['exe']).name
                if name=='clickhouse': label='clickhouse'
                elif name=='otelcol-contrib': label='collector'
            elif row['exe'].startswith('/System/Library/Frameworks/WebKit.framework/') and any(bypid.get(r['pid'],{}).get('start')==r['start'] for r in shells):
                if coalition(row['pid']) in owned_coalitions: label='owned-'+pathlib.Path(row['exe']).name
            if label: rows.append({k:v for k,v in row.items() if k!='exe'} | {'process':label,'at':at})
        time.sleep(1)
    root=pathlib.Path(tempfile.mkdtemp(prefix='otto-quality-live-')); os.chmod(root,0o700)
    report={'method':'Read-only ps, 1 second cadence; CPU platform estimate, 100% = one core; exact installed daemon listener + start identity; WebKit resource coalition; provider processes excluded','requested_seconds':args.seconds,'samples':rows,'summary':[]}
    write_report(report,args.output or root/'report.json')

if __name__=='__main__': main()
