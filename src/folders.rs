//! The standard plugin folders on this computer: which VST2 and VST3 files
//! exist, and the VST3 class IDs a bundle states in its `moduleinfo.json`
//! (VST3 SDK 3.7.5+; many plugins don't ship one).
//!
//! Only the file system and that text file are read — no plugin is loaded.
//! A VST2's unique ID can't be known without loading it, so VST2 files only
//! tell that something is installed under that name. This complements a
//! host's own database (`fldb.rs` for FL; Live's later), which has the IDs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Vst2,
    Vst3,
}

/// One plugin file or bundle found in a plugin folder.
#[derive(Debug, Clone)]
pub struct Found {
    pub kind: Kind,
    pub path: PathBuf,
    /// The file name without extension.
    pub name: String,
    /// From `moduleinfo.json`, when the bundle has one.
    pub info: Option<ModuleInfo>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleInfo {
    pub vendor: String,
    /// Audio module classes: (class ID as FL stores it, name, instrument?).
    pub classes: Vec<([u8; 16], String, bool)>,
    /// Older class IDs this bundle answers to → the class that replaces them.
    pub compat: Vec<([u8; 16], [u8; 16])>,
}

/// The standard plugin folders of this OS that exist. `BLACKSMITH_PLUGIN_DIRS`
/// (a path list) replaces them (debug aid).
pub fn standard_folders() -> Vec<PathBuf> {
    if let Some(v) = std::env::var_os("BLACKSMITH_PLUGIN_DIRS") {
        return std::env::split_paths(&v).filter(|p| p.is_dir()).collect();
    }
    let mut out: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        for base in [Some(PathBuf::from("/")), home].into_iter().flatten() {
            for kind in ["VST3", "VST"] {
                out.push(base.join("Library/Audio/Plug-Ins").join(kind));
            }
        }
    } else if cfg!(windows) {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        if let Some(common) = var("CommonProgramFiles") {
            out.push(common.join("VST3"));
            out.push(common.join("VST2"));
            out.push(common.join("Steinberg").join("VST2"));
        }
        if let Some(local) = var("LOCALAPPDATA") {
            out.push(local.join("Programs").join("Common").join("VST3"));
        }
        if let Some(pf) = var("ProgramFiles") {
            out.push(pf.join("VSTPlugins"));
            out.push(pf.join("Steinberg").join("VSTPlugins"));
        }
    } else {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        out.extend([PathBuf::from("/usr/lib/vst3"), PathBuf::from("/usr/local/lib/vst3")]);
        out.extend(home.iter().flat_map(|h| [h.join(".vst3"), h.join(".vst")]));
    }
    out.into_iter().filter(|p| p.is_dir()).collect()
}

/// Every plugin in `folders` (searched a few levels deep, bundles not entered).
pub fn scan(folders: &[PathBuf]) -> Vec<Found> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Found>) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let kind = match ext.as_str() {
                "vst3" => Some(Kind::Vst3),
                "vst" => Some(Kind::Vst2),
                "dll" if cfg!(windows) => Some(Kind::Vst2),
                _ => None,
            };
            match kind {
                Some(kind) => {
                    let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let info = (kind == Kind::Vst3).then(|| module_info(&p)).flatten();
                    out.push(Found { kind, path: p, name, info });
                }
                None if p.is_dir() && depth < 6 && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) => {
                    walk(&p, depth + 1, out)
                }
                None => {}
            }
        }
    }
    let mut out = Vec::new();
    for f in folders {
        walk(f, 0, &mut out);
    }
    out
}

/// A bundle's `moduleinfo.json` (in `Contents/Resources/`, or `Contents/` in
/// some older builds), if it has one that reads.
fn module_info(bundle: &Path) -> Option<ModuleInfo> {
    let file = ["Contents/Resources/moduleinfo.json", "Contents/moduleinfo.json"]
        .iter()
        .map(|r| bundle.join(r))
        .find(|p| p.is_file())?;
    parse_module_info(&std::fs::read_to_string(file).ok()?)
}

