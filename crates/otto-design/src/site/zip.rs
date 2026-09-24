//! A tiny, dependency-free ZIP writer (PKWARE APPNOTE 6.3, "stored" entries
//! only — HTML/CSS compress well but a static site is small, and this keeps
//! the crate free of a compression dependency). UTF-8 names (flag bit 11),
//! one fixed timestamp for every entry (the published version's time), no
//! ZIP64 (the exporter caps the archive far below 4 GiB). Pure; unit-tested
//! with a reader that checks names, sizes and CRCs.

/// CRC-32 (IEEE 802.3, reflected, poly 0xEDB88320).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// MS-DOS time + date of a UTC timestamp (clamped to 1980..=2107).
pub fn dos_datetime(t: chrono::DateTime<chrono::Utc>) -> (u16, u16) {
    use chrono::{Datelike, Timelike};
    let year = t.year().clamp(1980, 2107) as u32;
    let time = (t.hour() << 11) | (t.minute() << 5) | (t.second() / 2);
    let date = ((year - 1980) << 9) | (t.month() << 5) | t.day();
    (time as u16, date as u16)
}

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Build a ZIP archive of `files` (`(path, bytes)`, `/`-separated relative
/// paths) with every entry stamped `(time, date)` (see [`dos_datetime`]).
pub fn write(files: &[(String, Vec<u8>)], (time, date): (u16, u16)) -> Vec<u8> {
    const FLAG_UTF8: u16 = 1 << 11;
    const VERSION: u16 = 20;
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let name_bytes = name.as_bytes();
        // Local file header.
        u32le(&mut out, 0x0403_4b50);
        u16le(&mut out, VERSION);
        u16le(&mut out, FLAG_UTF8);
        u16le(&mut out, 0); // stored
        u16le(&mut out, time);
        u16le(&mut out, date);
        u32le(&mut out, crc);
        u32le(&mut out, size);
        u32le(&mut out, size);
        u16le(&mut out, name_bytes.len() as u16);
        u16le(&mut out, 0);
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);
        // Central directory entry.
        u32le(&mut central, 0x0201_4b50);
        u16le(&mut central, VERSION); // made by
        u16le(&mut central, VERSION); // needed
        u16le(&mut central, FLAG_UTF8);
        u16le(&mut central, 0);
        u16le(&mut central, time);
        u16le(&mut central, date);
        u32le(&mut central, crc);
        u32le(&mut central, size);
        u32le(&mut central, size);
        u16le(&mut central, name_bytes.len() as u16);
        u16le(&mut central, 0); // extra
        u16le(&mut central, 0); // comment
        u16le(&mut central, 0); // disk
        u16le(&mut central, 0); // internal attrs
        u32le(&mut central, 0); // external attrs
        u32le(&mut central, offset);
        central.extend_from_slice(name_bytes);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    // End of central directory.
    u32le(&mut out, 0x0605_4b50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, files.len() as u16);
    u16le(&mut out, files.len() as u16);
    u32le(&mut out, cd_size);
    u32le(&mut out, cd_offset);
    u16le(&mut out, 0);
    out
}

/// Read a stored archive back: `(name, bytes)` per central-directory entry,
/// with each entry's CRC verified. `None` when anything doesn't add up.
/// (Tests and sanity checks — the daemon never reads user ZIPs with this.)
pub fn read(zip: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let u16at =
        |i: usize| -> Option<u16> { Some(u16::from_le_bytes(zip.get(i..i + 2)?.try_into().ok()?)) };
    let u32at =
        |i: usize| -> Option<u32> { Some(u32::from_le_bytes(zip.get(i..i + 4)?.try_into().ok()?)) };
    let eocd = zip.len().checked_sub(22)?;
    if u32at(eocd)? != 0x0605_4b50 {
        return None;
    }
    let count = usize::from(u16at(eocd + 10)?);
    let mut at = u32at(eocd + 16)? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32at(at)? != 0x0201_4b50 {
            return None;
        }
        let crc = u32at(at + 16)?;
        let size = u32at(at + 24)? as usize;
        let name_len = usize::from(u16at(at + 28)?);
        let extra = usize::from(u16at(at + 30)?);
        let comment = usize::from(u16at(at + 32)?);
        let local = u32at(at + 42)? as usize;
        let name = String::from_utf8(zip.get(at + 46..at + 46 + name_len)?.to_vec()).ok()?;
        if u32at(local)? != 0x0403_4b50 {
            return None;
        }
        let lname = usize::from(u16at(local + 26)?);
        let lextra = usize::from(u16at(local + 28)?);
        let start = local + 30 + lname + lextra;
        let data = zip.get(start..start + size)?.to_vec();
        if crc32(&data) != crc {
            return None;
        }
        out.push((name, data));
        at += 46 + name_len + extra + comment;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_written_archive_reads_back_byte_for_byte() {
        let files = vec![
            (
                "index.html".to_string(),
                b"<!doctype html><p>Hi</p>".to_vec(),
            ),
            ("site.css".to_string(), b".os-site{}".to_vec()),
            (
                "assets/card-v7.png".to_string(),
                vec![0x89, 0x50, 0x4E, 0x47, 0, 1, 2, 255],
            ),
            ("empty.txt".to_string(), Vec::new()),
            ("תמונה.html".to_string(), "שלום".as_bytes().to_vec()),
        ];
        let when = chrono::Utc
            .with_ymd_and_hms(2026, 9, 23, 10, 30, 20)
            .unwrap();
        let zip = write(&files, dos_datetime(when));
        assert_eq!(
            &zip[..4],
            &[0x50, 0x4B, 0x03, 0x04],
            "starts with a local header"
        );
        let back = read(&zip).expect("readable");
        assert_eq!(back, files);
        // Corrupting a payload byte breaks its CRC.
        let mut bad = zip.clone();
        bad[30 + "index.html".len() + 2] ^= 0xFF;
        assert!(read(&bad).is_none());
    }

    #[test]
    fn dos_timestamps_pack_fields() {
        let t = chrono::Utc
            .with_ymd_and_hms(2026, 9, 23, 10, 30, 20)
            .unwrap();
        let (time, date) = dos_datetime(t);
        assert_eq!(time, (10 << 11) | (30 << 5) | 10);
        assert_eq!(date, ((2026 - 1980) << 9) | (9 << 5) | 23);
    }
}
