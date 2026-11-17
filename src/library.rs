//! The plugin library: what a host can load on this computer, and the VST2
//! each VST3 replaces. One `Library` per host (FL Studio, Ableton Live),
//! each merged from the same kinds of source:
//!
//! 1. the host's own database (`fldb.rs`, `livedb.rs` → `HostDb`). It has the
//!    IDs and is what the host can load, so when present it decides what
//!    counts as installed; entries whose file is gone are dropped.
//! 2. the standard plugin folders (`folders.rs`): what's on disk, class IDs from
//!    `moduleinfo.json`. With a host database it fills IDs the host didn't
//!    record (only for files the host lists) and reports plugins the host
//!    hasn't scanned; without one it is the list, plus built-in rows whose
//!    bundle is on disk.
//! 3. the built-in table `assets/known_ids.tsv` (`tools/plugindb.py --tsv`,
//!    from the owner's FL): VST2 → VST3 pairs for converting when nothing on
//!    this computer pairs a FourCC. Not evidence that anything is installed.
//!
//! How a plugin's state converts is not verified here (`plugins.rs` rules are).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;

use crate::{arch, fldb, folders, livedb};

const TABLE: &str = include_str!("../assets/known_ids.tsv");

/// A plugin a host can load (a VST3, with the VST2 it replaces if known).
pub struct Entry {
    /// VST2 unique ID (big-endian integer, as in `plugins.rs`); `None` for a
    /// VST3 without a VST2.
    pub fourcc: Option<u32>,
    /// VST3 class ID as FL stores it (COM byte order).
    pub class_id: [u8; 16],
    /// File name of the .vst3 bundle.
    pub vst3_file: &'static str,
    pub instrument: bool,
    pub vendor: &'static str,
    pub name: &'static str,
    /// The bundle's full path on this computer, when known.
    pub path: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Host {
    Fl,
    Live,
}

impl Host {
    pub fn name(self) -> &'static str {
        match self {
            Host::Fl => "FL Studio",
            Host::Live => "Ableton Live",
        }
    }
}

/// A host's own plugin database, as read by `fldb.rs` / `livedb.rs`.
pub struct HostDb {
    pub path: PathBuf,
    /// One entry per VST3 (with the VST2 it replaces, if the host pairs them).
    pub plugins: Vec<Entry>,
    /// Installed VST2 plugins: (unique ID, file path).
    pub vst2: Vec<(u32, String)>,
    /// Other class IDs a VST3 answers to → its own.
    pub compat: HashMap<[u8; 16], [u8; 16]>,
    /// VST3s listed without a class ID: (plugin key of file or name, file path).
    /// The host finds these by file.
    pub unscanned: Vec<(String, String)>,
    /// Every plugin file path the host lists (any format).
    pub files: HashSet<String>,
}

/// Where a library's "installed" comes from.
pub enum Source {
    /// The host's plugin database at this path.
    Host(PathBuf),
    /// No host database: the plugin folders (and built-in rows found there).
    Folders,
}

/// What the sources found, for the 已安装插件 page.
#[derive(Default)]
pub struct Scan {
    pub folders: Vec<PathBuf>,
    /// Plugin files and bundles in those folders.
    pub on_disk: usize,
    /// Host database entries whose file is gone.
    pub stale: usize,
    /// Plugins on disk the host database doesn't list (the host hasn't scanned them).
    pub unregistered: Vec<String>,
    /// Class IDs read from `moduleinfo.json` that the host database lacked.
    pub ids_from_files: usize,
}

static BUILTIN: LazyLock<Vec<Entry>> = LazyLock::new(|| TABLE.lines().filter_map(parse).collect());

