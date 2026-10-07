#!/usr/bin/env python3
"""Bounded disposable self_times capacity probe. No production endpoints/data."""
import argparse, hashlib, json, os, pathlib, re, socket, subprocess, tempfile, threading, time, urllib.request, urllib.error
from xml.sax.saxutils import escape

def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); return sock.getsockname()[1]

def stop(child):
    if child and child.poll() is None:
        child.terminate()
        try: child.wait(timeout=8)
        except subprocess.TimeoutExpired: child.kill(); child.wait(timeout=5)

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--repo', default=str(pathlib.Path(__file__).resolve().parents[4]))
    p.add_argument('--clickhouse', default=str(pathlib.Path.home()/'Library/Application Support/Otto/bin/clickhouse'))
    p.add_argument('--collector', default=str(pathlib.Path.home()/'Library/Application Support/Otto/telemetry/collector-0.162.0/otelcol-contrib'))
    args=p.parse_args()
    for value in (args.clickhouse,args.collector):
        if not pathlib.Path(value).is_file(): raise SystemExit('Installed binary missing: '+value)
    repo=pathlib.Path(args.repo)
    schema=(repo/'crates/otto-telemetry/src/schema.rs').read_text()
    fn=schema.split('pub(crate) fn self_times(',1)[1].split('\n}',1)[0]
    literal=re.search(r'format!\(("[^\n]+")\)',fn)
    if not literal: raise SystemExit('self_times source changed; inspect extractor before running')
    query=json.loads(literal.group(1)).replace('{hours}','24')
    if '{' in query: raise SystemExit('Unresolved query parameter')
    assert 'max_execution_time=10,max_memory_usage=268435456' in query
    root=pathlib.Path(tempfile.mkdtemp(prefix='otto-quality-self-times-'))
    os.chmod(root,0o700)
    (root/'tmp').mkdir()
    # Reuse the exact embedded-server config literal, change only owned paths,
    # query threads=2 and merge workers=2 (documented experimental controls).
    source=(repo/'crates/otto-usage/src/clickhouse.rs').read_text()
    literal=source.split('fn write_server_config(',1)[1].split('let xml = format!(',1)[1].split('\n    );',1)[0].strip()
    if not (literal.startswith('"') and literal.endswith('"')): raise SystemExit('Config source changed')
    xml=bytes(literal[1:-1],'utf8').decode('unicode_escape')
    http_port=port(); endpoint=f'http://127.0.0.1:{http_port}'
    xml=xml.replace('{port}',str(http_port)).replace('{path}',escape(str(root)+'/')).replace('{tmp}',escape(str(root/'tmp')+'/'))
    xml=xml.replace('<profiles><default/></profiles>','<profiles><default><max_threads>2</max_threads><max_insert_threads>2</max_insert_threads></default></profiles>')
    xml=xml.replace('<background_pool_size>4</background_pool_size>','<background_pool_size>2</background_pool_size>')
    (root/'config.xml').write_text(xml)
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def request(sql,timeout=70):
        # Fixed endpoint made from our freshly allocated port; never accepts URL input.
        req=urllib.request.Request(endpoint+'/?wait_end_of_query=1',data=sql.encode(),method='POST')
        try:
            with opener.open(req,timeout=timeout) as response: return response.status,response.read().decode(),dict(response.headers)
        except urllib.error.HTTPError as e: return e.code,e.read().decode(),dict(e.headers)
    def sql(text, timeout=70):
        status,body,_=request(text, timeout=timeout)
        if status!=200: raise RuntimeError(body[:2000])
        return body
    report={'directory':str(root),'query':query,'source_sha256':hashlib.sha256(schema.encode()).hexdigest(),
      'controls':{'max_threads':2,'background_pool_size':2,'server_memory_bytes':1073741824,'query_memory_bytes':268435456,'query_seconds':10,'seed_batch_spans':100000},
      'fixture':'paired parent/child spans; 50% distinct traces/parents,20 static operation names,4 components; all timestamps in last24h',
      'limitations':['Controlled 2-query-thread ceiling, not all server threads.','RSS is whole ClickHouse process; 100ms samples may miss short peaks.','Capacity error means production analyze would use inclusive-time fallback; fallback execution itself is not simulated.'], 'runs':[]}
    ch=collector=None
    try:
        chlog=open(root/'clickhouse.log','wb')
        ch=subprocess.Popen([args.clickhouse,'server','--config-file='+str(root/'config.xml')],cwd=root,env={**os.environ,'CLICKHOUSE_WATCHDOG_ENABLE':'0'},stdin=subprocess.DEVNULL,stdout=chlog,stderr=chlog)
        for _ in range(100):
            if ch.poll() is not None: raise RuntimeError('Owned ClickHouse exited; inspect artifact log')
            try:
                if sql('SELECT 1', timeout=1).strip()=='1': break
            except (OSError,RuntimeError): time.sleep(.3)
        else: raise RuntimeError('Owned ClickHouse readiness timed out')
        report['version']=sql('SELECT version()').strip()
        otlp_port=port()
        # Let the pinned installed exporter generate its REAL raw table DDL.
        # No collector download, telemetry ingestion or remote exporter occurs.
        cfg={'receivers':{'otlp':{'protocols':{'http':{'endpoint':f'127.0.0.1:{otlp_port}'}}}},
             'exporters':{'clickhouse':{'endpoint':endpoint,'database':'otto_telemetry','create_schema':True,'ttl':'24h','async_insert':True,'compress':'none','timeout':'5s'}},
             'service':{'telemetry':{'logs':{'level':'error'},'metrics':{'level':'none'}},'pipelines':{'traces':{'receivers':['otlp'],'exporters':['clickhouse']}}}}
        (root/'collector.json').write_text(json.dumps(cfg))
        collog=open(root/'collector.log','wb')
        collector=subprocess.Popen([args.collector,'--config',str(root/'collector.json')],cwd=root,env={**os.environ,'GOMAXPROCS':'2','GOMEMLIMIT':'160MiB'},stdin=subprocess.DEVNULL,stdout=collog,stderr=collog)
        for _ in range(100):
            if collector.poll() is not None: raise RuntimeError('Owned collector exited; inspect artifact log')
            try:
                if sql("EXISTS TABLE otto_telemetry.otel_traces", timeout=1).strip()=='1': break
            except (OSError,RuntimeError): pass
            time.sleep(.2)
        else: raise RuntimeError('Exporter DDL did not appear')
        report['raw_ddl']=sql('SHOW CREATE TABLE otto_telemetry.otel_traces')
        (root/'raw-ddl.sql').write_text(report['raw_ddl'])
        stop(collector); collector=None
        previous=0
        for total in (100000,1000000):
            # Bound fixture construction independently of the measured query.
            while previous < total:
                batch = min(100000, total - previous)
                seed=f"INSERT INTO otto_telemetry.otel_traces (Timestamp,TraceId,SpanId,ParentSpanId,SpanName,SpanKind,ServiceName,SpanAttributes,Duration) SELECT now64(9)-toIntervalSecond(number%3600),leftPad(hex(intDiv(number,2)+1),32,'0'),leftPad(hex(number+1),16,'0'),if(number%2=1,leftPad(hex(number),16,'0'),''),concat('fixture.op.',toString(number%20)),'Internal','otto',map('otto.component',concat('fixture.',toString(number%4))),toUInt64(if(number%2=0,100000000,25000000)) FROM numbers({previous},{batch}) SETTINGS max_threads=2,max_insert_threads=2,max_memory_usage=268435456,max_execution_time=60"
                sql(seed); previous += batch
            count=int(sql('SELECT count() FROM otto_telemetry.otel_traces').strip())
            if count!=total: raise RuntimeError(f'Wrong seed count {count}, expected {total}')
            for repeat in (1,2):
                samples=[]; done=threading.Event()
                def sample():
                    while not done.is_set():
                        try:
                            text=subprocess.check_output(['ps','-p',str(ch.pid),'-o','%cpu=,rss='],text=True).split()
                            if len(text)==2: samples.append({'at':time.time(),'cpu':float(text[0]),'rss_mb':int(text[1])/1024})
                        except (OSError,subprocess.CalledProcessError): pass
                        done.wait(.1)
                thread=threading.Thread(target=sample,daemon=True); thread.start()
                start=time.perf_counter()
                try: status,body,headers=request(query+' FORMAT JSON',timeout=20)
                finally: elapsed=time.perf_counter()-start; done.set(); thread.join(timeout=2)
                ok=status==200
                if ok:
                    try: decoded=json.loads(body)
                    except json.JSONDecodeError: ok=False
                result={'spans':total,'repeat':repeat,'elapsed_seconds':elapsed,'status':status,'fallback_expected':not ok,
                        'peak_rss_mb':max((r['rss_mb'] for r in samples),default=None),'samples':samples,
                        'summary':headers.get('X-ClickHouse-Summary'),'data':decoded if ok else body[:4000]}
                report['runs'].append(result)
                (root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
                print(json.dumps({k:v for k,v in result.items() if k not in ('samples','data')}),flush=True)
            time.sleep(1)
    except Exception as error:
        report['fatal_error']=str(error); raise
    finally:
        stop(collector); stop(ch)
        (root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
        print('Artifacts retained: '+str(root),flush=True)

if __name__=='__main__': main()
