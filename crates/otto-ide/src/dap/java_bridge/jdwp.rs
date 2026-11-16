//! Direct binary Java Debug Wire Protocol (JDWP) client over loopback TCP.
//!
//! Implements the Oracle Java SE JDWP binary framing (`JDWP-Handshake`, 11-byte
//! big-endian headers, dynamic `VirtualMachine.IDSizes`, `ClassesBySignature`,
//! `AllClasses`, `Method.LineTable`, `Method.VariableTable`, `ThreadReference.Frames`,
//! `StackFrame.GetValues`/`ThisObject`, `ObjectReference.GetValues`, `StringReference.Value`,
//! `ArrayReference.GetValues`, `EventRequest.Set`/`Clear`, and `Composite` event decoding).

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

pub(super) const EVENT_SINGLE_STEP: u8 = 1;
pub(super) const EVENT_BREAKPOINT: u8 = 2;
pub(super) const EVENT_EXCEPTION: u8 = 4;
pub(super) const EVENT_CLASS_PREPARE: u8 = 8;
pub(super) const EVENT_VM_START: u8 = 90;
pub(super) const EVENT_VM_DEATH: u8 = 99;

pub(super) const SUSPEND_ALL: u8 = 2;

pub(super) const STEP_SIZE_LINE: i32 = 1;
pub(super) const STEP_DEPTH_INTO: i32 = 0;
pub(super) const STEP_DEPTH_OVER: i32 = 1;
pub(super) const STEP_DEPTH_OUT: i32 = 2;

#[derive(Debug, Clone, Copy)]
pub(super) struct IdSizes {
    pub field_id: usize,
    pub method_id: usize,
    pub object_id: usize,
    pub ref_type_id: usize,
    pub frame_id: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct JdwpLocation {
    pub type_tag: u8,
    pub class_id: u64,
    pub method_id: u64,
    pub index: u64,
}

#[derive(Debug, Clone)]
pub(super) enum JdwpValue {
    Byte(i8),
    Char(char),
    Float(f32),
    Double(f64),
    Int(i32),
    Long(i64),
    Short(i16),
    Bool(bool),
    Void,
    Null,
    StringObj(u64),
    ArrayObj(u64),
    Object { id: u64 },
}

#[derive(Debug, Clone)]
pub(super) enum JdwpEvent {
    VmStart,
    VmDeath,
    ClassPrepare {
        ref_type_tag: u8,
        type_id: u64,
        signature: String,
    },
    Breakpoint {
        request_id: i32,
        thread: u64,
    },
    SingleStep {
        request_id: i32,
        thread: u64,
    },
    Exception {
        thread: u64,
    },
}

#[derive(Debug, Clone)]
pub(super) struct JdwpMethodInfo {
    pub method_id: u64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub(super) struct JdwpVarSlot {
    pub code_index: u64,
    pub name: String,
    pub signature: String,
    pub length: u32,
    pub slot: i32,
}

#[derive(Debug, Clone)]
pub(super) struct JdwpFieldInfo {
    pub field_id: u64,
    pub name: String,
    pub signature: String,
    pub mod_bits: i32,
}

pub(super) struct JdwpBuf {
    buf: Vec<u8>,
}

impl JdwpBuf {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(64),
        }
    }

    pub fn put_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn put_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_id(&mut self, val: u64, width: usize) {
        let be = val.to_be_bytes();
        let start = 8usize.saturating_sub(width.min(8));
        self.buf.extend_from_slice(&be[start..]);
    }