pub fn parse_module_info(text: &str) -> Option<ModuleInfo> {
    let json = Json::parse(text)?;
    let vendor = json.get("Factory Info").and_then(|f| f.get("Vendor")).and_then(Json::str).unwrap_or_default().to_string();
    let mut info = ModuleInfo { vendor, ..Default::default() };
    let mut compat_classes = Vec::new();
    for c in json.get("Classes").and_then(Json::array).unwrap_or_default() {
        let Some(cid) = c.get("CID").and_then(Json::str).and_then(cid_to_com) else { continue };
        match c.get("Category").and_then(Json::str) {
            Some("Audio Module Class") => {
                let subs = c.get("Sub Categories").and_then(Json::array).unwrap_or_default();
                let instrument = subs.iter().any(|s| s.str().is_some_and(|s| s.eq_ignore_ascii_case("Instrument")));
                let name = c.get("Name").and_then(Json::str).unwrap_or_default().to_string();
                info.classes.push((cid, name, instrument));
            }
            // JUCE's twin ID (`ABCDEF01-C0DE-F00D-…`): the same plugin, as FL's `ps_compatibility`.
            Some("Plugin Compatibility Class") => compat_classes.push(cid),
            _ => {}
        }
    }
    if let [(main, _, _)] = info.classes[..] {
        info.compat.extend(compat_classes.into_iter().map(|old| (old, main)));
    }
    for entry in json.get("Compatibility").and_then(Json::array).unwrap_or_default() {
        let Some(new) = entry.get("New").and_then(Json::str).and_then(cid_to_com) else { continue };
        for old in entry.get("Old").and_then(Json::array).unwrap_or_default() {
            if let Some(old) = old.str().and_then(cid_to_com) {
                info.compat.push((old, new));
            }
        }
    }
    (!info.classes.is_empty()).then_some(info)
}

