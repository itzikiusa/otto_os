//! Zero-install macOS/Linux LLDB Python `SBDebugger` Debug Adapter Protocol bridge.
//!
//! Used automatically by [`super::adapters_rust_sh`] when neither `codelldb` nor
//! `lldb-dap` is installed on the host, leveraging Xcode Command Line Tools' built-in
//! `lldb -P` (`LLDB.framework/Resources/Python`) + `/usr/bin/python3` so Rust binaries
//! and `#[test]` targets debug out of the box on every Mac.

pub(crate) const LLDB_PYTHON_DAP_BRIDGE: &str = r#"
import json, os, re, sys, threading, time
import lldb

_raw_in = os.fdopen(os.dup(0), 'rb', 0)
_raw_out = os.fdopen(os.dup(1), 'wb', 0)
_out_lock = threading.Lock()
_state_lock = threading.RLock()
_seq = 0

def _send(msg):
    global _seq
    with _out_lock:
        _seq += 1
        msg['seq'] = _seq
        data = json.dumps(msg).encode('utf-8')
        _raw_out.write(('Content-Length: %d\r\n\r\n' % len(data)).encode('ascii') + data)

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

class _LldbDapBridge:
    def __init__(self):
        lldb.SBDebugger.Initialize()
        self.dbg = lldb.SBDebugger.Create()
        self.dbg.SetAsync(True)
        self.listener = self.dbg.GetListener()
        self.target = None
        self.process = None
        self.launch_cfg = {}
        self.bp_ids_by_file = {}
        self.fn_bp_ids = []
        self.logpoints = {}
        self.panic_bp_ids = []
        self.frame_map = {}
        self.val_refs = {}
        self.next_ref = 1000
        self.suppress_pause_event = False

    def _alloc_val_ref(self, sbval):
        self.next_ref += 1
        self.val_refs[self.next_ref] = sbval
        return self.next_ref

    def _format_zig_sbval(self, v, tname):
        if tname.startswith('[]'):
            ptr = v.GetChildMemberWithName('ptr')
            ln_val = v.GetChildMemberWithName('len')
            if ptr and ptr.IsValid() and ln_val and ln_val.IsValid():
                length = ln_val.GetValueAsUnsigned(0)
                if length == 0:
                    return '"" (len=0)' if 'u8' in tname else '[] (len=0)'
                addr = ptr.GetValueAsUnsigned(0)
                if addr == 0:
                    return f'null (len={length})'
                elem_type = ptr.GetType().GetPointeeType()
                elem_name = elem_type.GetName() or ''
                if ('u8' in tname or elem_name in ('unsigned char', 'u8', 'const unsigned char', 'const u8')) and self.process and self.process.IsValid():
                    read_len = min(length, 256)
                    err = lldb.SBError()
                    data = self.process.ReadMemory(addr, read_len, err)
                    if err.Success() and data is not None:
                        try:
                            text = data.decode('utf-8')
                            if all(ch.isprintable() or ch in '\t\r\n' for ch in text):
                                esc = text.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n').replace('\r', '\\r').replace('\t', '\\t')
                                suffix = '...' if length > read_len else ''
                                return f'"{esc}{suffix}" (len={length})'
                        except Exception:
                            pass
                elem_size = elem_type.GetByteSize()
                if self.target and self.target.IsValid() and elem_size > 0:
                    shown = min(length, 8)
                    elems = []
                    for i in range(shown):
                        ev = self.target.CreateValueFromAddress(f'[{i}]', lldb.SBAddress(addr + i * elem_size, self.target), elem_type)
                        elems.append(ev.GetSummary() or ev.GetValue() or '{...}')
                    more = ', ...' if length > shown else ''
                    return f'[len={length}] {{{", ".join(elems)}{more}}}'
                return f'[len={length}]'
        if tname.startswith('?'):
            some = v.GetChildMemberWithName('some')
            data = v.GetChildMemberWithName('data')
            if some and some.IsValid():
                if some.GetValueAsUnsigned(0) == 0:
                    return 'null'
                if data and data.IsValid():
                    d, _, _ = self._format_sbval(data)
                    return d
                return 'some'
        if '!' in tname:
            tag = v.GetChildMemberWithName('tag')
            val = v.GetChildMemberWithName('value')
            if tag and tag.IsValid():
                code = tag.GetValueAsUnsigned(0)
                if code == 0:
                    if val and val.IsValid():
                        d, _, _ = self._format_sbval(val)
                        return f'ok({d})'
                    return 'ok'
                ts = tag.GetValue() or str(code)
                return f'error(#{code})' if ts.isdigit() else f'error.{ts}'
        if v.GetNumChildren(4) == 2:
            tag = v.GetChildMemberWithName('tag')
            payload = v.GetChildMemberWithName('payload')
            if tag and tag.IsValid() and payload and payload.IsValid():
                tstr = (tag.GetValue() or '').strip()
                if tstr:
                    active = payload.GetChildMemberWithName(tstr)
                    if active and active.IsValid():
                        d, _, _ = self._format_sbval(active)
                        return f'.{tstr} = {d}'
                    return f'.{tstr}'
        return None

    def _format_cpp_sbval(self, v, tname):
        if 'vector<' in tname and ('std::' in tname or 'TArray<' in tname):
            n = v.GetNumChildren(16)
            if n == 0:
                return '[] (size=0)'
            shown = min(n, 8)
            elems = []
            for i in range(shown):
                c = v.GetChildAtIndex(i)
                elems.append(c.GetSummary() or c.GetValue() or '{...}')
            more = ', ...' if n > shown else ''
            return f'[{", ".join(elems)}{more}] (size={n})'
        if ('map<' in tname or 'unordered_map<' in tname) and 'std::' in tname:
            n = v.GetNumChildren(16)
            if n == 0:
                return '{} (size=0)'
            shown = min(n, 6)
            pairs = []
            for i in range(shown):
                c = v.GetChildAtIndex(i)
                k = c.GetChildMemberWithName('first')
                val = c.GetChildMemberWithName('second')
                if k and k.IsValid() and val and val.IsValid():
                    pairs.append(f'{k.GetSummary() or k.GetValue()}: {val.GetSummary() or val.GetValue()}')
                else:
                    pairs.append(c.GetSummary() or c.GetValue() or '{...}')
            more = ', ...' if n > shown else ''
            return '{' + ', '.join(pairs) + more + f'}} (size={n})'
        if 'optional<' in tname and 'std::' in tname:
            if v.GetNumChildren(2) == 0:
                return 'std::nullopt'
            val = v.GetChildMemberWithName('Value') or v.GetChildAtIndex(0)
            if val and val.IsValid():
                return val.GetSummary() or val.GetValue() or '{...}'
        return None

    def _format_sbval(self, v):
        tname = v.GetDisplayTypeName() or v.GetTypeName() or ''
        zig_disp = self._format_zig_sbval(v, tname)
        cpp_disp = self._format_cpp_sbval(v, tname) if zig_disp is None else None
        summary = v.GetSummary()
        val = v.GetValue()
        if zig_disp is not None:
            disp = zig_disp
        elif cpp_disp is not None:
            disp = cpp_disp
        elif summary and val:
            disp = f'{summary}' if tname.endswith('str') or 'String' in tname else f'{val} ({summary})'
        elif summary:
            disp = summary
        elif val is not None:
            disp = val
        else:
            num_c = v.GetNumChildren(16)
            if num_c > 0:
                parts = []
                for idx in range(min(num_c, 6)):
                    ch = v.GetChildAtIndex(idx)
                    cn = ch.GetName() or str(idx)
                    cv = ch.GetSummary() or ch.GetValue() or '{...}'
                    parts.append(f'{cn}: {cv}')
                disp = '{ ' + ', '.join(parts) + (' ...' if num_c > 6 else '') + ' }'
            else:
                disp = '{...}'
        cref = 0
        if v.MightHaveChildren() and v.GetNumChildren(64) > 0 and not ('str' == tname or tname == '&str'):
            cref = self._alloc_val_ref(v)
        return disp, tname, cref

    def _drain_stdio(self):
        if not self.process or not self.process.IsValid():
            return
        while True:
            out = self.process.GetSTDOUT(8192)
            if not out:
                break
            _send({'type': 'event', 'event': 'output', 'body': {'category': 'stdout', 'output': out}})
        while True:
            err = self.process.GetSTDERR(8192)
            if not err:
                break
            _send({'type': 'event', 'event': 'output', 'body': {'category': 'stderr', 'output': err}})

    def _event_pump(self):
        ev = lldb.SBEvent()
        while True:
            if not self.process or not self.process.IsValid():
                break
            if self.listener.WaitForEvent(1, ev):
                self._drain_stdio()
                if not lldb.SBProcess.EventIsProcessEvent(ev):
                    continue
                ev_mask = ev.GetType()
                if not (ev_mask & lldb.SBProcess.eBroadcastBitStateChanged):
                    continue
                st = lldb.SBProcess.GetStateFromEvent(ev)
                if st == lldb.eStateStopped:
                    if lldb.SBProcess.GetRestartedFromEvent(ev):
                        continue
                    with _state_lock:
                        if self.suppress_pause_event:
                            continue
                        self.frame_map.clear()
                        self.val_refs.clear()
                        stopped_thr = None
                        for t in self.process:
                            sr = t.GetStopReason()
                            if sr in (lldb.eStopReasonBreakpoint, lldb.eStopReasonPlanComplete, lldb.eStopReasonException, lldb.eStopReasonSignal):
                                stopped_thr = t
                                break
                        if stopped_thr is None and self.process.GetNumThreads() > 0:
                            stopped_thr = self.process.GetThreadAtIndex(0)
                        if stopped_thr is not None and stopped_thr.GetStopReason() == lldb.eStopReasonBreakpoint:
                            bp_id = stopped_thr.GetStopReasonDataAtIndex(0)
                            if bp_id in self.logpoints:
                                tpl = self.logpoints[bp_id]
                                f0 = stopped_thr.GetFrameAtIndex(0)
                                def _sub(m):
                                    expr = m.group(1).strip()
                                    vv = f0.FindVariable(expr)
                                    if not vv or not vv.IsValid():
                                        vv = f0.EvaluateExpression(expr)
                                    return vv.GetSummary() or vv.GetValue() or ''
                                rendered = re.sub(r'\{([^}]+)\}', _sub, tpl)
                                _send({'type': 'event', 'event': 'output', 'body': {'category': 'console', 'output': rendered + '\n'}})
                                self.process.Continue()
                                continue
                        sr = stopped_thr.GetStopReason() if stopped_thr else 0
                        reason = 'breakpoint' if sr == lldb.eStopReasonBreakpoint else ('step' if sr == lldb.eStopReasonPlanComplete else ('exception' if sr == lldb.eStopReasonException else 'pause'))
                        tid = stopped_thr.GetThreadID() if stopped_thr else 1
                    _send({'type': 'event', 'event': 'stopped', 'body': {'reason': reason, 'threadId': tid, 'allThreadsStopped': True}})
                elif st in (lldb.eStateExited, lldb.eStateCrashed, lldb.eStateDetached):
                    self._drain_stdio()
                    _send({'type': 'event', 'event': 'terminated', 'body': {}})
                    break
            else:
                self._drain_stdio()

    def apply_breakpoints(self, rseq, args):
        raw_path = (args.get('source') or {}).get('path') or ''
        canon = os.path.realpath(raw_path) if raw_path else ''
        bps = args.get('breakpoints') or []
        with _state_lock:
            was_running = bool(self.process and self.process.IsValid() and self.process.GetState() == lldb.eStateRunning)
            if was_running:
                self.suppress_pause_event = True
                self.process.Stop()
                for _ in range(30):
                    if self.process.GetState() == lldb.eStateStopped:
                        break
                    time.sleep(0.01)
            for old_id in self.bp_ids_by_file.pop(canon, []):
                self.logpoints.pop(old_id, None)
                if self.target and self.target.IsValid():
                    self.target.BreakpointDelete(old_id)
            ack = []
            new_ids = []
            for b in bps:
                ln = int(b.get('line') or 1)
                if self.target and self.target.IsValid():
                    sbbp = self.target.BreakpointCreateByLocation(canon, ln)
                    if sbbp.GetNumLocations() == 0:
                        base = os.path.basename(canon)
                        if base:
                            self.target.BreakpointDelete(sbbp.GetID())
                            sbbp = self.target.BreakpointCreateByLocation(base, ln)
                    # A macro line (`println!`, `assert_eq!`) has several locations in one
                    # function, so Continue stopped on the same line again. Keep the first
                    # per function; other functions (generic copies) keep theirs.
                    first_by_fn = {}
                    for i in range(sbbp.GetNumLocations()):
                        loc = sbbp.GetLocationAtIndex(i)
                        key = loc.GetAddress().GetFunction().GetStartAddress().GetFileAddress()
                        prev = first_by_fn.get(key)
                        if prev is None or loc.GetAddress().GetFileAddress() < prev.GetAddress().GetFileAddress():
                            if prev is not None:
                                prev.SetEnabled(False)
                            first_by_fn[key] = loc
                        else:
                            loc.SetEnabled(False)
                    bid = sbbp.GetID()
                    new_ids.append(bid)
                    cond = (b.get('condition') or '').strip()
                    if cond:
                        sbbp.SetCondition(cond)
                    hit_s = str(b.get('hitCondition') or '').strip().lstrip('=> ')
                    if hit_s.isdigit() and int(hit_s) > 1:
                        sbbp.SetIgnoreCount(int(hit_s) - 1)
                    log_m = (b.get('logMessage') or '').strip()
                    if log_m:
                        self.logpoints[bid] = log_m
                    resolved_ln = ln
                    if sbbp.GetNumLocations() > 0:
                        loc0 = sbbp.GetLocationAtIndex(0)
                        le = loc0.GetAddress().GetLineEntry()
                        if le.IsValid() and le.GetLine() > 0:
                            resolved_ln = le.GetLine()
                    ack.append({'verified': sbbp.GetNumLocations() > 0 or not bool(self.process), 'line': resolved_ln})
                else:
                    ack.append({'verified': True, 'line': ln})
            self.bp_ids_by_file[canon] = new_ids
            if was_running:
                self.suppress_pause_event = False
                self.process.Continue()
        _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': 'setBreakpoints', 'body': {'breakpoints': ack}})

    def _detach_on_stop(self, cmd, args):
        return getattr(self, 'is_attach', False) and cmd == 'disconnect' and not args.get('terminateDebuggee', False)

    def handle(self, req):
        cmd = req.get('command')
        rseq = req.get('seq', 0)
        args = req.get('arguments') or {}
        if cmd == 'initialize':
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {
                'supportsConfigurationDoneRequest': True,
                'supportsConditionalBreakpoints': True,
                'supportsHitConditionalBreakpoints': True,
                'supportsLogPoints': True,
                'supportsFunctionBreakpoints': True,
                'supportsSetVariable': True,
                'supportsTerminateRequest': True,
                'exceptionBreakpointFilters': [
                    {'filter': 'rust_panic', 'label': 'Rust Panics'},
                    {'filter': 'zig_panic', 'label': 'Zig Panics'},
                    {'filter': 'cpp_throw', 'label': 'C++ Exceptions (throw)'}
                ]
            }})
        elif cmd in ('launch', 'attach'):
            self.launch_cfg = args
            self.is_attach = (cmd == 'attach')
            prog = args.get('program') or ''
            if prog:
                self.target = self.dbg.CreateTargetWithFileAndArch(os.path.realpath(prog), lldb.LLDB_ARCH_DEFAULT)
            elif self.is_attach:
                self.target = self.dbg.CreateTarget('')
            if not self.target or not self.target.IsValid():
                _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': f'LLDB failed to load target executable: {prog}'})
                return
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            _send({'type': 'event', 'event': 'initialized', 'body': {}})
        elif cmd == 'setBreakpoints':
            self.apply_breakpoints(rseq, args)
        elif cmd == 'setFunctionBreakpoints':
            fbps = args.get('breakpoints') or []
            ack = []
            with _state_lock:
                for old_id in self.fn_bp_ids:
                    if self.target and self.target.IsValid():
                        self.target.BreakpointDelete(old_id)
                self.fn_bp_ids = []
                for fb in fbps:
                    name = (fb.get('name') or '').strip()
                    if not name or not self.target or not self.target.IsValid():
                        ack.append({'verified': False})
                        continue
                    sbbp = self.target.BreakpointCreateByName(name)
                    self.fn_bp_ids.append(sbbp.GetID())
                    ack.append({'verified': sbbp.GetNumLocations() > 0})
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'breakpoints': ack}})
        elif cmd == 'setExceptionBreakpoints':
            filters = args.get('filters') or []
            with _state_lock:
                for old_id in self.panic_bp_ids:
                    if self.target and self.target.IsValid():
                        self.target.BreakpointDelete(old_id)
                self.panic_bp_ids = []
                if filters and self.target and self.target.IsValid():
                    lang = (self.launch_cfg.get('language') or '').lower()
                    if lang == 'zig':
                        names = ['debug.defaultPanic', 'builtin.default_panic', 'debug.panicImpl']
                    elif lang in ('c', 'cpp') or 'cpp_throw' in filters:
                        try:
                            ebp = self.target.BreakpointCreateForException(lldb.eLanguageTypeC_plus_plus, False, True)
                            if ebp and ebp.IsValid():
                                self.panic_bp_ids.append(ebp.GetID())
                        except Exception:
                            pass
                        names = ['__cxa_throw']
                    else:
                        names = ['rust_panic']
                    for sym in names:
                        pbp = self.target.BreakpointCreateByName(sym)
                        self.panic_bp_ids.append(pbp.GetID())
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
        elif cmd == 'configurationDone':
            err = lldb.SBError()
            if getattr(self, 'is_attach', False):
                core_file = self.launch_cfg.get('coreFile')
                pid = self.launch_cfg.get('pid')
                pname = self.launch_cfg.get('processName')
                if core_file:
                    self.process = self.target.LoadCore(os.path.realpath(core_file))
                elif pid:
                    self.process = self.target.AttachToProcessWithID(self.listener, int(pid), err)
                elif pname:
                    self.process = self.target.AttachToProcessWithName(self.listener, str(pname), False, err)
                else:
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': 'attach requires pid, processName, or coreFile'})
                    return
                if not self.process or not self.process.IsValid() or err.Fail():
                    msg = err.GetCString() or 'LLDB attach failed'
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': msg})
                    return
                if core_file:
                    _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
                    tid = self.process.GetThreadAtIndex(0).GetThreadID() if self.process.GetNumThreads() > 0 else 1
                    _send({'type': 'event', 'event': 'stopped', 'body': {'reason': 'exception', 'threadId': tid, 'allThreadsStopped': True}})
                    return
            else:
                argv = [str(x) for x in (self.launch_cfg.get('args') or [])]
                cwd = os.path.realpath(self.launch_cfg.get('cwd') or os.getcwd())
                li = lldb.SBLaunchInfo(argv)
                li.SetWorkingDirectory(cwd)
                li.SetListener(self.listener)
                env_list = [f'{k}={v}' for k, v in os.environ.items()]
                li.SetEnvironmentEntries(env_list, False)
                self.process = self.target.Launch(li, err)
                if not self.process or not self.process.IsValid() or err.Fail():
                    msg = err.GetCString() or 'LLDB process launch failed'
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': msg})
                    return
            threading.Thread(target=self._event_pump, daemon=True).start()
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
        elif cmd == 'threads':
            thrs = []
            with _state_lock:
                if self.process and self.process.IsValid():
                    for t in self.process:
                        thrs.append({'id': t.GetThreadID(), 'name': t.GetName() or f'Thread #{t.GetIndexID()}'})
            if not thrs:
                thrs = [{'id': 1, 'name': 'main'}]
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'threads': thrs}})
        elif cmd == 'stackTrace':
            tid = int(args.get('threadId') or 1)
            sfs = []
            with _state_lock:
                thr = self.process.GetThreadByID(tid) if (self.process and self.process.IsValid()) else None
                if (not thr or not thr.IsValid()) and self.process and self.process.GetNumThreads() > 0:
                    thr = self.process.GetThreadAtIndex(0)
                if thr and thr.IsValid():
                    for idx in range(min(32, thr.GetNumFrames())):
                        f = thr.GetFrameAtIndex(idx)
                        fid = (idx + 1) + int(thr.GetIndexID()) * 1000
                        self.frame_map[fid] = (thr.GetThreadID(), idx)
                        le = f.GetLineEntry()
                        src = None
                        ln = 1
                        col = 1
                        if le and le.IsValid():
                            ln = max(1, int(le.GetLine() or 1))
                            col = max(1, int(le.GetColumn() or 1))
                            fs = le.GetFileSpec()
                            if fs and fs.IsValid() and fs.GetFilename():
                                d = fs.GetDirectory() or ''
                                full = os.path.join(d, fs.GetFilename()) if d else fs.GetFilename()
                                if os.path.isfile(full):
                                    src = {'path': os.path.realpath(full)}
                        sfs.append({'id': fid, 'name': f.GetDisplayFunctionName() or f.GetFunctionName() or '(frame)', 'source': src, 'line': ln, 'column': col})
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'stackFrames': sfs, 'totalFrames': len(sfs)}})
        elif cmd == 'scopes':
            fid = int(args.get('frameId') or 1)
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {
                'scopes': [{'name': 'Locals', 'variablesReference': 100000 + fid, 'expensive': False}]
            }})
        elif cmd == 'variables':
            vref = int(args.get('variablesReference') or 0)
            out = []
            with _state_lock:
                if vref >= 100000:
                    fid = vref - 100000
                    tid, fidx = self.frame_map.get(fid, (0, 0))
                    thr = self.process.GetThreadByID(tid) if (self.process and tid) else (self.process.GetThreadAtIndex(0) if self.process and self.process.GetNumThreads() > 0 else None)
                    f = thr.GetFrameAtIndex(fidx) if thr and thr.IsValid() else None
                    if f and f.IsValid():
                        vlist = f.GetVariables(True, True, False, True)
                        for idx in range(min(150, vlist.GetSize())):
                            v = vlist.GetValueAtIndex(idx)
                            nm = v.GetName()
                            if not nm:
                                continue
                            disp, tname, cref = self._format_sbval(v)
                            out.append({'name': nm, 'value': disp, 'type': tname, 'variablesReference': cref})
                elif vref in self.val_refs:
                    parent = self.val_refs[vref]
                    for idx in range(min(120, parent.GetNumChildren(120))):
                        ch = parent.GetChildAtIndex(idx)
                        nm = ch.GetName() or f'[{idx}]'
                        disp, tname, cref = self._format_sbval(ch)
                        out.append({'name': nm, 'value': disp, 'type': tname, 'variablesReference': cref})
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'variables': out}})
        elif cmd == 'setVariable':
            vref = int(args.get('variablesReference') or 0)
            name = (args.get('name') or '').strip()
            new_val = str(args.get('value') or '')
            with _state_lock:
                target_var = None
                if vref >= 100000:
                    fid = vref - 100000
                    tid, fidx = self.frame_map.get(fid, (0, 0))
                    thr = self.process.GetThreadByID(tid) if (self.process and tid) else (self.process.GetThreadAtIndex(0) if self.process and self.process.GetNumThreads() > 0 else None)
                    f = thr.GetFrameAtIndex(fidx) if thr and thr.IsValid() else None
                    if f and f.IsValid():
                        target_var = f.FindVariable(name)
                elif vref in self.val_refs:
                    parent = self.val_refs[vref]
                    target_var = parent.GetChildMemberWithName(name)
                if not target_var or not target_var.IsValid():
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': f'variable {name} not found'})
                    return
                err = lldb.SBError()
                ok = target_var.SetValueFromCString(new_val, err)
                if not ok or err.Fail():
                    msg = err.GetCString() or f'failed to set variable {name}'
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': msg})
                    return
                disp, tname, cref = self._format_sbval(target_var)
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'value': disp, 'type': tname, 'variablesReference': cref}})
        elif cmd == 'evaluate':
            expr = (args.get('expression') or '').strip()
            fid = int(args.get('frameId') or 1)
            with _state_lock:
                tid, fidx = self.frame_map.get(fid, (0, 0))
                thr = self.process.GetThreadByID(tid) if (self.process and tid) else (self.process.GetThreadAtIndex(0) if self.process and self.process.GetNumThreads() > 0 else None)
                f = thr.GetFrameAtIndex(fidx) if thr and thr.IsValid() else None
                if not f or not f.IsValid():
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': 'No active stack frame'})
                    return
                res = f.GetValueForExpressionPath(expr)
                if not res or not res.IsValid() or res.GetError().Fail():
                    res = f.EvaluateExpression(expr)
                if not res or not res.IsValid() or res.GetError().Fail():
                    msg = res.GetError().GetCString() if res else 'evaluation failed'
                    _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': msg or 'evaluation failed'})
                    return
                disp, tname, cref = self._format_sbval(res)
                _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {'result': disp, 'type': tname, 'variablesReference': cref}})
        elif cmd in ('continue', 'next', 'stepIn', 'stepOut', 'pause'):
            tid = int(args.get('threadId') or 1)
            with _state_lock:
                if self.process and self.process.IsValid():
                    thr = self.process.GetThreadByID(tid)
                    if not thr or not thr.IsValid():
                        thr = self.process.GetThreadAtIndex(0)
                    if cmd == 'continue':
                        self.process.Continue()
                    elif cmd == 'pause':
                        self.process.Stop()
                    elif thr and thr.IsValid():
                        if cmd == 'next':
                            thr.StepOver()
                        elif cmd == 'stepIn':
                            thr.StepInto()
                        elif cmd == 'stepOut':
                            thr.StepOut()
            body = {'allThreadsContinued': True} if cmd == 'continue' else {}
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': body})
        elif cmd in ('disconnect', 'terminate'):
            with _state_lock:
                if self.process and self.process.IsValid():
                    # Stop on an attach session detaches (DAP terminateDebuggee
                    # defaults to false for attach); only a launch is killed.
                    if self._detach_on_stop(cmd, args):
                        self.process.Detach()
                    else:
                        self.process.Kill()
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            _send({'type': 'event', 'event': 'terminated', 'body': {}})
            lldb.SBDebugger.Destroy(self.dbg)
            os._exit(0)
        else:
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})

