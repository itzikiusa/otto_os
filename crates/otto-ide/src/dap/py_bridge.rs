//! Embedded zero-dependency Python 3 `bdb` Debug Adapter Protocol bridge.
//!
//! Used automatically by [`super::adapters`] whenever `debugpy` is not installed
//! in the selected Python environment, allowing Python scripts and unit tests to
//! be debugged out of the box with any standard Python 3 interpreter — including
//! live mid-run breakpoint updates, `pause`, exception breakpoints, `{var}` logpoints,
//! conditional/hit-count breakpoints, and compound dict/list/object variable expansion.

pub(crate) const PY_STDLIB_DAP_BRIDGE: &str = r#"
import bdb, json, os, queue, runpy, sys, threading, traceback

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
        super().__init__(skip=['bdb', 'runpy', 'importlib*', '_frozen_importlib*'])
        self._lock = threading.RLock()
        self.cmd_q = queue.Queue()
        self.bps = {}
        self.frames = []
        self.refs = {}
        self.next_ref = 1000
        self.launch_cfg = {}
        self.initial_run = True
        self.pause_requested = False
        self.break_on_exceptions = False
        self._bdb_file = os.path.realpath(bdb.__file__)
        self._runpy_file = os.path.realpath(runpy.__file__)
        # Keep a sentinel in self.breaks so bdb.set_continue() NEVER calls sys.settrace(None),
        # enabling instant mid-run breakpoint insertion & pause on running threads!
        self.breaks['__otto_keep_trace__'] = [1]

    def canonic(self, filename):
        if not filename or filename.startswith('<'):
            return filename
        return os.path.realpath(os.path.abspath(filename))

    def _alloc_ref(self, obj):
        self.next_ref += 1
        self.refs[self.next_ref] = obj
        return self.next_ref

    def _expandable(self, v):
        if isinstance(v, (dict, list, tuple, set)):
            return len(v) > 0
        return hasattr(v, '__dict__') and bool(getattr(v, '__dict__', None))

    def _internal(self, frame):
        co = frame.f_code
        fn = co.co_filename
        if not fn or fn.startswith('<'):
            return True
        if '_OttoDapBdb' in getattr(co, 'co_qualname', ''):
            return True
        return self.canonic(fn) in (self._bdb_file, self._runpy_file)

    def user_line(self, frame):
        if self._internal(frame):
            return
        fn = self.canonic(frame.f_code.co_filename)
        ln = frame.f_lineno
        with self._lock:
            bp_meta = self.bps.get((fn, ln))
            want_pause = self.pause_requested
            if want_pause:
                self.pause_requested = False
        if bp_meta:
            cond = bp_meta.get('condition')
            if cond:
                try:
                    if not eval(cond, frame.f_globals, frame.f_locals):
                        if self.initial_run:
                            self.set_continue()
                        return
                except Exception:
                    if self.initial_run:
                        self.set_continue()
                    return
            bp_meta['_hits'] = bp_meta.get('_hits', 0) + 1
            hit_cond = str(bp_meta.get('hitCondition') or '').strip().lstrip('=> ')
            if hit_cond.isdigit() and bp_meta['_hits'] < int(hit_cond):
                if self.initial_run:
                    self.set_continue()
                return
            log_msg = bp_meta.get('logMessage')
            if log_msg:
                try:
                    formatted = str(log_msg).format(**{**frame.f_globals, **frame.f_locals})
                except Exception:
                    formatted = str(log_msg)
                _send({'type': 'event', 'event': 'output', 'body': {'category': 'console', 'output': formatted + '\n'}})
                if self.initial_run:
                    self.initial_run = False
                    self.set_continue()
                return
        if self.initial_run and not bp_meta and not want_pause and not self.launch_cfg.get('stopOnEntry'):
            self.initial_run = False
            self.set_continue()
            return
        self.initial_run = False
        reason = 'breakpoint' if bp_meta else ('pause' if want_pause else 'step')
        self._pause(frame, reason)

    def user_exception(self, frame, exc_info):
        if not self.break_on_exceptions or self._internal(frame):
            return
        exc_type, exc_value, _ = exc_info
        if exc_type in (SystemExit, StopIteration, GeneratorExit):
            return
        _send({'type': 'event', 'event': 'output', 'body': {'category': 'stderr', 'output': f'{exc_type.__name__}: {exc_value}\n'}})
        self._pause(frame, 'exception')

    def _pause(self, frame, reason):
        self.frames = []
        cur = frame
        while cur is not None and len(self.frames) < 40:
            if not self._internal(cur):
                self.frames.append(cur)
            cur = cur.f_back
        if not self.frames:
            self.frames = [frame]
        _send({'type': 'event', 'event': 'stopped', 'body': {'reason': reason, 'threadId': 1, 'allThreadsStopped': True}})
        while True:
            req = self.cmd_q.get()
            if req is None:
                os._exit(0)
            cmd = req.get('command')
            args = req.get('arguments') or {}
            rseq = req.get('seq', 0)
            if cmd == 'threads':
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'threads': [{'id': 1, 'name': 'MainThread'}]}})
            elif cmd == 'stackTrace':
                sfs = [{'id': i + 1, 'name': f.f_code.co_name, 'source': {'path': self.canonic(f.f_code.co_filename)}, 'line': f.f_lineno, 'column': 1} for i, f in enumerate(self.frames)]
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
                for k, v in list(items)[:120]:
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
        raw_src = (args.get('source') or {}).get('path') or ''
        fn = self.canonic(raw_src)
        with self._lock:
            self.clear_all_file_breaks(fn)
            self.breaks['__otto_keep_trace__'] = [1]
            for k in [k for k in self.bps if k[0] == fn]:
                del self.bps[k]
            out = []
            for bp in (args.get('breakpoints') or []):
                ln = int(bp.get('line') or 1)
                self.set_break(fn, ln)
                self.bps[(fn, ln)] = dict(bp)
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
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {
                'supportsConfigurationDoneRequest': True,
                'supportsConditionalBreakpoints': True,
                'supportsHitConditionalBreakpoints': True,
                'supportsLogPoints': True,
                'supportsTerminateRequest': True,
                'exceptionBreakpointFilters': [{'filter': 'uncaught', 'label': 'Uncaught Exceptions'}, {'filter': 'all', 'label': 'All Exceptions'}]
            }})
        elif cmd in ('launch', 'attach'):
            dbg.launch_cfg = args
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            _send({'type': 'event', 'event': 'initialized', 'body': {}})
        elif cmd == 'setBreakpoints':
            dbg._apply_bps(rseq, args)
        elif cmd == 'setExceptionBreakpoints':
            dbg.break_on_exceptions = bool(args.get('filters'))
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
        elif cmd == 'pause':
            with dbg._lock:
                dbg.pause_requested = True
                if dbg.botframe is not None:
                    dbg._set_stopinfo(dbg.botframe, None, 0)
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
        elif cmd == 'configurationDone':
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            started.set()
        elif cmd in ('disconnect', 'terminate'):
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            _send({'type': 'event', 'event': 'terminated', 'body': {}})
            os._exit(0)
        else:
            dbg.cmd_q.put(m)

threading.Thread(target=_reader, daemon=True).start()
started.wait()
prog = dbg.launch_cfg.get('program') or ''
cwd = os.path.realpath(dbg.launch_cfg.get('cwd') or os.getcwd())
os.chdir(cwd)
sys.path.insert(0, cwd)
if prog:
    prog = os.path.realpath(prog if os.path.isabs(prog) else os.path.join(cwd, prog))
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
except Exception:
    traceback.print_exc()
_send({'type': 'event', 'event': 'terminated', 'body': {}})
"#;
