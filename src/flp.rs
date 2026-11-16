//! Lossless FL Studio project (.flp) reader/writer.
//!
//! An .flp is a `FLhd` header chunk followed by an `FLdt` chunk holding a flat
//! stream of events. The event id decides the payload: id < 64 → 1 byte,
//! < 128 → 2 bytes, < 192 → 4 bytes, otherwise a LEB128 length + that many bytes.
//! We keep every event as raw bytes so writing back is byte-identical.

pub struct Project {
    /// Everything before the `FLdt` chunk (the `FLhd` chunk).
    header: Vec<u8>,
    pub events: Vec<Event>,
}

pub struct Event {
    pub id: u8,
    pub data: Vec<u8>,
}

impl Project {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let bad = || "not an FL Studio project".to_string();
        if bytes.get(..4) != Some(b"FLhd") {
            return Err(bad());
        }
        let hlen = u32::from_le_bytes(bytes.get(4..8).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let dt = 8 + hlen;
        if bytes.get(dt..dt + 4) != Some(b"FLdt") {
            return Err(bad());
        }
        let len = u32::from_le_bytes(bytes.get(dt + 4..dt + 8).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let body = bytes.get(dt + 8..dt + 8 + len).ok_or("project file is truncated")?;

        let mut events = Vec::new();
        let mut pos = 0;
        while pos < body.len() {
            let id = body[pos];
            pos += 1;
            let n = match id {
                0..=63 => 1,
                64..=127 => 2,
                128..=191 => 4,
                _ => {
                    let (mut n, mut shift) = (0usize, 0);
                    loop {
                        let b = *body.get(pos).ok_or("project file is truncated")?;
                        pos += 1;
                        n |= ((b & 0x7f) as usize) << shift;
                        shift += 7;
                        if b < 0x80 {
                            break n;
                        }
                        if shift > 35 {
                            return Err("project file is corrupt".into());
                        }
                    }
                }
            };
            let data = body.get(pos..pos + n).ok_or("project file is truncated")?.to_vec();
            pos += n;
            events.push(Event { id, data });
        }
        Ok(Self { header: bytes[..dt].to_vec(), events })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut body = Vec::new();
        for e in &self.events {
            body.push(e.id);
            if e.id >= 192 {
                let mut n = e.data.len();
                loop {
                    let b = (n & 0x7f) as u8;
                    n >>= 7;
                    body.push(if n > 0 { b | 0x80 } else { b });
                    if n == 0 {
                        break;
                    }
                }
            }
            body.extend_from_slice(&e.data);
        }
        let mut out = self.header.clone();
        out.extend_from_slice(b"FLdt");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }
}

/// A list of `u32 id, u64 size, bytes` records, as used inside FL's plugin
/// wrapper data and its VST3 state.
pub type Records = Vec<(u32, Vec<u8>)>;

pub fn read_records(mut b: &[u8]) -> Option<Records> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let id = u32::from_le_bytes(b.get(..4)?.try_into().ok()?);
        // Untrusted: other plugins' state can look like a record list.
        let size = usize::try_from(u64::from_le_bytes(b.get(4..12)?.try_into().ok()?)).ok()?;
        let end = size.checked_add(12)?;
        out.push((id, b.get(12..end)?.to_vec()));
        b = &b[end..];
    }
    Some(out)
}

pub fn write_records(prefix: u32, records: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = prefix.to_le_bytes().to_vec();
    for (id, data) in records {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_reject_bogus_sizes() {
        let mut b = 1u32.to_le_bytes().to_vec();
        b.extend(u64::MAX.to_le_bytes());
        assert!(read_records(&b).is_none());
        let ok = write_records(0, &[(54, b"Serum".to_vec())]);
        assert_eq!(read_records(&ok[4..]), Some(vec![(54, b"Serum".to_vec())]));
    }
}
