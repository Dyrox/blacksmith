//! FL Studio's own plugin database on this computer: every plugin FL has
//! scanned, so exactly what FL can load here.
//!
//! FL writes one `.nfo` per plugin under `Presets/Plugin database/Installed/`
//! (`key=value` lines, UTF-8 with BOM), listing the plugin's files as
//! `ps_file_<field>_<n>`. A VST3 file carries its class ID (`guid`), a VST2
//! file its unique ID (`magic`, the FourCC as a big-endian integer). When FL
//! finds both formats of one plugin it lists them in the same `.nfo`.
//!
//! Only these text files are read: no plugin folder is scanned and no plugin
//! is loaded (installs are often messy; FL's scan is what counts).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::library::{Entry, HostDb, plugin_key};

/// `ps_file_plugclass` values: VST2 effect / instrument, VST3 effect / instrument.
const VST2_CLASSES: [i32; 2] = [0, 4];
const VST3_CLASSES: [i32; 2] = [7, 8];

/// Where FL keeps the database. `BLACKSMITH_FLDB=<Installed folder>` points
/// elsewhere; `BLACKSMITH_FLDB=none` ignores it (debug aids).
pub fn locate() -> Option<PathBuf> {
    match std::env::var_os("BLACKSMITH_FLDB") {
        Some(v) if v == "none" => return None,
        Some(v) => return Some(PathBuf::from(v)),
        None => {}
    }
    let tail = Path::new("Image-Line/FL Studio/Presets/Plugin database/Installed");
    let mut homes = Vec::new();
    for var in ["HOME", "USERPROFILE"] {
        if let Some(h) = std::env::var_os(var) {
            let h = PathBuf::from(h);
            homes.push(h.join("Documents"));
            // Windows with OneDrive moves Documents.
            homes.push(h.join("OneDrive").join("Documents"));
        }
    }
    homes.into_iter().map(|d| d.join(tail)).find(|p| p.is_dir())
}

/// The database at `dir`; `None` if it can't be read or lists no VST3.
/// The database at `dir`, as the library takes it; `None` if it can't be read
/// or lists no VST3. `ps_compatibility` IDs (JUCE's `ABCDEF01-C0DE-F00D-…` twin
/// of `ABCDEF01-9182-FAEB-…`) become `compat`; VST3s FL lists without a class
/// ID (its scan didn't finish, e.g. Kontakt 8) become `unscanned`.
pub fn load(dir: &Path) -> Option<HostDb> {
    let mut nfos = Vec::new();
    collect_nfos(dir, &mut nfos);
    nfos.sort();
    let lists: Vec<Vec<File>> = nfos
        .iter()
        .filter(|p| !p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("VerifiedIDs.nfo")))
        .filter_map(|p| std::fs::read(p).ok())
        .map(|b| files(&String::from_utf8_lossy(&b)))
        .collect();
    let plugins = entries(&lists);
    if plugins.is_empty() {
        return None;
    }
    let all = || lists.iter().flatten();
    let vst2 = all().filter(|f| f.is_vst2()).filter_map(|f| Some((f.magic?, f.path.clone()))).collect();
    let compat = all().filter(|f| f.is_vst3()).filter_map(|f| Some((f.compat?, f.class_id?))).collect();
    let unscanned = all()
        .filter(|f| f.class_id.is_none() && f.ends_with(&[".vst3"]))
        .flat_map(|f| [(bundle_key(&f.path), f.path.clone()), (plugin_key(&f.name, &f.vendor), f.path.clone())])
        .filter(|(k, _)| !k.is_empty())
        .collect();
    let files = all().map(|f| f.path.trim_end_matches(['/', '\\']).to_string()).filter(|p| !p.is_empty()).collect();
    Some(HostDb { path: dir.to_path_buf(), plugins, vst2, compat, unscanned, files })
}

fn collect_nfos(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for e in read.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_nfos(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("nfo")) {
            out.push(p);
        }
    }
}

#[derive(Clone)]
struct File {
    name: String,
    path: String,
    class: Option<i32>,
    class_id: Option<[u8; 16]>,
    magic: Option<u32>,
    vendor: String,
    compat: Option<[u8; 16]>,
}

impl File {
    fn ends_with(&self, ext: &[&str]) -> bool {
        let p = self.path.trim_end_matches(['/', '\\']).to_ascii_lowercase();
        ext.iter().any(|e| p.ends_with(e))
    }
    fn is_vst3(&self) -> bool {
        self.class_id.is_some() && (self.class.is_some_and(|c| VST3_CLASSES.contains(&c)) || self.ends_with(&[".vst3"]))
    }
    fn is_vst2(&self) -> bool {
        self.magic.is_some() && (self.class.is_some_and(|c| VST2_CLASSES.contains(&c)) || self.ends_with(&[".vst", ".dll"]))
    }
    fn bundle(&self) -> String {
        self.path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default().to_string()
    }
}

