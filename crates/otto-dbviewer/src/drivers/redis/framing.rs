//! Incremental RESP envelope validation, before redis-rs receives plaintext.
//! No values are decoded/retained here. Frame budgets reset at reply boundaries;
//! the shared operation budget also caps the sum of a pipeline's replies.
use std::{
    io,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(super) struct Budget {
    bytes: usize,
    nodes: usize,
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub bytes: usize,
    pub nodes: usize,
    pub depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 32 * 1024 * 1024,
            nodes: 100_000,
            depth: 64,
        }
    }
}

enum Phase {
    Prefix,
    Line { kind: u8, number: Vec<u8>, cr: bool },
    Bulk(usize),
    BulkCr,
    BulkLf,
}

pub(super) struct Frames {
    limits: Limits,
    pub budget: Arc<Mutex<Budget>>,
    phase: Phase,
    bytes: usize,
    nodes: usize,
    remaining: Vec<usize>,
}
impl Frames {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            budget: Arc::new(Mutex::new(Budget::default())),
            phase: Phase::Prefix,
            bytes: 0,
            nodes: 0,
            remaining: Vec::new(),
        }
    }
    fn charge(&mut self, bytes: usize, budget: &mut Budget) -> io::Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(limit)?;
        budget.bytes = budget.bytes.checked_add(bytes).ok_or_else(limit)?;
        if self.bytes > self.limits.bytes || budget.bytes > self.limits.bytes {
            return Err(limit());
        }
        Ok(())
    }
    fn complete(&mut self) {
        while let Some(left) = self.remaining.last_mut() {
            *left -= 1;
            if *left != 0 {
                self.phase = Phase::Prefix;
                return;
            }
            self.remaining.pop();
        }
        self.bytes = 0;
        self.nodes = 0;
        self.phase = Phase::Prefix;
    }
    pub fn accept(&mut self, mut input: &[u8]) -> io::Result<()> {
        let budget = self.budget.clone();
        let mut budget = budget.lock().unwrap_or_else(|e| e.into_inner());
        while !input.is_empty() {
            if let Phase::Bulk(left) = self.phase {
                let take = left.min(input.len());
                self.charge(take, &mut budget)?;
                input = &input[take..];
                self.phase = if left == take {
                    Phase::BulkCr
                } else {
                    Phase::Bulk(left - take)
                };
                continue;
            }
            let byte = input[0];
            input = &input[1..];
            self.charge(1, &mut budget)?;
            let phase = std::mem::replace(&mut self.phase, Phase::Prefix);
            match phase {
                Phase::Prefix => {
                    self.nodes += 1;
                    budget.nodes += 1;
                    if self.nodes > self.limits.nodes || budget.nodes > self.limits.nodes {
                        return Err(limit());
                    }
                    if !b"+-:,#_($!=*~>%|".contains(&byte) {
                        return Err(invalid());
                    }
                    self.phase = Phase::Line {
                        kind: byte,
                        number: Vec::new(),
                        cr: false,
                    };
                }
                Phase::Line {
                    kind,
                    mut number,
                    cr,
                } => {
                    if cr {
                        if byte != b'\n' {
                            return Err(invalid());
                        }
                        self.line(kind, &number, &budget)?;
                    } else if byte == b'\r' {
                        self.phase = Phase::Line {
                            kind,
                            number,
                            cr: true,
                        };
                    } else {
                        if byte == b'\n' {
                            return Err(invalid());
                        }
                        if b"$!=*~>%|".contains(&kind) {
                            if number.len() >= 20
                                || !(byte.is_ascii_digit() || (byte == b'-' && number.is_empty()))
                            {
                                return Err(invalid());
                            }
                            number.push(byte);
                        }
                        self.phase = Phase::Line {
                            kind,
                            number,
                            cr: false,
                        };
                    }
                }
                Phase::BulkCr => {
                    if byte != b'\r' {
                        return Err(invalid());
                    }
                    self.phase = Phase::BulkLf;
                }
                Phase::BulkLf => {
                    if byte != b'\n' {
                        return Err(invalid());
                    }
                    self.complete();
                }
                Phase::Bulk(_) => unreachable!(),
            }
        }
        Ok(())
    }
    fn line(&mut self, kind: u8, number: &[u8], budget: &Budget) -> io::Result<()> {
        if !b"$!=*~>%|".contains(&kind) {
            self.complete();
            return Ok(());
        }
        let length: i64 = std::str::from_utf8(number)
            .map_err(|_| invalid())?
            .parse()
            .map_err(|_| invalid())?;
        if length < 0 && matches!(kind, b'$' | b'*' | b'~' | b'>') {
            self.complete();
            return Ok(());
        }
        let length = usize::try_from(length).map_err(|_| invalid())?;
        if b"$!=".contains(&kind) {
            if length
                .checked_add(2)
                .and_then(|n| n.checked_add(budget.bytes))
                .is_none_or(|n| n > self.limits.bytes)
            {
                return Err(limit());
            }
            self.phase = if length == 0 {
                Phase::BulkCr
            } else {
                Phase::Bulk(length)
            };
        } else {
            let children = match kind {
                b'%' => length.checked_mul(2),
                // RESP3 attributes wrap their map AND the following value.
                b'|' => length.checked_mul(2).and_then(|n| n.checked_add(1)),
                _ => Some(length),
            }
            .ok_or_else(limit)?;
            if children > self.limits.nodes.saturating_sub(budget.nodes)
                || self.remaining.len() >= self.limits.depth
            {
                return Err(limit());
            }
            if children == 0 {
                self.complete();
            } else {
                self.remaining.push(children);
                self.phase = Phase::Prefix;
            }
        }
        Ok(())
    }
}
fn limit() -> io::Error {
    io::Error::other("Redis response exceeds the receive budget (32 MiB, 100000 values, 64 nesting levels); use GETRANGE or smaller range/SCAN reads and retry. Connection retired; earlier writes may already have applied.")
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid RESP response framing")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn small() -> Frames {
        Frames::new(Limits {
            bytes: 64,
            nodes: 8,
            depth: 3,
        })
    }
    #[test]
    fn fragmented_frames_and_pipeline_boundaries() {
        let wire = b"*2\r\n$3\r\nfoo\r\n*2\r\n:1\r\n$-1\r\n+OK\r\n%1\r\n+k\r\n#t\r\n|1\r\n+k\r\n+v\r\n$0\r\n\r\n";
        for size in 1..=wire.len() {
            let mut frame = Frames::new(Limits {
                bytes: 1024,
                nodes: 100,
                depth: 3,
            });
            for chunk in wire.chunks(size) {
                frame.accept(chunk).unwrap();
            }
            assert_eq!(frame.bytes, 0);
        }
    }
    #[test]
    fn rejects_declarations_before_receiving_the_payload() {
        for header in [
            "$1000000000\r\n",
            "*1000000000\r\n",
            "%1000000000\r\n",
            "*2\r\n$60\r\n",
            "$99999999999999999999999\r\n",
        ] {
            assert!(small().accept(header.as_bytes()).is_err(), "{header}");
        }
    }
    #[test]
    fn aggregate_node_depth_and_simple_line_limits() {
        assert!(small()
            .accept(b"*2\r\n*4\r\n:0\r\n:0\r\n:0\r\n:0\r\n*2\r\n")
            .is_err());
        assert!(small().accept(b"*1\r\n*1\r\n*1\r\n*1\r\n").is_err());
        assert!(small()
            .accept(format!("+{}\r\n", "x".repeat(65)).as_bytes())
            .is_err());
        assert!(small().accept(b"$3\r\nfooXX").is_err());
    }

    #[test]
    fn all_supported_resp_value_shapes_survive_fragmentation() {
        for wire in [
            "+hello\r\n",
            "-ERR fixture\r\n",
            ":42\r\n",
            ",1.25\r\n",
            "#f\r\n",
            "_\r\n",
            "(123456789012345678901\r\n",
            "$3\r\nfoo\r\n",
            "$-1\r\n",
            "!11\r\nERR fixture\r\n",
            "=7\r\ntxt:abc\r\n",
            "~2\r\n:1\r\n:2\r\n",
            ">2\r\n+message\r\n+payload\r\n",
            "|1\r\n+k\r\n+v\r\n+data\r\n",
            "%1\r\n+k\r\n+v\r\n",
            "*0\r\n",
            "~-1\r\n",
            ">0\r\n",
        ] {
            assert!(redis::parse_redis_value(wire.as_bytes()).is_ok(), "{wire}");
            let mut frames = Frames::new(Limits::default());
            for byte in wire.as_bytes() {
                frames.accept(&[*byte]).unwrap();
            }
            assert_eq!(frames.bytes, 0, "{wire}");
        }
    }

    #[test]
    fn pipeline_totals_cannot_evade_limits_by_resetting_each_frame() {
        assert!(small().accept(b":0\r\n".repeat(9).as_slice()).is_err());
        assert!(small()
            .accept(format!("+{}\r\n+{}\r\n", "x".repeat(35), "y".repeat(35)).as_bytes())
            .is_err());
    }
}