/// A class ID as written in `moduleinfo.json` (32 hex digits, FUID order) →
/// as FL stores it (COM order).
fn cid_to_com(hex: &str) -> Option<[u8; 16]> {
    let hex: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 {
        return None;
    }
    let mut b = [0u8; 16];
    for (i, x) in b.iter_mut().enumerate() {
        *x = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    b[..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    Some(b)
}

// ------------------------------------------------------------------ JSON5-ish

/// Just enough JSON for `moduleinfo.json`, which allows trailing commas and
/// comments (JSON5), and GitHub's release info (`update.rs`). Numbers are kept
/// as text.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn parse(text: &str) -> Option<Json> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        let v = p.value(0)?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        loop {
            while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
                self.i += 1;
            }
            match (self.s.get(self.i), self.s.get(self.i + 1)) {
                (Some(b'/'), Some(b'/')) => {
                    while self.s.get(self.i).is_some_and(|c| *c != b'\n') {
                        self.i += 1;
                    }
                }
                (Some(b'/'), Some(b'*')) => {
                    self.i += 2;
                    while self.i < self.s.len() && !(self.s[self.i] == b'*' && self.s.get(self.i + 1) == Some(&b'/')) {
                        self.i += 1;
                    }
                    self.i += 2;
                }
                _ => return,
            }
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        let hit = self.s.get(self.i) == Some(&c);
        if hit {
            self.i += 1;
        }
        hit
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > 64 {
            return None;
        }
        self.ws();
        match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut m = BTreeMap::new();
                loop {
                    if self.eat(b'}') {
                        return Some(Json::Obj(m));
                    }
                    self.ws();
                    let k = self.string()?;
                    if !self.eat(b':') {
                        return None;
                    }
                    m.insert(k, self.value(depth + 1)?);
                    if !self.eat(b',') {
                        return self.eat(b'}').then_some(Json::Obj(m));
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                loop {
                    if self.eat(b']') {
                        return Some(Json::Arr(a));
                    }
                    a.push(self.value(depth + 1)?);
                    if !self.eat(b',') {
                        return self.eat(b']').then_some(Json::Arr(a));
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            _ => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric() || b"+-.".contains(c)) {
                    self.i += 1;
                }
                match &self.s[start..self.i] {
                    b"true" => Some(Json::Bool(true)),
                    b"false" => Some(Json::Bool(false)),
                    b"null" => Some(Json::Null),
                    b"" => None,
                    n => Some(Json::Num(String::from_utf8_lossy(n).into_owned())),
                }
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match *self.s.get(self.i)? {
                b'"' => {
                    self.i += 1;
                    return String::from_utf8(out).ok();
                }
                b'\\' => {
                    let c = *self.s.get(self.i + 1)?;
                    self.i += 2;
                    match c {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' | b'f' => {}
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?).unwrap_or('\u{fffd}');
                            self.i += 4;
                            out.extend(ch.to_string().as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHROMA: &str = r#"{
      "Name": "Chroma",
      "Factory Info": { "Vendor": "Xynth Audio", "Flags": { "Unicode": true, }, },
      "Classes": [
        { "CID": "ABCDEF019182FAEB586E746843726D61", "Category": "Audio Module Class", "Name": "Chroma",
          "Sub Categories": [ "Fx", "EQ", ], "Cardinality": 2147483647, },
        { "CID": "ABCDEF011234ABCD586E746843726D61", "Category": "Component Controller Class", "Name": "Chroma", },
        { "CID": "ABCDEF01C0DEF00D586E746843726D61", "Category": "Plugin Compatibility Class", "Name": "Chroma", },
      ],
      // a comment
      "Compatibility": [ { "New": "ABCDEF019182FAEB586E746843726D61", "Old": [ "56535458667358736572756D00000000" ] } ],
    }"#;

    #[test]
    fn reads_moduleinfo_with_trailing_commas() {
        let info = parse_module_info(CHROMA).unwrap();
        assert_eq!(info.vendor, "Xynth Audio");
        assert_eq!(info.classes.len(), 1);
        let (cid, name, instrument) = &info.classes[0];
        assert_eq!(name, "Chroma");
        assert!(!instrument);
        // FUID ABCDEF01-9182-FAEB-… in FL's (COM) order.
        assert_eq!(cid[..8], [0x01, 0xef, 0xcd, 0xab, 0x82, 0x91, 0xeb, 0xfa]);
        assert_eq!(info.compat.len(), 2);
        assert!(info.compat.iter().all(|(_, new)| new == cid));
        assert_eq!(info.compat[1].0, *b"XTSVsfsXerum\0\0\0\0");
    }

    #[test]
    fn rejects_broken_json() {
        assert!(parse_module_info("{ \"Classes\": [ ").is_none());
        assert!(parse_module_info("not json").is_none());
        assert!(Json::parse(r#"{"a": "é", "b": [1, -2.5e3, null, false]}"#).is_some());
    }

    #[test]
    fn scans_a_folder_without_entering_bundles() {
        let dir = std::env::temp_dir().join(format!("blacksmith-folders-{}", std::process::id()));
        let chroma = dir.join("VST3/Xynth/Chroma.vst3/Contents/Resources");
        std::fs::create_dir_all(&chroma).unwrap();
        std::fs::write(chroma.join("moduleinfo.json"), CHROMA).unwrap();
        std::fs::create_dir_all(dir.join("VST3/Kontakt 8.vst3/Contents/x.vst3")).unwrap();
        std::fs::create_dir_all(dir.join("VST/Serum.vst/Contents")).unwrap();
        let found = scan(&[dir.join("VST3"), dir.join("VST")]);
        std::fs::remove_dir_all(&dir).unwrap();
        let names: Vec<(&str, Kind, bool)> = found.iter().map(|f| (f.name.as_str(), f.kind, f.info.is_some())).collect();
        assert_eq!(names, [("Kontakt 8", Kind::Vst3, false), ("Chroma", Kind::Vst3, true), ("Serum", Kind::Vst2, false)]);
    }
}