/// The files one `.nfo` lists.
fn files(nfo: &str) -> Vec<File> {
    let fields: HashMap<&str, &str> = nfo
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect();
    let count = fields.get("ps_files").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(64);
    (0..count)
        .map(|i| {
            let get = |k: &str| fields.get(format!("ps_file_{k}_{i}").as_str()).copied();
            File {
                name: get("name").or(fields.get("ps_name").copied()).unwrap_or_default().to_string(),
                path: get("filename").unwrap_or_default().to_string(),
                class: get("plugclass").and_then(|c| c.parse().ok()),
                class_id: get("guid").and_then(guid_to_com),
                magic: get("magic").and_then(|m| m.parse().ok()),
                vendor: get("vendorname").unwrap_or_default().to_string(),
                compat: fields.get(format!("ps_compatibility_{i}").as_str()).and_then(|g| guid_to_com(g)),
            }
        })
        .collect()
}

/// The plugin key of the `.vst3` bundle a path names: "…/VST3/Kontakt 8.vst3" and
/// "C:\\…\\Kontakt 8.vst3\\Contents\\x86_64-win\\Kontakt 8.vst3" → "kontakt8".
pub fn bundle_key(path: &str) -> String {
    path.split(['/', '\\'])
        .find(|c| c.to_ascii_lowercase().ends_with(".vst3"))
        .map(|c| plugin_key(&c[..c.len() - 5], ""))
        .unwrap_or_default()
}

