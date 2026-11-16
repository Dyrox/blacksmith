//! Ableton Live's own plugin database on this computer: every plugin Live has
//! scanned, with its ID — what Live can load here, the host source for `.als`.
//!
//! `Live Database/Live-plugins-1.db` (SQLite; Mac: `~/Library/Application
//! Support/Ableton`, Windows: `%LOCALAPPDATA%\Ableton`). Table `plugins` has
//! `dev_identifier`: `device:vst:<kind>:<unique ID>?n=<name>` for VST2,
//! `device:vst3:<kind>:<FUID with dashes>` for VST3 (`<kind>`: instr,
//! audiofx, …), plus name and vendor; `plugin_modules.path` is the file.
//! Opened read-only; Live may be running.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::library::{Entry, HostDb};

/// `BLACKSMITH_LIVEDB=<file>` points elsewhere, `=none` ignores it (debug aids).
pub fn locate() -> Option<PathBuf> {
    match std::env::var_os("BLACKSMITH_LIVEDB") {
        Some(v) if v == "none" => return None,
        Some(v) => return Some(PathBuf::from(v)),
        None => {}
    }
    let dir = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Ableton").join("Live Database")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Ableton/Live Database")
    };
    // Live-plugins-<n>.db; the highest schema number is the current one.
    let mut dbs: Vec<(u32, PathBuf)> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix("Live-plugins-")?.strip_suffix(".db")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    dbs.sort();
    dbs.pop().map(|(_, p)| p)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// A FUID written as hex (dashes allowed, FUID order) → the 16 bytes FL
/// stores, Blacksmith's form for a VST3 class ID.
pub fn fuid_to_com(hex: &str) -> Option<[u8; 16]> {
    let hex: String = hex.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut b = [0u8; 16];
    for (i, x) in b.iter_mut().enumerate() {
        *x = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    b[..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    Some(b)
}

/// What a `dev_identifier` names.
#[derive(Debug, PartialEq)]
pub enum Ident {
    Vst2 { id: u32, instrument: bool },
    Vst3 { class_id: [u8; 16], instrument: bool },
}

pub fn parse_ident(s: &str) -> Option<Ident> {
    let rest = s.strip_prefix("device:")?;
    let (format, rest) = rest.split_once(':')?;
    let (kind, id) = rest.split_once(':')?;
    let instrument = kind == "instr";
    match format {
        "vst" => {
            let id = id.split('?').next()?.parse::<i64>().ok()?;
            Some(Ident::Vst2 { id: id as u32, instrument })
        }
        "vst3" => Some(Ident::Vst3 { class_id: fuid_to_com(id.split('?').next()?)?, instrument }),
        _ => None,
    }
}

/// The database at `path`, as the library takes it; `None` if it can't be read.
pub fn load(path: &Path) -> Option<HostDb> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    let mut stmt = conn
        .prepare(
            "SELECT p.dev_identifier, p.name, p.vendor, m.path FROM plugins p \
             LEFT JOIN plugin_modules m ON m.module_id = p.module_id",
        )
        .ok()?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        })
        .ok()?;
    let mut db = HostDb {
        path: path.to_path_buf(),
        plugins: Vec::new(),
        vst2: Vec::new(),
        compat: HashMap::new(),
        unscanned: Vec::new(),
        files: HashSet::new(),
    };
    for (ident, name, vendor, file) in rows.flatten() {
        if !file.is_empty() {
            db.files.insert(file.trim_end_matches(['/', '\\']).to_string());
        }
        match parse_ident(&ident) {
            Some(Ident::Vst2 { id, .. }) => {
                if !db.vst2.iter().any(|(i, _)| *i == id) {
                    db.vst2.push((id, file));
                }
            }
            Some(Ident::Vst3 { class_id, instrument }) => {
                if db.plugins.iter().any(|k| k.class_id == class_id) {
                    continue;
                }
                let bundle = file.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default().to_string();
                db.plugins.push(Entry {
                    fourcc: None,
                    class_id,
                    vst3_file: leak(bundle),
                    instrument,
                    vendor: leak(vendor),
                    name: leak(name),
                    path: (!file.is_empty()).then(|| leak(file)),
                });
            }
            None => {}
        }
    }
    db.plugins.sort_by_key(|k| (k.vendor.to_lowercase(), k.name.to_lowercase()));
    Some(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        assert_eq!(parse_ident("device:vst:audiofx:1483101268?n=OTT"), Some(Ident::Vst2 { id: 1483101268, instrument: false }));
        assert_eq!(
            parse_ident("device:vst3:instr:56535458-6673-5873-6572-756d00000000"),
            Some(Ident::Vst3 { class_id: *b"XTSVsfsXerum\0\0\0\0", instrument: true })
        );
        assert_eq!(parse_ident("device:au:instr:aumu,Srum,XfeR"), None);
    }

    #[test]
    fn reads_a_live_database() {
        let path = std::env::temp_dir().join(format!("blacksmith-live-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE plugin_modules (module_id INTEGER PRIMARY KEY, path TEXT);
                 CREATE TABLE plugins (plugin_id INTEGER PRIMARY KEY, module_id INTEGER, dev_identifier TEXT, name TEXT, vendor TEXT);
                 INSERT INTO plugin_modules VALUES (1, '/Library/Audio/Plug-Ins/VST3/Kontakt 8.vst3'), (2, '/x/OTT.vst');
                 INSERT INTO plugins VALUES (1, 1, 'device:vst3:instr:5653544e-694b-386b-6f6e-74616b742038', 'Kontakt 8', 'Native Instruments'),
                   (2, 1, 'device:vst3:instr:5653544e-694b-386b-6f6e-74616b742038', 'Kontakt 8', 'Native Instruments'),
                   (3, 2, 'device:vst:audiofx:1483101268?n=OTT', 'OTT', 'Xfer Records');",
            )
            .unwrap();
        }
        let db = load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(db.plugins.len(), 1);
        assert_eq!(db.plugins[0].vst3_file, "Kontakt 8.vst3");
        assert_eq!(&db.plugins[0].class_id[..4], b"NTSV");
        assert_eq!(db.vst2, [(1483101268, "/x/OTT.vst".to_string())]);
        assert!(db.files.contains("/Library/Audio/Plug-Ins/VST3/Kontakt 8.vst3"));
    }
}