    pub fn put_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        self.buf
            .extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        self.buf.extend_from_slice(bytes);
    }

    pub fn put_location(&mut self, loc: &JdwpLocation, sz: &IdSizes) {
        self.put_u8(loc.type_tag);
        self.put_id(loc.class_id, sz.ref_type_id);
        self.put_id(loc.method_id, sz.method_id);
        self.buf.extend_from_slice(&loc.index.to_be_bytes());
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

pub(super) struct JdwpCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> JdwpCursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "truncated JDWP payload",
            ));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn i16(&mut self) -> io::Result<i16> {
        let b = self.take(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u16(&mut self) -> io::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn i32(&mut self) -> io::Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u32(&mut self) -> io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i64(&mut self) -> io::Result<i64> {
        let b = self.take(8)?;
        Ok(i64::from_be_bytes(b.try_into().unwrap()))
    }

    pub fn u64(&mut self) -> io::Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes(b.try_into().unwrap()))
    }

    pub fn id(&mut self, width: usize) -> io::Result<u64> {
        let w = width.clamp(1, 8);
        let b = self.take(w)?;
        let mut out = 0u64;
        for &byte in b {
            out = (out << 8) | u64::from(byte);
        }
        Ok(out)
    }

    pub fn string(&mut self) -> io::Result<String> {
        let len = self.u32()? as usize;
        let b = self.take(len)?;
        Ok(String::from_utf8_lossy(b).into_owned())
    }

    pub fn location(&mut self, sz: &IdSizes) -> io::Result<JdwpLocation> {
        Ok(JdwpLocation {
            type_tag: self.u8()?,
            class_id: self.id(sz.ref_type_id)?,
            method_id: self.id(sz.method_id)?,
            index: self.u64()?,
        })
    }

    pub fn tagged_value(&mut self, sz: &IdSizes) -> io::Result<JdwpValue> {
        let tag = self.u8()?;
        self.untagged_value(tag, sz)
    }

    pub fn untagged_value(&mut self, tag: u8, sz: &IdSizes) -> io::Result<JdwpValue> {
        match tag {
            b'B' => Ok(JdwpValue::Byte(self.u8()? as i8)),
            b'C' => Ok(JdwpValue::Char(
                char::from_u32(u32::from(self.u16()?)).unwrap_or('?'),
            )),
            b'F' => Ok(JdwpValue::Float(f32::from_bits(self.u32()?))),
            b'D' => Ok(JdwpValue::Double(f64::from_bits(self.u64()?))),
            b'I' => Ok(JdwpValue::Int(self.i32()?)),
            b'J' => Ok(JdwpValue::Long(self.i64()?)),
            b'S' => Ok(JdwpValue::Short(self.i16()?)),
            b'Z' => Ok(JdwpValue::Bool(self.u8()? != 0)),
            b'V' => Ok(JdwpValue::Void),
            b's' => {
                let id = self.id(sz.object_id)?;
                Ok(if id == 0 {
                    JdwpValue::Null
                } else {
                    JdwpValue::StringObj(id)
                })
            }
            b'[' => {
                let id = self.id(sz.object_id)?;
                Ok(if id == 0 {
                    JdwpValue::Null
                } else {
                    JdwpValue::ArrayObj(id)
                })
            }
            b'L' | b't' | b'g' | b'l' | b'c' => {
                let id = self.id(sz.object_id)?;
                Ok(if id == 0 {
                    JdwpValue::Null
                } else {
                    JdwpValue::Object { id }
                })
            }
            _ => Ok(JdwpValue::Void),
        }
    }
}

type PendingReplyMap = Arc<Mutex<HashMap<u32, oneshot::Sender<(u16, Vec<u8>)>>>>;

pub(super) struct JdwpClient {
    writer: tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>,
    pending: PendingReplyMap,
    next_id: AtomicU32,
    pub sizes: IdSizes,
}

impl JdwpClient {
    pub async fn connect(
        port: u16,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Option<Vec<u8>>>)> {
        let mut tcp = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| Error::Upstream(format!("connect JDWP 127.0.0.1:{port}: {e}")))?;
        let _ = tcp.set_nodelay(true);
        tcp.write_all(b"JDWP-Handshake")
            .await
            .map_err(|e| Error::Upstream(format!("JDWP handshake write: {e}")))?;
        let mut hs = [0u8; 14];
        tcp.read_exact(&mut hs)
            .await
            .map_err(|e| Error::Upstream(format!("JDWP handshake read: {e}")))?;
        if &hs != b"JDWP-Handshake" {
            return Err(Error::Upstream("JVM rejected JDWP-Handshake".into()));
        }

