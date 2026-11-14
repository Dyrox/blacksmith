//! Finding the VST3 version of a VST2 plugin.
//!
//! Steinberg's rule (VST3 SDK FAQ, `convertVST2UID_To_FUID`): a VST3 build
//! that replaces a VST2 plugin either reuses the class ID
//! `"VST" + VST2 unique ID (big-endian) + first 9 bytes of the lower-case name`,
//! or lists that ID under "Compatibility" → "Old" in its `moduleinfo.json`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::convert::Target;

/// FUID byte order → the COM/GUID byte order FL stores.
fn to_com(f: [u8; 16]) -> Vec<u8> {
    let mut c = f.to_vec();
    c[..4].reverse();
    c[4..6].reverse();
    c[6..8].reverse();
    c
}

fn fuid_from_vst2(fourcc: u32, name: &str) -> [u8; 16] {
    let mut f = [0u8; 16];
    f[..3].copy_from_slice(b"VST");
    f[3..7].copy_from_slice(&fourcc.to_be_bytes());
    for (i, b) in name.to_ascii_lowercase().bytes().take(9).enumerate() {
        f[7 + i] = b;
    }
    f
}

/// The class ID (as FL stores it) Steinberg's convention gives this VST2 plugin.
pub fn class_id_from_vst2(fourcc: u32, name: &str) -> Vec<u8> {
    to_com(fuid_from_vst2(fourcc, name))
}

#[cfg(test)]
fn hex(f: &[u8; 16]) -> String {
    f.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 16]> {
    let s = s.trim();
    if s.len() != 32 {
        return None;
    }
    let mut f = [0u8; 16];
    for (i, b) in f.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(f)
}

struct Installed {
    /// Bundle or file name without `.vst3`.
    stem: String,
    /// What FL stores as the plugin path.
    path: PathBuf,
    /// Class IDs this module declares.
    classes: Vec<[u8; 16]>,
    /// (new class, old IDs it replaces) from moduleinfo.json.
    compatibility: Vec<([u8; 16], Vec<[u8; 16]>)>,
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        if let Some(c) = std::env::var_os("CommonProgramFiles") {
            dirs.push(PathBuf::from(c).join("VST3"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(l).join("Programs").join("Common").join("VST3"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push("/Library/Audio/Plug-Ins/VST3".into());
        if let Some(h) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(h).join("Library/Audio/Plug-Ins/VST3"));
        }
    } else {
        dirs.extend(["/usr/lib/vst3", "/usr/local/lib/vst3"].map(PathBuf::from));
        if let Some(h) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(h).join(".vst3"));
        }
    }
    dirs
}

fn installed() -> &'static [Installed] {
    static CACHE: OnceLock<Vec<Installed>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut out = Vec::new();
        for d in search_dirs() {
            walk(&d, 0, &mut out);
        }
        out
    })
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Installed>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let is_vst3 = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3"));
        if is_vst3 {
            if let Some(i) = describe(&p) {
                out.push(i);
            }
        } else if p.is_dir() && depth < 4 {
            walk(&p, depth + 1, out);
        }
    }
}

fn describe(p: &Path) -> Option<Installed> {
    let stem = p.file_stem()?.to_string_lossy().into_owned();
    let mut path = p.to_path_buf();
    let mut info = None;
    if p.is_dir() {
        if cfg!(windows) {
            // FL points at the binary inside the bundle.
            let contents = p.join("Contents");
            if let Some(bin) = ["x86_64-win", "arm64x-win", "arm64ec-win"]
                .iter()
                .map(|a| contents.join(a).join(p.file_name().unwrap()))
                .find(|b| b.is_file())
            {
                path = bin;
            }
        }
        info = ["Contents/Resources/moduleinfo.json", "Contents/moduleinfo.json"]
            .iter()
            .find_map(|m| std::fs::read_to_string(p.join(m)).ok());
    }
    let (classes, compatibility) = info.as_deref().map(parse_moduleinfo).unwrap_or_default();
    Some(Installed { stem, path, classes, compatibility })
}

/// Pulls class IDs and the compatibility table out of a moduleinfo.json
/// (JSON5) by walking its strings; no full parser needed.
fn parse_moduleinfo(json: &str) -> (Vec<[u8; 16]>, Vec<([u8; 16], Vec<[u8; 16]>)>) {
    let mut strings = Vec::new();
    let mut chars = json.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut s = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        chars.next();
                    }
                    '"' => break,
                    _ => s.push(c),
                }
            }
            strings.push(s);
        }
    }
    let (mut classes, mut compat): (Vec<[u8; 16]>, Vec<([u8; 16], Vec<[u8; 16]>)>) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < strings.len() {
        match strings[i].as_str() {
            "CID" => classes.extend(strings.get(i + 1).and_then(|s| unhex(s))),
            "New" => {
                if let Some(n) = strings.get(i + 1).and_then(|s| unhex(s)) {
                    compat.push((n, Vec::new()));
                }
            }
            "Old" => {
                while let Some(o) = strings.get(i + 1).and_then(|s| unhex(s)) {
                    if let Some(last) = compat.last_mut() {
                        last.1.push(o);
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (classes, compat)
}

/// Looks for an installed VST3 that replaces this VST2 plugin.
pub fn find_installed(fourcc: u32, name: &str) -> Option<Target> {
    let wanted = fuid_from_vst2(fourcc, name);
    let target = |i: &Installed, id: [u8; 16]| Target { class_id: to_com(id), path: i.path.to_string_lossy().into_owned(), state: None };
    let all = installed();
    // 1. The module declares it: same class ID, or listed as replacing it.
    for i in all {
        if i.classes.contains(&wanted) {
            return Some(target(i, wanted));
        }
        if let Some((new, _)) = i.compatibility.iter().find(|(_, old)| old.contains(&wanted)) {
            return Some(target(i, *new));
        }
    }
    // 2. No moduleinfo: same name, and assume Steinberg's ID convention.
    all.iter()
        .find(|i| i.classes.is_empty() && i.stem.eq_ignore_ascii_case(name.trim()))
        .map(|i| target(i, wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steinberg_convention_matches_serum() {
        // Serum's VST2 ID is 'XfsX'; FL stores its VST3 class ID as below.
        assert_eq!(class_id_from_vst2(u32::from_be_bytes(*b"XfsX"), "Serum"), b"XTSVsfsXerum\0\0\0\0");
    }

    #[test]
    fn steinberg_forum_example() {
        // VST2 'lmsy' "superchord" → 56535479736D6C737570657263686F72 (SDK output)
        assert_eq!(hex(&fuid_from_vst2(u32::from_be_bytes(*b"ysml"), "superchord")), "56535479736D6C737570657263686F72");
    }

    #[test]
    fn reads_moduleinfo_compatibility() {
        let json = r#"{ "Classes": [ { "CID": "BD58B550F9E5634E9D2EFF39EA0927B1", "Category": "Audio Module Class" } ],
            "Compatibility": [ { "New": "BD58B550F9E5634E9D2EFF39EA0927B1", "Old": [ "56535479736D6C737570657263686F72", ], }, ] }"#;
        let (classes, compat) = parse_moduleinfo(json);
        assert_eq!(classes.len(), 1);
        assert_eq!(compat[0].1, [unhex("56535479736D6C737570657263686F72").unwrap()]);
    }
}
