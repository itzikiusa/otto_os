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
        self.logpoints = {}
        self.panic_bp_id = None
        self.frame_map = {}
        self.val_refs = {}
        self.next_ref = 1000
        self.suppress_pause_event = False

    def _alloc_val_ref(self, sbval):
        self.next_ref += 1
        self.val_refs[self.next_ref] = sbval
        return self.next_ref

    def _format_sbval(self, v):
        tname = v.GetDisplayTypeName() or v.GetTypeName() or ''
        summary = v.GetSummary()
        val = v.GetValue()
        if summary and val:
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
                'supportsTerminateRequest': True,
                'exceptionBreakpointFilters': [{'filter': 'rust_panic', 'label': 'Rust Panics'}]
            }})
        elif cmd in ('launch', 'attach'):
            self.launch_cfg = args
            prog = args.get('program') or ''
            if prog:
                self.target = self.dbg.CreateTargetWithFileAndArch(os.path.realpath(prog), lldb.LLDB_ARCH_DEFAULT)
            if not self.target or not self.target.IsValid():
                _send({'type': 'response', 'request_seq': rseq, 'success': False, 'command': cmd, 'message': f'LLDB failed to load target executable: {prog}'})
                return
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
            _send({'type': 'event', 'event': 'initialized', 'body': {}})
        elif cmd == 'setBreakpoints':
            self.apply_breakpoints(rseq, args)
        elif cmd == 'setExceptionBreakpoints':
            filters = args.get('filters') or []
            with _state_lock:
                if self.panic_bp_id and self.target and self.target.IsValid():
                    self.target.BreakpointDelete(self.panic_bp_id)
                    self.panic_bp_id = None
                if filters and self.target and self.target.IsValid():
                    pbp = self.target.BreakpointCreateByName('rust_panic')
                    self.panic_bp_id = pbp.GetID()
            _send({'type': 'response', 'request_seq': rseq, 'success': True, 'command': cmd, 'body': {}})
        elif cmd == 'configurationDone':
            argv = [str(x) for x in (self.launch_cfg.get('args') or [])]
            cwd = os.path.realpath(self.launch_cfg.get('cwd') or os.getcwd())
            li = lldb.SBLaunchInfo(argv)
            li.SetWorkingDirectory(cwd)
            li.SetListener(self.listener)
            env_list = [f'{k}={v}' for k, v in os.environ.items()]
            li.SetEnvironmentEntries(env_list, False)
            err = lldb.SBError()
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
            bridge.process.Kill()
        os._exit(0)
    bridge.handle(m)
"#;