bridge = _LldbDapBridge()
while True:
    m = _recv()
    if m is None:
        if bridge.process and bridge.process.IsValid():
            if getattr(bridge, 'is_attach', False):
                bridge.process.Detach()
            else:
                bridge.process.Kill()
        os._exit(0)
    bridge.handle(m)
"#;

#[cfg(all(test, unix))]
mod tests {
    use std::process::Stdio;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::process::Command;

    fn frame(v: serde_json::Value) -> Vec<u8> {
        let body = v.to_string();
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    /// RC6-DBG-3: Stop on an attach session detaches; the process keeps running.
    #[tokio::test]
    async fn disconnect_detaches_an_attached_process_instead_of_killing_it() {
        let Ok(p) = Command::new("lldb").arg("-P").output().await else {
            return; // no lldb on this host
        };
        let py_dir = String::from_utf8_lossy(&p.stdout).trim().to_string();
        let tmp = tempfile::tempdir().unwrap();
        let spin = tmp.path().join("spin");
        let mut cc = Command::new("cc")
            .args(["-x", "c", "-", "-o"])
            .arg(&spin)
            .stdin(Stdio::piped())
            .spawn();
        let Ok(cc) = cc.as_mut() else { return };
        let mut stdin = cc.stdin.take().unwrap();
        stdin
            .write_all(b"#include <unistd.h>\nint main(void) { for (;;) sleep(1); }\n")
            .await
            .unwrap();
        drop(stdin);
        if !p.status.success() || py_dir.is_empty() || !cc.wait().await.unwrap().success() {
            return;
        }
        let mut target = Command::new(&spin).kill_on_drop(true).spawn().unwrap();
        let pid = target.id().unwrap();
        // The interpreter lldb's Python module is built for, as the adapter picks it.
        let python = [
            "/usr/bin/python3",
            "/Library/Developer/CommandLineTools/usr/bin/python3",
        ]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
        .unwrap_or("python3");
        let Ok(mut bridge) = Command::new(python)
            .arg("-c")
            .arg(super::LLDB_PYTHON_DAP_BRIDGE)
            .env("PYTHONPATH", &py_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        else {
            return;
        };
        let mut to = bridge.stdin.take().unwrap();
        let mut from = bridge.stdout.take().unwrap();
        for req in [
            serde_json::json!({"seq": 1, "type": "request", "command": "initialize", "arguments": {}}),
            serde_json::json!({"seq": 2, "type": "request", "command": "attach", "arguments": {"pid": pid}}),
            serde_json::json!({"seq": 3, "type": "request", "command": "configurationDone"}),
        ] {
            to.write_all(&frame(req)).await.unwrap();
        }
        let mut seen = String::new();
        let attached = tokio::time::timeout(Duration::from_secs(30), async {
            let mut buf = [0u8; 4096];
            while !seen.contains("\"configurationDone\"") {
                let n = from.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        })
        .await;
        if attached.is_err()
            || !seen.contains("\"success\": true, \"command\": \"configurationDone\"")
        {
            eprintln!("attach not possible on this host: {seen}");
            return;
        }
        let stop = serde_json::json!({"seq": 4, "type": "request", "command": "disconnect", "arguments": {"terminateDebuggee": false}});
        to.write_all(&frame(stop)).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(15), bridge.wait()).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            target.try_wait().unwrap().is_none(),
            "the attached process must keep running after Stop"
        );
        let _ = target.kill().await;
    }
}