/// `{D1-D2-D3-D4-D5}` → the 16 bytes FL stores in record 52 (COM order).
fn guid_to_com(guid: &str) -> Option<[u8; 16]> {
    let hex: String = guid.trim_matches(['{', '}']).chars().filter(|c| *c != '-').collect();
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

/// Strings live as long as the app (the database is read once).
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn entry(fourcc: Option<u32>, three: &File, name: &str, vendor: &str, two_class: Option<i32>) -> Entry {
    Entry {
        fourcc,
        class_id: three.class_id.unwrap_or_default(),
        vst3_file: leak(three.bundle()),
        path: Some(leak(three.path.clone())),
        instrument: three.class == Some(8) || (three.class.is_none() && two_class == Some(4)),
        vendor: leak(vendor.to_string()),
        name: leak(name.to_string()),
    }
}

/// One entry per VST3: with the VST2 it replaces (paired by FL, else the only
/// VST3 of the same name and vendor), or on its own.
fn entries(lists: &[Vec<File>]) -> Vec<Entry> {
    let mut vst2: Vec<File> = Vec::new();
    let mut vst3: Vec<File> = Vec::new();
    let mut paired: HashMap<u32, File> = HashMap::new();
    let (mut seen2, mut seen3) = (HashSet::new(), HashSet::new());
    for files in lists {
        let twos: Vec<&File> = files.iter().filter(|f| f.is_vst2()).collect();
        let threes: Vec<&File> = files.iter().filter(|f| f.is_vst3()).collect();
        for f in &twos {
            if seen2.insert(f.magic) {
                vst2.push((*f).clone());
            }
        }
        for f in &threes {
            if seen3.insert(f.class_id) {
                vst3.push((*f).clone());
            }
        }
        if let (Some(two), Some(three)) = (twos.first(), threes.first()) {
            paired.entry(two.magic.unwrap()).or_insert_with(|| (*three).clone());
        }
    }

    let mut out = Vec::new();
    let mut used = HashSet::new();
    for two in &vst2 {
        let magic = two.magic.unwrap();
        let three = paired.get(&magic).cloned().or_else(|| {
            let key = plugin_key(&two.name, &two.vendor);
            let vendor = plugin_key(&two.vendor, "");
            let m: Vec<&File> = vst3
                .iter()
                .filter(|t| plugin_key(&t.vendor, "") == vendor && plugin_key(&t.name, &t.vendor) == key)
                .collect();
            (m.len() == 1).then(|| m[0].clone())
        });
        if let Some(three) = three {
            used.insert(three.class_id);
            let vendor = if two.vendor.is_empty() { &three.vendor } else { &two.vendor };
            out.push(entry(Some(magic), &three, &two.name, vendor, two.class));
        }
    }
    for three in vst3.iter().filter(|t| !used.contains(&t.class_id)) {
        out.push(entry(None, three, &three.name, &three.vendor, None));
    }
    out.sort_by_key(|k| (k.vendor.to_lowercase(), k.name.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERUM: &str = "\u{feff}fileversion=2\nps_name=Serum\nps_files=2\n\
        ps_file_name_0=Serum\nps_file_filename_0=/Library/Audio/Plug-Ins/VST3/Serum.vst3\nps_file_plugclass_0=8\n\
        ps_file_guid_0={56535458-6673-5873-6572-756D00000000}\nps_file_vendorname_0=Xfer Records\n\
        ps_file_name_1=Serum\nps_file_filename_1=/Library/Audio/Plug-Ins/VST/Serum.vst\nps_file_plugclass_1=4\n\
        ps_file_magic_1=1483109208\nps_file_guid_1={56535458-6673-5873-6572-756D00000000}\nps_file_vendorname_1=Xfer Records\n";
    // FabFilter: VST2 and VST3 in separate .nfo files, the VST3 under a shorter name.
    const PROQ_VST2: &str = "ps_files=1\nps_file_name_0=FabFilter Pro-Q 3\nps_file_filename_0=C:\\VstPlugins\\FabFilter Pro-Q 3.dll\n\
        ps_file_plugclass_0=0\nps_file_magic_0=1179726704\nps_file_guid_0={56535446-5133-7066-6162-66696C746572}\nps_file_vendorname_0=FabFilter\n";
    const PROQ_VST3: &str = "ps_files=1\nps_file_name_0=Pro-Q 3\nps_file_filename_0=C:\\Program Files\\Common Files\\VST3\\FabFilter Pro-Q 3.vst3\n\
        ps_file_plugclass_0=7\nps_file_guid_0={72C4DB71-7A4D-459A-B97E-51745D84B39D}\nps_file_vendorname_0=FabFilter\n";
    const SOOTHE: &str = "ps_files=1\nps_file_name_0=soothe2\nps_file_filename_0=/Library/Audio/Plug-Ins/VST3/soothe2.vst3\n\
        ps_file_plugclass_0=7\nps_file_guid_0={ABCDEF01-9182-FAEB-4F65-6B5353746842}\nps_file_vendorname_0=oeksound\n\
        ps_compatibility_0={ABCDEF01-C0DE-F00D-4F65-6B5353746842}\n";

    #[test]
    fn reads_pairs_name_matches_and_vst3_only() {
        let lists: Vec<Vec<File>> = [SERUM, PROQ_VST2, PROQ_VST3, SOOTHE, SERUM].iter().map(|s| files(s)).collect();
        let e = entries(&lists);
        assert_eq!(e.len(), 3);
        let serum = e.iter().find(|k| k.name == "Serum").unwrap();
        assert_eq!(serum.fourcc, Some(u32::from_be_bytes(*b"XfsX")));
        assert_eq!(&serum.class_id, b"XTSVsfsXerum\0\0\0\0");
        assert!(serum.instrument);
        assert_eq!(serum.vst3_file, "Serum.vst3");
        let proq = e.iter().find(|k| k.vendor == "FabFilter").unwrap();
        assert_eq!(proq.fourcc, Some(u32::from_be_bytes(*b"FQ3p")));
        assert_eq!(proq.class_id[..4], [0x71, 0xdb, 0xc4, 0x72]);
        assert_eq!(proq.vst3_file, "FabFilter Pro-Q 3.vst3");
        assert!(!proq.instrument);
        let soothe = e.iter().find(|k| k.name == "soothe2").unwrap();
        assert_eq!(soothe.fourcc, None);
    }

    #[test]
    fn unscanned_vst3s_are_known_by_file() {
        const KONTAKT: &str = "ps_files=1\nps_file_name_0=Kontakt 8\nps_file_filename_0=/Library/Audio/Plug-Ins/VST3/Kontakt 8.vst3\n\
            ps_file_plugclass_0=7\n";
        let dir = std::env::temp_dir().join(format!("blacksmith-fldb-k-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Kontakt 8.nfo"), KONTAKT).unwrap();
        std::fs::write(dir.join("soothe2.nfo"), SOOTHE).unwrap();
        let db = load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(db.unscanned.iter().any(|(k, _)| k == "kontakt8"));
        assert_eq!(bundle_key("C:\\VST3\\Kontakt 8.vst3\\Contents\\x86_64-win\\Kontakt 8.vst3"), "kontakt8");
        assert_eq!(bundle_key("/Library/Audio/Plug-Ins/VST3/Kontakt 8.vst3"), "kontakt8");
    }

    #[test]
    fn loads_a_folder() {
        let dir = std::env::temp_dir().join(format!("blacksmith-fldb-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Effects/VST3")).unwrap();
        std::fs::write(dir.join("Effects/VST3/soothe2.nfo"), SOOTHE).unwrap();
        std::fs::write(dir.join("VerifiedIDs.nfo"), "VST3:64:{x}=y").unwrap();
        std::fs::write(dir.join("Effects/VST3/Serum.nfo"), SERUM).unwrap();
        let db = load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(db.plugins.len(), 2);
        assert!(db.plugins.iter().any(|k| k.vst3_file == "soothe2.vst3"));
        assert!(db.vst2.iter().any(|(id, _)| *id == u32::from_be_bytes(*b"XfsX")));
        let twin = guid_to_com("{ABCDEF01-C0DE-F00D-4F65-6B5353746842}").unwrap();
        assert_eq!(db.compat.get(&twin), guid_to_com("{ABCDEF01-9182-FAEB-4F65-6B5353746842}").as_ref());
        assert!(load(Path::new("/nonexistent/blacksmith")).is_none());
    }
}