        let (mut r_half, w_half) = tcp.into_split();
        let pending: PendingReplyMap = Arc::new(Mutex::new(HashMap::new()));
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let pending_reader = Arc::clone(&pending);

        tokio::spawn(async move {
            loop {
                let mut hdr = [0u8; 11];
                if r_half.read_exact(&mut hdr).await.is_err() {
                    break;
                }
                let len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
                if !(11..=16 * 1024 * 1024).contains(&len) {
                    break;
                }
                let id = u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
                let flags = hdr[8];
                let mut body = vec![0u8; len - 11];
                if !body.is_empty() && r_half.read_exact(&mut body).await.is_err() {
                    break;
                }
                if (flags & 0x80) != 0 {
                    let err_code = u16::from_be_bytes([hdr[9], hdr[10]]);
                    let tx = pending_reader.lock().ok().and_then(|mut m| m.remove(&id));
                    if let Some(tx) = tx {
                        let _ = tx.send((err_code, body));
                    }
                } else if hdr[9] == 64 && hdr[10] == 100 {
                    let _ = ev_tx.send(Some(body));
                }
            }
            if let Ok(mut m) = pending_reader.lock() {
                m.clear();
            }
            let _ = ev_tx.send(None);
        });

        let mut client = Self {
            writer: tokio::sync::Mutex::new(w_half),
            pending,
            next_id: AtomicU32::new(1),
            sizes: IdSizes {
                field_id: 8,
                method_id: 8,
                object_id: 8,
                ref_type_id: 8,
                frame_id: 8,
            },
        };
        let (_, id_bytes) = client
            .cmd(1, 7, &[])
            .await
            .map_err(|e| Error::Upstream(format!("JDWP IDSizes failed: {e}")))?;
        let mut cur = JdwpCursor::new(&id_bytes);
        if let (Ok(f), Ok(m), Ok(o), Ok(r), Ok(fr)) =
            (cur.i32(), cur.i32(), cur.i32(), cur.i32(), cur.i32())
        {
            client.sizes = IdSizes {
                field_id: f as usize,
                method_id: m as usize,
                object_id: o as usize,
                ref_type_id: r as usize,
                frame_id: fr as usize,
            };
        }
        Ok((Arc::new(client), ev_rx))
    }

    pub async fn cmd(&self, cmd_set: u8, cmd: u8, payload: &[u8]) -> io::Result<(u16, Vec<u8>)> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut m) = self.pending.lock() {
            m.insert(id, tx);
        }
        let total_len = (11 + payload.len()) as u32;
        let mut pkt = Vec::with_capacity(11 + payload.len());
        pkt.extend_from_slice(&total_len.to_be_bytes());
        pkt.extend_from_slice(&id.to_be_bytes());
        pkt.push(0x00);
        pkt.push(cmd_set);
        pkt.push(cmd);
        pkt.extend_from_slice(payload);
        {
            let mut w = self.writer.lock().await;
            if let Err(e) = w.write_all(&pkt).await {
                if let Ok(mut m) = self.pending.lock() {
                    m.remove(&id);
                }
                return Err(e);
            }
            let _ = w.flush().await;
        }
        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(io::Error::new(ErrorKind::BrokenPipe, "JDWP socket closed")),
            Err(_) => {
                if let Ok(mut m) = self.pending.lock() {
                    m.remove(&id);
                }
                Err(io::Error::new(
                    ErrorKind::TimedOut,
                    "JDWP command timed out",
                ))
            }
        }
    }

    pub async fn vm_resume(&self) -> io::Result<()> {
        let _ = self.cmd(1, 9, &[]).await?;
        Ok(())
    }

    pub async fn vm_suspend(&self) -> io::Result<()> {
        let _ = self.cmd(1, 8, &[]).await?;
        Ok(())
    }

    pub async fn vm_exit(&self, code: i32) {
        let mut b = JdwpBuf::new();
        b.put_i32(code);
        let _ =
            tokio::time::timeout(Duration::from_millis(300), self.cmd(1, 10, &b.into_vec())).await;
    }

    pub async fn all_classes(&self) -> io::Result<Vec<(u8, u64, String, i32)>> {
        let (ec, data) = self.cmd(1, 3, &[]).await?;
        if ec != 0 {
            return Ok(Vec::new());
        }
        let mut cur = JdwpCursor::new(&data);
        let cnt = cur.i32()? as usize;
        let mut out = Vec::with_capacity(cnt.min(4096));
        for _ in 0..cnt {
            let tag = cur.u8()?;
            let tid = cur.id(self.sizes.ref_type_id)?;
            let sig = cur.string()?;
            let status = cur.i32()?;
            out.push((tag, tid, sig, status));
        }
        Ok(out)
    }

    pub async fn methods(&self, class_id: u64) -> io::Result<Vec<JdwpMethodInfo>> {
        let mut b = JdwpBuf::new();
        b.put_id(class_id, self.sizes.ref_type_id);
        let (ec, data) = self.cmd(2, 5, &b.into_vec()).await?;
        if ec != 0 {
            return Ok(Vec::new());
        }
        let mut cur = JdwpCursor::new(&data);
        let cnt = cur.i32()? as usize;
        let mut out = Vec::with_capacity(cnt);
        for _ in 0..cnt {
            let mid = cur.id(self.sizes.method_id)?;
            let name = cur.string()?;
            let _sig = cur.string()?;
            let _mods = cur.i32()?;
            out.push(JdwpMethodInfo {
                method_id: mid,
                name,
            });
        }
        Ok(out)
    }

    pub async fn line_table(&self, class_id: u64, method_id: u64) -> io::Result<Vec<(u64, u32)>> {
        let mut b = JdwpBuf::new();
        b.put_id(class_id, self.sizes.ref_type_id);
        b.put_id(method_id, self.sizes.method_id);
        let (ec, data) = self.cmd(6, 1, &b.into_vec()).await?;
        if ec != 0 {
            return Ok(Vec::new());
        }
        let mut cur = JdwpCursor::new(&data);
        let _start = cur.i64()?;
        let _end = cur.i64()?;
        let cnt = cur.i32()? as usize;
        let mut out = Vec::with_capacity(cnt);
        for _ in 0..cnt {
            let idx = cur.i64()?;
            let line = cur.i32()?;
            if idx >= 0 && line > 0 {
                out.push((idx as u64, line as u32));
            }
        }
        Ok(out)
    }

    pub async fn variable_table(
        &self,
        class_id: u64,
        method_id: u64,
    ) -> io::Result<Vec<JdwpVarSlot>> {
        let mut b = JdwpBuf::new();
        b.put_id(class_id, self.sizes.ref_type_id);
        b.put_id(method_id, self.sizes.method_id);
        let (ec, data) = self.cmd(6, 2, &b.into_vec()).await?;
        if ec != 0 {
            return Ok(Vec::new());
        }
        let mut cur = JdwpCursor::new(&data);
        let _arg_cnt = cur.i32()?;
        let slots = cur.i32()? as usize;
        let mut out = Vec::with_capacity(slots);
        for _ in 0..slots {
            let code_index = cur.u64()?;
            let name = cur.string()?;
            let signature = cur.string()?;
            let length = cur.u32()?;
            let slot = cur.i32()?;
            out.push(JdwpVarSlot {
                code_index,
                name,
                signature,
                length,
                slot,
            });
        }
        Ok(out)
    }

    pub async fn fields(&self, class_id: u64) -> io::Result<Vec<JdwpFieldInfo>> {
        let mut b = JdwpBuf::new();
        b.put_id(class_id, self.sizes.ref_type_id);
        let (ec, data) = self.cmd(2, 4, &b.into_vec()).await?;
        if ec != 0 {
            return Ok(Vec::new());
        }
        let mut cur = JdwpCursor::new(&data);
        let cnt = cur.i32()? as usize;
        let mut out = Vec::with_capacity(cnt);
        for _ in 0..cnt {
            out.push(JdwpFieldInfo {
                field_id: cur.id(self.sizes.field_id)?,
                name: cur.string()?,
                signature: cur.string()?,
                mod_bits: cur.i32()?,
            });
        }
        Ok(out)
    }

    pub async fn class_signature(&self, class_id: u64) -> Option<String> {
        let mut b = JdwpBuf::new();
        b.put_id(class_id, self.sizes.ref_type_id);
        let (ec, data) = self.cmd(2, 1, &b.into_vec()).await.ok()?;
        (ec == 0)
            .then(|| JdwpCursor::new(&data).string().ok())
            .flatten()
    }

    pub async fn source_file(&self, class_id: u64) -> Option<String> {
        let mut b = JdwpBuf::new();
        b.put_id(class_id, self.sizes.ref_type_id);
        let (ec, data) = self.cmd(2, 7, &b.into_vec()).await.ok()?;
        (ec == 0)
            .then(|| JdwpCursor::new(&data).string().ok())
            .flatten()
    }

    pub async fn string_value(&self, obj_id: u64) -> Option<String> {
        let mut b = JdwpBuf::new();
        b.put_id(obj_id, self.sizes.object_id);
        let (ec, data) = self.cmd(10, 1, &b.into_vec()).await.ok()?;
        (ec == 0)
            .then(|| JdwpCursor::new(&data).string().ok())
            .flatten()
    }

    pub async fn clear_event(&self, event_kind: u8, request_id: i32) {
        let mut b = JdwpBuf::new();
        b.put_u8(event_kind);
        b.put_i32(request_id);
        let _ = self.cmd(15, 2, &b.into_vec()).await;
    }

    pub fn parse_composite(&self, body: &[u8]) -> io::Result<(u8, Vec<JdwpEvent>)> {
        let mut cur = JdwpCursor::new(body);
        let suspend_policy = cur.u8()?;
        let count = cur.i32()? as usize;
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            let ek = cur.u8()?;
            let req_id = cur.i32()?;
            match ek {
                EVENT_VM_START => {
                    let _thread = cur.id(self.sizes.object_id)?;
                    events.push(JdwpEvent::VmStart);
                }
                EVENT_VM_DEATH => {
                    events.push(JdwpEvent::VmDeath);
                }
                EVENT_CLASS_PREPARE => {
                    let _thread = cur.id(self.sizes.object_id)?;
                    let ref_type_tag = cur.u8()?;
                    let type_id = cur.id(self.sizes.ref_type_id)?;
                    let signature = cur.string()?;
                    let _status = cur.i32()?;
                    events.push(JdwpEvent::ClassPrepare {
                        ref_type_tag,
                        type_id,
                        signature,
                    });
                }
                EVENT_BREAKPOINT => {
                    let thread = cur.id(self.sizes.object_id)?;
                    let _location = cur.location(&self.sizes)?;
                    events.push(JdwpEvent::Breakpoint {
                        request_id: req_id,
                        thread,
                    });
                }
                EVENT_SINGLE_STEP => {
                    let thread = cur.id(self.sizes.object_id)?;
                    let _location = cur.location(&self.sizes)?;
                    events.push(JdwpEvent::SingleStep {
                        request_id: req_id,
                        thread,
                    });
                }
                EVENT_EXCEPTION => {
                    let thread = cur.id(self.sizes.object_id)?;
                    let _location = cur.location(&self.sizes)?;
                    let _tag = cur.u8()?;
                    let _exception_id = cur.id(self.sizes.object_id)?;
                    let _catch_loc = cur.location(&self.sizes)?;
                    events.push(JdwpEvent::Exception { thread });
                }
                _ => break,
            }
        }
        Ok((suspend_policy, events))
    }
}
