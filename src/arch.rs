//! Which CPUs a Mac plugin runs on, from its binary's Mach-O header — the
//! file is read, never loaded. On an Apple Silicon Mac a host running natively
//! can't load an Intel-only plugin; it needs Rosetta (FL / Live opened "using
//! Rosetta"). Checked on the owner's Mac (Nov 2026): 96 of 685 plugin bundles
//! had no arm64 (Omnisphere, Massive, Keyscape, Nexus…).

use std::io::Read;
use std::path::{Path, PathBuf};

const CPU_ARM64: u32 = 0x0100_000c;

/// Whether the check means anything here: Blacksmith running natively on
/// Apple Silicon (as the hosts would).
pub fn applies() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// Whether the plugin at this path (a `.vst3` / `.vst` / `.component` bundle)
/// has a binary without arm64. `false` when it can't tell.
pub fn intel_only(plugin: &Path) -> bool {
    let Some(bin) = binary(plugin) else { return false };
    let mut head = Vec::with_capacity(4096);
    let Ok(file) = std::fs::File::open(bin) else { return false };
    if file.take(4096).read_to_end(&mut head).is_err() {
        return false;
    }
    cpu_types(&head).is_some_and(|cpus| !cpus.is_empty() && !cpus.contains(&CPU_ARM64))
}

/// The bundle's executable: `CFBundleExecutable` from `Info.plist` (XML), else
/// the only file in `Contents/MacOS`, else one named like the bundle.
fn binary(plugin: &Path) -> Option<PathBuf> {
    if plugin.is_file() {
        return Some(plugin.to_path_buf());
    }
    let macos = plugin.join("Contents/MacOS");
    if let Some(name) = std::fs::read_to_string(plugin.join("Contents/Info.plist")).ok().and_then(|p| plist_executable(&p)) {
        let f = macos.join(name);
        if f.is_file() {
            return Some(f);
        }
    }
    let files: Vec<PathBuf> = std::fs::read_dir(&macos).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    match &files[..] {
        [one] => Some(one.clone()),
        _ => files.into_iter().find(|f| f.file_stem() == plugin.file_stem()),
    }
}

fn plist_executable(plist: &str) -> Option<String> {
    let rest = &plist[plist.find("<key>CFBundleExecutable</key>")?..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = start + rest[start..].find("</string>")?;
    Some(rest[start..end].trim().to_string())
}

/// The CPU types a Mach-O binary holds (one for a thin binary, several for a
/// universal one); `None` if it isn't Mach-O.
fn cpu_types(head: &[u8]) -> Option<Vec<u32>> {
    let be = |at: usize| Some(u32::from_be_bytes(head.get(at..at + 4)?.try_into().ok()?));
    let le = |at: usize| Some(u32::from_le_bytes(head.get(at..at + 4)?.try_into().ok()?));
    match be(0)? {
        // Universal: big-endian header, then one entry per architecture (20
        // bytes, or 32 in the 64-bit variant). A small count tells it apart
        // from a Java class file, which shares the magic.
        magic @ (0xcafe_babe | 0xcafe_babf) => {
            let n = be(4)? as usize;
            if n == 0 || n > 16 {
                return None;
            }
            let size = if magic == 0xcafe_babe { 20 } else { 32 };
            (0..n).map(|i| be(8 + i * size)).collect()
        }
        // Thin, little-endian (every Intel and Apple Silicon binary).
        0xcffa_edfe | 0xcefa_edfe => Some(vec![le(4)?]),
        // Thin, big-endian: PowerPC.
        0xfeed_face | 0xfeed_facf => Some(vec![be(4)?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const X86_64: u32 = 0x0100_0007;

    fn fat(cpus: &[u32]) -> Vec<u8> {
        let mut h = 0xcafe_babeu32.to_be_bytes().to_vec();
        h.extend((cpus.len() as u32).to_be_bytes());
        for c in cpus {
            h.extend(c.to_be_bytes());
            h.extend([0; 16]);
        }
        h
    }

    #[test]
    fn reads_cpu_types() {
        assert_eq!(cpu_types(&fat(&[X86_64, CPU_ARM64])), Some(vec![X86_64, CPU_ARM64]));
        let mut thin = 0xfeed_facfu32.to_le_bytes().to_vec();
        thin.extend(X86_64.to_le_bytes());
        assert_eq!(cpu_types(&thin), Some(vec![X86_64]));
        // A Java class file: same magic, version numbers where the count is.
        assert_eq!(cpu_types(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52]), None);
        assert_eq!(cpu_types(b"MZ\x90\0"), None);
    }

    #[test]
    fn finds_the_executable_in_a_bundle() {
        let dir = std::env::temp_dir().join(format!("blacksmith-arch-{}", std::process::id()));
        let bundle = dir.join("Old.vst3");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            "<dict><key>CFBundleExecutable</key>\n<string>Old Plugin</string></dict>",
        )
        .unwrap();
        std::fs::write(bundle.join("Contents/MacOS/Old Plugin"), fat(&[X86_64])).unwrap();
        std::fs::write(bundle.join("Contents/MacOS/helper.txt"), "x").unwrap();
        assert!(intel_only(&bundle));
        std::fs::write(bundle.join("Contents/MacOS/Old Plugin"), fat(&[X86_64, CPU_ARM64])).unwrap();
        assert!(!intel_only(&bundle));
        assert!(!intel_only(&dir.join("Missing.vst3")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
