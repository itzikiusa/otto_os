//! Embedded zero-dependency Python 3 `bdb` Debug Adapter Protocol bridge.
//!
//! Used automatically by [`super::adapters`] whenever `debugpy` is not installed
//! in the selected Python environment, allowing Python scripts and unit tests to
//! be debugged out of the box with any standard Python 3 interpreter.

pub(crate) const PY_STDLIB_DAP_BRIDGE: &str = r#"
import bdb, json, os, queue, runpy, sys, threading

_raw_in = os.fdopen(os.dup(0), 'rb', 0)
_raw_out = os.fdopen(os.dup(1), 'wb', 0)
_out_lock = threading.Lock()
_seq = 0

def _send(msg):
    global _seq
    _seq += 1
    msg['seq'] = _seq
    data = json.dumps(msg).encode('utf-8')
    with _out_lock:
        _raw_out.write(('Content-Length: %d\r\n\r\n' % len(data)).encode('ascii') + data)

class _StreamProxy:
    def __init__(self, cat):
        self.cat = cat
    def write(self, s):
        if s:
            _send({'type': 'event', 'event': 'output', 'body': {'category': self.cat, 'output': s}})
        return len(s)
    def flush(self):
        pass

sys.stdout = _StreamProxy('stdout')
sys.stderr = _StreamProxy('stderr')

def _recv():
    hdr = b''
    while not hdr.endswith(b'\r\n\r\n'):
        ch = _raw_in.read(1)
        if not ch:
            return None
        hdr += ch
    ln = 0
    for line in hdr.decode('ascii', 'replace').split('\r\n'):
        if line.lower().startswith('content-length:'):
            ln = int(line.split(':', 1)[1].strip())
    body = b''
    while len(body) < ln:
        chunk = _raw_in.read(ln - len(body))
        if not chunk:
            return None
        body += chunk
    return json.loads(body.decode('utf-8'))

class _OttoDapBdb(bdb.Bdb):
    def __init__(self):
        super().__init__()
        self.cmd_q = queue.Queue()
        self.bps = {}
        self.frames = []
        self.refs = {}
        self.next_ref = 1000
        self.launch_cfg = {}

    def _alloc_ref(self, obj):
        self.next_ref += 1
        self.refs[self.next_ref] = obj
        return self.next_ref

    def _expandable(self, v):
        return isinstance(v, (dict, list, tuple, set)) or hasattr(v, '__dict__')

    def user_line(self, frame):
        fn = self.canonic(frame.f_code.co_filename)
        ln = frame.f_lineno
        bp_meta = self.bps.get((fn, ln))
        if bp_meta:
            cond = bp_meta.get('condition')
            if cond:
                try:
                    if not eval(cond, frame.f_globals, frame.f_locals):
                        return
                except Exception:
                    return
            log_msg = bp_meta.get('logMessage')
            if log_msg:
                _send({'type': 'event', 'event': 'output', 'body': {'category': 'console', 'output': str(log_msg) + '\n'}})
                return
        self._pause(frame, 'breakpoint' if bp_meta else 'step')

    def _pause(self, frame, reason):
        self.frames = []
        cur = frame
        while cur is not None and len(self.frames) < 40:
            if '_OttoDapBdb' not in cur.f_code.co_qualname and cur.f_code.co_filename != '<string>':
                self.frames.append(cur)
            cur = cur.f_back
        if not self.frames:
            self.frames = [frame]
        _send({'type': 'event', 'event': 'stopped', 'body': {'reason': reason, 'threadId': 1, 'allThreadsStopped': True}})
        while True:
            req = self.cmd_q.get()
            if req is None:
                sys.exit(0)
            cmd = req.get('command')
            args = req.get('arguments') or {}
            rseq = req.get('seq', 0)
            if cmd == 'threads':
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'threads': [{'id': 1, 'name': 'MainThread'}]}})
            elif cmd == 'stackTrace':
                sfs = [{'id': i + 1, 'name': f.f_code.co_name, 'source': {'path': os.path.realpath(f.f_code.co_filename)}, 'line': f.f_lineno, 'column': 1} for i, f in enumerate(self.frames)]
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'stackFrames': sfs, 'totalFrames': len(sfs)}})
            elif cmd == 'scopes':
                fid = max(1, int(args.get('frameId') or 1)) - 1
                f = self.frames[min(fid, len(self.frames) - 1)]
                lref = self._alloc_ref(dict(f.f_locals))
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'scopes': [{'name': 'Locals', 'variablesReference': lref, 'expensive': False}]}})
            elif cmd == 'variables':
                obj = self.refs.get(int(args.get('variablesReference') or 0), {})
                items = obj.items() if isinstance(obj, dict) else (enumerate(obj) if isinstance(obj, (list, tuple, set)) else getattr(obj, '__dict__', {}).items())
                vs = []
                for k, v in list(items)[:100]:
                    ks = str(k)
                    if ks.startswith('__'):
                        continue
                    cref = self._alloc_ref(v) if self._expandable(v) else 0
                    vs.append({'name': ks, 'value': repr(v), 'type': type(v).__name__, 'variablesReference': cref})
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'variables': vs}})
            elif cmd == 'evaluate':
                fid = max(1, int(args.get('frameId') or 1)) - 1
                f = self.frames[min(fid, len(self.frames) - 1)]
                expr = args.get('expression') or ''
                try:
                    val = eval(expr, f.f_globals, f.f_locals)
                    cref = self._alloc_ref(val) if self._expandable(val) else 0
                    _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'result': repr(val), 'type': type(val).__name__, 'variablesReference': cref}})
                except Exception as e:
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': str(e)})
            elif cmd == 'setBreakpoints':
                self._apply_bps(rseq, args)
            elif cmd == 'continue':
                self.set_continue()
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'allThreadsContinued': True}})
                return
            elif cmd == 'next':
                self.set_next(frame)
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
                return
            elif cmd == 'stepIn':
                self.set_step()
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
                return
            elif cmd == 'stepOut':
                self.set_return(frame)
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
                return
            elif cmd in ('disconnect', 'terminate'):
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
                _send({'type': 'event', 'event': 'terminated', 'body': {}})
                os._exit(0)
            else:
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})

    def _apply_bps(self, rseq, args):
        src = os.path.realpath((args.get('source') or {}).get('path') or '')
        fn = self.canonic(src)
        self.clear_all_file_breaks(fn)
        for k in [k for k in self.bps if k[0] == fn]:
            del self.bps[k]
        out = []
        for bp in (args.get('breakpoints') or []):
            ln = int(bp.get('line') or 1)
            self.set_break(fn, ln)
            self.bps[(fn, ln)] = bp
            out.append({'verified': True, 'line': ln})
        _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': 'setBreakpoints', 'body': {'breakpoints': out}})