pub struct Library {
    plugins: Vec<Entry>,
    source: Source,
    /// Installed VST2s: ID → its files; `None` when only the built-in table is known.
    vst2: Option<HashMap<u32, Vec<String>>>,
    compat: HashMap<[u8; 16], [u8; 16]>,
    /// VST3s installed without a known class ID (found by file): plugin key → file.
    unscanned: HashMap<String, String>,
    /// Plugin files without arm64 (`arch.rs`; Apple Silicon Macs only).
    intel: HashSet<String>,
    scan: Scan,
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Merges the sources (see the module notes).
fn build(db: Option<HostDb>, folder_list: Vec<PathBuf>, found: &[folders::Found]) -> Library {
    let exists = |p: &str| std::path::Path::new(p).exists();
    let mut scan = Scan { on_disk: found.len(), folders: folder_list, ..Scan::default() };
    let mut lib = match db {
        Some(db) => {
            let listed_plugins = db.plugins.len();
            let plugins: Vec<Entry> = db.plugins.into_iter().filter(|k| k.path.is_none_or(exists)).collect();
            let gone_vst2 = db.vst2.iter().filter(|(_, p)| !exists(p)).count();
            let mut vst2: HashMap<u32, Vec<String>> = HashMap::new();
            for (id, path) in db.vst2.into_iter().filter(|(_, p)| exists(p)) {
                vst2.entry(id).or_default().push(path);
            }
            scan.stale = listed_plugins - plugins.len() + gone_vst2;
            let unscanned = db.unscanned.into_iter().filter(|(_, p)| exists(p)).collect();
            // On disk but not in the host's list: it hasn't scanned them yet.
            let norm = |p: &str| p.trim_end_matches(['/', '\\']).to_lowercase();
            let listed: HashSet<String> = db.files.iter().map(|p| norm(p)).collect();
            scan.unregistered = found
                .iter()
                .filter(|f| !listed.contains(&norm(&f.path.to_string_lossy())))
                .map(|f| file_name(&f.path))
                .collect();
            let mut lib =
                Library { vst2: Some(vst2), compat: db.compat, unscanned, ..Library::empty(Source::Host(db.path)) };
            lib.plugins = plugins;
            // IDs the host didn't record, from moduleinfo.json.
            for f in found {
                if let Some(info) = &f.info {
                    scan.ids_from_files += add_module(&mut lib, f, info, false);
                }
            }
            lib
        }
        None => {
            let mut lib = Library::empty(Source::Folders);
            let bundles: HashSet<String> = found.iter().map(|f| file_name(&f.path).to_lowercase()).collect();
            for f in found {
                match &f.info {
                    Some(info) => scan.ids_from_files += add_module(&mut lib, f, info, true),
                    None if f.kind == folders::Kind::Vst3 => {
                        let path = f.path.to_string_lossy().into_owned();
                        lib.unscanned.insert(fldb::bundle_key(&path), path);
                    }
                    None => {}
                }
            }
            // Built-in rows whose bundle is on disk count as installed here.
            for k in TABLE.lines().filter_map(parse) {
                if bundles.contains(&k.vst3_file.to_lowercase()) && !lib.plugins.iter().any(|p| p.class_id == k.class_id) {
                    lib.plugins.push(k);
                }
            }
            lib
        }
    };
    lib.plugins.sort_by_key(|k| (k.vendor.to_lowercase(), k.name.to_lowercase()));
    if arch::applies() {
        let files = lib.plugins.iter().filter_map(|k| k.path.map(str::to_string));
        let files = files.chain(lib.unscanned.values().cloned()).chain(lib.vst2.iter().flat_map(|m| m.values().flatten().cloned()));
        lib.intel = files.filter(|f| arch::intel_only(std::path::Path::new(f))).collect();
    }
    lib.scan = scan;
    lib
}

/// Adds the classes a `moduleinfo.json` states; returns how many were new.
fn add_module(lib: &mut Library, f: &folders::Found, info: &folders::ModuleInfo, always: bool) -> usize {
    let mut added = 0;
    let bundle = file_name(&f.path);
    // With a host database, a bundle only adds IDs when the host lists its file
    // (its scan just didn't record them): the host must know the plugin.
    let listed = always || lib.unscanned.contains_key(&fldb::bundle_key(&f.path.to_string_lossy()));
    for (cid, name, instrument) in &info.classes {
        if !listed || lib.plugins.iter().any(|p| p.class_id == *cid) {
            continue;
        }
        lib.plugins.push(Entry {
            fourcc: None,
            class_id: *cid,
            vst3_file: leak(bundle.clone()),
            instrument: *instrument,
            vendor: leak(info.vendor.clone()),
            name: leak(if name.is_empty() { f.name.clone() } else { name.clone() }),
            path: Some(leak(f.path.to_string_lossy().into_owned())),
        });
        added += 1;
    }
    for (old, new) in &info.compat {
        if listed || lib.plugins.iter().any(|p| p.class_id == *new) {
            lib.compat.entry(*old).or_insert(*new);
        }
    }
    added
}

impl Library {
    fn empty(source: Source) -> Self {
        Library {
            plugins: Vec::new(),
            source,
            vst2: None,
            compat: HashMap::new(),
            unscanned: HashMap::new(),
            intel: HashSet::new(),
            scan: Scan::default(),
        }
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn scan(&self) -> &Scan {
        &self.scan
    }

    /// Every plugin in the library, sorted by vendor, then name.
    pub fn all(&self) -> &[Entry] {
        &self.plugins
    }

    pub fn by_class_id(&self, id: &[u8]) -> Option<&Entry> {
        self.plugins.iter().find(|k| k.class_id[..] == *id)
    }

    /// Whether the host has a VST3 that answers to this class ID (its own, or
    /// one it lists as compatible).
    pub fn is_installed(&self, class_id: &[u8]) -> bool {
        self.by_class_id(class_id).is_some() || <[u8; 16]>::try_from(class_id).is_ok_and(|id| self.compat.contains_key(&id))
    }

    /// Whether the host lists a VST3 by this bundle path or name without having
    /// recorded its class ID (it then finds it by file).
    pub fn is_unscanned_installed(&self, path: &str, name: &str, vendor: &str) -> bool {
        self.unscanned_file(path, name, vendor).is_some()
    }

    fn unscanned_file(&self, path: &str, name: &str, vendor: &str) -> Option<&String> {
        [fldb::bundle_key(path), plugin_key(name, vendor), plugin_key(name, "")]
            .iter()
            .filter(|k| !k.is_empty())
            .find_map(|k| self.unscanned.get(k))
    }

    /// Whether the host has this VST2; `None` without its database.
    pub fn vst2_installed(&self, fourcc: u32) -> Option<bool> {
        self.vst2.as_ref().map(|s| s.contains_key(&fourcc))
    }

    /// Whether this installed plugin only runs on Intel (needs Rosetta on
    /// Apple Silicon). Always `false` elsewhere.
    pub fn intel_only(&self, k: &Entry) -> bool {
        k.path.is_some_and(|p| self.intel.contains(p))
    }

    /// Whether the VST3 the host loads for this instance only runs on Intel.
    pub fn vst3_intel_only(&self, class_id: &[u8], path: &str, name: &str, vendor: &str) -> bool {
        let own = <[u8; 16]>::try_from(class_id).ok().and_then(|id| self.compat.get(&id)).map_or(class_id, |id| &id[..]);
        match self.by_class_id(own) {
            Some(k) => self.intel_only(k),
            None => self.unscanned_file(path, name, vendor).is_some_and(|f| self.intel.contains(f)),
        }
    }

    /// Whether every file of this installed VST2 only runs on Intel.
    pub fn vst2_intel_only(&self, fourcc: u32) -> bool {
        let files = self.vst2.as_ref().and_then(|m| m.get(&fourcc));
        files.is_some_and(|f| !f.is_empty() && f.iter().all(|f| self.intel.contains(f)))
    }

    /// How many installed plugin files only run on Intel.
    pub fn intel_count(&self) -> usize {
        self.intel.len()
    }

    /// The VST3 that replaces a VST2: this computer's, else the built-in table's.
    pub fn find(&self, fourcc: u32) -> Option<&Entry> {
        self.plugins.iter().chain(BUILTIN.iter()).find(|k| k.fourcc == Some(fourcc))
    }

    /// The library's distinct class IDs for a plugin of this name and vendor;
    /// an empty vendor (Live sets don't store one) matches by name alone.
    pub fn matching(&self, name: &str, vendor: &str) -> Vec<&Entry> {
        let key = plugin_key(name, vendor);
        let v = plugin_key(vendor, "");
        let mut out: Vec<&Entry> = self
            .plugins
            .iter()
            .filter(|k| {
                if v.is_empty() {
                    !key.is_empty() && (plugin_key(k.name, k.vendor) == key || plugin_key(k.name, "") == key)
                } else {
                    plugin_key(k.vendor, "") == v && plugin_key(k.name, k.vendor) == key
                }
            })
            .collect();
        out.dedup_by(|a, b| a.class_id == b.class_id);
        out
    }
}

fn load(host: Host, folder_list: &[PathBuf], found: &[folders::Found]) -> Library {
    // Tests use the built-in table, whatever this machine has installed.
    if cfg!(test) {
        let mut lib = Library::empty(Source::Folders);
        lib.plugins = TABLE.lines().filter_map(parse).collect();
        return lib;
    }
    let db = match host {
        Host::Fl => fldb::locate().and_then(|d| fldb::load(&d)),
        Host::Live => livedb::locate().and_then(|d| livedb::load(&d)),
    };
    build(db, folder_list.to_vec(), found)
}

/// The plugin folders, scanned once for both hosts.
static FOUND: LazyLock<(Vec<PathBuf>, Vec<folders::Found>)> = LazyLock::new(|| {
    if cfg!(test) {
        return (Vec::new(), Vec::new());
    }
    let list = folders::standard_folders();
    let found = folders::scan(&list);
    (list, found)
});

static FL: LazyLock<Library> = LazyLock::new(|| load(Host::Fl, &FOUND.0, &FOUND.1));
static LIVE: LazyLock<Library> = LazyLock::new(|| load(Host::Live, &FOUND.0, &FOUND.1));

/// The library of a host.
pub fn get(host: Host) -> &'static Library {
    match host {
        Host::Fl => &FL,
        Host::Live => &LIVE,
    }
}

/// Reads both libraries now (they're otherwise read on first use).
pub fn warm() {
    LazyLock::force(&FL);
    LazyLock::force(&LIVE);
}

/// Lower-case letters and digits only, without the vendor's name in front and
/// a "x64" build suffix: "FabFilter Pro-Q 3" → "proq3", "soothe2_x64" → "soothe2".
pub fn plugin_key(name: &str, vendor: &str) -> String {
    let squash = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    let (n, v) = (squash(name), squash(vendor));
    let n = n.strip_prefix(&v).filter(|r| !r.is_empty()).unwrap_or(&n);
    n.strip_suffix("x64").filter(|r| !r.is_empty()).unwrap_or(n).to_string()
}

/// One row: `fourcc  class_id  kind  vst3_file  vendor  name`, tab-separated.
/// Comments, the header and malformed rows yield `None`.
fn parse(line: &'static str) -> Option<Entry> {
    let mut f = line.split('\t');
    let fourcc = match f.next()? {
        "-" => None,
        hex => Some(u32::from_str_radix(hex, 16).ok()?),
    };
    let hex = f.next()?;
    if hex.len() != 32 {
        return None;
    }
    let mut class_id = [0u8; 16];
    for (i, b) in class_id.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    let instrument = f.next()? == "instrument";
    let vst3_file = f.next()?;
    let vendor = f.next()?;
    let name = f.next()?;
    Some(Entry { fourcc, class_id, vst3_file, instrument, vendor, name, path: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_parses() {
        let rows = TABLE.lines().filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("fourcc\t"));
        for line in rows {
            assert!(parse(line).is_some(), "bad row in assets/known_ids.tsv: {line:?}");
        }
        assert_eq!(BUILTIN.len(), TABLE.lines().filter(|l| parse(l).is_some()).count());
    }

    #[test]
    fn finds_by_fourcc() {
        let k = get(Host::Fl).find(u32::from_be_bytes(*b"XfsX")).unwrap();
        assert_eq!(&k.class_id, b"XTSVsfsXerum\0\0\0\0");
        assert_eq!(k.vst3_file, "Serum.vst3");
    }

    #[test]
    fn sources_merge_host_first() {
        let dir = std::env::temp_dir().join(format!("blacksmith-lib-{}", std::process::id()));
        let here = |p: &str| {
            let p = dir.join(p);
            std::fs::create_dir_all(&p).unwrap();
            p.to_string_lossy().into_owned()
        };
        let (serum, kontakt, extra) = (here("VST3/Serum.vst3"), here("VST3/Kontakt 8.vst3"), here("VST3/New.vst3"));
        let gone = dir.join("VST3/Gone.vst3").to_string_lossy().into_owned();
        let k = |name: &'static str, id: u8, path: &str| Entry {
            fourcc: None,
            class_id: [id; 16],
            vst3_file: "x.vst3",
            instrument: false,
            vendor: "V",
            name,
            path: Some(leak(path.to_string())),
        };
        let db = HostDb {
            path: dir.clone(),
            plugins: vec![k("Serum", 1, &serum), k("Gone", 2, &gone)],
            vst2: vec![(7, serum.clone()), (8, gone.clone())],
            compat: HashMap::new(),
            unscanned: vec![("kontakt8".into(), kontakt.clone())],
            files: [serum.clone(), gone.clone(), kontakt.clone()].into_iter().collect(),
        };
        let info = folders::ModuleInfo { vendor: "NI".into(), classes: vec![([3; 16], "Kontakt 8".into(), true)], compat: vec![] };
        let found = vec![
            folders::Found { kind: folders::Kind::Vst3, path: serum.into(), name: "Serum".into(), info: None },
            folders::Found { kind: folders::Kind::Vst3, path: kontakt.into(), name: "Kontakt 8".into(), info: Some(info.clone()) },
            folders::Found { kind: folders::Kind::Vst3, path: extra.into(), name: "New".into(), info: Some(folders::ModuleInfo {
                classes: vec![([4; 16], "New".into(), false)], ..info }) },
        ];
        let lib = build(Some(db), vec![], &found);
        std::fs::remove_dir_all(&dir).unwrap();
        let ids: Vec<u8> = lib.plugins.iter().map(|p| p.class_id[0]).collect();
        // Serum stays, Gone is dropped, Kontakt gains its ID from moduleinfo,
        // New isn't in FL's list so it's reported, not added.
        assert_eq!(ids, [3, 1]); // sorted by vendor: NI, V
        assert_eq!(lib.vst2.as_ref().map(|m| m.keys().copied().collect::<Vec<_>>()), Some(vec![7]));
        assert_eq!(lib.scan.stale, 2);
        assert_eq!(lib.scan.unregistered, ["New.vst3"]);
        assert_eq!(lib.scan.ids_from_files, 1);
    }

    #[test]
    fn plugin_keys_ignore_vendor_prefix_and_build_suffix() {
        assert_eq!(plugin_key("FabFilter Pro-Q 3", "FabFilter"), "proq3");
        assert_eq!(plugin_key("Pro-Q 3", "FabFilter"), "proq3");
        assert_eq!(plugin_key("soothe2_x64", "oeksound"), "soothe2");
        assert_eq!(get(Host::Fl).matching("OTT", "Xfer Records").len(), 1);
        assert_eq!(get(Host::Live).matching("ott", "").len(), 1);
    }
}