dbg = _OttoDapBdb()
started = threading.Event()

def _reader():
    while True:
        m = _recv()
        if m is None:
            dbg.cmd_q.put(None)
            os._exit(0)
        cmd = m.get('command')
        rseq = m.get('seq', 0)
        args = m.get('arguments') or {}
        if cmd == 'initialize':
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'supportsConfigurationDoneRequest': True, 'supportsConditionalBreakpoints': True, 'supportsLogPoints': True}})
        elif cmd in ('launch', 'attach'):
            dbg.launch_cfg = args
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            _send({'type': 'event', 'event': 'initialized', 'body': {}})
        elif cmd == 'setBreakpoints' and not started.is_set():
            dbg._apply_bps(rseq, args)
        elif cmd == 'setExceptionBreakpoints' and not started.is_set():
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
        elif cmd == 'configurationDone':
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            started.set()
        elif cmd in ('disconnect', 'terminate') and not started.is_set():
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            os._exit(0)
        else:
            dbg.cmd_q.put(m)

threading.Thread(target=_reader, daemon=True).start()
started.wait()
prog = dbg.launch_cfg.get('program') or ''
cwd = dbg.launch_cfg.get('cwd') or os.getcwd()
os.chdir(cwd)
sys.path.insert(0, cwd)
tname = dbg.launch_cfg.get('testName')
try:
    if tname and os.path.isfile(prog):
        import importlib.util
        spec = importlib.util.spec_from_file_location('__otto_test_mod__', prog)
        mod = importlib.util.module_from_spec(spec)
        dbg.runcall(spec.loader.exec_module, mod)
        parts = [p for p in str(tname).split('::') if p and not p.endswith('.py')]
        target_obj = mod
        for idx_p, part in enumerate(parts):
            if hasattr(target_obj, part):
                attr = getattr(target_obj, part)
                target_obj = attr() if (isinstance(attr, type) and idx_p + 1 < len(parts)) else attr
        if callable(target_obj):
            dbg.runcall(target_obj)
    else:
        sys.argv = [prog] + list(dbg.launch_cfg.get('args') or [])
        dbg.run('runpy.run_path(%r, run_name="__main__")' % prog, {'runpy': runpy})
except SystemExit:
    pass
_send({'type': 'event', 'event': 'terminated', 'body': {}})
"#;
